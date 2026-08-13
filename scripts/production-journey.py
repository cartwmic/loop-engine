#!/usr/bin/env python3
"""Run the source and packaged production-boundary journey.

The runner is intentionally a process harness, not another workflow engine.  It
uses one scenario contract for both adapters:

* ``source`` invokes separately built ``loop-engine`` processes, a TOML
  provider registration, SQLite, and the checkout's production provider.
* ``packaged`` invokes extracted binaries, dumps provider data into an empty
  root, and uses only the dumped high-rigor profile for a checked prefix.

CLI contract permits only ``source/full`` and ``packaged/checked-prefix``;
invalid adapter/depth pairs fail before any work-root mutation.

The evidence records are synthetic, conforming records.  They exercise schema,
revision-link, author-independence, aggregation, routing, and persistence
mechanics only; they are not semantic review judgments.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence
PROFILE_SUBPATH = Path("crates/software-change-provider/data/configs/high-rigor.json")
FIXTURE_SUBPATH = Path(
    "crates/software-change-provider/data/calibration/fixtures"
)
SUBJECTS = {
    "intent.json": "intent-good.json",
    "design.json": "design-good.json",
    "plan.json": "plan-good.json",
    "implementation-report.json": "implementation-report-good.json",
    "validation-report.json": "validation-report-good.json",
}
GATE_SUBJECT = {
    "intent": "intent.json",
    "design-review": "design.json",
    "plan-review": "plan.json",
    "implementation-review": "implementation-report.json",
    "validation": "validation-report.json",
}


class JourneyFailure(RuntimeError):
    """A failure with enough state to diagnose a stopped journey."""

    def __init__(
        self,
        message: str,
        *,
        state: str = "unknown",
        event: str = "none",
        axis: str = "none",
    ) -> None:
        super().__init__(message)
        self.state = state
        self.event = event
        self.axis = axis


class Journey:
    def __init__(self, args: argparse.Namespace) -> None:
        self.mode = args.mode
        self.depth = args.traversal_depth
        self.engine = Path(args.engine).expanduser().resolve()
        self.provider = Path(args.provider).expanduser().resolve()
        self.data_root = Path(args.data_root).expanduser().resolve()
        self.work_root = Path(args.work_root).expanduser().resolve()
        self.profile_arg = args.profile
        self.profile_source: Optional[Path] = None
        self.fixture_root: Optional[Path] = None
        self.profile: Dict[str, Any] = {}
        self.run_dir: Optional[Path] = None
        self.database: Optional[Path] = None
        self.provider_config: Optional[Path] = None
        self.profile_path: Optional[Path] = None
        self.artifact_root: Optional[Path] = None
        self.run_id = "journey-production-run"
        self.state = "not-started"

    def preflight(self) -> None:
        """Reject bad inputs before creating or mutating any run state."""
        expected_depth = {
            "source": "full",
            "packaged": "checked-prefix",
        }[self.mode]
        if self.depth != expected_depth:
            raise JourneyFailure(
                f"unsupported mode/traversal-depth pair: {self.mode}/{self.depth}; "
                f"only {expected_depth} is valid for {self.mode}"
            )

        for label, path in (("engine binary", self.engine), ("provider binary", self.provider)):
            if not path.is_file():
                raise JourneyFailure(f"{label} does not exist: {path}")
            if not os.access(path, os.X_OK):
                raise JourneyFailure(f"{label} is not executable: {path}")

        if not self.work_root.exists() and self.work_root.parent and not self.work_root.parent.is_dir():
            raise JourneyFailure(f"work-root parent does not exist: {self.work_root.parent}")
        if self.work_root.exists() and not self.work_root.is_dir():
            raise JourneyFailure(f"work-root is not a directory: {self.work_root}")

        if self.mode == "source":
            if not self.data_root.is_dir():
                raise JourneyFailure(f"source data-root does not exist: {self.data_root}")
            self.profile_source = Path(self.profile_arg).expanduser().resolve()
            if not self.profile_source.is_file():
                raise JourneyFailure(f"source profile does not exist: {self.profile_source}")
            self.profile = self._read_json(self.profile_source, "source profile")
            self.fixture_root = self.data_root / FIXTURE_SUBPATH
        else:
            # The packaged adapter deliberately accepts a profile name, not a
            # checkout profile path.  The actual file is found only after
            # data-dump, which proves the package's embedded data is used.
            if Path(self.profile_arg).name != "high-rigor.json":
                raise JourneyFailure(
                    "packaged profile must be the dumped high-rigor.json profile"
                )
            if self.data_root.exists():
                if not self.data_root.is_dir():
                    raise JourneyFailure(f"packaged data-root is not a directory: {self.data_root}")
                if any(self.data_root.iterdir()):
                    raise JourneyFailure(
                        f"packaged data-root must be empty before data-dump: {self.data_root}"
                    )

        self._validate_profile_shape(self.profile, require_loaded=self.mode == "source")
        if self.mode == "source":
            self._validate_scenario_fixtures()

    def _validate_scenario_fixtures(self) -> None:
        assert self.fixture_root is not None
        for subject, fixture in SUBJECTS.items():
            fixture_path = self.fixture_root / fixture
            if not fixture_path.is_file():
                raise JourneyFailure(f"scenario fixture is missing: {fixture_path}")
            # Parse all fixtures before mutating state. The provider performs
            # the authoritative schema check later.
            self._read_json(fixture_path, f"scenario fixture {subject}")

    def run(self) -> Path:
        self.preflight()
        self.work_root.mkdir(parents=True, exist_ok=True)
        self.run_dir = Path(tempfile.mkdtemp(prefix="production-journey-", dir=self.work_root))
        self.database = self.run_dir / "loop.sqlite"
        self.provider_config = self.run_dir / "providers.toml"
        self.profile_path = self.run_dir / "high-rigor.json"
        self.artifact_root = self.run_dir / "artifacts"
        self.artifact_root.mkdir()

        if self.mode == "packaged":
            self._dump_packaged_data()
        else:
            assert self.profile_source is not None

        self._prepare_profile()
        self._write_provider_config()
        self._probe_startup()
        self._start()
        self._assert_show("explore", "start")
        self._append_marker("journey-marker-separate", equals=False)
        self._append_marker("journey-marker-equals", equals=True)
        self._assert_marker_persistence()

        if self.mode == "source" and self.depth == "full":
            self._run_full_source()
        else:
            self._run_checked_prefix()

        result = self.run_dir / "journey-result.json"
        result.write_text(
            json.dumps(
                {
                    "mode": self.mode,
                    "traversal_depth": self.depth,
                    "run_id": self.run_id,
                    "database": str(self.database),
                    "artifact_root": str(self.artifact_root),
                    "synthetic_evidence_scope": (
                        "Deterministic mechanics only; synthetic records are not semantic verdict quality."
                    ),
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        print(f"production journey passed: mode={self.mode} depth={self.depth}")
        print(f"journey artifacts: {self.run_dir}")
        print("synthetic evidence scope: deterministic mechanics only; no semantic verdict claim")
        return result

    def _read_json(self, path: Path, description: str) -> Dict[str, Any]:
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise JourneyFailure(f"could not read {description} {path}: {error}") from error
        if not isinstance(value, dict):
            raise JourneyFailure(f"{description} must be a JSON object: {path}")
        return value

    def _validate_profile_shape(self, profile: Dict[str, Any], *, require_loaded: bool) -> None:
        if not require_loaded:
            return
        required = {"config_version", "artifact_root", "review_policies", "artifact_schemas", "revision_links"}
        missing = sorted(required.difference(profile))
        if missing:
            raise JourneyFailure(f"high-rigor profile is missing fields: {', '.join(missing)}")
        if profile.get("config_version") != "high-rigor-3":
            raise JourneyFailure(
                f"journey requires high-rigor-3, got {profile.get('config_version')!r}"
            )
        schemas = profile.get("artifact_schemas")
        if not isinstance(schemas, dict) or set(schemas) != set(SUBJECTS):
            raise JourneyFailure("high-rigor profile must declare all five artifact schemas")
        links = profile.get("revision_links")
        expected_links = [
            {"from": "design.json", "field": "intent_revision", "to": "intent.json"},
            {"from": "plan.json", "field": "design_revision", "to": "design.json"},
            {
                "from": "implementation-report.json",
                "field": "plan_revision",
                "to": "plan.json",
            },
            {"from": "validation-report.json", "field": "intent_revision", "to": "intent.json"},
        ]
        if links != expected_links:
            raise JourneyFailure("high-rigor profile revision-link table is not the shipped table")
        policies = profile.get("review_policies")
        if not isinstance(policies, dict):
            raise JourneyFailure("high-rigor profile review_policies must be an object")
        for gate in GATE_SUBJECT:
            if not isinstance(policies.get(gate), list):
                raise JourneyFailure(f"high-rigor profile is missing policy gate {gate}")
            for entry in policies[gate]:
                if not isinstance(entry, dict) or not isinstance(entry.get("id"), str):
                    raise JourneyFailure(f"high-rigor profile has malformed {gate} axis")

    def _dump_packaged_data(self) -> None:
        assert self.data_root is not None
        self.data_root.parent.mkdir(parents=True, exist_ok=True)
        command = [str(self.provider), "data-dump", str(self.data_root)]
        completed = subprocess.run(command, text=True, capture_output=True, check=False)
        if completed.returncode != 0:
            raise JourneyFailure(
                "packaged provider data-dump failed: "
                + (completed.stderr.strip() or f"exit {completed.returncode}")
            )
        dumped = self.data_root / PROFILE_SUBPATH
        if not dumped.is_file():
            raise JourneyFailure(f"data-dump did not materialize the high-rigor profile: {dumped}")
        self.profile_source = dumped
        self.fixture_root = self.data_root / FIXTURE_SUBPATH
        self.profile = self._read_json(dumped, "dumped high-rigor profile")
        self._validate_profile_shape(self.profile, require_loaded=True)
        self._validate_scenario_fixtures()

    def _prepare_profile(self) -> None:
        assert self.profile_path is not None
        assert self.artifact_root is not None
        profile = dict(self.profile)
        profile["artifact_root"] = str(self.artifact_root)
        self.profile_path.write_text(json.dumps(profile, indent=2) + "\n", encoding="utf-8")
        # All five artifact files are the shipped good calibration shapes.  A
        # source full run intentionally starts without intent to force the
        # deterministic schema-denial path before copying it in.
        assert self.fixture_root is not None
        for subject, fixture in SUBJECTS.items():
            if self.mode == "source" and subject == "intent.json":
                continue
            shutil.copy2(self.fixture_root / fixture, self.artifact_root / subject)

    def _write_provider_config(self) -> None:
        assert self.provider_config is not None
        # JSON string quoting is valid TOML basic-string quoting for these
        # paths and also handles spaces and backslashes portably.
        command = json.dumps(str(self.provider))
        self.provider_config.write_text(
            "[providers.software-change]\n"
            f"command = {command}\n"
            "args = []\n",
            encoding="utf-8",
        )

    def _probe_startup(self) -> None:
        for flag in ("--help", "-h"):
            help_output = subprocess.run(
                [str(self.provider), flag], input="", text=True, capture_output=True, check=False
            )
            if help_output.returncode != 0 or help_output.stderr or "software-change" not in help_output.stdout:
                raise JourneyFailure(
                    f"software-change {flag} startup probe failed: "
                    f"{help_output.stderr.strip() or help_output.returncode}"
                )
        for flag in ("--version", "-V"):
            provider_version = subprocess.run(
                [str(self.provider), flag], input="", text=True, capture_output=True, check=False
            )
            if provider_version.returncode != 0 or provider_version.stderr or not provider_version.stdout.strip():
                raise JourneyFailure(
                    f"software-change {flag} startup probe failed: "
                    f"{provider_version.stderr.strip() or provider_version.returncode}"
                )
            # The binary is authoritative for its package identity.  Keep the
            # probe independent of checkout metadata when running archives.
            if not provider_version.stdout.startswith("software-change ") or len(provider_version.stdout.splitlines()) != 1:
                raise JourneyFailure("software-change version probe returned malformed identity")

        version = subprocess.run(
            [str(self.engine), "--version"], text=True, capture_output=True, check=False
        )
        if version.returncode != 0 or not version.stdout.strip():
            raise JourneyFailure(
                f"loop-engine startup probe failed: {version.stderr.strip() or version.returncode}"
            )
        describe = subprocess.run(
            [str(self.provider)],
            input=json.dumps({"operation": "describe"}),
            text=True,
            capture_output=True,
            check=False,
        )
        if describe.returncode != 0:
            raise JourneyFailure(
                f"software-change startup probe failed: {describe.stderr.strip() or describe.returncode}"
            )
        try:
            workflow = json.loads(describe.stdout)
        except json.JSONDecodeError as error:
            raise JourneyFailure(f"provider describe did not return JSON: {error}") from error
        if workflow.get("id") != "software-change" or workflow.get("initial_state") != "explore":
            raise JourneyFailure("provider startup probe returned the wrong workflow")

    def _engine(self, operation: Sequence[str], *, state: str, event: str = "none", axis: str = "none") -> Dict[str, Any]:
        assert self.database is not None
        command = [str(self.engine), "--database", str(self.database), "--json"]
        command.extend(operation)
        try:
            completed = subprocess.run(command, text=True, capture_output=True, check=False)
        except OSError as error:
            raise JourneyFailure(
                f"engine {operation[0] if operation else 'operation'} could not start: {error}",
                state=state,
                event=event,
                axis=axis,
            ) from error
        try:
            response = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise JourneyFailure(
                f"engine {operation[0] if operation else 'operation'} returned non-JSON "
                f"(exit={completed.returncode}): {error}; stderr={completed.stderr.strip()!r}",
                state=state,
                event=event,
                axis=axis,
            ) from error
        if not isinstance(response, dict):
            raise JourneyFailure("engine response is not an object", state=state, event=event, axis=axis)
        return response

    def _start(self) -> None:
        assert self.profile_path is not None
        assert self.provider_config is not None
        response = self._engine(
            [
                "--config",
                str(self.provider_config),
                "--timeout-ms",
                "30000",
                "start",
                "--id",
                self.run_id,
                "software-change",
                "@" + str(self.profile_path),
                "production journey",
            ],
            state="explore",
            event="start",
        )
        self._expect_status(response, "completed", event="start", state="explore")
        result = response.get("result", {})
        if result.get("run", {}).get("id") != self.run_id:
            raise JourneyFailure("start did not preserve the caller-owned run ID", state="explore", event="start")

    def _show(self) -> Dict[str, Any]:
        response = self._engine(["show", self.run_id], state=self.state, event="show")
        self._expect_status(response, "completed", event="show", state=self.state)
        return response["result"]

    def _assert_show(self, expected_state: str, event: str) -> Dict[str, Any]:
        shown = self._show()
        actual = shown.get("current_state")
        if actual != expected_state:
            raise JourneyFailure(
                f"expected state {expected_state}, got {actual}", state=self.state, event=event
            )
        self.state = expected_state
        if not isinstance(shown.get("requestable_events"), list):
            raise JourneyFailure("show omitted requestable_events", state=self.state, event=event)
        return shown

    def _append_marker(self, record_id: str, *, equals: bool) -> None:
        data = json.dumps(
            {
                "scope": "deterministic-mechanics",
                "synthetic_evidence": True,
                "semantic_verdict_quality": "not tested",
            },
            separators=(",", ":"),
        )
        record_option = f"--record-id={record_id}" if equals else "--record-id"
        operation: List[str] = ["append", record_option]
        if not equals:
            operation.append(record_id)
        operation.extend([self.run_id, "journey-marker", data])
        response = self._engine(operation, state=self.state, event="append", axis="record-id")
        self._expect_status(
            response, "completed", event="append", axis="record-id", state=self.state
        )
        context = response.get("result", {}).get("context", {})
        if context.get("id") != record_id:
            raise JourneyFailure(
                f"append did not preserve exact record ID {record_id!r}",
                state=self.state,
                event="append",
                axis="record-id",
            )

    def _assert_marker_persistence(self) -> None:
        shown = self._assert_show(self.state, "marker-show")
        context_ids = [record.get("id") for record in shown.get("context", [])]
        for record_id in ("journey-marker-separate", "journey-marker-equals"):
            if record_id not in context_ids:
                raise JourneyFailure(
                    f"show lost caller-owned record ID {record_id!r}", state=self.state, event="show", axis="record-id"
                )
        history = self._engine(["history", self.run_id], state=self.state, event="history")
        self._expect_status(
            history, "completed", event="history", axis="record-id", state=self.state
        )
        history_ids = [
            entry.get("action", {}).get("context_record_id")
            for entry in history.get("result", [])
        ]
        for record_id in ("journey-marker-separate", "journey-marker-equals"):
            if record_id not in history_ids:
                raise JourneyFailure(
                    f"history lost caller-owned record ID {record_id!r}", state=self.state, event="history", axis="record-id"
                )

    def _event(self, event: str, axis: str = "none") -> Dict[str, Any]:
        response = self._engine(["event", self.run_id, event], state=self.state, event=event, axis=axis)
        return response

    def _expect_denial(self, event: str, axis: str, code: str) -> Dict[str, Any]:
        response = self._event(event, axis)
        self._expect_status(
            response, "rejected", event=event, axis=axis, state=self.state
        )
        if response.get("code") != code:
            raise JourneyFailure(
                f"expected denial {code}, got {response.get('code')}", state=self.state, event=event, axis=axis
            )
        self._assert_show(self.state, event + "-denied")
        return response

    def _expect_allow(self, event: str, target: str) -> Dict[str, Any]:
        response = self._event(event)
        self._expect_status(response, "completed", event=event, state=self.state)
        if response.get("result", {}).get("run", {}).get("current_state") != target:
            raise JourneyFailure(
                f"event {event} did not reach {target}", state=self.state, event=event
            )
        self._assert_show(target, event)
        return response

    def _append_evidence(self, gate: str) -> None:
        subject = GATE_SUBJECT[gate]
        revision = self._fixture_revision(subject)
        axes = self.profile["review_policies"][gate]
        for entry in axes:
            axis = entry["id"]
            required_authors = int(entry.get("required_authors", 1))
            # Two authors are used for every axis, including N=1 axes.  This
            # makes independence explicit while keeping the fixture synthetic.
            for index, suffix in enumerate(("a", "b")):
                if index >= max(2, required_authors):
                    break
                author = f"synthetic-{gate}-{axis}-{suffix}"
                record_id = f"evidence-{gate}-{axis}-{suffix}"
                data = {
                    "gate": gate,
                    "policy_id": axis,
                    "result": "pass",
                    "findings": "",
                    "author": {"name": author, "kind": "script"},
                    "subject": subject,
                    "subject_revision": revision,
                    "config_version": self.profile["config_version"],
                }
                record = json.dumps(data, separators=(",", ":"))
                response = self._engine(
                    ["append", f"--record-id={record_id}", self.run_id, "review-evidence", record],
                    state=self.state,
                    event="append",
                    axis=axis,
                )
                self._expect_status(
                    response, "completed", event="append", axis=axis, state=self.state
                )
                if response.get("result", {}).get("context", {}).get("id") != record_id:
                    raise JourneyFailure(
                        f"evidence record ID was changed for {gate}/{axis}",
                        state=self.state,
                        event="append",
                        axis=axis,
                    )

    def _fixture_revision(self, subject: str) -> str:
        assert self.fixture_root is not None
        fixture = self._read_json(
            self.fixture_root / SUBJECTS[subject], f"fixture {subject}"
        )
        revision = fixture.get("revision")
        if not isinstance(revision, str) or not revision:
            raise JourneyFailure(f"fixture {subject} has no revision")
        return revision

    def _run_checked_prefix(self) -> None:
        if self.mode == "source":
            self._expect_denial("intent-ready", "intent", "software-change-schema-invalid")
            assert self.artifact_root is not None
            assert self.fixture_root is not None
            shutil.copy2(
                self.fixture_root / SUBJECTS["intent.json"],
                self.artifact_root / "intent.json",
            )
        self._expect_denial("intent-ready", "intent", "software-change-review-incomplete")
        self._append_evidence("intent")
        self._expect_allow("intent-ready", "design")
        if self.mode == "packaged":
            # Packaged smoke intentionally ends after one checked production
            # transition; the source adapter owns full graph traversal.
            self._assert_show("design", "packaged-prefix-end")

    def _run_full_source(self) -> None:
        self._expect_denial("intent-ready", "intent", "software-change-schema-invalid")
        assert self.artifact_root is not None
        assert self.fixture_root is not None
        shutil.copy2(
            self.fixture_root / SUBJECTS["intent.json"],
            self.artifact_root / "intent.json",
        )
        self._expect_denial("intent-ready", "intent", "software-change-review-incomplete")
        self._append_evidence("intent")
        self._expect_allow("intent-ready", "design")

        # The design route checks every configured revision link.  Mutating the
        # copied shipped-shape link must deny deterministically before any
        # evidence is considered; restore it before the real traversal.
        design_path = self.artifact_root / "design.json"
        design = self._read_json(design_path, "design artifact")
        original_link = design["intent_revision"]
        design["intent_revision"] = "journey-link-mismatch"
        design_path.write_text(json.dumps(design, indent=2) + "\n", encoding="utf-8")
        denial = self._expect_denial("design-ready", "design", "software-change-schema-invalid")
        violations = denial.get("details", {}).get("violations", [])
        if not any("revision link" in str(item.get("message", "")) for item in violations):
            raise JourneyFailure(
                "design link mutation did not produce a revision-link diagnostic",
                state=self.state,
                event="design-ready",
                axis="design",
            )
        design["intent_revision"] = original_link
        design_path.write_text(json.dumps(design, indent=2) + "\n", encoding="utf-8")
        self._expect_allow("design-ready", "design-review")

        self._expect_denial("approved", "design-review", "software-change-review-incomplete")
        self._append_evidence("design-review")
        self._expect_allow("approved", "plan")

        self._expect_allow("plan-ready", "plan-review")
        self._expect_denial("approved", "plan-review", "software-change-review-incomplete")
        self._append_evidence("plan-review")
        self._expect_allow("approved", "implement")

        self._expect_allow("implementation-ready", "implementation-review")
        self._expect_denial("approved", "implementation-review", "software-change-review-incomplete")
        self._append_evidence("implementation-review")
        self._expect_allow("approved", "validation")

        self._expect_denial("passed", "validation", "software-change-review-incomplete")
        self._append_evidence("validation")
        self._expect_allow("passed", "end")
        shown = self._assert_show("end", "terminal-show")
        if shown.get("lifecycle") != "final":
            raise JourneyFailure("full journey did not reach final lifecycle", state=self.state, event="passed")
        if shown.get("requestable_events") != []:
            raise JourneyFailure("final journey exposed requestable events", state=self.state, event="show")

        history = self._engine(["history", self.run_id], state=self.state, event="history")
        self._expect_status(history, "completed", event="history", state=self.state)
        entries = history.get("result", [])
        transitions = [entry for entry in entries if entry.get("action", {}).get("kind") == "transition"]
        if len(transitions) < 15:
            raise JourneyFailure(
                f"history omitted expected checked denials/commits: only {len(transitions)} transitions",
                state=self.state,
                event="history",
            )
        if not any(
            entry.get("action", {}).get("outcome", {}).get("outcome") == "denied"
            for entry in transitions
        ):
            raise JourneyFailure("history omitted expected denial lineage", state=self.state, event="history")

    @staticmethod
    def _expect_status(
        response: Dict[str, Any],
        expected: str,
        *,
        event: str,
        axis: str = "none",
        state: str = "unknown",
    ) -> None:
        actual = response.get("status")
        if actual != expected:
            raise JourneyFailure(
                f"expected {expected} response, got {actual}: {response}",
                state=state,
                event=event,
                axis=axis,
            )


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("source", "packaged"), required=True)
    parser.add_argument("--engine", required=True, help="loop-engine executable")
    parser.add_argument("--provider", required=True, help="software-change executable")
    parser.add_argument("--data-root", required=True, help="source root or empty packaged dump root")
    parser.add_argument("--work-root", required=True, help="isolated temporary journey parent")
    parser.add_argument(
        "--profile",
        required=True,
        help="source profile path, or high-rigor.json in packaged mode",
    )
    parser.add_argument(
        "--traversal-depth",
        choices=("full", "checked-prefix"),
        required=True,
        help="full source graph or checked production prefix",
    )
    return parser.parse_args(argv)


def self_test() -> int:
    """Prove unsupported adapter/depth pairs fail before touching work roots."""
    invalid_pairs = (("source", "checked-prefix"), ("packaged", "full"))
    with tempfile.TemporaryDirectory(prefix="production-journey-self-test-") as temp:
        root = Path(temp)
        executable = Path(sys.executable).resolve()
        for mode, depth in invalid_pairs:
            data_root = root / f"{mode}-data"
            work_root = root / f"{mode}-work"
            args = argparse.Namespace(
                mode=mode,
                traversal_depth=depth,
                engine=str(executable),
                provider=str(executable),
                data_root=str(data_root),
                work_root=str(work_root),
                profile="high-rigor.json",
            )
            try:
                Journey(args).preflight()
            except JourneyFailure as error:
                if "unsupported mode/traversal-depth pair" not in str(error):
                    raise JourneyFailure(
                        f"negative self-test got wrong error for {mode}/{depth}: {error}"
                    ) from error
            else:
                raise JourneyFailure(f"invalid pair unexpectedly accepted: {mode}/{depth}")
            if data_root.exists() or work_root.exists():
                raise JourneyFailure(
                    f"invalid pair mutated filesystem for {mode}/{depth}"
                )
    print("production journey interface self-test passed: invalid adapter/depth pairs rejected pre-mutation")
    return 0


def main(argv: Optional[Sequence[str]] = None) -> int:
    raw_argv = list(sys.argv[1:] if argv is None else argv)
    try:
        if raw_argv == ["--self-test"]:
            return self_test()
        args = parse_args(raw_argv)
        Journey(args).run()
        return 0
    except JourneyFailure as error:
        print(
            "production journey failed: "
            f"{error} (state={error.state}, event={error.event}, axis={error.axis})",
            file=sys.stderr,
        )
        return 1
    except (OSError, subprocess.SubprocessError) as error:
        print(f"production journey failed before assertion: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
