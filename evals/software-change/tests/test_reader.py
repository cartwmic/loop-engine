"""The optional reader step (scevals/reader.py and its hooks in cases.py and target.py).

No Docker, no network and no credentials: the model is Inspect's `mockllm/model` with scripted replies, and the
scoring sandbox is a small object with `exec`, `write_file` and `read_file`. The collector's own output is data here;
the real collector of a real case is exercised by that case's own checks.
"""
import asyncio
import datetime
from collections import deque
import json
import logging
import os
from pathlib import Path
import shutil
import stat
import sys
import time
from types import SimpleNamespace

import pytest
from inspect_ai import Task, eval as inspect_eval
from inspect_ai.dataset import Sample
from inspect_ai.model import ModelOutput, ModelUsage, get_model
from inspect_ai.solver import solver

from evalcore.target import InfrastructureError
from scevals import cases, reader, target
from scevals.cases import CaseFormatError, load_case

EXAMPLE = Path(__file__).resolve().parents[1] / "example-case"
MODEL = "mockllm/model"
LEAF = {"one_of": ["yes", "no", "not_said", "mixed"]}
SHAPES = {
    "message": {"offers": {"set_of": ["ask", "grant", "no_key", "other"]}},
    "bullet": {"ask": {"loads": LEAF, "write_with_ask": LEAF}, "grant": {"loads": LEAF, "write_with_ask": LEAF}},
}
TEXT = 'Jira can only use "ask".'
OTHER_TEXT = "Run the config check first."
MESSAGE = '{"offers": ["ask"]}'
BULLET = {"ask": {"loads": "yes", "write_with_ask": "yes"}, "grant": {"loads": "no", "write_with_ask": "no"}}


# ---------------------------------------------------------------- helpers

def make_case(tmp_path, *, reader_table=True, shapes=SHAPES, collector_exists=True):
    """A copy of the example case with a `[reader]` table, a guide, a shapes file and a collector file."""
    path = tmp_path / "case"
    shutil.copytree(EXAMPLE, path)
    if reader_table:
        (path / "hidden/reader").mkdir()
        (path / "hidden/reader/guide.md").write_text("Fill the form.\n")
        (path / "hidden/reader/form-shapes.json").write_text(json.dumps(shapes))
        if collector_exists:
            (path / "hidden/tests/collect_texts.py").write_text("print('{}')\n")
        with open(path / "case.toml", "a") as manifest:
            manifest.write('\n[reader]\nmodel = "mockllm/model"\nguide = "hidden/reader/guide.md"\n'
                           'shapes = "hidden/reader/form-shapes.json"\ncollector = "collect_texts.py"\n'
                           'saved = "hidden/reader/saved-forms.json"\n')
    return path


def collected(*entries):
    return json.dumps({"texts": [{"id": i, "kind": k, "text": t} for i, k, t in entries]})


def scripted(replies, calls=None):
    """A mockllm model. `replies` maps a text to the replies for its calls, in order; an Exception is raised."""
    queues = {text: deque(items) for text, items in replies.items()}

    def outputs(input, tools, tool_choice, config):
        payload = json.loads(input[-1].content)
        if calls is not None:
            calls.append({"system": input[0].content, "user": payload, "config": config})
        reply = queues[payload["text"]].popleft()
        if isinstance(reply, Exception):
            raise reply
        output = ModelOutput.from_content(MODEL, reply)
        output.usage = ModelUsage(input_tokens=10, output_tokens=5, total_tokens=15)
        return output
    return get_model(MODEL, custom_outputs=outputs)


def reader_of(tmp_path, **options):
    return reader.load_reader(load_case(make_case(tmp_path, **options)).reader)


class Env:
    """The scoring sandbox: records exec calls and writes, answers the collector with `stdout`."""
    def __init__(self, stdout="", success=True):
        self.stdout, self.success, self.writes, self.commands = stdout, success, {}, []

    async def exec(self, command, *, cwd=None, timeout=120):
        self.commands.append((command, cwd))
        return SimpleNamespace(success=self.success, returncode=0 if self.success else 1, stdout=self.stdout, stderr="")

    async def write_file(self, path, content):
        self.writes[path] = content


def trial_of(case, env, capture_dir=None):
    return SimpleNamespace(case=case, env=env, capture_dir=capture_dir,
                           evidence={"copy_log": [], "started": time.monotonic()})


def step(trial, model):
    """Run the scoring step with `model` standing in for the case's pinned model."""
    original = reader.get_model
    reader.get_model = lambda name: model
    try:
        return asyncio.run(reader.run_reader_step(trial))
    finally:
        reader.get_model = original


# ---------------------------------------------------------------- shapes, parsing, voting

@pytest.mark.parametrize("bad", [
    {}, {"k": {}}, {"k": {"one_of": []}}, {"k": {"one_of": ["a", "a"]}}, {"k": {"set_of": "ab"}},
    {"k": {"one_of": ["a"], "set_of": ["b"]}}, {"k": {"a": {"one_of": ["x"]}, "one_of": {"one_of": ["x"]}}}, {"": LEAF},
    {"k": {"a": []}}, [], {"k": LEAF}, {"k": {"set_of": ["a"]}}])
def test_shapes_reject_malformed_documents(bad):
    with pytest.raises(reader.ReaderInputError):
        reader.check_shapes(bad)


def test_shapes_accept_nested_objects_and_leaves():
    assert reader.check_shapes(SHAPES) is SHAPES


def test_form_must_match_the_shape_exactly_and_set_leaves_are_sorted():
    node = SHAPES["message"]
    assert reader.normalize_form(node, {"offers": ["grant", "ask", "ask"]}) == {"offers": ["ask", "grant"]}
    for form in ({"offers": ["bogus"]}, {"offers": "ask"}, {"offers": ["ask"], "x": 1}, {}, [], None, {"offers": [1]}):
        assert reader.normalize_form(node, form) is None
    assert reader.normalize_form(SHAPES["bullet"], BULLET) == BULLET
    assert reader.normalize_form(SHAPES["bullet"], {"ask": BULLET["ask"]}) is None
    assert reader.normalize_form(SHAPES["bullet"], {**BULLET, "grant": {"loads": "maybe", "write_with_ask": "no"}}) is None


def test_parse_reading_takes_the_last_fenced_or_bare_object_and_refuses_the_wrong_shape():
    node = SHAPES["message"]
    assert reader.parse_reading('thinking {"offers": []}\n```json\n{"offers": ["grant"]}\n```', node) == {"offers": ["grant"]}
    assert reader.parse_reading('first {"offers": ["no_key"]} then {"offers": ["ask"]}', node) == {"offers": ["ask"]}
    assert reader.parse_reading('```json\n{"offers": ["ask"], "extra": 1}\n```', node) is None
    assert reader.parse_reading("no json here", node) is None
    assert reader.parse_reading("", node) is None


def test_vote_one_leaf_needs_a_strict_majority():
    node = SHAPES["bullet"]
    yes, no = BULLET, {**BULLET, "ask": {"loads": "no", "write_with_ask": "yes"}}
    form, split = reader.vote(node, [yes, yes, no])
    assert form == yes and split == ["ask.loads"]
    assert reader.vote(node, [yes, no]) == (None, ["ask.loads"])  # 1-1 is no majority
    three = [yes, no, {**BULLET, "ask": {"loads": "mixed", "write_with_ask": "yes"}}]
    assert reader.vote(node, three)[0] is None
    assert reader.vote(node, [yes, yes, yes]) == (yes, [])


def test_vote_set_leaf_votes_per_element():
    node = SHAPES["message"]
    forms = [{"offers": ["ask"]}, {"offers": ["ask", "grant"]}, {"offers": ["ask", "grant"]}]
    assert reader.vote(node, forms) == ({"offers": ["ask", "grant"]}, ["offers.grant"])
    assert reader.vote(node, [{"offers": ["ask"]}, {"offers": ["ask", "grant"]}]) == (None, ["offers.grant"])  # 1-1 tie
    assert reader.vote(node, [{"offers": ["ask"]}, {"offers": ["ask"]}]) == ({"offers": ["ask"]}, [])
    assert reader.vote(node, [{"offers": []}, {"offers": []}, {"offers": ["other"]}]) == ({"offers": []}, ["offers.other"])
    assert reader.vote(node, []) == (None, [])


def test_a_voted_set_leaf_is_sorted_whatever_order_the_shape_lists_it_in():
    node = {"offers": {"set_of": ["grant", "ask"]}}
    forms = [{"offers": ["ask", "grant"]}] * 3
    assert reader.vote(node, forms) == ({"offers": ["ask", "grant"]}, [])


def test_settings_hash_changes_with_the_model_the_guide_and_the_shapes():
    base = reader.settings_sha256("m", "guide", "{}")
    assert base == reader.settings_sha256("m", "guide", "{}")
    assert len({base, reader.settings_sha256("n", "guide", "{}"), reader.settings_sha256("m", "guide2", "{}"),
                reader.settings_sha256("m", "guide", "{ }")}) == 4


# ---------------------------------------------------------------- load_case

def test_load_case_without_reader_is_unchanged():
    case = load_case(EXAMPLE)
    assert case.reader is None


def test_load_case_reads_the_reader_table_as_data_and_does_not_need_the_saved_file(tmp_path):
    case = load_case(make_case(tmp_path))
    spec = case.reader
    assert spec.model == MODEL and spec.collector == "collect_texts.py"
    assert spec.guide.read_text() == "Fill the form.\n"
    assert spec.shapes.name == "form-shapes.json" and spec.saved.name == "saved-forms.json" and not spec.saved.exists()


@pytest.mark.parametrize("edit", [
    ("guide", "../outside.md"), ("guide", "/abs/guide.md"), ("shapes", "hidden/reader/missing.json"),
    ("collector", "sub/collect_texts.py"), ("collector", "missing.py"), ("saved", "../saved.json"),
    ("model", ""), ("extra", "x")])
def test_load_case_refuses_a_bad_reader_table(tmp_path, edit):
    path = make_case(tmp_path)
    text = (path / "case.toml").read_text()
    key, value = edit
    if key == "extra":
        text += f'{key} = "{value}"\n'
    else:
        lines = [f'{key} = "{value}"' if line.startswith(f"{key} =") and line.split("=")[0].strip() == key else line
                 for line in text.splitlines()]
        text = "\n".join(lines) + "\n"
    (path / "case.toml").write_text(text)
    (tmp_path / "outside.md").write_text("x")
    with pytest.raises(CaseFormatError):
        load_case(path)


def test_load_case_refuses_malformed_shapes(tmp_path):
    with pytest.raises(CaseFormatError):
        load_case(make_case(tmp_path, shapes={"k": {"one_of": []}}))


# ---------------------------------------------------------------- the collector contract

@pytest.mark.parametrize("output", [
    "", "not json", "[]", "{}", '{"texts": {}}', '{"texts": [], "extra": 1}',
    '{"texts": [{"id": "a", "kind": "message"}]}',
    '{"texts": [{"id": "a", "kind": "nope", "text": "x"}]}',
    '{"texts": [{"id": "a", "kind": "message", "text": 5}]}',
    '{"texts": [{"id": "a", "kind": "message", "text": "x"}, {"id": "a", "kind": "message", "text": "y"}]}',
    '{"texts": [{"id": "", "kind": "message", "text": "x"}]}'])
def test_collector_output_in_the_wrong_shape_is_refused(output):
    with pytest.raises(reader.ReaderInputError):
        reader.parse_collected(output, SHAPES)


def test_collector_output_in_the_right_shape_is_returned():
    out = collected(("a", "message", "x"), ("b", "bullet", None))
    assert [t["id"] for t in reader.parse_collected(out, SHAPES)] == ["a", "b"]


@pytest.mark.parametrize("env", [Env(success=False), Env("not json"), Env(collected(("a", "nope", "x")))])
def test_a_bad_collector_is_an_infrastructure_failure_not_a_trial_failure(tmp_path, env):
    case = load_case(make_case(tmp_path))
    with pytest.raises(InfrastructureError, match="reader collector failed"):
        step(trial_of(case, env), scripted({}))
    assert env.writes == {}


# ---------------------------------------------------------------- the scoring step

def test_forms_are_written_beside_the_tests_and_a_null_text_gets_no_reading(tmp_path):
    case = load_case(make_case(tmp_path))
    env = Env(collected(("m", "message", TEXT), ("b", "bullet", None)))
    calls = []
    metrics = step(trial_of(case, env), scripted({TEXT: [MESSAGE] * 3}, calls))
    assert env.commands == [(["python3", "/opt/scoring/tests/collect_texts.py"], "/workspace/public/repo")]
    assert json.loads(env.writes["/opt/scoring/tests/forms.json"]) == {
        "forms": {"m": {"form": {"offers": ["ask"]}}, "b": {"form": None}}}
    assert len(calls) == 3 and calls[0]["system"] == "Fill the form.\n"
    assert calls[0]["user"] == {"kind": "message", "text": TEXT}
    assert (calls[0]["config"].temperature, calls[0]["config"].max_tokens) == (0.7, 8192)
    assert (calls[0]["config"].timeout, calls[0]["config"].max_retries, calls[0]["config"].cache_prompt) == (120, 2, False)
    assert metrics["calls"] == 3 and metrics["input_tokens"] == 30 and metrics["output_tokens"] == 15
    assert metrics["texts"] == 2 and metrics["unread_null_texts"] == 1 and metrics["failed_readings"] == 0
    assert metrics["model"] == MODEL and metrics["split"] == []


def test_metrics_carry_counts_and_split_fields_but_no_text(tmp_path):
    case = load_case(make_case(tmp_path))
    env = Env(collected(("m", "message", TEXT)))
    replies = [MESSAGE, '{"offers": ["ask", "grant"]}', MESSAGE]
    metrics = step(trial_of(case, env), scripted({TEXT: replies}))
    assert metrics["split"] == ["m:offers.grant"]
    assert TEXT not in json.dumps(metrics) and "ask" not in json.dumps(metrics)


def test_one_unparseable_reading_is_counted_and_the_other_two_vote(tmp_path):
    case = load_case(make_case(tmp_path))
    metrics = step(trial_of(case, Env(collected(("m", "message", TEXT)))), scripted({TEXT: [MESSAGE, "no idea", MESSAGE]}))
    assert metrics["failed_readings"] == 1


@pytest.mark.parametrize("replies", [["no idea"] * 3, [MESSAGE, "x", "y"],
                                     [MESSAGE, '{"offers": ["grant"]}', "z"]])
def test_a_text_with_no_usable_vote_is_an_infrastructure_failure(tmp_path, replies):
    case = load_case(make_case(tmp_path))
    env = Env(collected(("m", "message", TEXT)))
    with pytest.raises(InfrastructureError, match="reader failed: no usable vote for m"):
        step(trial_of(case, env), scripted({TEXT: replies}))
    assert env.writes == {}


def test_a_text_with_no_usable_vote_is_kept_to_inspect_and_is_not_reused(tmp_path):
    case = load_case(make_case(tmp_path))
    capture = tmp_path / "capture"
    capture.mkdir()
    with pytest.raises(InfrastructureError, match="no usable vote for m"):
        step(trial_of(case, Env(collected(("m", "message", TEXT))), capture), scripted({TEXT: ["no idea", MESSAGE, "x"]}))
    (name,) = cache_files(capture)
    kept = json.loads((capture / name).read_text())
    assert kept["form"] is None and kept["raw"] == ["no idea", MESSAGE, "x"] and kept["text"] == TEXT
    calls = []  # a rescore reads again; the kept file is not a hit, and a good reading then replaces it
    step(trial_of(case, Env(collected(("m", "message", TEXT))), capture), scripted({TEXT: [MESSAGE] * 3}, calls))
    assert len(calls) == 3 and json.loads((capture / name).read_text())["form"] == {"offers": ["ask"]}


def test_a_call_that_fails_is_an_infrastructure_failure(tmp_path):
    case = load_case(make_case(tmp_path))
    env = Env(collected(("m", "message", TEXT)))
    with pytest.raises(InfrastructureError, match="reader failed"):
        step(trial_of(case, env), scripted({TEXT: [MESSAGE, RuntimeError("down"), MESSAGE]}))


def test_the_step_cap_is_an_infrastructure_failure(tmp_path, monkeypatch):
    case = load_case(make_case(tmp_path))
    monkeypatch.setattr(reader, "STEP_SECONDS", 0.05)

    async def slow(*args, **kwargs):
        await asyncio.sleep(5)
    monkeypatch.setattr(reader, "read_texts", slow)
    with pytest.raises(InfrastructureError, match="reader failed"):
        step(trial_of(case, Env(collected(("m", "message", TEXT)))), scripted({}))


# ---------------------------------------------------------------- the cache

def cache_files(directory):
    return sorted(p.name for p in Path(directory).iterdir() if p.name.startswith("reader-"))


def test_readings_are_kept_per_text_and_a_rescore_reuses_them_with_no_model_call(tmp_path):
    case = load_case(make_case(tmp_path))
    capture = tmp_path / "capture"
    capture.mkdir()
    env = Env(collected(("m", "message", TEXT), ("n", "message", OTHER_TEXT)))
    first_calls = []
    first = step(trial_of(case, env, capture), scripted({TEXT: [MESSAGE] * 3, OTHER_TEXT: [MESSAGE, MESSAGE, "x"]}, first_calls))
    key = reader.reading_key("message", TEXT, reader_of(tmp_path / "again").settings_sha256)
    assert cache_files(capture) == sorted([f"reader-{key}.json", f"reader-{reader.reading_key('message', OTHER_TEXT, reader_of(tmp_path / 'third').settings_sha256)}.json"])
    assert len(first_calls) == 6 and first["cached_texts"] == 0
    saved = json.loads((capture / f"reader-{key}.json").read_text())
    assert saved["form"] == {"offers": ["ask"]} and saved["raw"] == [MESSAGE] * 3
    assert saved["kind"] == "message" and saved["text"] == TEXT  # the text stays in the private capture folder
    forms = env.writes["/opt/scoring/tests/forms.json"]

    env2, second_calls = Env(env.stdout), []
    second = step(trial_of(case, env2, capture), scripted({}, second_calls))
    assert second_calls == [] and second["cached_texts"] == 2 and second["calls"] == 0
    assert env2.writes["/opt/scoring/tests/forms.json"] == forms  # same forms: parity


def test_a_changed_text_or_a_changed_setting_reads_again(tmp_path):
    case = load_case(make_case(tmp_path))
    capture = tmp_path / "capture"
    capture.mkdir()
    step(trial_of(case, Env(collected(("m", "message", TEXT))), capture), scripted({TEXT: [MESSAGE] * 3}))
    calls = []
    step(trial_of(case, Env(collected(("m", "message", OTHER_TEXT))), capture), scripted({OTHER_TEXT: [MESSAGE] * 3}, calls))
    assert len(calls) == 3
    (case.reader.guide).write_text("A changed guide.\n")
    calls.clear()
    step(trial_of(case, Env(collected(("m", "message", TEXT))), capture), scripted({TEXT: [MESSAGE] * 3}, calls))
    assert len(calls) == 3 and len(cache_files(capture)) == 3


def test_cache_writes_are_atomic_and_add_only(tmp_path):
    cache = reader.ReadingCache(tmp_path)
    first = reader.TextReading("a", "message", {"offers": ["ask"]}, [{"offers": ["ask"]}] * 3, ["r"] * 3, 0, [],
                               {"calls": 3, "input_tokens": 1, "output_tokens": 1})
    item = {"id": "a", "kind": "message", "text": TEXT}
    cache.put("k", first, item)
    assert [p.name for p in tmp_path.iterdir()] == ["reader-k.json"]  # no temp file left behind
    other = reader.TextReading("a", "message", {"offers": ["grant"]}, [{"offers": ["grant"]}] * 3, ["s"] * 3, 0, [],
                               {"calls": 3, "input_tokens": 1, "output_tokens": 1})
    cache.put("k", other, item)  # a valid file is never replaced
    assert cache.get("k")["form"] == {"offers": ["ask"]}
    (tmp_path / "reader-k.json").write_text("{broken")
    assert cache.get("k") is None
    cache.put("k", other, item)  # a broken file is repaired
    assert cache.get("k")["form"] == {"offers": ["grant"]}
    assert [p.name for p in tmp_path.iterdir()] == ["reader-k.json"]


def test_without_a_capture_folder_nothing_is_kept_and_each_scoring_reads_fresh(tmp_path):
    case = load_case(make_case(tmp_path))
    out = collected(("m", "message", TEXT))
    for _ in range(2):
        calls = []
        step(trial_of(case, Env(out), None), scripted({TEXT: [MESSAGE] * 3}, calls))
        assert len(calls) == 3
    assert not any(p.name.startswith("reader-") for p in tmp_path.rglob("*"))


def test_a_folder_that_cannot_be_written_skips_the_cache_with_one_log_line(tmp_path, caplog):
    case = load_case(make_case(tmp_path))
    capture = tmp_path / "capture"
    capture.mkdir()
    capture.chmod(stat.S_IRUSR | stat.S_IXUSR)
    try:
        if os.access(capture, os.W_OK):
            pytest.skip("directory permissions are not enforced for this user")
        with caplog.at_level(logging.WARNING, logger="scevals.reader"):
            metrics = step(trial_of(case, Env(collected(("m", "message", TEXT), ("n", "message", OTHER_TEXT))), capture),
                           scripted({TEXT: [MESSAGE] * 3, OTHER_TEXT: [MESSAGE] * 3}))
    finally:
        capture.chmod(stat.S_IRWXU)
    assert metrics["calls"] == 6
    assert [r.getMessage() for r in caplog.records if "cache skipped" in r.getMessage()] == [
        f"reader cache skipped: {capture} cannot be written"]
    assert os.listdir(capture) == []


# ---------------------------------------------------------------- saved forms (case-check)

def saved_for(rd, scenario, entries):
    return {"settings_sha256": rd.settings_sha256, "scenarios": {scenario: entries}}


def test_saved_forms_are_looked_up_by_text_hash_with_no_model(tmp_path):
    rd = reader_of(tmp_path)
    texts = [{"id": "m", "kind": "message", "text": TEXT}, {"id": "g", "kind": "message", "text": None}]
    form = {"offers": ["ask"]}
    saved = saved_for(rd, "reference", {"m": {"text_sha256": reader.text_sha256(TEXT), "form": form},
                                        "g": {"text_sha256": None, "form": None}})
    assert reader.lookup_saved(saved, rd, "reference", texts) == {"forms": {"m": {"form": form}, "g": {"form": None}}}


@pytest.mark.parametrize("change", ["text", "missing-entry", "extra-entry", "settings", "scenario", "null-became-text"])
def test_stale_saved_forms_are_refused(tmp_path, change):
    rd = reader_of(tmp_path)
    texts = [{"id": "m", "kind": "message", "text": TEXT}, {"id": "g", "kind": "message", "text": None}]
    entries = {"m": {"text_sha256": reader.text_sha256(TEXT), "form": {"offers": ["ask"]}},
               "g": {"text_sha256": None, "form": None}}
    saved = saved_for(rd, "reference", entries)
    if change == "text":
        texts[0] = {**texts[0], "text": TEXT + " "}
    elif change == "missing-entry":
        del entries["g"]
    elif change == "extra-entry":
        entries["z"] = {"text_sha256": None, "form": None}
    elif change == "settings":
        saved["settings_sha256"] = "0" * 64
    elif change == "scenario":
        saved["scenarios"] = {"noop": entries}
    else:
        texts[1] = {**texts[1], "text": "now there is a refusal"}
    with pytest.raises(reader.SavedFormsStale, match="run case-check --save-forms"):
        reader.lookup_saved(saved, rd, "reference", texts)


def test_a_missing_saved_file_is_stale_not_a_crash(tmp_path):
    with pytest.raises(reader.SavedFormsStale):
        reader.load_saved(tmp_path / "saved-forms.json")


def test_save_forms_reads_with_the_model_and_the_next_check_needs_no_model(tmp_path, monkeypatch):
    case = load_case(make_case(tmp_path))
    texts = [{"id": "m", "kind": "message", "text": TEXT}, {"id": "g", "kind": "message", "text": None}]
    calls = []
    monkeypatch.setattr(reader, "get_model", lambda name: scripted({TEXT: [MESSAGE] * 3}, calls))
    resolve, saved = cases._forms_resolver(case, True)
    assert resolve("reference", texts) == {"forms": {"m": {"form": {"offers": ["ask"]}}, "g": {"form": None}}}
    assert len(calls) == 3
    case.reader.saved.write_text(json.dumps(saved))

    monkeypatch.setattr(reader, "get_model", lambda name: pytest.fail("no model call is allowed"))
    resolve, _ = cases._forms_resolver(case, False)
    assert resolve("reference", texts) == {"forms": {"m": {"form": {"offers": ["ask"]}}, "g": {"form": None}}}
    with pytest.raises(reader.SavedFormsStale):
        resolve("noop", texts)


def test_save_forms_refuses_to_write_a_text_with_no_majority(tmp_path, monkeypatch):
    case = load_case(make_case(tmp_path))
    monkeypatch.setattr(reader, "get_model", lambda name: scripted({TEXT: ["x"] * 3}))
    resolve, _ = cases._forms_resolver(case, True)
    with pytest.raises(InfrastructureError):
        resolve("reference", [{"id": "m", "kind": "message", "text": TEXT}])


# ---------------------------------------------------------------- the hook in target.run_hidden_tests

class ScoringEnv(Env):
    async def exec(self, command, *, cwd=None, timeout=120):
        if command[1].endswith("case_runtime.py"):
            self.commands.append((command, cwd))
            return SimpleNamespace(success=True, returncode=0, stdout="", stderr="")
        return await super().exec(command, cwd=cwd, timeout=timeout)

    async def read_file(self, path, text=True):
        return json.dumps({"tests": [], "collection_error": False})


def scoring_trial(case, env, capture_dir=None):
    async def copy_scoring_material():
        env.commands.append((["copy_scoring_material"], None))
    trial = trial_of(case, env, capture_dir)
    trial.copy_scoring_material = copy_scoring_material
    trial.evidence["copy_out_log"] = []
    return trial


def test_the_reader_runs_after_the_copy_and_before_the_tests_and_lands_in_hidden_tests_reader(tmp_path, monkeypatch):
    case = load_case(make_case(tmp_path))
    env = ScoringEnv(collected(("m", "message", TEXT)))
    monkeypatch.setattr(reader, "get_model", lambda name: scripted({TEXT: [MESSAGE] * 3}))
    tests = asyncio.run(target.run_hidden_tests(scoring_trial(case, env)))
    order = [c[0][0] if c[0][0] == "copy_scoring_material" else c[0][1].rsplit("/", 1)[1] for c in env.commands]
    assert order == ["copy_scoring_material", "collect_texts.py", "case_runtime.py"]
    assert tests["reader"]["calls"] == 3 and tests["tests"] == []


def test_a_case_without_a_reader_scores_exactly_as_before(tmp_path):
    case = load_case(EXAMPLE)
    env = ScoringEnv()
    tests = asyncio.run(target.run_hidden_tests(scoring_trial(case, env)))
    assert "reader" not in tests and env.writes == {}
    assert [c[0][1].rsplit("/", 1)[-1] for c in env.commands[1:]] == ["case_runtime.py"]


# ---------------------------------------------------------------- evalctl read-texts

def run_read_texts(tmp_path, texts, model, *, case=None):
    source, out = tmp_path / "texts.json", tmp_path / "readings.json"
    source.write_text(json.dumps({"texts": texts}) if isinstance(texts, list) else texts)
    case = case or make_case(tmp_path)
    original = reader.get_model
    reader.get_model = lambda name: model
    try:
        code = cases.read_texts_command(SimpleNamespace(path=case, source=source, out=out))
    finally:
        reader.get_model = original
    return code, out


def test_read_texts_writes_one_result_per_text_in_input_order(tmp_path):
    texts = [{"id": "w:b", "kind": "message", "text": OTHER_TEXT}, {"id": "s:a", "kind": "message", "text": TEXT}]
    replies = {TEXT: [MESSAGE] * 3, OTHER_TEXT: [MESSAGE, '{"offers": ["grant"]}', MESSAGE]}
    code, out = run_read_texts(tmp_path, texts, scripted(replies))
    assert code == 0
    data = json.loads(out.read_text())
    assert list(data) == ["settings_sha256", "model", "results"] and data["model"] == MODEL
    assert data["settings_sha256"] == reader_of(tmp_path / "x").settings_sha256
    first, second = data["results"]
    assert list(first) == ["id", "kind", "form", "readings", "failed", "split", "usage"]
    assert (first["id"], second["id"]) == ("w:b", "s:a")
    assert first["form"] == {"offers": ["ask"]} and first["split"] == ["offers.ask", "offers.grant"] and len(first["readings"]) == 3
    assert second["split"] == [] and second["failed"] == 0
    assert first["usage"] == {"calls": 3, "input_tokens": 30, "output_tokens": 15}
    assert OTHER_TEXT not in out.read_text() and MESSAGE not in out.read_text()


def test_read_texts_unreadable_text_is_data_not_a_failure(tmp_path):
    code, out = run_read_texts(tmp_path, [{"id": "a", "kind": "message", "text": TEXT}], scripted({TEXT: ["?", "?", MESSAGE]}))
    assert code == 0
    result = json.loads(out.read_text())["results"][0]
    assert result["form"] is None and result["failed"] == 2 and result["readings"] == [None, None, {"offers": ["ask"]}]


def test_read_texts_does_not_use_the_cache_and_has_no_step_cap(tmp_path, monkeypatch):
    monkeypatch.setattr(reader, "STEP_SECONDS", 0.0001)
    calls = []
    code, _ = run_read_texts(tmp_path, [{"id": "a", "kind": "message", "text": TEXT}], scripted({TEXT: [MESSAGE] * 3}, calls))
    assert code == 0 and len(calls) == 3


@pytest.mark.parametrize("texts", [
    [{"id": "a", "kind": "message", "text": TEXT}, {"id": "a", "kind": "bullet", "text": OTHER_TEXT}],
    [{"id": "a", "kind": "nope", "text": TEXT}], [{"id": "a", "kind": ["message"], "text": TEXT}],
    [{"id": "a", "kind": "message", "text": None}],
    [{"id": "a", "kind": "message", "text": ""}], [{"id": "a", "kind": "message"}], "not json", '{"texts": 1}', '{"other": []}'])
def test_read_texts_refuses_bad_input_with_exit_2_and_writes_nothing(tmp_path, texts, capsys):
    code, out = run_read_texts(tmp_path, texts, scripted({}))
    assert code == 2 and not out.exists()
    assert TEXT not in capsys.readouterr().err


def test_read_texts_names_the_duplicate_id(tmp_path, capsys):
    texts = [{"id": "a", "kind": "message", "text": TEXT}, {"id": "a", "kind": "message", "text": OTHER_TEXT}]
    assert run_read_texts(tmp_path, texts, scripted({}))[0] == 2
    assert "duplicate id: a" in capsys.readouterr().err


def test_read_texts_on_a_case_with_no_reader_exits_2(tmp_path):
    code, out = run_read_texts(tmp_path, [{"id": "a", "kind": "message", "text": TEXT}], scripted({}),
                               case=make_case(tmp_path, reader_table=False))
    assert code == 2 and not out.exists()


def test_read_texts_failed_call_exits_3_and_writes_nothing(tmp_path):
    texts = [{"id": "a", "kind": "message", "text": TEXT}]
    code, out = run_read_texts(tmp_path, texts, scripted({TEXT: [MESSAGE, RuntimeError("down"), MESSAGE]}))
    assert code == 3 and not out.exists()


# ---------------------------------------------------------------- accounting inside a sample

def test_a_reader_call_inside_a_sample_is_not_recorded_under_any_role(tmp_path):
    """Pins what `sample_role_usage()` holds after one reader call, which feeds the campaign dollar stop.

    The reader is not a bound role, so its call leaves the role map empty. Inspect's per-model usage does see it.
    """
    rd = reader_of(tmp_path)
    seen = {}

    @solver
    def read_once():
        async def solve(state, generate):
            from inspect_ai.model._model import sample_model_usage, sample_role_usage
            await reader.read_texts(scripted({TEXT: [MESSAGE] * 3}), rd, [{"id": "a", "kind": "message", "text": TEXT}])
            seen["roles"] = dict(sample_role_usage())
            seen["models"] = {name: usage.total_tokens for name, usage in sample_model_usage().items()}
            return state
        return solve

    inspect_eval(Task(dataset=[Sample(input="x")], solver=read_once()), model=MODEL, log_dir=str(tmp_path / "logs"), display="none")
    assert seen["roles"] == {}
    assert seen["models"] == {MODEL: 45}


# ---------------------------------------------------------------- case-check end to end (container work stubbed)

def test_the_readers_own_keys_in_a_collector_are_not_unstated_interfaces(tmp_path):
    path = make_case(tmp_path)
    (path / "hidden/tests/collect_texts.py").write_text(
        'import json\nprint(json.dumps({"texts": [{"id": "x", "kind": "message", "text": None}]}))\n'
        'forms = json.load(open("forms.json"))["forms"]\nforms["x"]["form"]\n')
    assert cases.unstated_interfaces(load_case(path)) == []


def fake_container_work(case, texts_for):
    """Stands in for run_hidden_tests: asks for forms as the real one does, then reports the reference passing."""
    def run(case_arg, reference, image, resolve_forms=None):
        resolve_forms("reference" if reference else "noop", texts_for(reference))
        statuses = ["passed" if reference or i else "failed" for i, _ in enumerate(case.test_requirements)]
        return {"collection_error": False, "container": "stub",
                "tests": [{"requirement_id": tag, "status": status} for tag, status in zip(case.test_requirements, statuses)]}
    return run


def test_case_check_save_forms_writes_the_saved_file_and_a_later_check_reads_it_with_no_model(tmp_path, monkeypatch, capsys):
    path = make_case(tmp_path)
    case = load_case(path)
    texts = [{"id": "m", "kind": "message", "text": TEXT}]
    monkeypatch.setattr(cases, "verify_image", lambda ref, tag: "image")
    monkeypatch.setattr(cases, "run_hidden_tests", fake_container_work(case, lambda reference: texts))
    monkeypatch.setattr(reader, "get_model", lambda name: scripted({TEXT: [MESSAGE] * 3}))
    args = SimpleNamespace(path=path, expect_invalid=False, ref="r", tag="t", save_forms=True)
    assert cases.case_check(args) == 0
    saved = json.loads(case.reader.saved.read_text())
    assert set(saved["scenarios"]) == {"reference", "noop"}
    assert "VALID" in capsys.readouterr().out

    monkeypatch.setattr(reader, "get_model", lambda name: pytest.fail("no model call is allowed"))
    args.save_forms = False
    assert cases.case_check(args) == 0

    monkeypatch.setattr(cases, "run_hidden_tests", fake_container_work(case, lambda reference: [{**texts[0], "text": TEXT + "!"}]))
    assert cases.case_check(args) == 3
    assert "run case-check --save-forms" in capsys.readouterr().out


# ---------------------------------------------------------------- the campaign's classifier sees infrastructure

def classified(run):
    from evalcore.accounting import classify_exception
    try:
        run()
    except Exception as error:
        return classify_exception(error)[0]
    raise AssertionError("no failure was raised")


def test_a_model_call_that_times_out_is_classified_as_infrastructure(tmp_path):
    case = load_case(make_case(tmp_path))
    text = collected(("m", "message", TEXT))
    assert classified(lambda: step(trial_of(case, Env(text)), scripted({TEXT: [MESSAGE, TimeoutError("slow"), MESSAGE]}))) == "infrastructure"


def test_the_step_cap_is_classified_as_infrastructure(tmp_path, monkeypatch):
    case = load_case(make_case(tmp_path))
    monkeypatch.setattr(reader, "STEP_SECONDS", 0.05)

    async def slow(*args, **kwargs):
        await asyncio.sleep(5)
    monkeypatch.setattr(reader, "read_texts", slow)
    assert classified(lambda: step(trial_of(case, Env(collected(("m", "message", TEXT)))), scripted({}))) == "infrastructure"


def test_a_collector_timeout_is_classified_as_infrastructure(tmp_path):
    class TimedOut(Env):
        async def exec(self, command, **kwargs):
            raise TimeoutError("collector")
    case = load_case(make_case(tmp_path))
    assert classified(lambda: step(trial_of(case, TimedOut("")), scripted({}))) == "infrastructure"


class NoKey(Exception):
    """Stands in for Inspect's PrerequisiteError (no API key) or a ValueError for an unknown provider."""


def _no_model(name):
    raise NoKey(name)


def test_a_model_that_cannot_be_built_is_classified_as_infrastructure(tmp_path, monkeypatch):
    case = load_case(make_case(tmp_path))
    monkeypatch.setattr(reader, "get_model", _no_model)
    trial = trial_of(case, Env(collected(("m", "message", TEXT))))
    assert classified(lambda: asyncio.run(reader.run_reader_step(trial))) == "infrastructure"


def test_read_texts_with_a_model_that_cannot_be_built_exits_3_and_writes_nothing(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(reader, "get_model", _no_model)
    source = tmp_path / "texts.json"
    source.write_text(json.dumps({"texts": [{"id": "a", "kind": "message", "text": TEXT}]}))
    out = tmp_path / "readings.json"
    code = cases.read_texts_command(SimpleNamespace(path=make_case(tmp_path), source=source, out=out))
    assert code == 3 and not out.exists()


def test_case_check_save_forms_with_a_model_that_cannot_be_built_is_an_infrastructure_failure(tmp_path, monkeypatch):
    case = load_case(make_case(tmp_path))
    monkeypatch.setattr(reader, "get_model", _no_model)
    resolve, _ = cases._forms_resolver(case, True)
    with pytest.raises(InfrastructureError):
        resolve("s", [{"id": "a", "kind": "message", "text": TEXT}])


# ---------------------------------------------------------------- follow-ups from #190's review (#193)

def test_a_rescore_whose_readings_are_all_cached_builds_no_model(tmp_path, monkeypatch):
    case = load_case(make_case(tmp_path))
    capture = tmp_path / "capture"
    capture.mkdir()
    text = collected(("m", "message", TEXT))
    step(trial_of(case, Env(text), capture), scripted({TEXT: [MESSAGE] * 3}))
    monkeypatch.setattr(reader, "get_model", _no_model)  # no API key now: the cache must be enough
    metrics = asyncio.run(reader.run_reader_step(trial_of(case, Env(text), capture)))
    assert metrics["cached_texts"] == 1 and metrics["calls"] == 0


def test_the_model_is_built_once_on_the_first_cache_miss(tmp_path):
    rd = reader_of(tmp_path)
    built = []

    def build():
        built.append(1)
        return scripted({TEXT: [MESSAGE] * 3, OTHER_TEXT: [MESSAGE] * 3})
    items = [{"id": "a", "kind": "message", "text": TEXT}, {"id": "b", "kind": "message", "text": OTHER_TEXT}]
    asyncio.run(reader.read_texts(build, rd, items))
    assert built == [1]
    built.clear()
    asyncio.run(reader.read_texts(build, rd, []))
    assert built == []


def test_a_guide_that_is_not_utf8_is_refused_when_the_case_loads(tmp_path):
    path = make_case(tmp_path)
    (path / "hidden/reader/guide.md").write_bytes(b"Fill the form \xff\xfe.\n")
    with pytest.raises(CaseFormatError, match="reader guide"):
        load_case(path)


def test_a_collector_that_prints_bytes_that_are_not_utf8_is_read_with_replacement(tmp_path, monkeypatch):
    """case-check decodes the collector's output with replacement, as campaign scoring does; it does not crash."""
    case = load_case(make_case(tmp_path))
    printed = (b'{"texts": [{"id": "m", "kind": "message", "text": "caf\xff"}]}')
    script = "import sys; sys.stdout.buffer.write(" + repr(printed) + ")"
    real_run = cases.run
    monkeypatch.setattr(cases, "run", lambda command, **kwargs: real_run([sys.executable, "-c", script], **kwargs))
    texts = cases._collect_in_container(case, "ignored")
    assert texts == [{"id": "m", "kind": "message", "text": "caf\ufffd"}]


def test_failed_readings_leave_out_cached_texts_like_calls_and_tokens(tmp_path):
    case = load_case(make_case(tmp_path))
    capture = tmp_path / "capture"
    capture.mkdir()
    text = collected(("m", "message", TEXT))
    first = step(trial_of(case, Env(text), capture), scripted({TEXT: [MESSAGE, "no idea", MESSAGE]}))
    assert first["failed_readings"] == 1 and first["calls"] == 3
    second = step(trial_of(case, Env(text), capture), scripted({}))
    assert (second["cached_texts"], second["calls"], second["input_tokens"], second["failed_readings"]) == (1, 0, 0, 0)


@pytest.mark.parametrize("expect_invalid", [False, True])
def test_stale_saved_readings_have_their_own_exit_code_that_expect_invalid_does_not_flip(tmp_path, monkeypatch, capsys, expect_invalid):
    path = make_case(tmp_path)
    case = load_case(path)
    texts = [{"id": "m", "kind": "message", "text": TEXT}]
    monkeypatch.setattr(cases, "verify_image", lambda ref, tag: "image")
    monkeypatch.setattr(cases, "run_hidden_tests", fake_container_work(case, lambda reference: texts))
    args = SimpleNamespace(path=path, expect_invalid=expect_invalid, ref="r", tag="t", save_forms=False)
    assert cases.case_check(args) == 3  # no saved file at all
    assert "run case-check --save-forms" in capsys.readouterr().out


def test_a_collector_bug_is_still_invalid_and_expect_invalid_still_flips_it(tmp_path, monkeypatch):
    path = make_case(tmp_path)
    case = load_case(path)
    monkeypatch.setattr(cases, "verify_image", lambda ref, tag: "image")
    saved = {"settings_sha256": reader.load_reader(case.reader).settings_sha256, "scenarios": {}}
    case.reader.saved.write_text(json.dumps(saved))

    def broken(*args, **kwargs):
        raise cases.ReaderCollectorFailed("reader collector failed")
    monkeypatch.setattr(cases, "run_hidden_tests", broken)
    for expect_invalid, code in ((False, 1), (True, 0)):
        args = SimpleNamespace(path=path, expect_invalid=expect_invalid, ref="r", tag="t", save_forms=False)
        assert cases.case_check(args) == code


@pytest.mark.parametrize("failure", [RuntimeError("sandbox write failed"), OSError("disk full"), PermissionError("denied")])
def test_a_failed_write_of_forms_is_infrastructure_not_a_failed_trial(tmp_path, failure):
    class WriteFails(Env):
        async def write_file(self, path, content):
            raise failure
    case = load_case(make_case(tmp_path))
    trial = trial_of(case, WriteFails(collected(("m", "message", TEXT))))
    assert classified(lambda: step(trial, scripted({TEXT: [MESSAGE] * 3}))) == "infrastructure"
    assert trial.evidence["copy_log"] == []


def test_the_warning_for_a_failed_forms_write_names_the_reason(tmp_path, caplog):
    class WriteFails(Env):
        async def write_file(self, path, content):
            raise OSError("disk full")
    trial = trial_of(load_case(make_case(tmp_path)), WriteFails(collected(("m", "message", TEXT))))
    with caplog.at_level(logging.WARNING), pytest.raises(InfrastructureError):
        step(trial, scripted({TEXT: [MESSAGE] * 3}))
    assert "OSError: disk full" in caplog.text


def test_a_bug_in_building_the_forms_document_is_not_scored_as_infrastructure(tmp_path, monkeypatch):
    monkeypatch.setattr(reader, "forms_document", lambda texts, readings: {"bad": object()})
    trial = trial_of(load_case(make_case(tmp_path)), Env(collected(("m", "message", TEXT))))
    assert classified(lambda: step(trial, scripted({TEXT: [MESSAGE] * 3}))) != "infrastructure"


def test_a_snapshot_replay_of_a_case_with_a_reader_says_it_cannot_be_replayed(tmp_path):
    from scevals import snapshots
    case = load_case(make_case(tmp_path))
    directory = tmp_path / "snapshots"
    directory.mkdir()
    (directory / "snapshots.json").write_text(json.dumps([{"sequence": 1, "file": "snapshot-000001.tar", "elapsed_seconds": 1.0}]))
    ran = []

    async def replay(archive, case_arg, image):
        ran.append(archive)
        return {"tests": []}
    with pytest.raises(ValueError, match="reader"):
        asyncio.run(snapshots.replay_snapshots(directory, case, "image:test", replay=replay))
    with pytest.raises(ValueError, match="reader"):
        asyncio.run(snapshots.replay_hidden_tests(directory / "snapshot-000001.tar", case, "image:test"))
    assert ran == []


# ---------------------------------------------------------------- case-check keeps a log (principle 14)

def case_check_lines():
    log = cases.case_check_log()
    return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []


def test_case_check_appends_one_json_line_per_run_with_the_outcome_exit_code_and_time(tmp_path, monkeypatch):
    path = make_case(tmp_path)
    case = load_case(path)
    texts = [{"id": "m", "kind": "message", "text": TEXT}]
    monkeypatch.setattr(cases, "verify_image", lambda ref, tag: "image")
    monkeypatch.setattr(cases, "run_hidden_tests", fake_container_work(case, lambda reference: texts))
    args = SimpleNamespace(path=path, expect_invalid=False, ref="r", tag="t", save_forms=False)
    assert cases.case_check(args) == 3  # stale: no saved readings yet
    monkeypatch.setattr(reader, "get_model", lambda name: scripted({TEXT: [MESSAGE] * 3}))
    args.save_forms = True
    assert cases.case_check(args) == 0

    def failing(*a, **k):
        raise InfrastructureError("sandbox gone")
    monkeypatch.setattr(cases, "run_hidden_tests", failing)
    assert cases.case_check(args) == 2
    monkeypatch.setattr(cases, "run_hidden_tests", lambda *a, **k: (_ for _ in ()).throw(cases.ReaderCollectorFailed("bad collector")))
    args.expect_invalid = True
    assert cases.case_check(args) == 0  # invalid, as expected

    lines = case_check_lines()
    assert [(l["outcome"], l["exit_code"]) for l in lines] == [("stale", 3), ("ok", 0), ("failed", 2), ("invalid", 0)]
    assert all(l["case"] == str(path) for l in lines)
    assert all(datetime.datetime.fromisoformat(l["time"]).utcoffset() == datetime.timedelta(0) for l in lines)


def test_case_check_logs_a_run_that_stops_before_the_image_check(tmp_path, monkeypatch):
    def missing(*args):
        raise RuntimeError("no image")
    monkeypatch.setattr(cases, "verify_image", missing)
    path = make_case(tmp_path)
    assert cases.case_check(SimpleNamespace(path=path, expect_invalid=False, ref="r", tag="t", save_forms=False)) == 2
    assert [(l["outcome"], l["exit_code"]) for l in case_check_lines()] == [("failed", 2)]


def test_a_case_check_that_crashes_is_logged_as_failed_and_still_raises(tmp_path, monkeypatch):
    monkeypatch.setattr(cases, "_case_check", lambda args: 1 / 0)
    with pytest.raises(ZeroDivisionError):
        cases.case_check(SimpleNamespace(path=tmp_path, expect_invalid=False))
    assert [l["outcome"] for l in case_check_lines()] == ["failed"]


def test_a_log_that_cannot_be_written_does_not_hide_the_result(tmp_path, monkeypatch, capsys):
    blocker = tmp_path / "state-is-a-file"
    blocker.write_text("")
    monkeypatch.setenv("XDG_STATE_HOME", str(blocker))
    monkeypatch.setattr(cases, "_case_check", lambda args: (3, "stale"))
    assert cases.case_check(SimpleNamespace(path=tmp_path)) == 3
    assert "case-check log not written" in capsys.readouterr().err
