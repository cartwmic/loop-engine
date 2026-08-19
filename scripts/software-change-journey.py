#!/usr/bin/env python3
"""Run the source and packaged software-change public-boundary journey.

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

``--self-test`` executes the three provider skill constructors against shipped
profiles (software-change high-rigor design-review, policy-document semantic
policies/target/mode, research verify and synthesize), asserts root AGENTS
rules, and prints ``worker-data skill/root policy assertions passed`` only after
all pass. Source full mode binds deterministic stdin-capturing workers that emit
conforming JSON or exit-0 refusal text; after overlay failure, persisted
summary/captures, and the compact one-key ``artifact_root`` stdin proof, it
prints ``contracted fan-out failure``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

import work_slot_journey

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
SUCCESSOR_ROUTE_CASES = (
    ("design-review", "revise-intent", "explore"),
    ("plan-review", "revise-design", "design"),
    ("plan-review", "revise-intent", "explore"),
    ("implementation-review", "revise-plan", "plan"),
    ("implementation-review", "revise-design", "design"),
    ("implementation-review", "revise-intent", "explore"),
    ("validation", "revise-plan", "plan"),
    ("validation", "revise-design", "design"),
    ("validation", "revise-intent", "explore"),
)
BOUND_SLOT_ID = "explore-intent"
UNBOUND_INVOKE_SLOT_ID = "design-draft"
SOFTWARE_CHANGE_SLOT_IDS = (
    "explore-intent",
    "design-draft",
    "design-review",
    "plan-draft",
    "plan-review",
    "implement",
    "implementation-review",
    "validate",
)
WORK_SLOT_PROOF = [
    "frozen sparse work_slot_bindings in initial_input",
    "show work_slots catalog snapshot",
    "bound instruction redaction",
    "unbound invoke rejection",
    "event gated before succeeded invoke",
    "dummy worker packet receipt",
    "overlay succeeded then checked event",
    "unbound states keep stored instructions",
    "history invocation started and succeeded",
]
DUMMY_WORKER_PROOF = [
    "copied shipped profiles omit work_slot_bindings so implement and reviews are unbound",
    "graph-runner dummy --task-worker capture_dir and inner exits",
    "fan-out dummy --worker bound/ad hoc, capture_dir, inner nonzero collector 0",
    "preview-bindings exits nonzero on zero-worker fan-out and creates no run",
    "preview-bindings warns when pi has --no-extensions and no -e",
    "opt-in dummy implement/review bindings may include -e args",
    "PATH stub pi default argv --print --no-skills --no-extensions without --no-context-files or --tools",
    "bound fan-out show heartbeat overlay_meaning elapsed remaining capture_dir inner_workers",
    "contracted fan-out exit-0 conformance summary and failed-overlay capture persistence",
    "bound run-plan-graph inner workers in task order plus capture isolation",
    "dummy plan-graph summarizer writes implementation-report.json; ordinary dummy tasks do not",
    "overlay-running bound fan-out invocation-progress names invocation capture_dir graph steps; show inner_workers empty",
    "overlay-running bound run-plan-graph invocation-progress names task ids plus summarizer; show inner_workers empty",
    "omitted fan-out yaml has no max_active_steps; omitted run-plan-graph yaml has max_active_steps: 4",
    "set --max-active N in bound argv and ad-hoc fan-out yaml; N=1 never two ordinary steps running",
    "progress-query failure leaves overlay running or succeeded from facade waitpid",
    "invocation-progress names sidecar and session traces with last_modified_ms without parsing stdout",
    "no live model",
]


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
        self.work_slot_bindings: Dict[str, Any] = {}
        self.dummy_worker_proof: List[str] = []
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
        self.run_dir = Path(tempfile.mkdtemp(prefix="software-change-journey-", dir=self.work_root))
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
        self._prove_work_slots_at_start()

        successor_route_cases = 0
        if self.mode == "source" and self.depth == "full":
            self._run_full_source()
            successor_route_cases = len(SUCCESSOR_ROUTE_CASES)
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
                    "successor_route_cases": successor_route_cases,
                    "work_slot_proof": WORK_SLOT_PROOF,
                    "dummy_worker_proof": self.dummy_worker_proof,
                    "synthetic_evidence_scope": (
                        "Deterministic mechanics only; synthetic records are not semantic verdict quality."
                    ),
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        print(f"software-change journey passed: mode={self.mode} depth={self.depth}")
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
        required = {"config_version", "review_policies", "artifact_schemas", "revision_links"}
        missing = sorted(required.difference(profile))
        if missing:
            raise JourneyFailure(f"high-rigor profile is missing fields: {', '.join(missing)}")
        if profile.get("config_version") != "high-rigor-5":
            raise JourneyFailure(
                f"journey requires high-rigor-5, got {profile.get('config_version')!r}"
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
        shipped = profile.get("work_slot_bindings")
        try:
            work_slot_journey.assert_shipped_path_names(shipped)
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state="explore", event="start") from error
        # Keep the existing sparse dummy overlay: only explore-intent is bound so
        # implement and review slots stay unbound on this full-journey start.
        self.work_slot_bindings = work_slot_journey.bindings_for([BOUND_SLOT_ID])
        profile["work_slot_bindings"] = self.work_slot_bindings
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

    def _engine_for(
        self,
        run_id: str,
        operation: Sequence[str],
        *,
        state: str,
        event: str = "none",
        axis: str = "none",
    ) -> Dict[str, Any]:
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

    def _engine(
        self,
        operation: Sequence[str],
        *,
        state: str,
        event: str = "none",
        axis: str = "none",
    ) -> Dict[str, Any]:
        return self._engine_for(
            self.run_id, operation, state=state, event=event, axis=axis
        )

    def _engine_call(self, run_id: str, *, state: str) -> work_slot_journey.EngineCall:
        def call(operation: Sequence[str]) -> Dict[str, Any]:
            event = operation[0] if operation else "none"
            return self._engine_for(run_id, operation, state=state, event=event)

        return call

    def _prove_work_slots_at_start(self) -> None:
        assert self.artifact_root is not None
        try:
            work_slot_journey.prove_bound_visit(
                self._engine_call(self.run_id, state="explore"),
                run_id=self.run_id,
                catalog=SOFTWARE_CHANGE_SLOT_IDS,
                bindings=self.work_slot_bindings,
                bound_slot_id=BOUND_SLOT_ID,
                unbound_slot_id=UNBOUND_INVOKE_SLOT_ID,
                gated_event="intent-ready",
                artifact_root=self.artifact_root,
                expected_state="explore",
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state="explore", event="invoke") from error
        self.state = "explore"

    def _invoke_bound_slot(self, run_id: str, *, state: str) -> None:
        try:
            work_slot_journey.invoke_until_succeeded(
                self._engine_call(run_id, state=state),
                run_id,
                BOUND_SLOT_ID,
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state=state, event="invoke") from error

    def _assert_unbound_design(self) -> None:
        shown = self._assert_show("design", "unbound-instructions")
        try:
            work_slot_journey.assert_unbound_instructions(shown, "design.json")
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state="design", event="show") from error

    def _start(self) -> None:
        self._start_run(self.run_id)

    def _start_run(self, run_id: str) -> None:
        assert self.profile_path is not None
        assert self.provider_config is not None
        response = self._engine_for(
            run_id,
            [
                "--config",
                str(self.provider_config),
                "--timeout-ms",
                "30000",
                "start",
                "--id",
                run_id,
                "software-change",
                "@" + str(self.profile_path),
                "software-change journey",
            ],
            state="explore",
            event="start",
        )
        self._expect_status(response, "completed", event="start", state="explore")
        result = response.get("result", {})
        if result.get("run", {}).get("id") != run_id:
            raise JourneyFailure(
                "start did not preserve the caller-owned run ID", state="explore", event="start"
            )

    def _show_for(self, run_id: str, *, state: str, event: str) -> Dict[str, Any]:
        response = self._engine_for(
            run_id, ["show", run_id], state=state, event=event
        )
        self._expect_status(response, "completed", event=event, state=state)
        return response["result"]

    def _show(self) -> Dict[str, Any]:
        return self._show_for(self.run_id, state=self.state, event="show")

    def _assert_show_for(
        self, run_id: str, expected_state: str, event: str
    ) -> Dict[str, Any]:
        shown = self._show_for(run_id, state=expected_state, event=event)
        actual = shown.get("current_state")
        if actual != expected_state:
            raise JourneyFailure(
                f"expected state {expected_state}, got {actual}",
                state=expected_state,
                event=event,
            )
        if not isinstance(shown.get("requestable_events"), list):
            raise JourneyFailure(
                "show omitted requestable_events", state=expected_state, event=event
            )
        return shown

    def _assert_show(self, expected_state: str, event: str) -> Dict[str, Any]:
        shown = self._assert_show_for(self.run_id, expected_state, event)
        self.state = expected_state
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

    def _event_for(
        self, run_id: str, event: str, *, state: str, axis: str = "none"
    ) -> Dict[str, Any]:
        return self._engine_for(
            run_id, ["event", run_id, event], state=state, event=event, axis=axis
        )

    def _event(self, event: str, axis: str = "none") -> Dict[str, Any]:
        return self._event_for(self.run_id, event, state=self.state, axis=axis)

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

    def _expect_allow_for(
        self, run_id: str, state: str, event: str, target: str
    ) -> Dict[str, Any]:
        response = self._event_for(run_id, event, state=state)
        self._expect_status(response, "completed", event=event, state=state)
        if response.get("result", {}).get("run", {}).get("current_state") != target:
            raise JourneyFailure(
                f"event {event} did not reach {target}", state=state, event=event
            )
        self._assert_show_for(run_id, target, event)
        return response

    def _expect_allow(self, event: str, target: str) -> Dict[str, Any]:
        response = self._expect_allow_for(self.run_id, self.state, event, target)
        self.state = target
        return response

    def _append_evidence(self, gate: str) -> None:
        self._append_evidence_for(self.run_id, gate, state=self.state)

    def _append_evidence_for(
        self, run_id: str, gate: str, *, state: str, record_prefix: str = ""
    ) -> None:
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
                record_id = f"{record_prefix}evidence-{gate}-{axis}-{suffix}"
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
                response = self._engine_for(
                    run_id,
                    ["append", f"--record-id={record_id}", run_id, "review-evidence", record],
                    state=state,
                    event="append",
                    axis=axis,
                )
                self._expect_status(
                    response, "completed", event="append", axis=axis, state=state
                )
                if response.get("result", {}).get("context", {}).get("id") != record_id:
                    raise JourneyFailure(
                        f"evidence record ID was changed for {gate}/{axis}",
                        state=state,
                        event="append",
                        axis=axis,
                    )

    def _prepare_successor_state(self, run_id: str, target: str) -> None:
        self._start_run(run_id)
        self._invoke_bound_slot(run_id, state="explore")
        self._append_evidence_for(
            run_id, "intent", state="explore", record_prefix=f"{run_id}-"
        )
        self._expect_allow_for(run_id, "explore", "intent-ready", "design")
        self._expect_allow_for(run_id, "design", "design-ready", "design-review")
        if target == "design-review":
            return

        self._append_evidence_for(
            run_id, "design-review", state="design-review", record_prefix=f"{run_id}-"
        )
        self._expect_allow_for(run_id, "design-review", "approved", "plan")
        self._expect_allow_for(run_id, "plan", "plan-ready", "plan-review")
        if target == "plan-review":
            return

        self._append_evidence_for(
            run_id, "plan-review", state="plan-review", record_prefix=f"{run_id}-"
        )
        self._expect_allow_for(run_id, "plan-review", "approved", "implement")
        self._expect_allow_for(
            run_id, "implement", "implementation-ready", "implementation-review"
        )
        if target == "implementation-review":
            return

        self._append_evidence_for(
            run_id,
            "implementation-review",
            state="implementation-review",
            record_prefix=f"{run_id}-",
        )
        self._expect_allow_for(
            run_id, "implementation-review", "approved", "validation"
        )
        if target != "validation":
            raise JourneyFailure(f"unsupported successor route source state: {target}")

    def _run_successor_route_proof(self) -> None:
        for index, (source, event, target) in enumerate(SUCCESSOR_ROUTE_CASES, start=1):
            run_id = f"successor-route-{index:02d}-{event}"
            self._prepare_successor_state(run_id, source)
            shown = self._assert_show_for(run_id, source, "route-exposure")
            candidates = [
                candidate
                for candidate in shown["requestable_events"]
                if candidate.get("event") == event
            ]
            if len(candidates) != 1:
                raise JourneyFailure(
                    f"successor run exposed {len(candidates)} {event!r} routes from {source}",
                    state=source,
                    event=event,
                    axis="route",
                )
            candidate = candidates[0]
            if candidate.get("target") != target or candidate.get("kind") != "check-free":
                raise JourneyFailure(
                    f"successor run exposed wrong {source}/{event} route: {candidate}",
                    state=source,
                    event=event,
                    axis="route",
                )

            response = self._event_for(run_id, event, state=source, axis="route")
            self._expect_status(response, "completed", event=event, state=source, axis="route")
            committed = response.get("result", {}).get("run", {})
            if committed.get("id") != run_id or committed.get("current_state") != target:
                raise JourneyFailure(
                    f"live {source}/{event} request committed wrong run target: {committed}",
                    state=source,
                    event=event,
                    axis="route",
                )
            self._assert_show_for(run_id, target, "route-persisted")
            history = self._engine_for(
                run_id, ["history", run_id], state=target, event="history", axis="route"
            )
            self._expect_status(history, "completed", event="history", state=target, axis="route")
            if not any(
                entry.get("action", {}).get("kind") == "transition"
                and entry["action"].get("transition", {}).get("source") == source
                and entry["action"]["transition"].get("event") == event
                and entry["action"]["transition"].get("target") == target
                and entry["action"].get("outcome", {}).get("outcome") == "committed"
                for entry in history.get("result", [])
            ):
                raise JourneyFailure(
                    f"history omitted committed {source}/{event}/{target} route",
                    state=target,
                    event="history",
                    axis="route",
                )
        print(f"successor route proof passed: {len(SUCCESSOR_ROUTE_CASES)} fresh runs")

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
        self._assert_unbound_design()
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
        self._assert_unbound_design()

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

        self._run_successor_route_proof()
        self._run_dummy_worker_proofs()

    def _run_dummy_worker_proofs(self) -> None:
        """Prove heartbeat, capture isolation, preview fail-closed, and sandbox argv."""
        assert self.run_dir is not None
        assert self.profile_source is not None
        assert self.fixture_root is not None
        assert_worker_data_skill_and_root_policy()
        proof_root = self.run_dir / "dummy-worker-proofs"
        try:
            work_slot_journey.prove_shipped_software_change_profiles(self.data_root)
            work_slot_journey.prove_graph_runner(
                provider=self.provider,
                work_dir=proof_root / "graph-runner",
            )
            work_slot_journey.prove_fan_out(
                engine=self.engine,
                work_dir=proof_root / "fan-out",
            )
            work_slot_journey.prove_preview_fail_closed(
                engine=self.engine,
                work_dir=proof_root / "preview-fail-closed",
            )
            work_slot_journey.prove_preview_pi_extension_warnings(
                engine=self.engine,
                work_dir=proof_root / "preview-pi-extension-warnings",
            )
            work_slot_journey.prove_default_sandbox_argv(
                provider=self.provider,
                work_dir=proof_root / "default-sandbox-argv",
            )
            work_slot_journey.prove_bound_fan_out_heartbeat(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "bound-fan-out-heartbeat",
            )
            work_slot_journey.prove_bound_contracted_fan_out_failure(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "bound-contracted-fan-out-failure",
            )
            work_slot_journey.prove_bound_graph_runner_heartbeat(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "bound-graph-runner-heartbeat",
            )
            work_slot_journey.prove_overlay_running_bound_fan_out_progress(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "overlay-running-bound-fan-out",
            )
            work_slot_journey.prove_overlay_running_bound_graph_runner_progress(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "overlay-running-bound-graph-runner",
            )
            work_slot_journey.prove_max_active_bound_fan_out(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "max-active-bound-fan-out",
            )
            work_slot_journey.prove_max_active_bound_graph_runner(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "max-active-bound-graph-runner",
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(
                str(error),
                state="end",
                event="dummy-worker-proofs",
            ) from error
        self.dummy_worker_proof = list(DUMMY_WORKER_PROOF)
        print(
            "dummy worker proofs passed: shipped profiles, graph-runner, fan-out, "
            "preview-bindings fail-closed, missing -e warning, default sandbox argv, bound heartbeats, "
            "overlay-running invocation-progress, omitted vs set --max-active, progress-query overlay-untouched"
        )
        print("contracted fan-out failure")

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
        help="full source graph or checked software-change prefix",
    )
    return parser.parse_args(argv)


def _sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class ConstructorClosed(RuntimeError):
    """A provider skill constructor rejected invalid input."""


def _extract_jq_after(skill: str, anchor: str) -> str:
    start = skill.index(anchor) + len(anchor)
    if start >= len(skill) or skill[start] != "'":
        raise JourneyFailure(f"skill constructor was not a quoted jq program after {anchor!r}")
    start += 1
    if start < len(skill) and skill[start] == "\n":
        start += 1
    end = skill.index("' \"$PROFILE\"", start)
    return skill[start:end]


def _extract_heredoc_jq(skill: str) -> str:
    marker = "<<'JQ'\n"
    start = skill.index(marker) + len(marker)
    end = skill.index("\nJQ\n", start)
    return skill[start:end]


def _run_jq(filter_text: str, profile: Path, extra: Sequence[str]) -> str:
    result = subprocess.run(
        ["jq", *extra, filter_text, str(profile)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise ConstructorClosed(detail or f"jq exited {result.returncode}")
    return result.stdout


def _write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def _load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def _fan_out_workers(binding: Dict[str, Any], *, engine: str) -> List[Dict[str, Any]]:
    if binding.get("command") != engine:
        raise JourneyFailure(f"constructor binding command {binding.get('command')!r} != {engine!r}")
    args = binding.get("args")
    if not isinstance(args, list) or not args or args[0] != "fan-out":
        raise JourneyFailure(f"constructor binding was not fan-out: {binding}")
    workers: List[Dict[str, Any]] = []
    index = 1
    while index < len(args):
        if args[index] != "--worker" or index + 1 >= len(args):
            raise JourneyFailure(f"constructor fan-out args were not worker pairs: {args}")
        worker = json.loads(args[index + 1])
        if not isinstance(worker, dict):
            raise JourneyFailure(f"constructor worker is not an object: {worker}")
        workers.append(worker)
        index += 2
    return workers


def _policy_author_pairs(
    policies: Sequence[Dict[str, Any]], roster: Sequence[Dict[str, Any]]
) -> List[tuple[Dict[str, Any], Dict[str, Any]]]:
    pairs: List[tuple[Dict[str, Any], Dict[str, Any]]] = []
    for policy in policies:
        count = policy.get("required_authors", 1)
        if count is None:
            count = 1
        if not isinstance(count, int) or isinstance(count, bool) or count < 1:
            raise JourneyFailure(f"source required_authors is not a positive integer: {policy}")
        if count > len(roster):
            raise JourneyFailure("source policy needs more authors than the roster provides")
        for index in range(count):
            pairs.append((policy, dict(roster[index])))
    return pairs


def _assert_worker_assignment(
    worker: Dict[str, Any],
    *,
    policy: Dict[str, Any],
    roster_entry: Dict[str, Any],
    base_preamble: str,
    schema: Dict[str, Any],
    pi_command: str,
    fragments: Sequence[str],
) -> None:
    if worker.get("command") != pi_command:
        raise JourneyFailure(f"worker command {worker.get('command')!r} != {pi_command!r}")
    if worker.get("output_schema") != schema:
        raise JourneyFailure(
            f"worker output_schema {worker.get('output_schema')} != {schema}"
        )
    preamble = worker.get("preamble")
    if not isinstance(preamble, str) or not preamble.startswith(base_preamble):
        raise JourneyFailure("worker preamble did not start with exact provider bytes")
    prompt = policy["example_prompt"]
    if prompt not in preamble:
        raise JourneyFailure(f"worker omitted exact example_prompt for {policy.get('id')}")
    if policy["id"] not in preamble:
        raise JourneyFailure(f"worker omitted exact axis id {policy['id']!r}")
    if roster_entry["author"] not in preamble:
        raise JourneyFailure(f"worker omitted exact author {roster_entry['author']!r}")
    args = worker.get("args")
    if not isinstance(args, list) or "--model" not in args:
        raise JourneyFailure(f"worker omitted --model: {worker}")
    model_index = args.index("--model")
    if model_index + 1 >= len(args) or args[model_index + 1] != roster_entry["model"]:
        raise JourneyFailure(
            f"worker model {args} did not freeze {roster_entry['model']!r}"
        )
    for fragment in fragments:
        if fragment not in preamble:
            raise JourneyFailure(f"worker omitted subject/assignment metadata {fragment!r}")


def _assert_preview_visibility(
    repository: Path, bindings: Dict[str, Any], workers: Sequence[Dict[str, Any]]
) -> None:
    if len(workers) == 0:
        raise JourneyFailure("constructor preview input had no workers")
    full_preambles = []
    for worker in workers:
        if "preamble" not in worker or "output_schema" not in worker:
            raise JourneyFailure(f"preview input omitted preamble/schema: {worker}")
        required = (worker.get("output_schema") or {}).get("required")
        if required != ["axis", "author", "result", "findings"]:
            raise JourneyFailure(f"preview input omitted required keys: {worker}")
        preamble = worker.get("preamble")
        if isinstance(preamble, str) and preamble:
            full_preambles.append(preamble)
    engine = repository / "target/debug/loop-engine"
    if not engine.is_file():
        raise JourneyFailure(
            "preview-bindings visibility requires target/debug/loop-engine; build loop-cli first"
        )
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        json.dump(bindings, handle)
        bindings_path = Path(handle.name)
    try:
        result = subprocess.run(
            [str(engine), "preview-bindings", f"@{bindings_path}"],
            capture_output=True,
            text=True,
        )
    finally:
        bindings_path.unlink(missing_ok=True)
    if result.returncode != 0:
        raise JourneyFailure(
            f"preview-bindings rejected constructor output: {result.stderr or result.stdout}"
        )
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise JourneyFailure(
            f"preview-bindings stdout was not JSON: {result.stdout}"
        ) from error
    preview_workers: List[Dict[str, Any]] = []
    for slot in report.get("bindings") or []:
        if not isinstance(slot, dict):
            continue
        slot_workers = slot.get("workers") or []
        if isinstance(slot_workers, list):
            preview_workers.extend(
                item for item in slot_workers if isinstance(item, dict)
            )
        args = slot.get("args") or []
        for arg in args:
            if not isinstance(arg, str):
                continue
            try:
                parsed_arg = json.loads(arg)
            except json.JSONDecodeError:
                continue
            if isinstance(parsed_arg, dict) and "preamble" in parsed_arg:
                if parsed_arg.get("preamble") != "<redacted>":
                    raise JourneyFailure(
                        "preview-bindings printed unredacted preamble in binding argv"
                    )
    if len(preview_workers) != len(workers):
        raise JourneyFailure(
            f"preview-bindings worker count {len(preview_workers)} != {len(workers)}"
        )
    for preview_worker in preview_workers:
        if preview_worker.get("has_preamble") is not True:
            raise JourneyFailure(
                f"preview-bindings omitted has_preamble: {preview_worker}"
            )
        required = (preview_worker.get("output_schema") or {}).get("required")
        if required != ["axis", "author", "result", "findings"]:
            raise JourneyFailure(
                f"preview-bindings omitted output_schema.required: {preview_worker}"
            )
        if "preamble" in preview_worker and preview_worker.get("preamble") not in (
            None,
            "<redacted>",
        ):
            raise JourneyFailure(
                f"preview-bindings exposed preamble text: {preview_worker}"
            )
    stdout = result.stdout
    for preamble in full_preambles:
        if preamble and preamble in stdout:
            raise JourneyFailure("preview-bindings leaked full provider preamble text")


def _assert_hash_guard(profile: Path) -> None:
    confirmed = _sha256_file(profile)
    original = profile.read_bytes()
    profile.write_bytes(original + b"\n")
    if _sha256_file(profile) == confirmed:
        raise JourneyFailure("pre-start hash guard would not detect a post-preview mutation")
    profile.write_bytes(original)
    if _sha256_file(profile) != confirmed:
        raise JourneyFailure("hash-guard restore mutated the resulting profile")


def _expect_constructor_closed(run, *, needle: str, context: str) -> None:
    try:
        run()
    except ConstructorClosed as error:
        if needle not in str(error):
            raise JourneyFailure(f"{context} failed for the wrong reason: {error}") from error
    else:
        raise JourneyFailure(f"{context} unexpectedly succeeded")


def assert_worker_data_skill_and_root_policy() -> None:
    """Execute provider constructors and assert root policy against revision-18 contracts."""
    repository = Path(__file__).resolve().parent.parent
    dummy_engine = "/tmp/loop-engine-constructor-proof"
    dummy_pi = "/tmp/pi-constructor-proof"
    dummy_cursor = "/tmp/cursor-provider-extension"
    dummy_bridge = "/tmp/claude-bridge-extension"
    roster = [
        {"author": "reviewer-a", "model": "model-a"},
        {"author": "reviewer-b", "model": "model-b"},
    ]
    schema_required = {"required": ["axis", "author", "result", "findings"]}

    sc_skill_path = (
        repository
        / "crates/software-change-provider/skills/using-software-change-provider/SKILL.md"
    )
    pd_skill_path = (
        repository
        / "crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md"
    )
    research_skill_path = (
        repository / "crates/research-provider/skills/using-research-provider/SKILL.md"
    )
    sc_skill = sc_skill_path.read_text(encoding="utf-8")
    pd_skill = pd_skill_path.read_text(encoding="utf-8")
    research_skill = research_skill_path.read_text(encoding="utf-8")
    if '--rawfile preamble "' in sc_skill:
        raise JourneyFailure("software-change skill still uses obsolete --rawfile preamble")
    for skill, name in (
        (sc_skill, "software-change"),
        (pd_skill, "policy-document"),
        (research_skill, "research"),
    ):
        if "--rawfile base_preamble" not in skill:
            raise JourneyFailure(f"{name} constructor omitted --rawfile base_preamble")
        if "preview-bindings" not in skill:
            raise JourneyFailure(f"{name} constructor omitted preview-bindings")
        if "SHA-256" not in skill and "SHA256" not in skill:
            raise JourneyFailure(f"{name} constructor omitted SHA-256 confirmation")

    sc_preamble_path = (
        repository / "crates/software-change-provider/data/review-worker-preamble.txt"
    )
    sc_schema_path = (
        repository / "crates/software-change-provider/data/review-worker-output-schema.json"
    )
    pd_preamble_path = (
        repository / "crates/policy-document-provider/data/semantic-review-worker-preamble.md"
    )
    pd_schema_path = (
        repository
        / "crates/policy-document-provider/data/semantic-review-worker-output-schema.json"
    )
    research_preamble_path = (
        repository / "crates/research-provider/data/review-worker-preamble.txt"
    )
    research_schema_path = (
        repository / "crates/research-provider/data/review-worker-output-schema.json"
    )
    for path in (
        sc_preamble_path,
        sc_schema_path,
        pd_preamble_path,
        pd_schema_path,
        research_preamble_path,
        research_schema_path,
    ):
        if not path.is_file():
            raise JourneyFailure(f"shipped worker data is missing: {path}")
    sc_preamble = sc_preamble_path.read_text(encoding="utf-8")
    pd_preamble = pd_preamble_path.read_text(encoding="utf-8")
    research_preamble = research_preamble_path.read_text(encoding="utf-8")
    sc_schema = _load_json(sc_schema_path)
    pd_schema = _load_json(pd_schema_path)
    research_schema = _load_json(research_schema_path)
    if sc_schema != schema_required or pd_schema != schema_required or research_schema != schema_required:
        raise JourneyFailure("provider output_schema bytes do not require axis/author/result/findings")

    sc_jq = _extract_jq_after(sc_skill, '--slurpfile roster "$ROSTER" ')
    pd_jq = _extract_heredoc_jq(pd_skill)
    research_validate_jq = _extract_jq_after(research_skill, '--argjson roster "$ROSTER_JSON" ')
    research_jq = _extract_jq_after(
        research_skill, '--slurpfile output_schema "$OUTPUT_SCHEMA_PATH" '
    )
    high_rigor = repository / "crates/software-change-provider/data/configs/high-rigor.json"
    readme_profile = repository / "crates/policy-document-provider/data/readme.json"
    agents_profile = repository / "crates/policy-document-provider/data/agents.json"
    research_profile = repository / "crates/research-provider/data/configs/standard.json"
    for shipped in (high_rigor, readme_profile, agents_profile, research_profile):
        if _load_json(shipped).get("work_slot_bindings"):
            raise JourneyFailure(f"shipped profile unexpectedly binds slots: {shipped}")

    def sc_args(slot_id: str, roster_path: Path) -> List[str]:
        return [
            "--arg",
            "slot",
            slot_id,
            "--arg",
            "engine",
            dummy_engine,
            "--arg",
            "pi",
            dummy_pi,
            "--arg",
            "cursor",
            dummy_cursor,
            "--arg",
            "bridge",
            dummy_bridge,
            "--rawfile",
            "base_preamble",
            str(sc_preamble_path),
            "--slurpfile",
            "output_schema",
            str(sc_schema_path),
            "--slurpfile",
            "roster",
            str(roster_path),
        ]

    def pd_args(roster_path: Path) -> List[str]:
        return [
            "--arg",
            "slot",
            "semantic-review",
            "--arg",
            "loop_engine",
            dummy_engine,
            "--arg",
            "pi",
            dummy_pi,
            "--arg",
            "cursor_extension",
            dummy_cursor,
            "--arg",
            "claude_bridge_extension",
            dummy_bridge,
            "--rawfile",
            "base_preamble",
            str(pd_preamble_path),
            "--slurpfile",
            "schema_documents",
            str(pd_schema_path),
            "--slurpfile",
            "roster_documents",
            str(roster_path),
        ]

    def research_args(slot_id: str, roster_json: str) -> List[str]:
        return [
            "--arg",
            "slot",
            slot_id,
            "--argjson",
            "roster",
            roster_json,
            "--arg",
            "loop_engine",
            dummy_engine,
            "--arg",
            "pi",
            dummy_pi,
            "--arg",
            "cursor_extension",
            dummy_cursor,
            "--arg",
            "claude_bridge_extension",
            dummy_bridge,
            "--rawfile",
            "base_preamble",
            str(research_preamble_path),
            "--slurpfile",
            "output_schema",
            str(research_schema_path),
        ]

    def run_sc(profile: Path, slot_id: str, roster_path: Path) -> Dict[str, Any]:
        stdout = _run_jq(sc_jq, profile, sc_args(slot_id, roster_path))
        profile.write_text(stdout, encoding="utf-8")
        return _load_json(profile)

    def run_pd(profile: Path, roster_path: Path, *, slot_id: str = "semantic-review") -> Dict[str, Any]:
        extra = pd_args(roster_path)
        extra[2] = slot_id
        stdout = _run_jq(pd_jq, profile, extra)
        profile.write_text(stdout, encoding="utf-8")
        return _load_json(profile)

    def run_research(profile: Path, slot_id: str, roster_json: str) -> Dict[str, Any]:
        extra = research_args(slot_id, roster_json)
        try:
            _run_jq(research_validate_jq, profile, ["-e", *extra[:6]])
        except ConstructorClosed as error:
            raise ConstructorClosed(
                f"invalid or insufficient policies/roster for {slot_id}: {error}"
            ) from error
        stdout = _run_jq(research_jq, profile, extra)
        profile.write_text(stdout, encoding="utf-8")
        return _load_json(profile)

    with tempfile.TemporaryDirectory(prefix="worker-data-constructor-") as temp:
        root = Path(temp)
        roster_path = root / "roster.json"
        _write_json(roster_path, roster)
        roster_json = json.dumps(roster, separators=(",", ":"))

        design_profile = root / "high-rigor-design-review.json"
        shutil.copy2(high_rigor, design_profile)
        source = _load_json(design_profile)
        result = run_sc(design_profile, "design-review", roster_path)
        if result.get("review_policies") != source.get("review_policies"):
            raise JourneyFailure("constructor mutated software-change review_policies")
        bindings = result.get("work_slot_bindings")
        if not isinstance(bindings, dict) or "design-review" not in bindings:
            raise JourneyFailure("design-review constructor omitted work_slot_bindings")
        if bindings != result["work_slot_bindings"]:
            raise JourneyFailure("preview input diverged from resulting profile bindings")
        workers = _fan_out_workers(bindings["design-review"], engine=dummy_engine)
        expected = _policy_author_pairs(source["review_policies"]["design-review"], roster)
        if len(workers) != len(expected):
            raise JourneyFailure(
                f"design-review worker count {len(workers)} != {len(expected)}"
            )
        for worker, (policy, entry) in zip(workers, expected):
            _assert_worker_assignment(
                worker,
                policy=policy,
                roster_entry=entry,
                base_preamble=sc_preamble,
                schema=sc_schema,
                pi_command=dummy_pi,
                fragments=(
                    "software-change",
                    "design-review",
                    "artifact_root",
                    f"required_author_claim: {entry['author']}",
                ),
            )
        _assert_preview_visibility(repository, bindings, workers)
        _assert_hash_guard(design_profile)

        plan_profile = root / "high-rigor-plan-review.json"
        shutil.copy2(high_rigor, plan_profile)
        plan_source = _load_json(plan_profile)
        plan_policies = plan_source["review_policies"]["plan-review"]
        if any("required_authors" in entry for entry in plan_policies):
            raise JourneyFailure("high-rigor plan-review unexpectedly sets required_authors")
        plan_result = run_sc(plan_profile, "plan-review", roster_path)
        plan_workers = _fan_out_workers(
            plan_result["work_slot_bindings"]["plan-review"], engine=dummy_engine
        )
        plan_expected = _policy_author_pairs(plan_policies, roster)
        if len(plan_expected) != len(plan_policies):
            raise JourneyFailure("absent required_authors did not default to one worker per axis")
        if len(plan_workers) != len(plan_policies):
            raise JourneyFailure(
                f"plan-review worker count {len(plan_workers)} != {len(plan_policies)}"
            )
        for worker, (policy, entry) in zip(plan_workers, plan_expected):
            if entry["author"] != roster[0]["author"]:
                raise JourneyFailure("plan-review used more than the default one roster author")
            _assert_worker_assignment(
                worker,
                policy=policy,
                roster_entry=entry,
                base_preamble=sc_preamble,
                schema=sc_schema,
                pi_command=dummy_pi,
                fragments=("software-change", "plan-review", "artifact_root"),
            )
        _assert_preview_visibility(
            repository, plan_result["work_slot_bindings"], plan_workers
        )
        _assert_hash_guard(plan_profile)

        empty_policies = root / "sc-empty-policies.json"
        shutil.copy2(high_rigor, empty_policies)
        empty_doc = _load_json(empty_policies)
        empty_doc["review_policies"]["design-review"] = []
        _write_json(empty_policies, empty_doc)
        _expect_constructor_closed(
            lambda: run_sc(empty_policies, "design-review", roster_path),
            needle="unsupported or empty policy list",
            context="software-change empty policies",
        )

        missing_prompt = root / "sc-missing-prompt.json"
        shutil.copy2(high_rigor, missing_prompt)
        missing_doc = _load_json(missing_prompt)
        missing_doc["review_policies"]["design-review"][0]["example_prompt"] = ""
        _write_json(missing_prompt, missing_doc)
        _expect_constructor_closed(
            lambda: run_sc(missing_prompt, "design-review", roster_path),
            needle="example_prompt",
            context="software-change missing example_prompt",
        )

        short_roster = root / "short-roster.json"
        _write_json(short_roster, [roster[0]])
        short_profile = root / "sc-short-roster.json"
        shutil.copy2(high_rigor, short_profile)
        _expect_constructor_closed(
            lambda: run_sc(short_profile, "design-review", short_roster),
            needle="too few entries",
            context="software-change insufficient roster",
        )

        duplicate_roster = root / "duplicate-roster.json"
        _write_json(
            duplicate_roster,
            [roster[0], {"author": roster[0]["author"], "model": "model-c"}],
        )
        duplicate_profile = root / "sc-duplicate-roster.json"
        shutil.copy2(high_rigor, duplicate_profile)
        _expect_constructor_closed(
            lambda: run_sc(duplicate_profile, "design-review", duplicate_roster),
            needle="pairwise distinct",
            context="software-change duplicate author",
        )

        empty_author = root / "empty-author-roster.json"
        _write_json(empty_author, [{"author": "", "model": "model-a"}, roster[1]])
        empty_author_profile = root / "sc-empty-author.json"
        shutil.copy2(high_rigor, empty_author_profile)
        _expect_constructor_closed(
            lambda: run_sc(empty_author_profile, "design-review", empty_author),
            needle="non-empty strings",
            context="software-change empty author",
        )

        unsupported_profile = root / "sc-unsupported-slot.json"
        shutil.copy2(high_rigor, unsupported_profile)
        _expect_constructor_closed(
            lambda: run_sc(unsupported_profile, "semantic-review", roster_path),
            needle="unsupported or empty policy list",
            context="software-change unsupported slot",
        )

        for source_profile, label in ((readme_profile, "readme"), (agents_profile, "agents")):
            target_file = root / f"{label}-target.md"
            dest = root / f"{label}-semantic.json"
            copied = _load_json(source_profile)
            copied["target"]["path"] = str(target_file.resolve())
            target_file.write_text("# target\n", encoding="utf-8")
            _write_json(dest, copied)
            pd_source = _load_json(dest)
            pd_result = run_pd(dest, roster_path)
            pd_bindings = pd_result.get("work_slot_bindings")
            if not isinstance(pd_bindings, dict) or "semantic-review" not in pd_bindings:
                raise JourneyFailure(f"{label} constructor omitted semantic-review bindings")
            pd_workers = _fan_out_workers(pd_bindings["semantic-review"], engine=dummy_engine)
            pd_expected = _policy_author_pairs(pd_source["semantic_policies"], roster)
            if len(pd_workers) != len(pd_expected):
                raise JourneyFailure(
                    f"{label} worker count {len(pd_workers)} != {len(pd_expected)}"
                )
            target_json = json.dumps(pd_source["target"], separators=(",", ":"))
            for worker, (policy, entry) in zip(pd_workers, pd_expected):
                _assert_worker_assignment(
                    worker,
                    policy=policy,
                    roster_entry=entry,
                    base_preamble=pd_preamble,
                    schema=pd_schema,
                    pi_command=dummy_pi,
                    fragments=(
                        "policy-document",
                        "semantic-review",
                        pd_source["mode"],
                        pd_source["target"]["id"],
                        pd_source["target"]["path"],
                    ),
                )
                if target_json not in worker["preamble"]:
                    raise JourneyFailure(
                        f"{label} worker omitted complete target object {target_json}"
                    )
            _assert_preview_visibility(repository, pd_bindings, pd_workers)
            _assert_hash_guard(dest)

        pd_unsupported = root / "pd-unsupported.json"
        shutil.copy2(root / "readme-semantic.json", pd_unsupported)
        _expect_constructor_closed(
            lambda: run_pd(pd_unsupported, roster_path, slot_id="design-review"),
            needle="unsupported slot",
            context="policy-document unsupported slot",
        )

        pd_empty = root / "pd-empty.json"
        empty_pd = _load_json(root / "readme-semantic.json")
        empty_pd["semantic_policies"] = []
        _write_json(pd_empty, empty_pd)
        _expect_constructor_closed(
            lambda: run_pd(pd_empty, roster_path),
            needle="semantic_policies must be non-empty",
            context="policy-document empty policies",
        )

        pd_prompt = root / "pd-missing-prompt.json"
        prompt_pd = _load_json(root / "readme-semantic.json")
        prompt_pd["semantic_policies"][0]["example_prompt"] = ""
        _write_json(pd_prompt, prompt_pd)
        _expect_constructor_closed(
            lambda: run_pd(pd_prompt, roster_path),
            needle="example_prompt",
            context="policy-document missing prompt",
        )

        pd_mode = root / "pd-missing-mode.json"
        mode_pd = _load_json(root / "readme-semantic.json")
        del mode_pd["mode"]
        _write_json(pd_mode, mode_pd)
        _expect_constructor_closed(
            lambda: run_pd(pd_mode, roster_path),
            needle="mode must be draft or audit",
            context="policy-document missing mode",
        )

        pd_target = root / "pd-missing-target.json"
        target_pd = _load_json(root / "readme-semantic.json")
        target_pd["target"]["path"] = "relative/README.md"
        _write_json(pd_target, target_pd)
        _expect_constructor_closed(
            lambda: run_pd(pd_target, roster_path),
            needle="complete {id,path}",
            context="policy-document incomplete target",
        )

        for slot_id in ("verify", "synthesize"):
            research_dest = root / f"research-{slot_id}.json"
            shutil.copy2(research_profile, research_dest)
            research_source = _load_json(research_dest)
            research_result = run_research(research_dest, slot_id, roster_json)
            research_bindings = research_result.get("work_slot_bindings")
            if not isinstance(research_bindings, dict) or slot_id not in research_bindings:
                raise JourneyFailure(f"research {slot_id} constructor omitted bindings")
            research_workers = _fan_out_workers(
                research_bindings[slot_id], engine=dummy_engine
            )
            research_expected = _policy_author_pairs(
                research_source["review_policies"][slot_id], roster
            )
            if len(research_workers) != len(research_expected):
                raise JourneyFailure(
                    f"research {slot_id} worker count {len(research_workers)} != {len(research_expected)}"
                )
            for worker, (policy, entry) in zip(research_workers, research_expected):
                _assert_worker_assignment(
                    worker,
                    policy=policy,
                    roster_entry=entry,
                    base_preamble=research_preamble,
                    schema=research_schema,
                    pi_command=dummy_pi,
                    fragments=("research", slot_id, "artifact_root"),
                )
            _assert_preview_visibility(repository, research_bindings, research_workers)
            _assert_hash_guard(research_dest)

        research_bad_slot = root / "research-bad-slot.json"
        shutil.copy2(research_profile, research_bad_slot)
        _expect_constructor_closed(
            lambda: run_research(research_bad_slot, "gather", roster_json),
            needle="invalid or insufficient",
            context="research unsupported slot",
        )

        research_empty = root / "research-empty.json"
        empty_research = _load_json(research_profile)
        empty_research["review_policies"]["verify"] = []
        _write_json(research_empty, empty_research)
        _expect_constructor_closed(
            lambda: run_research(research_empty, "verify", roster_json),
            needle="invalid or insufficient",
            context="research empty policies",
        )

        research_prompt = root / "research-missing-prompt.json"
        prompt_research = _load_json(research_profile)
        prompt_research["review_policies"]["verify"][0]["example_prompt"] = ""
        _write_json(research_prompt, prompt_research)
        _expect_constructor_closed(
            lambda: run_research(research_prompt, "verify", roster_json),
            needle="invalid or insufficient",
            context="research missing prompt",
        )

        bad_roster_json = json.dumps(
            [{"author": "reviewer-a", "model": "model-a"}, {"author": "reviewer-a", "model": "model-b"}],
            separators=(",", ":"),
        )
        research_dup = root / "research-duplicate.json"
        shutil.copy2(research_profile, research_dup)
        _expect_constructor_closed(
            lambda: run_research(research_dup, "verify", bad_roster_json),
            needle="invalid or insufficient",
            context="research duplicate author",
        )

    policy = (repository / "AGENTS.md").read_text(encoding="utf-8")
    policy_fragments = (
        "Fan-out spawn/capture/conformance mechanics belong to the engine.",
        "Providers/callers own role framing and output content",
        "Reviewers produce judgments only.",
        "Drivers run deterministic checks, `show`, capture triage, `append`, `event`, and progression.",
        "Exit 0 alone does not establish deliverable validity.",
        "overrun re-show and zero-axis review-binding rules",
        "[skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md)",
        "[docs/agent-usage.md](docs/agent-usage.md)",
    )
    for fragment in policy_fragments:
        if fragment not in policy:
            raise JourneyFailure(f"root AGENTS.md omitted policy fragment {fragment!r}")
    print("worker-data skill/root policy assertions passed")


def self_test() -> int:
    """Prove interface rejection plus worker-data and root-policy contracts."""
    invalid_pairs = (("source", "checked-prefix"), ("packaged", "full"))
    with tempfile.TemporaryDirectory(prefix="software-change-journey-self-test-") as temp:
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
    try:
        work_slot_journey.self_test_helpers()
    except work_slot_journey.WorkSlotJourneyFailure as error:
        raise JourneyFailure(f"work-slot helper self-test failed: {error}") from error
    assert_worker_data_skill_and_root_policy()
    print(
        "software-change journey interface self-test passed: invalid adapter/depth pairs rejected pre-mutation; dummy-worker helpers checked"
    )
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
            "software-change journey failed: "
            f"{error} (state={error.state}, event={error.event}, axis={error.axis})",
            file=sys.stderr,
        )
        return 1
    except (OSError, subprocess.SubprocessError) as error:
        print(f"software-change journey failed before assertion: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
