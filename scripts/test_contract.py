"""Shared discovery and validation helpers for the workspace test contracts."""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any, Iterable, NoReturn

ROOT = Path(__file__).resolve().parents[1]

# These are the binaries exercised by the current black-box integration
# suites.  Keep the production and fixture package names separate: two fixture
# binaries intentionally have the same executable names as production-facing
# provider aliases only at the process boundary.
REQUIRED_BINARIES = (
    ("bookends-check", "bookends-check", "production"),
    ("loop-cli", "loop-engine", "production"),
    ("policy-document-provider", "policy-document", "production"),
    ("research-provider", "research", "production"),
    ("software-change-provider", "software-change", "production"),
    ("loop-reference-fixtures", "policy-document-provider", "reference-fixture"),
    ("loop-reference-fixtures", "research-provider", "reference-fixture"),
    ("loop-reference-fixtures", "software-change-provider", "reference-fixture"),
)


class ContractError(RuntimeError):
    """A repository-owned test contract is not satisfied."""


def fail(message: str) -> NoReturn:
    raise ContractError(message)


def relpath(path: Path, repo: Path) -> str:
    """Return a stable repository-relative path when possible."""
    try:
        return path.resolve().relative_to(repo.resolve()).as_posix()
    except ValueError:
        return str(path.resolve())


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def repository_proof_identity(repo: Path) -> str:
    """Compact current-tree identity for local command receipts (not a policy).

    HEAD/path names alone miss an edit to an already modified file. Include
    Git's index/status and actual tracked/untracked contents, without asking
    the driver to maintain a second file inventory. Ignored build outputs are
    deliberately outside repository proof identity.
    """
    def git(*args: str) -> bytes:
        return subprocess.check_output(["git", *args], cwd=repo, stderr=subprocess.PIPE)

    digest = hashlib.sha256()

    def add(value: bytes) -> None:
        digest.update(len(value).to_bytes(8, "big"))
        digest.update(value)

    add(git("rev-parse", "HEAD"))
    add(git("ls-files", "--stage", "-z"))
    add(git("status", "--porcelain=v1", "--untracked-files=all", "-z"))
    paths = sorted(set(git("ls-files", "--cached", "--others", "--exclude-standard", "-z").split(b"\0")) - {b""})
    for name in paths:
        path = repo / os.fsdecode(name)
        add(name)
        if path.is_symlink():
            add(b"symlink")
            add(os.fsencode(path.readlink()))
        elif path.is_file():
            add(str(path.stat().st_mode & 0o777).encode())
            add(path.read_bytes())
        elif not path.exists():
            add(b"deleted")
        else:
            fail(f"unsupported repository proof entry: {path}")
    return "sha256:" + digest.hexdigest()


def cargo_metadata(repo: Path) -> dict[str, Any]:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "no diagnostic"
        fail(f"cargo metadata failed with exit {result.returncode}: {detail}")
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        fail(f"cargo metadata returned invalid JSON: {error}")
    if not isinstance(value, dict):
        fail("cargo metadata result must be an object")
    return value


def package_maps(metadata: dict[str, Any], repo: Path) -> tuple[dict[str, str], dict[str, dict[str, Any]]]:
    manifest_to_name: dict[str, str] = {}
    name_to_package: dict[str, dict[str, Any]] = {}
    packages = metadata.get("packages")
    if not isinstance(packages, list):
        fail("cargo metadata packages must be an array")
    for package in packages:
        if not isinstance(package, dict):
            fail("cargo metadata package entries must be objects")
        name = package.get("name")
        manifest = package.get("manifest_path")
        if not isinstance(name, str) or not isinstance(manifest, str):
            fail("cargo metadata package lacks name or manifest_path")
        manifest_to_name[str(Path(manifest).resolve())] = name
        name_to_package[name] = package
    return manifest_to_name, name_to_package


def _toml_string(block: str, key: str) -> str | None:
    match = re.search(rf"(?m)^\s*{re.escape(key)}\s*=\s*([\"'])(.*?)\1\s*$", block)
    return match.group(2) if match else None


def explicit_test_sources(manifest: Path) -> dict[str, str]:
    """Return explicit [[test]] source path -> target name for one manifest."""
    try:
        text = manifest.read_text(encoding="utf-8")
    except OSError as error:
        fail(f"could not read {manifest}: {error}")
    declarations: dict[str, str] = {}
    blocks = re.finditer(
        r"(?ms)^\[\[test\]\]\s*(.*?)(?=^\[\[|\Z)",
        text,
    )
    for block_match in blocks:
        block = block_match.group(1)
        name = _toml_string(block, "name")
        path_value = _toml_string(block, "path")
        if not name:
            fail(f"explicit [[test]] in {manifest} lacks name")
        source = manifest.parent / (path_value or f"tests/{name}.rs")
        declarations[str(source.resolve())] = name
    return declarations


def integration_targets(metadata: dict[str, Any], repo: Path) -> list[dict[str, Any]]:
    """Discover Cargo's integration-test targets, including auto roots."""
    manifest_to_name, _ = package_maps(metadata, repo)
    targets: list[dict[str, Any]] = []
    packages = metadata.get("packages", [])
    for package in packages:
        manifest = Path(str(package["manifest_path"])).resolve()
        package_name = str(package["name"])
        explicit = explicit_test_sources(manifest)
        for target in package.get("targets", []):
            if not isinstance(target, dict) or target.get("kind") != ["test"]:
                continue
            source = Path(str(target.get("src_path", ""))).resolve()
            target_name = str(target.get("name", ""))
            row = {
                "id": f"{package_name}/{target_name}",
                "package": package_name,
                "manifest": relpath(manifest, repo),
                "target": target_name,
                "source": relpath(source, repo),
                "source_absolute": str(source),
                "crate_types": target.get("crate_types", []),
                "explicit": str(source) in explicit,
                "explicit_name": explicit.get(str(source)),
            }
            targets.append(row)
    targets.sort(key=lambda row: (row["package"], row["target"], row["source"]))
    return targets


def binary_targets(metadata: dict[str, Any], repo: Path) -> dict[tuple[str, str], dict[str, Any]]:
    result: dict[tuple[str, str], dict[str, Any]] = {}
    for package in metadata.get("packages", []):
        package_name = str(package["name"])
        manifest = Path(str(package["manifest_path"])).resolve()
        for target in package.get("targets", []):
            if not isinstance(target, dict) or target.get("kind") != ["bin"]:
                continue
            name = str(target.get("name", ""))
            result[(package_name, name)] = {
                "package": package_name,
                "target": name,
                "source": relpath(Path(str(target.get("src_path", ""))), repo),
                "manifest": relpath(manifest, repo),
            }
    return result


def compiler_artifact_targets(path: Path, repo: Path) -> list[dict[str, Any]]:
    """Read Cargo JSON compiler-artifact messages for integration executables."""
    rows: list[dict[str, Any]] = []
    manifest_to_name, _ = package_maps(cargo_metadata(repo), repo)
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        fail(f"could not read Cargo artifact listing {path}: {error}")
    for line_number, line in enumerate(lines, 1):
        if not line.strip():
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError as error:
            fail(f"invalid Cargo JSON at {path}:{line_number}: {error}")
        target = message.get("target") if isinstance(message, dict) else None
        if (
            not isinstance(message, dict)
            or message.get("reason") != "compiler-artifact"
            or not isinstance(target, dict)
            or target.get("kind") != ["test"]
            or "bin" not in target.get("crate_types", [])
        ):
            continue
        executable = message.get("executable")
        if not isinstance(executable, str) or not executable:
            continue
        manifest = Path(str(message.get("manifest_path", ""))).resolve()
        package_id = str(message.get("package_id", ""))
        package = manifest_to_name.get(
            str(manifest), package_id.split("#", 1)[0].rsplit("/", 1)[-1]
        )
        rows.append(
            {
                "id": f"{package}/{target.get('name', '')}",
                "package": package,
                "target": str(target.get("name", "")),
                "manifest": relpath(manifest, repo),
                "source": relpath(Path(str(target.get("src_path", ""))), repo),
                "executable": executable,
                "fresh": message.get("fresh"),
            }
        )
    rows.sort(key=lambda row: (row["package"], row["target"], row["source"]))
    return rows


def topology_errors(
    targets: list[dict[str, Any]],
    binaries: dict[tuple[str, str], dict[str, Any]],
    emitted: list[dict[str, Any]] | None = None,
) -> list[str]:
    errors: list[str] = []
    ids = [str(row["id"]) for row in targets]
    if len(targets) != 1:
        errors.append(
            "expected exactly one Cargo integration-test target across the workspace; "
            f"found {len(targets)}; extra integration targets: {', '.join(ids) or '(none)'}"
        )
    auto = [str(row["id"]) for row in targets if not row.get("explicit", False)]
    if auto:
        errors.append(
            "package auto-discovered integration roots outside the explicit central target: "
            + ", ".join(auto)
        )
    missing = [
        f"{package}/{name}"
        for package, name, _kind in REQUIRED_BINARIES
        if (package, name) not in binaries
    ]
    if missing:
        errors.append("missing required production/reference-fixture binaries: " + ", ".join(missing))

    if emitted is not None:
        emitted_ids = [str(row["id"]) for row in emitted]
        if len(emitted) != 1:
            errors.append(
                "expected exactly one emitted Cargo integration-test executable; "
                f"found {len(emitted)}; extra integration-test executables: "
                f"{', '.join(emitted_ids) or '(none)'}"
            )
        if emitted_ids != ids:
            errors.append(
                "Cargo metadata/compiler-artifact integration target mismatch: "
                f"metadata={ids}, artifacts={emitted_ids}"
            )
    return errors


class NativeBootIdentityReadinessTests(unittest.TestCase):
    @staticmethod
    def published(boot_id: str = "fixture-boot") -> dict[str, Any]:
        return {
            "ownership": {
                "execution": {"root_pid": 123, "root_identity": {
                    "pid": 123, "boot_id": boot_id, "start_time": 1,
                }},
                "live_owned_work": True,
            },
            "visibility": {"execution": {"state": "running"}},
        }

    def probe(self, results, *, expired: bool = False):
        import dogfood_observation as observation
        from unittest.mock import patch
        status = {"result": {"invocations": {"items": [{
            "invocation_id": "fixture-invocation", "execution": {"state": "running"},
        }]}}}
        progress = [({"result": result}, 1, None) for result in results]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(observation.sys, "platform", "darwin"), \
                    patch.object(observation.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, b"fixture-boot\n", b"")), \
                    patch.object(observation, "_show_status", return_value=(status, 1, None)), \
                    patch.object(observation, "_engine", side_effect=progress) as read, \
                    patch.object(observation.time, "sleep"), \
                    patch.object(observation.time, "monotonic", side_effect=[0, 0, 6] if expired else None, return_value=0):
                value = observation._assert_native_boot_session_identity(
                    None, root / "loop.sqlite", "fixture-run", "fixture-invocation", root,
                )
                return value, read.call_count

    def test_running_status_waits_for_ownership_publication(self):
        value, reads = self.probe([{"ownership": None}, self.published()])
        self.assertEqual(reads, 2)
        self.assertEqual(value["boot_session_uuid"], "fixture-boot")
        self.assertTrue(value["live_owned_work"])

    def test_missing_ownership_publication_fails_at_deadline(self):
        with self.assertRaisesRegex(ValueError, "ownership did not publish"):
            self.probe([{"ownership": None}], expired=True)

    def test_published_wrong_boot_identity_still_fails(self):
        with self.assertRaisesRegex(ValueError, "boot identity differs"):
            self.probe([self.published("wrong-boot")])

    def test_published_empty_ownership_is_not_startup(self):
        with self.assertRaisesRegex(ValueError, "boot identity differs"):
            self.probe([{"ownership": {}}])


class CalibrationPacketCliTests(unittest.TestCase):
    """Exercise calibration preparation and capture verification via their CLIs."""

    DATA = ROOT / "crates/software-change-provider/data/calibration"
    PREPARE = ROOT / "scripts/prepare-calibration-input.py"
    CONSUMER = ROOT / "scripts/assert-calibration-capture.py"

    GATES = {
        "intent-review": ("intent.json", "intent.md", ()),
        "intent-adversarial-review": ("intent.json", "intent.md", ()),
        "design-review": ("design.json", "design.md", ("intent-good",)),
        "plan-review": ("plan.json", "task-packet.md", ("intent-good", "design-good")),
        "implementation-review": (
            "implementation-report.json",
            "implementation-report.md",
            ("intent-good", "design-good", "plan-good"),
        ),
        "validation-review": (
            "validation-report.json",
            "validation-report.md",
            ("intent-good", "design-good", "plan-good", "implementation-report-good"),
        ),
    }

    COVERAGE_PROMPTS = {
        "intent-review": (
            "Judge ids-grounded only. Confirm each promised enduring outcome is checked against "
            "the actual accepted requirement text and every authoritative document it explicitly "
            "names. Distinguish sufficient existing wording, missing or changed enduring meaning, "
            "and change-specific proof. A live or related ID, shared topic, or matching token is "
            "not semantic coverage; an implementation defect under sufficient wording does not "
            "require a new requirement. Candidates remain provisional until exact owner acceptance "
            "and separately authorized application and commit. Do not re-judge Bookends checker "
            "red/green."
        ),
        "intent-adversarial-review": (
            "Falsify ids-grounded only. Attack the ordinary ids-grounded pass by finding a "
            "promised enduring outcome whose cited requirement text or explicit cross-reference "
            "does not demand it, or by showing that a sufficient requirement is being misclassified "
            "as missing when supplied evidence instead shows an implementation defect. A related "
            "ID, shared topic, matching token, candidate flag, or parser success cannot establish "
            "semantic coverage. Do not re-judge Bookends checker red/green."
        ),
    }

    def prepare(self, output: Path, profile: str, row_keys: list[str]) -> None:
        command = [
            sys.executable,
            str(self.PREPARE),
            "--procedure",
            str(self.DATA / "PROCEDURE.md"),
            "--manifest",
            str(self.DATA / "manifest.json"),
            "--profile",
            str(ROOT / f"crates/software-change-provider/data/configs/{profile}.json"),
            "--repository",
            str(ROOT),
            "--output-root",
            str(output),
        ]
        for row_key in row_keys:
            command.extend(("--row-key", row_key))
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)

    def prepare_coverage(self, output: Path, case: str, gate: str) -> None:
        fixture = self.DATA / "fixtures" / f"requirement-coverage-{case}.json"
        companion = self.DATA / "companions/fictional-repo/docs/requirement-coverage.md"
        result = subprocess.run(
            [
                sys.executable,
                str(self.PREPARE),
                "--procedure",
                str(self.DATA / "PROCEDURE.md"),
                "--manifest",
                str(self.DATA / "manifest.json"),
                "--profile",
                str(ROOT / "crates/software-change-provider/data/configs/high-rigor.json"),
                "--repository",
                str(ROOT),
                "--output-root",
                str(output),
                "--coverage-case",
                case,
                "--coverage-gate",
                gate,
                "--fixture",
                str(fixture),
                "--companion",
                str(companion),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def expected_companion_labels(self, metadata: dict[str, Any], subject: dict[str, Any]) -> list[str]:
        gate = metadata["gate"]
        axis = metadata["policy_id"]
        fixture_id = metadata["fixture_id"]
        selected = subject
        labels: list[str] = []
        if gate.startswith("validation-") and subject.get("command_evidence_ids") is not None:
            evidence_id = subject["command_evidence_ids"][0]
            suffix = evidence_id.removeprefix("calibration-")
            selected = json.loads(
                (self.DATA / "fixtures" / f"validation-evidence-{suffix}.json").read_bytes()
            )
            labels.append("companion:validation-evidence.json")

        family = gate.split("-", 1)[0]
        if family == "implementation" or (
            family == "validation" and axis in ("intent-delivered", "requirement-proof-mapping")
        ):
            commit = selected["coverage"]["commit"]
            self.assertIn(commit, ("repo-state-2026-08-12", "repo-state-2026-08-13"))
            labels.append(
                "companion:fictional-repo/implementation-evidence/repository-state.txt"
            )
        if family == "validation" and axis == "docs-integrated":
            documents = selected["coverage"]["documents"]
            paths = sorted((doc["path"] for doc in documents), key=lambda value: value.encode("utf-8"))
            labels.extend(f"companion:{path}" for path in paths)
        if family == "validation" and axis == "requirement-proof-mapping":
            labels.extend(
                (
                    "companion:fictional-repo/docs/PRD.md",
                    "companion:fictional-repo/implementation-evidence/requirement-to-proof.md",
                    "companion:fictional-repo/scripts/assert-requirement-proof.py",
                    "companion:fictional-repo/scripts/production-journey.py",
                )
            )
        if metadata.get("coverage_case"):
            labels.append("companion:fictional-repo/docs/requirement-coverage.md")
        return labels

    def expected_labels(self, metadata: dict[str, Any]) -> list[str]:
        subject_name, template, predecessors = self.GATES[metadata["gate"]]
        profile = metadata["profile"]
        fixture_id = metadata["fixture_id"]
        labels = [
            "system-developer-instruction:data/calibration/reviewer-instruction.txt",
            "example_prompt",
            "reviewer-protocol:data/reviewer-protocol.md",
            f"template:data/templates/{template}",
            f"schema:data/configs/{profile}.json#/artifact_schemas/{subject_name}",
            f"subject:data/calibration/fixtures/{fixture_id}.json",
        ]
        labels.extend(
            f"required predecessor:data/calibration/fixtures/{fixture}.json"
            for fixture in predecessors
        )
        subject_path = self.DATA / "fixtures" / f"{fixture_id}.json"
        subject = json.loads(subject_path.read_bytes())
        labels.extend(self.expected_companion_labels(metadata, subject))
        labels.append("request-json")
        return labels

    def expected_content(self, label: str, metadata: dict[str, Any]) -> bytes:
        if label == "example_prompt":
            if metadata.get("coverage_case"):
                return self.COVERAGE_PROMPTS[metadata["gate"]].encode("utf-8")
            profile = json.loads(
                (ROOT / f"crates/software-change-provider/data/configs/{metadata['profile']}.json").read_bytes()
            )
            matches = [
                item
                for item in profile["review_policies"][metadata["gate"]]
                if item["id"] == metadata["policy_id"]
                and item.get("review_stage", "aggregate") == metadata["review_stage"]
            ]
            self.assertEqual(len(matches), 1)
            return matches[0]["example_prompt"].encode("utf-8")
        if label.startswith("schema:"):
            profile = json.loads(
                (ROOT / f"crates/software-change-provider/data/configs/{metadata['profile']}.json").read_bytes()
            )
            value = profile["artifact_schemas"][metadata["subject"]]

            def sorted_value(item: Any) -> Any:
                if isinstance(item, dict):
                    return {
                        key: sorted_value(item[key])
                        for key in sorted(item, key=lambda key: key.encode("utf-8"))
                    }
                if isinstance(item, list):
                    return [sorted_value(child) for child in item]
                return item

            return json.dumps(
                sorted_value(value), ensure_ascii=False, separators=(",", ":")
            ).encode("utf-8")
        if label == "request-json":
            subject = json.loads(
                (self.DATA / "fixtures" / f"{metadata['fixture_id']}.json").read_bytes()
            )
            request = {
                "gate": metadata["gate"],
                "policy_id": metadata["policy_id"],
                "review_stage": metadata["review_stage"],
                "subject": metadata["subject"],
                "subject_revision": subject["revision"],
                "config_version": metadata["config_version"],
            }
            return json.dumps(request, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        if label == "companion:validation-evidence.json":
            subject = json.loads(
                (self.DATA / "fixtures" / f"{metadata['fixture_id']}.json").read_bytes()
            )
            suffix = subject["command_evidence_ids"][0].removeprefix("calibration-")
            return (self.DATA / "fixtures" / f"validation-evidence-{suffix}.json").read_bytes()
        if label == "companion:fictional-repo/implementation-evidence/repository-state.txt":
            selected = json.loads(
                (self.DATA / "fixtures" / f"{metadata['fixture_id']}.json").read_bytes()
            )
            if metadata["gate"].startswith("validation-"):
                suffix = selected["command_evidence_ids"][0].removeprefix("calibration-")
                selected = json.loads(
                    (self.DATA / "fixtures" / f"validation-evidence-{suffix}.json").read_bytes()
                )
            commit = selected["coverage"]["commit"]
            return (
                self.DATA
                / "companions/fictional-repo/implementation-evidence"
                / f"{commit}.txt"
            ).read_bytes()
        if label.startswith("companion:fictional-repo/"):
            relative = label.removeprefix("companion:")
            return (self.DATA / "companions" / relative).read_bytes()
        prefixes = (
            "system-developer-instruction:",
            "reviewer-protocol:",
            "template:",
            "subject:",
            "required predecessor:",
        )
        for prefix in prefixes:
            if label.startswith(prefix):
                relative = label.removeprefix(prefix)
                return (ROOT / "crates/software-change-provider" / relative).read_bytes()
        self.fail(f"no expected source bytes for {label}")

    def display_labels(self, labels: list[str], metadata: dict[str, Any]) -> list[str]:
        fixed = {
            "system-developer-instruction:data/calibration/reviewer-instruction.txt": "reviewer instruction",
            "example_prompt": "selected review prompt",
            "reviewer-protocol:data/reviewer-protocol.md": "review protocol",
            "request-json": "canonical request JSON",
            "companion:validation-evidence.json": "validation evidence companion",
            "companion:fictional-repo/implementation-evidence/repository-state.txt": "implementation evidence companion",
            "companion:fictional-repo/docs/requirement-coverage.md": "requirement coverage companion",
            "required predecessor:data/calibration/fixtures/intent-good.json": "required predecessor: intent",
            "required predecessor:data/calibration/fixtures/design-good.json": "required predecessor: design",
            "required predecessor:data/calibration/fixtures/plan-good.json": "required predecessor: plan",
            "required predecessor:data/calibration/fixtures/implementation-report-good.json": "required predecessor: implementation report",
        }
        display = []
        companion_number = 0
        for label in labels:
            if label in fixed:
                display.append(fixed[label])
            elif label.startswith("template:"):
                display.append(f"artifact template: {metadata['template']}")
            elif label.startswith("schema:"):
                display.append(f"artifact schema: {metadata['subject']}")
            elif label.startswith("subject:"):
                display.append(f"subject artifact: {metadata['subject']}")
            elif label.startswith("companion:fictional-repo/"):
                companion_number += 1
                display.append(f"supporting document {companion_number}")
            else:
                self.fail(f"unexpected canonical label {label}")
        return display

    def verify_packet(self, case_dir: Path) -> dict[str, Any]:
        metadata = json.loads((case_dir / "preparation.json").read_bytes())
        self.assertFalse({"expected", "observed", "oracle"} & set(metadata))
        records_meta = metadata["source_records"]
        labels = [record["label"] for record in records_meta]
        self.assertEqual(labels, self.expected_labels(metadata))
        records = []
        for index, record in enumerate(records_meta):
            self.assertEqual(record["path"], f"source-records/{index:03d}.bin")
            content = (case_dir / record["path"]).read_bytes()
            self.assertEqual(len(content), record["byte_length"])
            self.assertEqual(hashlib.sha256(content).hexdigest(), record["sha256"])
            self.assertEqual(content, self.expected_content(record["label"], metadata))
            records.append((record["label"], content))

        stream = bytearray(len(records).to_bytes(8, "big"))
        for label, content in records:
            label_bytes = label.encode("utf-8")
            stream.extend(len(label_bytes).to_bytes(8, "big"))
            stream.extend(label_bytes)
            stream.extend(len(content).to_bytes(8, "big"))
            stream.extend(content)
        input_hash = hashlib.sha256(stream).hexdigest()
        self.assertEqual(input_hash, metadata["input_sha256"])
        self.assertEqual((case_dir / "input_sha256").read_text().strip(), input_hash)
        self.assertEqual((case_dir / "request.json").read_bytes(), records[-1][1])

        expected_instructions = bytearray()
        for role, (_label, content) in zip(self.display_labels(labels, metadata), records):
            expected_instructions.extend(f"=== supplied material: {role} ===\n".encode("utf-8"))
            expected_instructions.extend(content)
            if not content.endswith(b"\n"):
                expected_instructions.extend(b"\n")
            expected_instructions.extend(b"\n")
        instructions = (case_dir / "instructions.txt").read_bytes()
        self.assertEqual(instructions, bytes(expected_instructions))
        instructions_hash = hashlib.sha256(instructions).hexdigest()
        self.assertEqual(metadata["instructions_sha256"], instructions_hash)
        return metadata

    def add_dummy_captures(self, root: Path, packet_dirs: list[Path]) -> None:
        index_cases = []
        for case_dir in packet_dirs:
            metadata = json.loads((case_dir / "preparation.json").read_bytes())
            attempt_dir = root / "dummy-captures" / case_dir.name / "attempt-1"
            attempt_dir.mkdir(parents=True, exist_ok=True)
            returned = b'{"kind":"dummy-capture","purpose":"mechanical consumer compatibility only"}\n'
            (attempt_dir / "returned-output").write_bytes(returned)
            worker_stdout = attempt_dir / "worker.stdout"
            worker_stderr = attempt_dir / "worker.stderr"
            worker_stdout.write_bytes(returned)
            worker_stderr.write_bytes(b"")
            worker = {
                "command": "/scripted/dummy-worker",
                "args": ["--no-context-files", "--tools", "", "--model", "dummy-model", "--thinking", "none"],
            }
            command = [
                "/scripted/loop-engine",
                "fan-out",
                "--worker",
                json.dumps(worker, separators=(",", ":")),
            ]
            summary = {
                "workers": [
                    {
                        "exit_code": 0,
                        "command": worker["command"],
                        "args": worker["args"],
                        "stdout_path": str(worker_stdout),
                        "stderr_path": str(worker_stderr),
                    }
                ]
            }
            raw_stdout = json.dumps(summary, separators=(",", ":")).encode("utf-8")
            (attempt_dir / "stdout").write_bytes(raw_stdout)
            (attempt_dir / "stderr").write_bytes(b"")
            receipt_path = attempt_dir / "receipt.json"
            receipt = {
                "owner_attestation": "pending",
                "semantic_disposition": "pending-driver-inspection",
                "exit_code": 0,
                "instructions_sha256": metadata["instructions_sha256"],
                "command": command,
                "model": "dummy-model",
                "thinking": "none",
                "returned_output_sha256": hashlib.sha256(returned).hexdigest(),
                "captured_stdout": str(worker_stdout),
                "captured_stderr": str(worker_stderr),
                "case": case_dir.name,
            }
            receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
            index_cases.append(
                {
                    "case": case_dir.name,
                    "receipt": receipt_path.relative_to(root).as_posix(),
                }
            )
        (root / "driver-capture-index-dummy.json").write_text(
            json.dumps({"owner_attestation": "pending", "cases": index_cases}),
            encoding="utf-8",
        )

    def run_consumer(self, root: Path, *selectors: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(self.CONSUMER), "--root", str(root), *selectors],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_public_preparation_and_capture_consumer_preserve_neutral_packets(self) -> None:
        with tempfile.TemporaryDirectory(prefix="calibration-packets-") as temporary:
            output = Path(temporary) / "packets"
            minimal_rows = [
                "minimal-12|intent-review|solution-agnostic|aggregate|intent-defective",
                "minimal-12|intent-review|owner-comprehensible|aggregate|intent-owner-unstated-background",
            ]
            self.prepare(output, "minimal", minimal_rows)
            high_rigor_rows = [
                "high-rigor-12|plan-review|design-faithful|aggregate|plan-good",
                "high-rigor-12|implementation-review|tasks-actually-done|aggregate|implementation-report-good",
                "high-rigor-12|validation-review|docs-integrated|aggregate|validation-report-good",
                "high-rigor-12|validation-review|requirement-proof-mapping|aggregate|validation-report-good",
            ]
            self.prepare(output, "high-rigor", high_rigor_rows)
            for case in ("sufficient", "related-insufficient", "implementation-defect"):
                for gate in ("intent-review", "intent-adversarial-review"):
                    self.prepare_coverage(output, case, gate)

            packets = sorted(
                child for child in output.iterdir() if child.is_dir() and (child / "preparation.json").is_file()
            )
            self.assertEqual(len(packets), 12)
            metadata_by_key = {}
            for packet in packets:
                metadata = self.verify_packet(packet)
                metadata_by_key[metadata["row_key"]] = metadata
            defective_key = minimal_rows[0]
            defective_metadata = metadata_by_key[defective_key]
            self.assertEqual(defective_metadata["input_sha256"], "519dc8fd0a9d17a211604f1a2b209077050ed7b479427ec6600fb08c26d3a4b4")

            self.add_dummy_captures(output, packets)
            fresh_result = self.run_consumer(
                output,
                "--row-key",
                defective_key,
                "--coverage-case",
                "sufficient",
            )
            self.assertEqual(fresh_result.returncode, 0, fresh_result.stderr)
            self.assertIn('"status": "verified"', fresh_result.stdout)

            stale_root = Path(temporary) / "stale-current-v12"
            stale_case = stale_root / defective_metadata["row_key"]
            source_case = next(packet for packet in packets if packet.name == defective_metadata["row_key"])
            shutil.copytree(source_case, stale_case)
            stale_metadata = json.loads((stale_case / "preparation.json").read_bytes())
            legacy = bytearray()
            for record in stale_metadata["source_records"]:
                content = (stale_case / record["path"]).read_bytes()
                legacy.extend(b"=== source-record: " + record["label"].encode("utf-8") + b" ===\n")
                legacy.extend(content)
                if not content.endswith(b"\n"):
                    legacy.extend(b"\n")
                legacy.extend(b"\n")
            (stale_case / "instructions.txt").write_bytes(legacy)
            stale_metadata["instructions_sha256"] = hashlib.sha256(legacy).hexdigest()
            (stale_case / "preparation.json").write_text(
                json.dumps(stale_metadata, indent=2) + "\n", encoding="utf-8"
            )
            self.add_dummy_captures(stale_root, [stale_case])
            stale_result = self.run_consumer(stale_root, "--row-key", defective_key)
            self.assertNotEqual(stale_result.returncode, 0)
            self.assertIn("instructions do not preserve the prepared source framing", stale_result.stderr)


def mask_rust(source: str) -> str:
    """Blank comments and literals while retaining offsets and newlines."""
    output = list(source)
    index = 0
    length = len(source)

    def blank(start: int, end: int) -> None:
        for position in range(start, end):
            if source[position] != "\n":
                output[position] = " "

    def boundary(position: int) -> bool:
        return position == 0 or not (source[position - 1].isalnum() or source[position - 1] == "_")

    while index < length:
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = length if end < 0 else end
            blank(index, end)
            index = end
            continue
        if source.startswith("/*", index):
            depth = 1
            end = index + 2
            while end < length and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            blank(index, end)
            index = end
            continue

        prefix = next(
            (
                candidate
                for candidate in ("br", "rb", "r")
                if source.startswith(candidate, index) and boundary(index)
            ),
            None,
        )
        if prefix:
            cursor = index + len(prefix)
            hashes = 0
            while cursor < length and source[cursor] == "#":
                hashes += 1
                cursor += 1
            if cursor < length and source[cursor] == '"':
                terminator = '"' + ('#' * hashes)
                end = source.find(terminator, cursor + 1)
                end = length if end < 0 else end + len(terminator)
                blank(index, end)
                index = end
                continue

        if source[index] == '"':
            end = index + 1
            while end < length:
                if source[end] == "\\":
                    end += 2
                    continue
                if source[end] == '"':
                    end += 1
                    break
                end += 1
            blank(index, end)
            index = end
            continue

        # Mask a closed character literal, but leave Rust lifetimes such as
        # 'static alone.  A citation or test declaration cannot occur in a
        # character literal, so an unclosed quote is intentionally retained.
        if source[index] == "'" or (
            source[index] == "b"
            and index + 1 < length
            and source[index + 1] == "'"
            and boundary(index)
        ):
            start = index
            quote = index + 1 if source[index] == "b" else index
            end = quote + 1
            while end < length and source[end] != "\n":
                if source[end] == "\\":
                    end += 2
                    continue
                if source[end] == "'":
                    end += 1
                    break
                end += 1
            if end <= length and end > quote + 1 and source[end - 1] == "'":
                blank(start, end)
                index = end
                continue
        index += 1
    return "".join(output)


_TEST_ATTRIBUTE = re.compile(r"#\[(?:test|tokio::test)(?:\([^\]]*\))?\]")
_TEST_FUNCTION = re.compile(r"\b(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")
_PATH_MODULE = re.compile(
    r'#\[path\s*=\s*"([^"]+)"\]\s*mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;'
)
_MODULE = re.compile(r"(?m)^\s*mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")


def module_files(root: Path) -> list[Path]:
    pending = [root.resolve()]
    seen: set[Path] = set()
    result: list[Path] = []
    while pending:
        path = pending.pop()
        if path in seen or not path.exists():
            continue
        seen.add(path)
        result.append(path)
        text = path.read_text(encoding="utf-8")
        explicit_names = {name for _relative, name in _PATH_MODULE.findall(text)}
        for relative, _name in _PATH_MODULE.findall(text):
            pending.append((path.parent / relative).resolve())
        masked = mask_rust(text)
        for name in _MODULE.findall(masked):
            if name in explicit_names:
                continue
            candidate = (path.parent / f"{name}.rs").resolve()
            if not candidate.exists():
                candidate = (path.parent / name / "mod.rs").resolve()
            if candidate.exists():
                pending.append(candidate)
    return result


def test_declarations(path: Path, repo: Path) -> list[dict[str, Any]]:
    text = path.read_text(encoding="utf-8")
    masked = mask_rust(text)
    declarations: list[dict[str, Any]] = []
    for attribute in _TEST_ATTRIBUTE.finditer(masked):
        function = _TEST_FUNCTION.search(masked, attribute.end())
        if function is None:
            fail(f"test attribute in {relpath(path, repo)} has no following function")
        declarations.append(
            {
                "source": relpath(path, repo),
                "function": function.group(1),
                "line": text.count("\n", 0, function.start()) + 1,
                "attribute": attribute.group(0),
            }
        )
    return declarations


def source_files(metadata: dict[str, Any], repo: Path) -> list[dict[str, Any]]:
    """Inventory every Rust file below a package tests directory plus shared support."""
    entries: list[dict[str, Any]] = []
    seen: set[Path] = set()
    for package in metadata.get("packages", []):
        package_name = str(package["name"])
        tests_dir = Path(str(package["manifest_path"])).resolve().parent / "tests"
        if not tests_dir.is_dir():
            continue
        for path in sorted(tests_dir.rglob("*.rs")):
            path = path.resolve()
            if path in seen:
                continue
            seen.add(path)
            data = path.read_bytes()
            entries.append(
                {
                    "path": relpath(path, repo),
                    "package": package_name,
                    "role": "root" if path.parent == tests_dir else "module-or-support",
                    "sha256": sha256_bytes(data),
                    "size_bytes": len(data),
                    "line_count": len(data.decode("utf-8").splitlines()),
                }
            )

    shared = (repo / "tests/bounded_process.rs").resolve()
    if shared.exists() and shared not in seen:
        data = shared.read_bytes()
        entries.append(
            {
                "path": relpath(shared, repo),
                "package": None,
                "role": "shared-test-support-outside-package",
                "sha256": sha256_bytes(data),
                "size_bytes": len(data),
                "line_count": len(data.decode("utf-8").splitlines()),
            }
        )
    entries.sort(key=lambda entry: entry["path"])
    return entries


def _source_entry_map(entries: Iterable[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    return {str(entry["path"]): entry for entry in entries}


def source_test_inventory(
    metadata: dict[str, Any], repo: Path, targets: list[dict[str, Any]], test_lists: list[dict[str, Any]]
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Map exact libtest names back to source declarations without hand counts."""
    target_by_id = {str(target["id"]): target for target in targets}
    list_by_id: dict[str, dict[str, Any]] = {}
    for listing in test_lists:
        package = str(listing.get("package", ""))
        target = str(listing.get("target", ""))
        identifier = f"{package}/{target}"
        if identifier in list_by_id:
            fail(f"duplicate test-list entry for {identifier}")
        list_by_id[identifier] = listing

    cases: list[dict[str, Any]] = []
    target_inventory: list[dict[str, Any]] = []
    for identifier, target in sorted(target_by_id.items()):
        listing = list_by_id.get(identifier)
        if listing is None:
            fail(f"Cargo test listing lacks integration target {identifier}")
        names = listing.get("tests")
        if not isinstance(names, list) or not all(isinstance(name, str) for name in names):
            fail(f"test-list entry {identifier} lacks a string tests array")
        root_source = Path(str(target["source_absolute"]))
        declarations: list[dict[str, Any]] = []
        for path in module_files(root_source):
            declarations.extend(test_declarations(path, repo))
        by_function: dict[str, list[dict[str, Any]]] = {}
        for declaration in declarations:
            by_function.setdefault(str(declaration["function"]), []).append(declaration)

        # A central target intentionally combines roots that historically had
        # repeated helper/test names.  Its first Cargo name component is the
        # explicit module name in workspace.rs, so use that namespace to
        # select the same source traversal before matching the function.  For
        # ordinary package roots there is no such map and the historical
        # function-only mapping remains exact within that target.
        module_sources: dict[str, Path] = {}
        if root_source.exists():
            root_text = root_source.read_text(encoding="utf-8")
            for relative, module_name in _PATH_MODULE.findall(root_text):
                module_sources[module_name] = (root_source.parent / relative).resolve()

        target_cases: list[dict[str, Any]] = []
        for cargo_name in names:
            function = cargo_name.rsplit("::", 1)[-1]
            matches = by_function.get(function, [])
            module_name = cargo_name.split("::", 1)[0]
            module_source = module_sources.get(module_name)
            if module_source is not None:
                source_paths = {str(path) for path in module_files(module_source)}
                matches = [
                    declaration
                    for declaration in matches
                    if str((repo / str(declaration["source"])).resolve()) in source_paths
                ]
            if len(matches) != 1:
                fail(
                    f"{identifier} test {cargo_name!r} maps to {len(matches)} source declarations; "
                    "source inventory is not an aggregate count"
                )
            declaration = matches[0]
            case = {
                "id": f"{declaration['source']}::{declaration['function']}",
                "target": identifier,
                "cargo_name": cargo_name,
                "source": declaration["source"],
                "function": declaration["function"],
                "line": declaration["line"],
            }
            target_cases.append(case)
            cases.append(case)
        if len(target_cases) != len(declarations):
            fail(
                f"{identifier} Cargo listed {len(target_cases)} tests but source traversal found "
                f"{len(declarations)} declarations"
            )
        target_inventory.append(
            {
                "id": identifier,
                "source": target["source"],
                "explicit": target["explicit"],
                "test_count": len(target_cases),
                "cargo_names": list(names),
                "cases": target_cases,
            }
        )
    cases.sort(key=lambda case: str(case["id"]))
    return target_inventory, cases


def _line_entries(
    source_entries: list[dict[str, Any]], patterns: list[tuple[str, re.Pattern[str]]], repo: Path
) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    for source_entry in source_entries:
        path = repo / str(source_entry["path"])
        text = path.read_text(encoding="utf-8")
        masked = mask_rust(text)
        original_lines = text.splitlines()
        masked_lines = masked.splitlines()
        for line_number, (original, safe) in enumerate(zip(original_lines, masked_lines), 1):
            for kind, pattern in patterns:
                for match in pattern.finditer(safe):
                    entries.append(
                        {
                            "source": source_entry["path"],
                            "line": line_number,
                            "column": match.start() + 1,
                            "kind": kind,
                            "match": match.group(0),
                            "text": original.strip(),
                        }
                    )
    entries.sort(
        key=lambda entry: (
            str(entry["source"]),
            int(entry["line"]),
            int(entry.get("column", 0)),
            str(entry["kind"]),
        )
    )
    return entries


def extracted_inventories(source_entries: list[dict[str, Any]], repo: Path) -> dict[str, Any]:
    assertion_patterns = [
        ("assert", re.compile(r"\bassert(?:_eq|_ne|_matches)?\s*!")),
        ("debug-assert", re.compile(r"\bdebug_assert(?:_eq|_ne)?\s*!")),
        ("panic", re.compile(r"\bpanic\s*!")),
        ("matches", re.compile(r"\bmatches\s*!")),
    ]
    failure_patterns = [
        ("fixture-response-failure", re.compile(r"FixtureResponse::Failure")),
        ("fixture-response-raw", re.compile(r"FixtureResponse::Raw")),
        ("fixture-failure-selector", re.compile(r'"(?:failure|malformed|unsupported)"')),
        ("nonzero-process-result", re.compile(r"non.?zero|status\.code\(\)|exit_code")),
        ("bounded-timeout-or-wedge", re.compile(r"timeout|deadline|process_timeout_probe|sleep\(")),
        ("process-group-cleanup", re.compile(r"prepare_process_group|wait_existing|kill\(|SIGTERM|SIGKILL|pgid")),
    ]
    citation_entries: list[dict[str, Any]] = []
    binary_entries: list[dict[str, Any]] = []
    literal_failure_entries: list[dict[str, Any]] = []
    literal_failure_patterns = [
        ("fixture-response-selector", re.compile(r'"(?:failure|malformed|unsupported)"')),
        ("injected-wedge-script", re.compile(r"sleep\(|process_timeout_probe")),
    ]
    citation_pattern = re.compile(r"(?:bookends[^\n]*LE-\d+|bookends:\s*LE-\d+|LE-\d+)", re.IGNORECASE)
    binary_pattern = re.compile(
        r"CARGO_BIN_EXE_[A-Za-z0-9_-]+|fixture_binary\(\s*\"[^\"]+\"\s*\)|provider_binary\(\)"
    )
    for source_entry in source_entries:
        path = repo / str(source_entry["path"])
        text = path.read_text(encoding="utf-8")
        for line_number, original in enumerate(text.splitlines(), 1):
            for kind, pattern in literal_failure_patterns:
                for match in pattern.finditer(original):
                    literal_failure_entries.append(
                        {
                            "source": source_entry["path"],
                            "line": line_number,
                            "column": match.start() + 1,
                            "kind": kind,
                            "match": match.group(0),
                            "text": original.strip(),
                        }
                    )
            for match in citation_pattern.finditer(original):
                citation_entries.append(
                    {
                        "source": source_entry["path"],
                        "line": line_number,
                        "match": match.group(0),
                        "text": original.strip(),
                    }
                )
            for match in binary_pattern.finditer(original):
                binary_entries.append(
                    {
                        "source": source_entry["path"],
                        "line": line_number,
                        "match": match.group(0),
                        "text": original.strip(),
                    }
                )
    citation_entries.sort(key=lambda entry: (str(entry["source"]), int(entry["line"]), str(entry["match"])))
    binary_entries.sort(key=lambda entry: (str(entry["source"]), int(entry["line"]), str(entry["match"])))
    assertions = _line_entries(source_entries, assertion_patterns, repo)
    failures = _line_entries(source_entries, failure_patterns, repo) + literal_failure_entries
    failures.sort(
        key=lambda entry: (
            str(entry["source"]),
            int(entry["line"]),
            int(entry.get("column", 0)),
            str(entry["kind"]),
        )
    )
    return {
        "assertions": {"count": len(assertions), "entries": assertions},
        "failure_injection": {"count": len(failures), "entries": failures},
        "bookends_citations": {"count": len(citation_entries), "entries": citation_entries},
        "binary_references": {"count": len(binary_entries), "entries": binary_entries},
    }


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"could not read JSON {path}: {error}")


def load_test_lists(path: Path) -> list[dict[str, Any]]:
    value = load_json(path)
    if isinstance(value, dict):
        entries = value.get("entries")
    else:
        entries = value
    if not isinstance(entries, list) or not all(isinstance(entry, dict) for entry in entries):
        fail(f"test listing {path} must contain an entries array")
    return entries


def baseline_case_ids(baseline: dict[str, Any]) -> set[str]:
    integration = baseline.get("integration")
    if not isinstance(integration, dict):
        fail("baseline integration section must be an object")
    cases = integration.get("cases")
    if not isinstance(cases, list) or not all(isinstance(case, dict) for case in cases):
        fail("baseline integration cases must be an array of objects")
    ids = [case.get("id") for case in cases]
    if not all(isinstance(case_id, str) for case_id in ids):
        fail("baseline integration case ids must be strings")
    if len(set(ids)) != len(ids):
        fail("baseline integration case ids must be unique")
    return set(ids)


def inventory_manifest_errors(baseline: dict[str, Any], manifest: dict[str, Any]) -> list[str]:
    """Validate the final handoff shape without accepting aggregate counts."""
    errors: list[str] = []
    expected_ids = baseline_case_ids(baseline)
    cases = manifest.get("cases")
    if not isinstance(cases, list) or not all(isinstance(case, dict) for case in cases):
        return ["final inventory manifest cases must be an array of objects"]
    actual_ids = [case.get("baseline_case_id", case.get("id")) for case in cases]
    if not all(isinstance(case_id, str) for case_id in actual_ids):
        errors.append("final inventory case ids must be strings")
    else:
        actual_set = set(actual_ids)
        missing = sorted(expected_ids - actual_set)
        extra = sorted(actual_set - expected_ids)
        if missing:
            errors.append("missing baseline integration cases: " + ", ".join(missing))
        if extra:
            errors.append("unexpected integration cases: " + ", ".join(extra))
        if len(actual_ids) != len(actual_set):
            errors.append("final inventory contains duplicate baseline integration cases")

    required = {f"{package}/{name}" for package, name, _kind in REQUIRED_BINARIES}
    binaries = manifest.get("required_binaries")
    if isinstance(binaries, list) and all(isinstance(item, str) for item in binaries):
        missing = sorted(required - set(binaries))
        if missing:
            errors.append("final inventory missing required binaries: " + ", ".join(missing))
    else:
        errors.append("final inventory required_binaries must be a string array")

    return errors