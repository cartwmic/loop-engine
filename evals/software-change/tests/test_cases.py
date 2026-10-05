"""Case-format checks and black-box CLI proof in real mount-free containers."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

import pytest

from scevals.cases import CaseFormatError, check_results, load_case, run_hidden_tests

ROOT = Path(__file__).resolve().parents[3]
TARGET = ROOT / "evals/software-change"
EXAMPLE = TARGET / "example-case"
BROKEN = TARGET / "tests/fixtures/broken-case"
CAPTURES = TARGET / "captures/T3-tests"


@pytest.fixture
def case_copy():
    CAPTURES.mkdir(parents=True, exist_ok=True)
    destination = Path(tempfile.mkdtemp(prefix="case-", dir=CAPTURES)) / "case"
    shutil.copytree(EXAMPLE, destination)
    return destination


def test_example_metadata_and_hidden_requirements():
    case = load_case(EXAMPLE)
    assert case.id == "label-keys" and case.version == "1"
    assert case.allowed_edit_scope == ("labels.py",)
    assert case.requirement_ids == ("REQ-1", "REQ-2", "REQ-3")
    assert len(case.test_requirements) == 6
    assert "casefold" not in case.request.read_text()
    assert "inbox" not in case.request.read_text()
    assert "casefold" not in (case.repository / "labels.py").read_text()
    assert "inbox" not in (case.repository / "labels.py").read_text()


@pytest.mark.parametrize("problem", ["missing-brief", "scope", "traversal", "untagged", "unknown-tag",
                                      "untested", "duplicate-heading", "syntax", "symlink", "empty-tests"])
def test_format_rejects_bad_cases(case_copy, problem):
    manifest = case_copy / "case.toml"
    brief = case_copy / "hidden/brief.md"
    tests = case_copy / "hidden/tests/test_labels.py"
    if problem == "missing-brief":
        brief.unlink()
    elif problem == "scope":
        manifest.write_text(manifest.read_text().replace("labels.py", "other.py"))
    elif problem == "traversal":
        manifest.write_text(manifest.read_text().replace("labels.py", "../labels.py"))
    elif problem == "untagged":
        tests.write_text(tests.read_text().replace('    @requirement("REQ-1")\n', "", 1))
    elif problem == "unknown-tag":
        tests.write_text(tests.read_text().replace('"REQ-1"', '"REQ-UNKNOWN"', 1))
    elif problem == "untested":
        brief.write_text(brief.read_text() + "\n## REQ-4: Uncovered\n")
    elif problem == "duplicate-heading":
        brief.write_text(brief.read_text() + "\n## REQ-1: Duplicate\n")
    elif problem == "syntax":
        tests.write_text("this is invalid python !")
    elif problem == "symlink":
        (case_copy / "public/repo/alias.py").symlink_to("labels.py")
    elif problem == "empty-tests":
        tests.unlink()
    with pytest.raises(CaseFormatError):
        load_case(case_copy)


def test_loading_never_executes_case_code(case_copy):
    sentinel = case_copy / "executed"
    tests = case_copy / "hidden/tests/test_labels.py"
    tests.write_text(f"raise RuntimeError({str(sentinel)!r})\n" + tests.read_text())
    assert load_case(case_copy).id == "label-keys"
    assert not sentinel.exists()


def test_result_checks_fail_closed():
    case = load_case(EXAMPLE)

    def report(status):
        return {"collection_error": False, "tests": [{"requirement_id": tag, "status": status}
                                                      for tag in case.test_requirements]}

    assert check_results(case, report("passed"), report("failed")) == []
    assert check_results(case, report("failed"), report("failed"))
    assert check_results(case, report("passed"), report("passed"))
    assert check_results(case, report("passed"), report("skipped"))
    collection = report("passed")
    collection["collection_error"] = True
    assert check_results(case, collection, report("failed"))
    missing = report("passed")
    missing["tests"].pop()
    assert check_results(case, missing, report("failed"))


def cli_check(case, expect_invalid, capture_name):
    command = ["uv", "run", "--project", "evals", "evalctl", "case-check"]
    if expect_invalid:
        command.append("--expect-invalid")
    command.append(str(case.relative_to(ROOT)))
    process = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, timeout=240)
    CAPTURES.mkdir(parents=True, exist_ok=True)
    (CAPTURES / f"{capture_name}.log").write_text(process.stdout + process.stderr)
    (CAPTURES / f"{capture_name}.exit").write_text(f"{process.returncode}\n")
    receipts = [line.removeprefix("CASE_CHECK ") for line in process.stdout.splitlines() if line.startswith("CASE_CHECK ")]
    assert len(receipts) == 1, f"see {capture_name}.log"
    return process.returncode, json.loads(receipts[0])


@pytest.fixture(scope="module")
def official_checks():
    return {"example": cli_check(EXAMPLE, False, "example"),
            "broken": cli_check(BROKEN, False, "broken-normal"),
            "expected_broken": cli_check(BROKEN, True, "broken-expected"),
            "unexpected_valid": cli_check(EXAMPLE, True, "valid-expected-invalid")}


def test_example_black_box(official_checks):
    code, receipt = official_checks["example"]
    assert code == 0 and receipt["valid"]
    assert receipt["reference"]["passed"] == receipt["reference"]["total"] == 6
    assert receipt["no_op"]["passed"] == 0 and receipt["no_op"]["total"] == 6
    assert all(counts == {"passed": 2, "total": 2} for counts in receipt["reference"]["requirements"].values())
    for attempt in ("reference", "no_op"):
        config = receipt[attempt]["container"]
        assert config["mounts"] == []
        assert all(copy["kind"] != "hidden_brief" for copy in config["copies"])
    assert "reference_overlay" not in [copy["kind"] for copy in receipt["no_op"]["container"]["copies"]]


def test_broken_black_box_and_exit_contract(official_checks):
    code, receipt = official_checks["broken"]
    assert code == 1 and not receipt["valid"]
    assert receipt["reference"]["passed"] == 5 and receipt["reference"]["total"] == 6
    assert receipt["reference"]["requirements"]["REQ-2"] == {"passed": 1, "total": 2}
    assert "reference: not every tagged hidden test passed" in receipt["reasons"]
    assert official_checks["expected_broken"][0] == 0
    assert official_checks["unexpected_valid"][0] == 1


@pytest.mark.parametrize("scenario", ["class-fixture", "setup-error", "skipped", "import-error", "subtest-error"])
def test_real_container_runner_edge_cases(case_copy, official_checks, scenario):
    test_file = case_copy / "hidden/tests/test_labels.py"
    text = test_file.read_text()
    if scenario == "class-fixture":
        text = text.replace("class LabelKeyTests(unittest.TestCase):", """class LabelKeyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ready = True

    def setUp(self):
        self.assertTrue(self.ready)
""")
    elif scenario == "setup-error":
        text = text.replace("class LabelKeyTests(unittest.TestCase):", """class LabelKeyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        raise RuntimeError("private-error-canary")
""")
    elif scenario == "skipped":
        text = text.replace('    @requirement(', '    @unittest.skip("private-skip-canary")\n    @requirement(')
    elif scenario == "import-error":
        text = 'raise RuntimeError("private-import-canary")\n' + text
    elif scenario == "subtest-error":
        text = text.replace('        self.assertEqual(label_key("MiXeD"), "mixed")', '''        with self.subTest(case="private-subtest-canary"):
            self.assertEqual(label_key("MiXeD"), "wrong")''')
    test_file.write_text(text)
    case = load_case(case_copy)
    report = run_hidden_tests(case, reference=True)
    encoded = json.dumps(report)
    assert "canary" not in encoded
    if scenario == "class-fixture":
        assert not report["collection_error"]
        assert all(test["status"] == "passed" for test in report["tests"])
    elif scenario in ("setup-error", "import-error"):
        assert report["collection_error"]
    elif scenario == "skipped":
        assert all(test["status"] == "skipped" for test in report["tests"])
        assert check_results(case, report, report)
    elif scenario == "subtest-error":
        assert sum(test["status"] == "failed" for test in report["tests"]) == 1


def test_expect_invalid_does_not_swallow_infrastructure():
    command = ["uv", "run", "--project", "evals", "evalctl", "case-check", "--expect-invalid",
               "--ref", "refs/does-not-exist-t3-proof", str(EXAMPLE.relative_to(ROOT))]
    process = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, timeout=30)
    CAPTURES.mkdir(parents=True, exist_ok=True)
    (CAPTURES / "infrastructure.log").write_text(process.stdout + process.stderr)
    assert process.returncode == 2
    assert "CASE_CHECK " not in process.stdout
