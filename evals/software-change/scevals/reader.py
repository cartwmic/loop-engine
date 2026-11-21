"""The optional reader step: a model fills a case-declared form for each short text a case collects.

A case with a `[reader]` table (see CASE-FORMAT.md) has a collector that runs in the scoring sandbox and prints
short texts. This module reads each text three times with the case's pinned Inspect model, checks every reading
against the case's form shapes, votes leaf by leaf and writes the voted forms beside the hidden tests, so the
tests grade forms and never wording. Everything here runs on the host; the case's code never does.

The core (shapes, parsing, voting, keys) has no I/O. The model call, the sandbox and the cache sit at the edges.
"""
import hashlib
import json
import logging
import os
import re
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

import anyio
from inspect_ai.model import ChatMessageSystem, ChatMessageUser, GenerateConfig, get_model

from evalcore.target import InfrastructureError

log = logging.getLogger(__name__)

# The reader's settings. They are stamped into settings_sha256, so a change of any of them
# makes saved forms and cached readings stale.
CONTRACT = "case-reader-v1"
READINGS = 3
TEMPERATURE = 0.7  # Inspect drops it for models that refuse it (Haiku 5.5); the stamp still names the intent
MAX_TOKENS = 8192
CALL_TIMEOUT_SECONDS = 120
CALL_RETRIES = 2
CONCURRENT_CALLS = 4
STEP_SECONDS = 600  # the scoring step as a whole; read-texts has no cap
COLLECTOR_SECONDS = 120
FORMS_FILE = "forms.json"


class ReaderInputError(ValueError):
    """A text file, collector output or shapes file that is not in the documented shape."""


class ReaderCallError(InfrastructureError):
    """A model call failed after its retries."""


# ---------------------------------------------------------------- shapes

def check_shapes(shapes):
    """Validate a form-shapes document: {kind: node}; a node is a leaf or an object of nodes."""
    def check_node(node):
        if not isinstance(node, dict) or not node:
            raise ReaderInputError("form shape node must be a nonempty object")
        if set(node) in ({"one_of"}, {"set_of"}):
            values = next(iter(node.values()))
            if (not isinstance(values, list) or not values or not all(isinstance(v, str) for v in values)
                    or len(set(values)) != len(values)):
                raise ReaderInputError("one_of and set_of need a nonempty list of distinct strings")
            return
        for key, child in node.items():
            if not isinstance(key, str) or key in ("one_of", "set_of"):
                raise ReaderInputError("form shape keys must be strings other than one_of and set_of")
            check_node(child)
    if not isinstance(shapes, dict) or not shapes:
        raise ReaderInputError("form shapes must be a nonempty object of kinds")
    for kind, node in shapes.items():
        if not isinstance(kind, str) or not kind:
            raise ReaderInputError("form shape kinds must be nonempty strings")
        check_node(node)
    return shapes


def load_shapes(path):
    try:
        return check_shapes(json.loads(Path(path).read_text()))
    except (OSError, ValueError) as error:
        if isinstance(error, ReaderInputError):
            raise
        raise ReaderInputError("form shapes file is not readable JSON") from None


def _leaf(node):
    return next(iter(node)) if set(node) in ({"one_of"}, {"set_of"}) else None


def normalize_form(node, form):
    """The form if it matches the shape exactly (a set leaf comes back sorted and without repeats), else None."""
    kind = _leaf(node)
    if kind == "one_of":
        return form if isinstance(form, str) and form in node["one_of"] else None
    if kind == "set_of":
        if (isinstance(form, list) and all(isinstance(v, str) for v in form)
                and all(v in node["set_of"] for v in form)):
            return sorted(set(form))
        return None
    if not isinstance(form, dict) or set(form) != set(node):
        return None
    result = {}
    for key, child in node.items():
        value = normalize_form(child, form[key])
        if value is None:
            return None
        result[key] = value
    return result


# ---------------------------------------------------------------- parsing and voting

def parse_reading(reply, node):
    """The form in a reply (the last fenced block, else the last top-level {...}), or None when it is not this shape."""
    candidates = re.findall(r"```(?:json)?\s*(\{.*?\})\s*```", reply, re.S)
    if not candidates:
        decoder, position, found = json.JSONDecoder(), 0, []
        while (position := reply.find("{", position)) != -1:
            try:
                value, end = decoder.raw_decode(reply, position)
            except ValueError:
                position += 1
                continue
            found.append(json.dumps(value))
            position = end
        candidates = found
    for candidate in reversed(candidates):
        try:
            form = json.loads(candidate)
        except ValueError:
            continue
        return normalize_form(node, form)
    return None


def vote(node, forms):
    """Vote leaf by leaf over the parsed forms. Returns (form or None, split paths).

    A `one_of` leaf needs a strict majority. A `set_of` leaf votes per element (an element needs strictly more than
    half; a tie has no majority). Any leaf with no majority makes the whole form None. `split` lists every leaf
    the readings did not all agree on: a one_of leaf by its dotted path, a set_of element as `<path>.<element>`.
    """
    total = len(forms)
    split = []

    def walk(shape, values, path):
        kind = _leaf(shape)
        if kind == "one_of":
            counts = {}
            for value in values:
                counts[value] = counts.get(value, 0) + 1
            winner, votes = max(counts.items(), key=lambda item: item[1])
            if len(counts) > 1:
                split.append(path)
            return winner if 2 * votes > total else None
        if kind == "set_of":
            chosen, decided = [], True
            for element in shape["set_of"]:
                votes = sum(element in value for value in values)
                if 0 < votes < total:
                    split.append(f"{path}.{element}")
                if 2 * votes == total:
                    decided = False
                elif 2 * votes > total:
                    chosen.append(element)
            return sorted(chosen) if decided else None
        result = {}
        for key, child in shape.items():
            value = walk(child, [v[key] for v in values], f"{path}.{key}" if path else key)
            if value is None:
                result = None
            elif result is not None:
                result[key] = value
        return result

    if not forms:
        return None, split
    return walk(node, forms, ""), sorted(split)


# ---------------------------------------------------------------- settings and keys

@dataclass(frozen=True)
class Reader:
    """What the reader step needs from a case, read once: the pin, the guide, the shapes and the settings hash."""
    model: str
    guide: str
    shapes: dict
    settings_sha256: str


def settings_sha256(model, guide, shapes_text):
    settings = {"contract": CONTRACT, "model": model, "temperature": TEMPERATURE, "max_tokens": MAX_TOKENS,
                "readings": READINGS, "guide_sha256": hashlib.sha256(guide.encode()).hexdigest(),
                "shapes_sha256": hashlib.sha256(shapes_text.encode()).hexdigest()}
    return hashlib.sha256(json.dumps(settings, sort_keys=True).encode()).hexdigest()


def load_reader(spec):
    """Read a case's ReaderSpec (cases.py) into a Reader."""
    guide = spec.guide.read_text()
    shapes_text = spec.shapes.read_text()
    return Reader(spec.model, guide, load_shapes(spec.shapes), settings_sha256(spec.model, guide, shapes_text))


def text_sha256(text):
    return hashlib.sha256(text.encode()).hexdigest()


def reading_key(kind, text, settings):
    return hashlib.sha256(json.dumps([kind, text, settings]).encode()).hexdigest()


# ---------------------------------------------------------------- the model call

@dataclass
class TextReading:
    id: str
    kind: str
    form: dict | None
    readings: list
    raw: list
    failed: int
    split: list
    usage: dict
    cached: bool = False

    def export(self):
        """The read-texts result: no raw replies and no text."""
        return {"id": self.id, "kind": self.kind, "form": self.form, "readings": self.readings,
                "failed": self.failed, "split": self.split, "usage": self.usage}


async def _one_call(model, reader, kind, text):
    config = GenerateConfig(temperature=TEMPERATURE, max_tokens=MAX_TOKENS, timeout=CALL_TIMEOUT_SECONDS,
                            max_retries=CALL_RETRIES, cache_prompt=False)  # the proxy refuses Inspect's cache headers
    try:
        output = await model.generate([ChatMessageSystem(content=reader.guide),
                                       ChatMessageUser(content=json.dumps({"kind": kind, "text": text}))], config=config)
    except Exception:
        raise ReaderCallError("reader failed") from None
    usage = output.usage
    return output.completion or "", (usage.input_tokens or 0) if usage else 0, (usage.output_tokens or 0) if usage else 0


def _assemble(item, reader, raws, tokens):
    node = reader.shapes[item["kind"]]
    parsed = [parse_reading(raw, node) for raw in raws]
    good = [form for form in parsed if form is not None]
    form, split = vote(node, good) if len(good) >= 2 else (None, [])
    return TextReading(item["id"], item["kind"], form, parsed, raws, READINGS - len(good), split,
                       {"calls": READINGS, "input_tokens": tokens[0], "output_tokens": tokens[1]})


async def read_texts(model, reader, items, *, cache=None):
    """Read each {"id","kind","text"} READINGS times. At most CONCURRENT_CALLS model calls run at once.

    Returns one TextReading per item, in order. A text whose readings are in the cache costs no call. A call that
    fails after its retries raises ReaderCallError; a text with no usable vote comes back with form None.
    """
    results = [None] * len(items)
    pending = []
    for index, item in enumerate(items):
        hit = cache.get(reading_key(item["kind"], item["text"], reader.settings_sha256)) if cache else None
        if hit is not None:
            results[index] = TextReading(item["id"], item["kind"], cached=True, **{k: v for k, v in hit.items() if k not in ("kind", "text")})
        else:
            pending.append(index)
    replies = {(index, n): None for index in pending for n in range(READINGS)}
    gate = anyio.Semaphore(CONCURRENT_CALLS)
    failure = []

    async def call(index, n, scope):
        async with gate:
            if failure:
                return
            try:
                replies[index, n] = await _one_call(model, reader, items[index]["kind"], items[index]["text"])
            except ReaderCallError as error:
                failure.append(error)
                scope.cancel()

    async with anyio.create_task_group() as group:
        for key in replies:
            group.start_soon(call, *key, group.cancel_scope)
    if failure:
        raise failure[0]
    for index in pending:
        parts = [replies[index, n] for n in range(READINGS)]
        reading = _assemble(items[index], reader, [p[0] for p in parts], (sum(p[1] for p in parts), sum(p[2] for p in parts)))
        results[index] = reading
        if cache is not None:
            cache.put(reading_key(items[index]["kind"], items[index]["text"], reader.settings_sha256), reading, items[index])
    return results


# ---------------------------------------------------------------- the reading cache

class ReadingCache:
    """One file per text, `reader-<key>.json`, in a capture folder. Writes are atomic and add-only.

    A rescore reuses a file whose key matches (same kind, text and settings) and so gets the same forms with no
    model call. The folder may hold other captures, and only `reader-*.json` files are touched. When the folder
    cannot be written the cache is skipped with one log line and scoring goes on.
    """

    def __init__(self, directory):
        self.directory = Path(directory)
        self.write_failed = False

    def _path(self, key):
        return self.directory / f"reader-{key}.json"

    def get(self, key):
        try:
            data = json.loads(self._path(key).read_text())
            keys = {"kind", "text", "form", "readings", "raw", "failed", "split", "usage"}
            if (set(data) != keys or not isinstance(data["readings"], list) or len(data["readings"]) != READINGS
                    or not isinstance(data["raw"], list) or data["form"] is None):
                return None
            return data
        except (OSError, ValueError, TypeError):
            return None

    def put(self, key, reading, item):
        """Keep the reading, its raw replies and the text. A reading with no form is kept to be inspected and is not a hit."""
        if self.write_failed:
            return
        payload = {"kind": item["kind"], "text": item["text"], "form": reading.form, "readings": reading.readings, "raw": reading.raw,
                   "failed": reading.failed, "split": reading.split, "usage": reading.usage}
        final = self._path(key)
        try:
            handle, name = tempfile.mkstemp(dir=self.directory, prefix=".reader-tmp-")
            try:
                with os.fdopen(handle, "w") as out:
                    out.write(json.dumps(payload, sort_keys=True))
                try:
                    os.link(name, final)  # add-only: an existing file is never replaced by a valid one
                except FileExistsError:
                    if self.get(key) is None:
                        os.replace(name, final)
            finally:
                if os.path.exists(name):
                    os.unlink(name)
        except OSError:
            self.write_failed = True
            log.warning("reader cache skipped: %s cannot be written", self.directory)


# ---------------------------------------------------------------- the collector

def parse_collected(stdout, shapes):
    """The collector's {"texts": [{"id","kind","text"|null}]} as a list. Anything else is a case bug."""
    try:
        data = json.loads(stdout)
        texts = data["texts"]
        if set(data) != {"texts"} or not isinstance(texts, list):
            raise ReaderInputError("collector output must be {\"texts\": [...]}")
        seen = set()
        for entry in texts:
            if (not isinstance(entry, dict) or set(entry) != {"id", "kind", "text"} or not isinstance(entry["id"], str)
                    or not entry["id"] or entry["id"] in seen or entry["kind"] not in shapes
                    or not (entry["text"] is None or isinstance(entry["text"], str))):
                raise ReaderInputError("collector text entry is malformed, repeated or of an unknown kind")
            seen.add(entry["id"])
    except (ValueError, KeyError, TypeError):
        raise ReaderInputError("collector output is not valid") from None
    return texts


def forms_document(texts, readings):
    """`forms.json`: only forms. A text the collector returned as null, or that has no form, is {"form": null}."""
    by_id = {r.id: r.form for r in readings}
    return {"forms": {entry["id"]: {"form": by_id.get(entry["id"])} for entry in texts}}


# ---------------------------------------------------------------- saved forms (case-check)

class SavedFormsStale(ReaderInputError):
    """The saved forms do not match what the collector printed or what the reader would be today."""


STALE = "saved readings are stale; run case-check --save-forms"


def load_saved(path):
    try:
        data = json.loads(Path(path).read_text())
        if not isinstance(data, dict) or not isinstance(data.get("scenarios"), dict):
            raise ValueError()
        return data
    except (OSError, ValueError):
        raise SavedFormsStale(STALE) from None


def saved_entries(texts, readings):
    """One saved entry per collected text: the text's hash and its voted form (both null for a null text)."""
    by_id = {r.id: r.form for r in readings}
    return {e["id"]: {"text_sha256": None if e["text"] is None else text_sha256(e["text"]), "form": by_id.get(e["id"])}
            for e in texts}


def lookup_saved(saved, reader, scenario, texts):
    """The forms document for one scenario from saved forms. Any difference at all is stale; no model is called."""
    entries = saved.get("scenarios", {}).get(scenario)
    if saved.get("settings_sha256") != reader.settings_sha256 or not isinstance(entries, dict):
        raise SavedFormsStale(STALE)
    wanted = {e["id"]: None if e["text"] is None else text_sha256(e["text"]) for e in texts}
    if set(entries) != set(wanted) or any(
            not isinstance(entries[i], dict) or entries[i].get("text_sha256") != sha for i, sha in wanted.items()):
        raise SavedFormsStale(STALE)
    return {"forms": {i: {"form": entries[i].get("form")} for i in wanted}}


# ---------------------------------------------------------------- the scoring hook

def usage_metrics(reader, texts, readings):
    """Counts only: no text, no raw reply. `split` names `<id>:<leaf>`; ids are the collector's labels."""
    fresh = [r for r in readings if not r.cached]
    return {"model": reader.model, "settings_sha256": reader.settings_sha256, "texts": len(texts),
            "unread_null_texts": sum(entry["text"] is None for entry in texts), "cached_texts": len(readings) - len(fresh),
            "calls": sum(r.usage["calls"] for r in fresh), "input_tokens": sum(r.usage["input_tokens"] for r in fresh),
            "output_tokens": sum(r.usage["output_tokens"] for r in fresh),
            "failed_readings": sum(r.failed for r in readings),
            "split": [f"{r.id}:{leaf}" for r in readings for leaf in r.split]}


async def run_reader_step(trial):
    """Collect, read, write `forms.json` beside the hidden tests. Returns the metrics for `hidden_tests.reader`.

    Runs after the hidden tests were copied and before they run. A bad collector, a failed call, a text with no
    usable vote or a step past STEP_SECONDS is an infrastructure failure: the delivery did not cause it.
    """
    spec = trial.case.reader
    reader = load_reader(spec)
    try:
        result = await trial.env.exec(["python3", f"/opt/scoring/tests/{spec.collector}"],
                                      cwd="/workspace/public/repo", timeout=COLLECTOR_SECONDS)
        if not result.success:
            raise ReaderInputError("collector exit")
        texts = parse_collected(result.stdout, reader.shapes)
    except (ReaderInputError, TimeoutError, OSError):
        raise InfrastructureError("reader collector failed") from None
    items = [entry for entry in texts if entry["text"] is not None]
    capture_dir = getattr(trial, "capture_dir", None)
    cache = ReadingCache(capture_dir) if capture_dir is not None else None
    try:
        with anyio.fail_after(STEP_SECONDS):
            readings = await read_texts(get_model(reader.model), reader, items, cache=cache)
    except TimeoutError:
        raise InfrastructureError("reader failed") from None
    unread = [r.id for r in readings if r.form is None]
    if unread:
        raise InfrastructureError("reader failed: no usable vote for " + ", ".join(unread))
    document = forms_document(texts, readings)
    await trial.env.write_file(f"/opt/scoring/tests/{FORMS_FILE}", json.dumps(document))
    trial.evidence["copy_log"].append({"kind": "reader_forms", "source": "harness/reader",
                                       "destination": f"/opt/scoring/tests/{FORMS_FILE}", "phase": "after_terminal",
                                       "elapsed_seconds": time.monotonic() - trial.evidence["started"]})
    return usage_metrics(reader, texts, readings)
