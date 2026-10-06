"""Software-change case format and mount-free reference/no-op validation."""
import argparse
import ast
from collections import Counter
from dataclasses import dataclass
from fnmatch import fnmatchcase
import json
from pathlib import Path
import re
import subprocess
import tomllib

from .image import DEFAULT_IMAGE, DEFAULT_REF, build_image, output, run, trial_container

RUNTIME = Path(__file__).with_name("case_runtime.py")
REQUIREMENT = r"REQ-[A-Z0-9-]+"


class CaseFormatError(ValueError):
    """A fixed, export-safe explanation of an invalid case."""


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
        return EvalCase(path, case_id, version, tuple(scope), tuple(requirements), tuple(tags), timeout)
    except CaseFormatError:
        raise
    except (KeyError, TypeError, ValueError, SyntaxError, UnicodeError):
        raise CaseFormatError("malformed case metadata or test tags") from None


def run_hidden_tests(case: EvalCase, *, reference: bool, image=DEFAULT_IMAGE):
    """For case-check only; campaign scoring must enforce its own terminal gate.

    Return per-test statuses plus copy/config evidence, without source or failure
    messages. Both attempts start in a fresh container. The brief never enters.
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


def case_check(args):
    try:
        case = load_case(args.path)
    except CaseFormatError as error:
        print(f"INVALID: {error}")
        return 0 if args.expect_invalid else 1
    # Build even when a cached image exists, so the pinned source identity is checked.
    built = build_image(args.ref, args.tag)
    reference = run_hidden_tests(case, reference=True, image=args.tag)
    noop = run_hidden_tests(case, reference=False, image=args.tag)
    reasons = check_results(case, reference, noop)
    valid = not reasons
    receipt = {"id": case.id, "version": case.version, "valid": valid, "reasons": reasons,
               "image": built, "reference": _summary(case, reference), "no_op": _summary(case, noop)}
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
    parser.set_defaults(handler=case_check)
    # Reuse the installed scevals command plugin; no new entry-point sync needed.
    from .calibration import register_subcommands as register_calibration
    register_calibration(subparsers)
    from .demo import register_subcommands as register_demo
    register_demo(subparsers)
    from .results import register_subcommands as register_results
    register_results(subparsers)
