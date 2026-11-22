"""Software-change case format and mount-free reference/no-op validation."""
import argparse
import ast
import asyncio
from collections import Counter
from dataclasses import dataclass
from fnmatch import fnmatchcase
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib


from evalcore.target import InfrastructureError
from .reader import (COLLECTOR_SECONDS, FORMS_FILE, ReaderCallError, ReaderInputError, SavedFormsStale, forms_document, load_reader, load_saved, reader_model,
                     load_shapes, lookup_saved, parse_collected, read_texts, saved_entries)
from .image import DEFAULT_IMAGE, DEFAULT_REF, verify_image, output, run, trial_container

RUNTIME = Path(__file__).with_name("case_runtime.py")
STALE_EXIT = 3  # case-check: the saved readings are stale, so the case was not checked (1 is invalid, 2 infrastructure)
REQUIREMENT = r"REQ-[A-Z0-9-]+"


class CaseFormatError(ValueError):
    """A fixed, export-safe explanation of an invalid case."""


class ReaderCollectorFailed(CaseFormatError):
    """The case's collector failed or printed something that is not the documented shape: a case bug."""


@dataclass(frozen=True)
class ReaderSpec:
    """A case's optional reader (CASE-FORMAT.md): the pinned model and the case files it reads, all host-side."""
    model: str
    guide: Path
    shapes: Path
    collector: str  # a file name inside hidden/tests; it runs in the scoring sandbox
    saved: Path     # saved forms for case-check; may not exist yet


@dataclass(frozen=True)
class EvalCase:
    path: Path
    id: str
    version: str
    allowed_edit_scope: tuple[str, ...]
    requirement_ids: tuple[str, ...]
    test_requirements: tuple[str, ...]
    # Hidden-test run limit; Rust cases need a cold build first.
    test_timeout_seconds: int = 60
    reader: ReaderSpec | None = None

    @property
    def repository(self):
        return self.path / "public/repo"

    @property
    def request(self):
        return self.path / "public/request.md"

    @property
    def brief(self):
        return self.path / "hidden/brief.md"

    @property
    def tests(self):
        return self.path / "hidden/tests"

    @property
    def reference(self):
        return self.path / "hidden/reference"


def _relative_path(value):
    return (isinstance(value, str) and bool(value) and not value.startswith("/")
            and not any(part in ("", ".", "..", ".git") for part in value.split("/"))
            and "\\" not in value)


def _reader_spec(path, table):
    """Read `[reader]` as data. The files must exist (but `saved`, which `case-check --save-forms` creates)."""
    keys = {"model", "guide", "shapes", "collector", "saved"}
    if not isinstance(table, dict) or set(table) != keys or not all(isinstance(v, str) and v for v in table.values()):
        raise CaseFormatError("[reader] needs exactly model, guide, shapes, collector and saved as nonempty strings")
    for key in ("guide", "shapes", "saved"):
        if not _relative_path(table[key]):
            raise CaseFormatError(f"reader {key} must be a relative path inside the case")
    if not _relative_path(table["collector"]) or "/" in table["collector"]:
        raise CaseFormatError("reader collector must be a file name in hidden/tests")
    guide, shapes, saved = (path / table[key] for key in ("guide", "shapes", "saved"))
    collector = path / "hidden/tests" / table["collector"]
    if not guide.is_file() or not shapes.is_file() or not collector.is_file():
        raise CaseFormatError("reader guide, shapes or collector file is missing")
    try:
        load_shapes(shapes)
    except ReaderInputError as error:
        raise CaseFormatError(f"reader shapes: {error}") from None
    try:
        guide.read_text()  # read at load, so a guide that is not UTF-8 is a case bug here and not a failed trial later
    except (OSError, UnicodeError):
        raise CaseFormatError("reader guide is not readable UTF-8 text") from None
    return ReaderSpec(table["model"], guide, shapes, table["collector"], saved)


def load_case(path: Path) -> EvalCase:
    """Read metadata/tags only. Never import or execute case code on the host."""
    path = Path(path).resolve()
    required_files = ("case.toml", "public/request.md", "hidden/brief.md")
    required_dirs = ("public/repo", "hidden/tests", "hidden/reference")
    if not path.is_dir() or any(not (path / p).is_file() for p in required_files):
        raise CaseFormatError("missing case metadata, request or brief")
    if any(not (path / p).is_dir() for p in required_dirs):
        raise CaseFormatError("missing repository, tests or reference directory")
    if any(p.is_symlink() for p in path.rglob("*")):
        raise CaseFormatError("case symlinks are not supported")
    try:
        metadata = tomllib.loads((path / "case.toml").read_text())
        case_id, version = metadata["id"], metadata["version"]
        scope = metadata["allowed_edit_scope"]
        if not isinstance(case_id, str) or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9._-]*", case_id):
            raise CaseFormatError("case id must be a safe identifier")
        if not isinstance(version, str) or not version.strip():
            raise CaseFormatError("case version must be a nonempty string")
        if not isinstance(scope, list) or not scope or not all(_relative_path(p) for p in scope):
            raise CaseFormatError("allowed_edit_scope must contain relative path patterns")
        brief = (path / "hidden/brief.md").read_text()
        requirements = re.findall(rf"^## ({REQUIREMENT})(?=\s|:|$)", brief, re.MULTILINE)
        if not requirements or len(requirements) != len(set(requirements)):
            raise CaseFormatError("brief needs unique ## REQ-* headings")
        if not (path / "public/request.md").read_text().strip():
            raise CaseFormatError("request must not be empty")
        tags = []
        for test_file in sorted((path / "hidden/tests").rglob("test_*.py")):
            tree = ast.parse(test_file.read_text())
            for node in ast.walk(tree):
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith("test_"):
                    decorators = [d for d in node.decorator_list if isinstance(d, ast.Call)
                                  and isinstance(d.func, ast.Name) and d.func.id == "requirement"]
                    if (len(decorators) != 1 or len(decorators[0].args) != 1
                            or decorators[0].keywords or isinstance(node, ast.AsyncFunctionDef)):
                        raise CaseFormatError("every synchronous test needs one @requirement tag")
                    tag = ast.literal_eval(decorators[0].args[0])
                    if not isinstance(tag, str) or tag not in requirements:
                        raise CaseFormatError("test tag does not identify a brief requirement")
                    tags.append(tag)
        if set(tags) != set(requirements):
            raise CaseFormatError("every brief requirement must have a tagged test")
        reference_files = [p for p in (path / "hidden/reference").rglob("*") if p.is_file()]
        if not reference_files:
            raise CaseFormatError("reference overlay must contain files")
        for file in reference_files:
            relative = file.relative_to(path / "hidden/reference").as_posix()
            if not _relative_path(relative) or not any(fnmatchcase(relative, pattern) for pattern in scope):
                raise CaseFormatError("reference edits outside allowed scope")
        timeout = metadata.get("test_timeout_seconds", 60)
        if not isinstance(timeout, int) or isinstance(timeout, bool) or not 1 <= timeout <= 7200:
            raise CaseFormatError("test_timeout_seconds must be an integer from 1 to 7200")
        reader = _reader_spec(path, metadata["reader"]) if "reader" in metadata else None
        return EvalCase(path, case_id, version, tuple(scope), tuple(requirements), tuple(tags), timeout, reader)
    except CaseFormatError:
        raise
    except (KeyError, TypeError, ValueError, SyntaxError, UnicodeError):
        raise CaseFormatError("malformed case metadata or test tags") from None


def _collect_in_container(case, container):
    """Run the case's collector in the check container, as scoring does. Returns the collected texts."""
    try:
        done = run(["docker", "exec", "--workdir", "/workspace/repo", container, "python3",
                    f"/opt/case-check/tests/{case.reader.collector}"],
                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=COLLECTOR_SECONDS,
                   errors="replace")  # as campaign scoring decodes: bytes that are not UTF-8 never crash the check
        return parse_collected(done.stdout, load_shapes(case.reader.shapes))
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, ReaderInputError):
        raise ReaderCollectorFailed("reader collector failed") from None


def run_hidden_tests(case: EvalCase, *, reference: bool, image=DEFAULT_IMAGE, resolve_forms=None):
    """For case-check only; campaign scoring must enforce its own terminal gate.

    Return per-test statuses plus copy/config evidence, without source or failure
    messages. Both attempts start in a fresh container. The brief never enters.
    A case with a reader gets `forms.json` from `resolve_forms(scenario, texts)` before its tests run.
    """
    with trial_container(image) as (container, config):
        run(["docker", "exec", container, "mkdir", "-p", "/workspace/repo", "/opt/case-check/tests"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        copies = []

        def copy(source, destination, kind):
            run(["docker", "cp", str(source), f"{container}:{destination}"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            copies.append({"kind": kind, "destination": destination})

        copy(str(case.repository) + "/.", "/workspace/repo", "public_repository")
        if reference:
            copy(str(case.reference) + "/.", "/workspace/repo", "reference_overlay")
        copy(str(case.tests) + "/.", "/opt/case-check/tests", "hidden_tests")
        copy(RUNTIME, "/opt/case-check/case_runtime.py", "test_runner")
        if case.reader is not None:
            forms = resolve_forms("reference" if reference else "noop", _collect_in_container(case, container))
            run(["docker", "exec", "-i", container, "sh", "-c", f"cat > /opt/case-check/tests/{FORMS_FILE}"],
                input=json.dumps(forms), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            copies.append({"kind": "reader_forms", "destination": f"/opt/case-check/tests/{FORMS_FILE}"})
        run(["docker", "exec", "--workdir", "/workspace/repo", container, "python3",
             "/opt/case-check/case_runtime.py", "--tests", "/opt/case-check/tests",
             "--output", "/tmp/case-results.json"], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            timeout=case.test_timeout_seconds)
        report = json.loads(output(["docker", "exec", container, "cat", "/tmp/case-results.json"]))
        report["container"] = {"image_id": config["Image"], "mounts": config["Mounts"],
                               "copies": copies}
        return report


def check_results(case, reference, noop):
    reasons = []
    for name, result in (("reference", reference), ("no-op", noop)):
        tests = result["tests"]
        if result["collection_error"] or Counter(t["requirement_id"] for t in tests) != Counter(case.test_requirements):
            reasons.append(f"{name}: tagged test collection differs from case format")
        if any(t["status"] in ("skipped", "not_run") for t in tests):
            reasons.append(f"{name}: every tagged test must execute (no skips)")
    if not reference["tests"] or any(t["status"] != "passed" for t in reference["tests"]):
        reasons.append("reference: not every tagged hidden test passed")
    if not any(t["status"] in ("failed", "error") for t in noop["tests"]):
        reasons.append("no-op: at least one tagged hidden test must fail")
    return reasons


def _summary(case, result):
    tests = result["tests"]
    return {"passed": sum(t["status"] == "passed" for t in tests), "total": len(tests),
            "requirements": {tag: {"passed": sum(t["status"] == "passed" for t in tests if t["requirement_id"] == tag),
                                   "total": sum(t["requirement_id"] == tag for t in tests)}
                             for tag in case.requirement_ids}, "container": result["container"]}


# Deliberately small/common vocabulary and runner-only names, not feature names.
INTERFACE_STOPLIST = {
    "true", "false", "none", "null", "test", "tests", "main", "mixed", "value", "values",
    "name", "text", "data", "result", "error", "message", "python", "python3", "pytest",
    "unittest", "case_runtime", "requirement", "cargo", "rustc", "build", "release",
    "--quiet", "--offline", "--manifest-path", "--target-dir", "--nocapture",
    "stdout", "stderr", "utf-8", "encoding", "check", "capture_output", "timeout",
    "texts", "kind", "forms", "form",  # the reader's collector and forms.json keys: the harness's own, not the case's
}


def unstated_interfaces(case):
    """Static heuristic only: never execute hidden tests or consult reference code."""
    metadata = tomllib.loads((case.path / "case.toml").read_text())
    allowlist = metadata.get("interface_allowlist", [])
    if not isinstance(allowlist, list) or not all(isinstance(x, str) for x in allowlist):
        raise CaseFormatError("interface_allowlist must be a list of strings")
    names = set()
    for file in sorted(case.tests.rglob("*.py")):
        tree = ast.parse(file.read_text())
        for node in ast.walk(tree):
            if not isinstance(node, ast.Constant) or not isinstance(node.value, str):
                continue
            # Also finds dict/JSON/json! keys in embedded source or JSON payloads.
            literals = [node.value, *re.findall(r'''["']([A-Za-z_][A-Za-z0-9_-]*|--[A-Za-z][A-Za-z0-9_-]*)["']''', node.value)]
            for value in literals:
                if (len(value.removeprefix("--")) >= 4 and
                        re.fullmatch(r"(?:--)?[A-Za-z_][A-Za-z0-9_-]*", value) and
                        value.lower() not in INTERFACE_STOPLIST and
                        not value.startswith("REQ-") and value not in allowlist):
                    names.add(value)
    public = [case.request, case.brief, *(p for p in case.repository.rglob("*")
                                        if p.is_file() and ".git" not in p.relative_to(case.repository).parts)]
    corpus = "\n".join(p.read_bytes().decode("utf-8", errors="replace") for p in public)
    return sorted(name for name in names if not re.search(
        r"(?<![A-Za-z0-9_-])" + re.escape(name) + r"(?![A-Za-z0-9_-])", corpus, re.IGNORECASE))


def _forms_resolver(case, save):
    """How case-check gets forms: read them now with the model (--save-forms, remembered in `saved`) or look them up."""
    reader = load_reader(case.reader)
    saved = {"settings_sha256": reader.settings_sha256, "scenarios": {}} if save else load_saved(case.reader.saved)

    def resolve(scenario, texts):
        if not save:
            return lookup_saved(saved, reader, scenario, texts)
        items = [e for e in texts if e["text"] is not None]
        readings = asyncio.run(read_texts(lambda: reader_model(reader), reader, items))
        if any(r.form is None for r in readings):
            raise InfrastructureError("a text has no majority reading")
        saved["scenarios"][scenario] = saved_entries(texts, readings)
        return forms_document(texts, readings)
    return resolve, saved


def case_check(args):
    try:
        case = load_case(args.path)
        unstated = unstated_interfaces(case)
        if unstated:
            raise CaseFormatError("hidden tests depend on unstated interface identifiers: " + ", ".join(unstated[:20]))
    except CaseFormatError as error:
        print(f"INVALID: {error}")
        return 0 if args.expect_invalid else 1
    try:
        built = verify_image(args.ref, args.tag)
    except RuntimeError:
        print("Trial image missing or mismatched; run image-check first.")
        return 2
    resolve_forms, saved = None, None
    try:
        if case.reader is not None:
            resolve_forms, saved = _forms_resolver(case, args.save_forms)
        reference = run_hidden_tests(case, reference=True, image=args.tag, resolve_forms=resolve_forms)
        noop = run_hidden_tests(case, reference=False, image=args.tag, resolve_forms=resolve_forms)
    except SavedFormsStale as error:  # not a verdict on the case: exit 3 whatever --expect-invalid says
        print(f"Not checked: {error}")
        return STALE_EXIT
    except ReaderCollectorFailed as error:
        print(f"INVALID: {error}")
        return 0 if args.expect_invalid else 1
    except InfrastructureError as error:
        print(f"Reader failed ({error}); nothing was written.")
        return 2
    if args.save_forms and saved is not None:
        case.reader.saved.parent.mkdir(parents=True, exist_ok=True)
        case.reader.saved.write_text(json.dumps(saved, indent=1, sort_keys=True) + "\n")
    reasons = check_results(case, reference, noop)
    valid = not reasons
    receipt = {"id": case.id, "version": case.version, "valid": valid, "reasons": reasons,
               "image": built, "reference": _summary(case, reference), "no_op": _summary(case, noop)}
    if case.reader is not None:
        receipt["reader"] = {"model": case.reader.model, "forms": "read just now" if args.save_forms else "saved"}
    for name, result in (("reference", reference), ("no-op", noop)):
        summary = _summary(case, result)
        print(f"{name}: {summary['passed']}/{summary['total']} tagged hidden tests passed")
        for tag, counts in summary["requirements"].items():
            print(f"  {tag}: {counts['passed']}/{counts['total']} passed")
    print(f"{'VALID' if valid else 'INVALID'}: {case.id}")
    for reason in reasons:
        print(f"  reason: {reason}")
    print("CASE_CHECK " + json.dumps(receipt, sort_keys=True))
    if args.expect_invalid:
        return 1 if valid else 0
    return 0 if valid else 1


def register_subcommands(subparsers: argparse._SubParsersAction):
    parser = subparsers.add_parser("case-check", help="validate reference and no-op in mount-free trial containers")
    parser.add_argument("path", type=Path)
    parser.add_argument("--expect-invalid", action="store_true", help="succeed only for an invalid case (not infrastructure errors)")
    parser.add_argument("--ref", default=DEFAULT_REF)
    parser.add_argument("--tag", default=DEFAULT_IMAGE)
    parser.add_argument("--save-forms", action="store_true",
                        help="a case with a reader: read the collected texts with the model and rewrite the saved forms")
    parser.set_defaults(handler=case_check)
    read = subparsers.add_parser("read-texts", help="read texts with a case's reader (a case's reader check)")
    read.add_argument("path", type=Path)
    read.add_argument("--in", dest="source", type=Path, required=True)
    read.add_argument("--out", type=Path, required=True)
    read.set_defaults(handler=read_texts_command)
    # Reuse the installed scevals command plugin; no new entry-point sync needed.
    from .calibration import register_subcommands as register_calibration
    register_calibration(subparsers)
    from .demo import register_subcommands as register_demo
    register_demo(subparsers)
    from .results import register_subcommands as register_results
    register_results(subparsers)
    from .lifecycle import register_subcommands as register_lifecycle
    register_lifecycle(subparsers)


def read_texts_command(args):
    """`evalctl read-texts CASE --in FILE --out FILE`. Exit 0 written, 2 bad case or input, 3 a model call failed."""
    try:
        case = load_case(args.path)
        if case.reader is None:
            raise ReaderInputError("the case has no [reader]")
        reader = load_reader(case.reader)
        try:
            data = json.loads(args.source.read_text())
        except (OSError, ValueError):
            raise ReaderInputError("input is not a readable JSON file") from None
        texts = data.get("texts") if isinstance(data, dict) and set(data) == {"texts"} else None
        if not isinstance(texts, list):
            raise ReaderInputError('input must be {"texts": [...]}')
        seen = set()
        for entry in texts:
            if (not isinstance(entry, dict) or set(entry) != {"id", "kind", "text"} or not isinstance(entry["id"], str)
                    or not entry["id"] or not isinstance(entry["text"], str) or not entry["text"]):
                raise ReaderInputError("each text needs an id, a kind and a nonempty string text")
            if entry["id"] in seen:
                raise ReaderInputError(f"duplicate id: {entry['id']}")
            if not isinstance(entry["kind"], str) or entry["kind"] not in reader.shapes:
                raise ReaderInputError(f"unknown kind for id {entry['id']}")
            seen.add(entry["id"])
    except (CaseFormatError, ReaderInputError) as error:
        print(f"read-texts: {error}", file=sys.stderr)
        return 2
    try:
        readings = asyncio.run(read_texts(lambda: reader_model(reader), reader, texts))
    except ReaderCallError:
        print("read-texts: a model call failed; nothing was written", file=sys.stderr)
        return 3
    args.out.write_text(json.dumps({"settings_sha256": reader.settings_sha256, "model": reader.model,
                                    "results": [r.export() for r in readings]},
                                   indent=1) + "\n")
    return 0
