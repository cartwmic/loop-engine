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
prints ``contracted fan-out failure``. It also drives the real engine/provider
boundary cases for structural workflow rejection, final-state topology,
an initially-final run, terminal mutation rejection, a changed provider
``describe``, and an unavailable stored evaluation. It then starts a second run
from shipped minimal.json and walks the stitched hops (empty review lists
omitted, last-hop ``passed`` on the live validation review).
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

import work_slot_journey

PROFILE_SUBPATH = Path("crates/software-change-provider/data/configs/high-rigor.json")
STITCHED_PROFILE_SUBPATH = Path(
    "crates/software-change-provider/data/configs/minimal.json"
)
BOOKENDS_TOMBSTONE_ID = "LE-9000"
BOOKENDS_SCENARIO_RUN_ID = "bookends-enabled-journey"
BOOKENDS_SCENARIO_PROFILE = Path(
    "crates/software-change-provider/data/configs/minimal.json"
)
STITCHED_RUN_ID = "journey-stitched-run"
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
    "intent-review": "intent.json",
    "intent-adversarial-review": "intent.json",
    "design-review": "design.json",
    "design-adversarial-review": "design.json",
    "plan-review": "plan.json",
    "plan-adversarial-review": "plan.json",
    "implementation-review": "implementation-report.json",
    "implementation-adversarial-review": "implementation-report.json",
    "validation-review": "validation-report.json",
    "validation-adversarial-review": "validation-report.json",
}
SUCCESSOR_ROUTE_CASES = (
    ("design-review", "revise-intent", "explore"),
    ("plan-review", "revise-design", "design"),
    ("plan-review", "revise-intent", "explore"),
    ("implementation-review", "revise-plan", "plan"),
    ("implementation-review", "revise-design", "design"),
    ("implementation-review", "revise-intent", "explore"),
    ("validation-review", "revise", "validation"),
    ("validation-review", "revise-implementation", "implement"),
    ("validation-review", "revise-plan", "plan"),
    ("validation-review", "revise-design", "design"),
    ("validation-review", "revise-intent", "explore"),
)
BOUND_SLOT_ID = "intent-draft"
UNBOUND_INVOKE_SLOT_ID = "design-draft"
SOFTWARE_CHANGE_SLOT_IDS = (
    "intent-draft",
    "intent-review",
    "intent-adversarial-review",
    "design-draft",
    "design-review",
    "design-adversarial-review",
    "plan-draft",
    "plan-review",
    "plan-adversarial-review",
    "implement",
    "implementation-review",
    "implementation-adversarial-review",
    "validation-draft",
    "validation-review",
    "validation-adversarial-review",
)
STITCHED_SLOT_IDS = (
    "intent-draft",
    "design-draft",
    "plan-draft",
    "implement",
    "validation-draft",
    "validation-review",
)
STITCHED_HOPS = (
    ("explore", "intent-ready", "design"),
    ("design", "design-ready", "plan"),
    ("plan", "plan-ready", "implement"),
    ("implement", "implementation-ready", "validation"),
    ("validation", "validation-ready", "validation-review"),
)
CHECKPOINT_MUTATIONS = ("head", "add", "delete", "rename", "status", "type", "bytes")
COMPANION_SCENARIO_SUBPATH = Path(
    "crates/software-change-provider/data/calibration/companions/"
    "fictional-repo/scripts/production-journey.py"
)


def _review_stdin_kinds(slot_ids: Sequence[str]) -> dict[str, list[str]]:
    return {
        slot_id: ["finding-ledger"]
        for slot_id in slot_ids
        if "-review" in slot_id or slot_id == "implement"
    }
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
    "same-reviewer invalid-then-valid retry and invalid-twice exhaustion preserve raw attempts",
    "selected retry attempt links into a driver ledger before review evidence",
    "show exposes durable change report and provider content-agreement refusal",
    "observation-before-mutation refuses all four guarded acts",
    "invoke subset starts only selected assignments",
    "unchanged-carry and override-carry preserve explicit provenance",
    "plan-graph subset refuses missing prerequisites and summarizes resulting tree",
    "overrun overlay show/retry and distinct captures",
    "stdin-exec sidecar/propagate, spawn failure, and session directory",
    "bound run-plan-graph inner workers in task order plus capture isolation",
    "graph-level run-plan-graph working_dir reaches every task and summarizer",
    "symlink-selected checkout cwd receipts are filesystem-equivalent and see .git",
    "dummy plan-graph summarizer writes implementation-report.json; ordinary dummy tasks do not",
    "implementation ledger routing enriches exact tasks, keeps proposal inert, and preserves task stdin envelope",
    "bound reviewer reads frozen operating_context from artifact_root",
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


def assert_semantic_outcome_proof_contract(
    plan: Dict[str, Any], validation: Dict[str, Any], scenario_source: str
) -> None:
    """Check objective outcome-proof markers at the public journey boundary.

    This is deliberately not a semantic reviewer.  It rejects activity-only
    reports and token-only citations by requiring the frozen policy's named
    outcome/proof shape and by resolving each cited scenario to executable
    public CLI assertions.
    """
    objective = plan.get("objective")
    if not isinstance(objective, str):
        raise JourneyFailure("plan objective is not a string")
    objective_text = objective.lower()
    for term in ("operator", "observable", "black-box", "impracticality"):
        if term not in objective_text:
            raise JourneyFailure(
                f"plan objective omitted required outcome-proof policy term {term!r}"
            )

    outcome = validation.get("outcome")
    requirements = validation.get("requirements")
    if not isinstance(outcome, str) or len(outcome.split()) < 6:
        raise JourneyFailure("validation outcome is not a named observable outcome")
    outcome_text = outcome.lower()
    if "operator" not in outcome_text and "user" not in outcome_text:
        raise JourneyFailure("validation outcome names neither a user nor an operator")
    if not any(term in outcome_text for term in ("observe", "reach", "use", "deny", "allow")):
        raise JourneyFailure("validation outcome has no observable result")
    if not isinstance(requirements, list) or not requirements:
        raise JourneyFailure("validation report omitted requirement proof entries")

    citation_prefix = "scripts/production-journey.py::"
    for index, item in enumerate(requirements):
        if not isinstance(item, dict):
            raise JourneyFailure(f"validation requirement {index} is not an object")
        requirement = item.get("requirement")
        proof = item.get("proof")
        if (
            not isinstance(requirement, str)
            or len(requirement.split()) < 5
            or not isinstance(proof, str)
            or len(proof.split()) < 12
            or requirement == proof
        ):
            raise JourneyFailure(
                f"validation requirement {index} is activity/token-only rather than outcome proof"
            )
        citation_start = proof.find(citation_prefix)
        if citation_start < 0:
            raise JourneyFailure(
                f"validation requirement {index} omitted a public scenario citation"
            )
        scenario_start = citation_start + len(citation_prefix)
        scenario_end = scenario_start
        while scenario_end < len(proof) and (
            proof[scenario_end].isalnum() or proof[scenario_end] == "_"
        ):
            scenario_end += 1
        scenario = proof[scenario_start:scenario_end]
        if not scenario or f"def {scenario}(" not in scenario_source:
            raise JourneyFailure(
                f"validation requirement {index} cited an unknown public scenario {scenario!r}"
            )
        function_start = scenario_source.index(f"def {scenario}(")
        function_end = scenario_source.find("\ndef ", function_start + 1)
        function_source = scenario_source[
            function_start : function_end if function_end >= 0 else len(scenario_source)
        ]
        if "invoke(" not in function_source or "assert " not in function_source:
            raise JourneyFailure(
                f"validation requirement {index} cited {scenario} without executable CLI assertions"
            )
        proof_text = proof.lower()
        if not any(term in proof_text for term in ("scenario", "assert", "observes", "denial")):
            raise JourneyFailure(
                f"validation requirement {index} proof does not describe observable assertions"
            )


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
        self.repository_root: Optional[Path] = None
        self.work_slot_bindings: Dict[str, Any] = {}
        self.dummy_worker_proof: List[str] = []
        self.engine_boundary_proof: List[str] = []
        self.bookends_proof: Optional[Path] = None
        self.command_cwd: Optional[Path] = None
        self.command_env: Dict[str, str] = {}
        self.run_id = "journey-production-run"
        self.stitched_run_id: Optional[str] = None
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
        self._run_unavailable_event_proof()
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
                    "stitched_run_id": self.stitched_run_id,
                    "database": str(self.database),
                    "artifact_root": str(self.artifact_root),
                    "bookends_enabled_proof": (
                        str(self.bookends_proof) if self.bookends_proof is not None else None
                    ),
                    "successor_route_cases": successor_route_cases,
                    "work_slot_proof": WORK_SLOT_PROOF,
                    "dummy_worker_proof": self.dummy_worker_proof,
                    "engine_boundary_proof": self.engine_boundary_proof,
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
        if profile.get("config_version") != "high-rigor-7":
            raise JourneyFailure(
                f"journey requires high-rigor-7, got {profile.get('config_version')!r}"
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
        # bookends:LE-106 — packaged data-dump feeds describe/evaluate without checkout profile lookup.
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
        assert self.profile_source is not None
        self.profile = self._read_json(self.profile_source, "profile")
        profile = dict(self.profile)
        profile["artifact_root"] = str(self.artifact_root)
        shipped = profile.get("work_slot_bindings")
        try:
            work_slot_journey.assert_shipped_path_names(shipped)
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state="explore", event="start") from error
        # Keep the existing sparse dummy overlay: only intent-draft is bound so
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
        self._assert_challenge_review_contract(workflow)

    @staticmethod
    def _assert_challenge_review_contract(workflow: Dict[str, Any]) -> None:
        expected = {
            "intent-adversarial-review": "Intent challenge review",
            "design-adversarial-review": "Design challenge review",
            "plan-adversarial-review": "Plan challenge review",
            "implementation-adversarial-review": "Implementation challenge review",
            "validation-adversarial-review": "Validation challenge review",
        }
        states = {
            state.get("id"): state
            for state in workflow.get("states", [])
            if isinstance(state, dict)
        }
        for state_id, title in expected.items():
            state = states.get(state_id)
            if not isinstance(state, dict):
                raise JourneyFailure(f"challenge-review state missing machine ID {state_id}")
            if state.get("title") != title:
                raise JourneyFailure(
                    f"{state_id} exposed human title {state.get('title')!r}, expected {title!r}"
                )
            instructions = str(state.get("instructions", "")).lower()
            for clause in (
                "challenge review",
                "meaningfully falsify",
                "current supplied evidence",
                "violated frozen obligation",
                "concrete consequence",
                "why existing validation does not resolve",
                "hypothetical threats",
                "invented requirements",
                "mechanism-for-its-own-sake",
            ):
                if clause not in instructions:
                    raise JourneyFailure(f"{state_id} challenge guidance omitted {clause!r}")
            if "adversarial review" in instructions:
                raise JourneyFailure(f"{state_id} leaked machine wording into instructions")

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
        environment = os.environ.copy()
        environment.update(self.command_env)
        try:
            completed = subprocess.run(
                command,
                text=True,
                capture_output=True,
                check=False,
                cwd=str(self.command_cwd) if self.command_cwd is not None else None,
                env=environment,
            )
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
            # bookends:LE-75 — show exposes the frozen catalog and sparse bindings before work.
            # bookends:LE-77 — bound instructions expose the slot and frozen CLI binding.
            # bookends:LE-78 — invoke allocates capture state and sends the worker packet.
            # bookends:LE-81 — invocation history is engine-authored, not append-authored.
            # bookends:LE-82 — show/invocation views expose the reader overlay fields.
            # bookends:LE-87 — the public helper checks the slot-visit subject and digest.
            # bookends:LE-88 — this is the shared public-boundary sparse-binding scenario.
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
                stdin_context_kinds=_review_stdin_kinds(SOFTWARE_CHANGE_SLOT_IDS),
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state="explore", event="invoke") from error
        self.state = "explore"

    def _run_unavailable_event_proof(self) -> None:
        """Prove an unavailable event is a rejection, not a state mutation."""
        response = self._event("event-that-is-not-stored", axis="unavailable")
        self._expect_status(
            response,
            "rejected",
            event="event-that-is-not-stored",
            axis="unavailable",
            state=self.state,
        )
        shown = self._assert_show("explore", "unavailable-event-show")
        history = self._engine(
            ["history", self.run_id], state=self.state, event="unavailable-event-history"
        )
        self._expect_status(
            history,
            "completed",
            event="history",
            axis="unavailable",
            state=self.state,
        )
        transitions = [
            entry
            for entry in history.get("result", [])
            if entry.get("action", {}).get("kind") == "transition"
        ]
        # bookends:LE-4 — this real unavailable event assertion preserves the shown state and adds no transition history.
        if response.get("code") != "event-unavailable":
            raise JourneyFailure(
                f"unavailable event was not rejected as unavailable: {response}",
                state=self.state,
                event="event-that-is-not-stored",
            )
        if shown.get("current_state") != "explore" or transitions:
            raise JourneyFailure(
                f"unavailable event changed state or history: show={shown}, history={history}",
                state=self.state,
                event="event-that-is-not-stored",
            )
        print("unavailable-event scenario passed: state and semantic history unchanged")

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
        # bookends:LE-21 — the show instructions retain the external artifact identity.
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
        # bookends:LE-41 — the committed start response freezes the selected review-policy configuration.
        expected_initial_input = self._read_json(self.profile_path, "started profile")
        if result.get("run", {}).get("initial_input") != expected_initial_input:
            raise JourneyFailure(
                "start did not freeze the caller input", state="explore", event="start"
            )
        # Observation-before-mutation: arm the newly created state visit before
        # the journey invokes its bound work slot.
        self._show_for(run_id, state="explore", event="start-observation")

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
        # bookends:LE-10 — each show is a fresh CLI process and must recover current state.
        # bookends:LE-20 — show is the fresh-actor handoff surface.
        # bookends:LE-53 — the journey crosses process boundaries at every public command.
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
        # bookends:LE-42 — this fresh show projection carries frozen review policies without a describe/discovery call.
        if not isinstance(shown.get("initial_input"), dict) or "review_policies" not in shown["initial_input"]:
            raise JourneyFailure(
                "show omitted frozen review policies", state=expected_state, event=event
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
                "steering": "preserve the caller's durable direction",
                "synthetic_evidence": True,
                "semantic_verdict_quality": "not tested",
            },
            separators=(",", ":"),
        )
        record_option = f"--record-id={record_id}" if equals else "--record-id"
        operation: List[str] = ["append", record_option]
        if not equals:
            operation.append(record_id)
        kind = "user-steering" if record_id == "journey-marker-separate" else "journey-marker"
        operation.extend([self.run_id, kind, data])
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
        context = shown.get("context", [])
        context_ids = [record.get("id") for record in context]
        for record_id in ("journey-marker-separate", "journey-marker-equals"):
            if record_id not in context_ids:
                raise JourneyFailure(
                    f"show lost caller-owned record ID {record_id!r}", state=self.state, event="show", axis="record-id"
                )
        if context_ids.index("journey-marker-separate") >= context_ids.index("journey-marker-equals"):
            raise JourneyFailure("show did not preserve append order for caller context")
        marker_records = {
            record.get("id"): record for record in context if isinstance(record, dict)
        }
        if any(
            marker_records[record_id].get("data", {}).get("semantic_verdict_quality")
            != "not tested"
            for record_id in ("journey-marker-separate", "journey-marker-equals")
        ):
            raise JourneyFailure("marker data was interpreted or rewritten by the engine")
        expected_initial_input = self._read_json(self.profile_path, "started profile")
        if shown.get("initial_input") != expected_initial_input:
            raise JourneyFailure("show changed immutable initial input after append")
        if marker_records["journey-marker-separate"].get("kind") != "user-steering":
            raise JourneyFailure("steering marker was not retained as caller context")
        # bookends:LE-16 — a fresh show after append retains the exact immutable initial input.
        # bookends:LE-17 — show proves durable context records retain append order.
        # bookends:LE-18 — marker data remains opaque caller context rather than engine truth.
        # bookends:LE-22 — appended steering is visible to the next public actor.
        history = self._engine(["history", self.run_id], state=self.state, event="history")
        self._expect_status(
            history, "completed", event="history", axis="record-id", state=self.state
        )
        allowed_history_kinds = {
            "run_created",
            "context_appended",
            "transition",
            "terminated",
            "invocation_started",
            "invocation_status_changed",
        }
        unexpected_history_kinds = [
            entry.get("action", {}).get("kind")
            for entry in history.get("result", [])
            if entry.get("action", {}).get("kind") not in allowed_history_kinds
        ]
        # bookends:LE-25 — the public history projection contains only the semantic action kinds defined by the product contract.
        if unexpected_history_kinds:
            raise JourneyFailure(
                f"history exposed non-semantic action kinds: {unexpected_history_kinds}"
            )
        history_sequences = [entry.get("sequence") for entry in history.get("result", [])]
        # bookends:LE-28 — the public history sequence is ordered after separate CLI reads.
        if history_sequences != sorted(history_sequences):
            raise JourneyFailure(f"history sequence order changed: {history_sequences}")
        history_again = self._engine(
            ["history", self.run_id], state=self.state, event="history-again"
        )
        self._expect_status(
            history_again, "completed", event="history-again", axis="record-id", state=self.state
        )
        if history_again.get("result") != history.get("result"):
            raise JourneyFailure("history read changed semantic history", state=self.state, event="history")
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
                f"expected denial {code}, got {response.get('code')}: {response}", state=self.state, event=event, axis=axis
            )
        if not response.get("message"):
            raise JourneyFailure(
                f"denial omitted actionable message: {response}", state=self.state, event=event, axis=axis
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

    def _append_finding_ledger_for(
        self, run_id: str, gate: str, *, state: str, record_prefix: str = ""
    ) -> None:
        subject = GATE_SUBJECT[gate]
        revision = self._fixture_revision(subject)
        record_id = f"{record_prefix}finding-ledger-{gate}"
        data = {
            "schema_version": "1",
            "gate": gate,
            "subject": subject,
            "subject_revision": revision,
            "author": {"name": "journey-driver", "kind": "agent"},
            "repository_state": self._checkpoint_state_for_subject(subject),
            "findings": [],
        }
        record = json.dumps(data, separators=(",", ":"))
        response = self._engine_for(
            run_id,
            ["append", f"--record-id={record_id}", run_id, "finding-ledger", record],
            state=state,
            event="append",
            axis=gate,
        )
        self._expect_status(
            response, "completed", event="append", axis=gate, state=state
        )
        if response.get("result", {}).get("context", {}).get("id") != record_id:
            raise JourneyFailure(
                f"finding-ledger record ID was changed for {gate}",
                state=state,
                event="append",
                axis=gate,
            )

    def _checkpoint_state_for_subject(self, subject: str) -> Optional[str]:
        if subject in {"intent.json", "design.json", "plan.json"}:
            return None
        assert self.artifact_root is not None
        checkpoint_name = (
            "implementation-checkpoint.json"
            if subject == "implementation-report.json"
            else "validation-checkpoint.json"
        )
        checkpoint = self._read_json(
            self.artifact_root / checkpoint_name, f"{subject} checkpoint"
        )
        state = checkpoint.get("repository", {}).get("state_sha256")
        if not isinstance(state, str) or not state:
            raise JourneyFailure(
                f"{checkpoint_name} omitted repository.state_sha256",
                state=self.state,
                event="append",
            )
        return state

    def _expect_denial_for(
        self, run_id: str, state: str, event: str, axis: str, code: str
    ) -> Dict[str, Any]:
        response = self._event_for(run_id, event, state=state, axis=axis)
        self._expect_status(
            response, "rejected", event=event, axis=axis, state=state
        )
        if response.get("code") != code:
            raise JourneyFailure(
                f"expected denial {code}, got {response.get('code')}: {response}",
                state=state,
                event=event,
                axis=axis,
            )
        shown = self._assert_show_for(run_id, state, event + "-denied")
        latest = [
            evaluation
            for evaluation in shown.get("latest_evaluations", [])
            if evaluation.get("transition", {}).get("source") == state
            and evaluation.get("transition", {}).get("event") == event
        ]
        # bookends:LE-46 — missing review evidence is an actionable checked denial.
        if (
            len(latest) != 1
            or latest[0].get("result", {}).get("result") != "deny"
            or latest[0].get("result", {}).get("feedback", {}).get("code") != code
            or not latest[0].get("result", {}).get("feedback", {}).get("message")
        ):
            raise JourneyFailure(
                f"show omitted actionable denial lineage for {state}/{event}: {shown}",
                state=state,
                event=event,
                axis=axis,
            )
        return response

    def _pass_review_for(
        self,
        run_id: str,
        state: str,
        gate: str,
        event: str,
        target: str,
        *,
        record_prefix: str = "",
    ) -> None:
        # bookends:LE-6 — this provider-denied checked approval does not advance without allow.
        # bookends:LE-44 — the driver appends externally produced review evidence.
        # bookends:LE-45 — the provider validates evidence; it does not author a review.
        # bookends:LE-52 — prior evidence and denial lineage are carried into the next check.
        first_denial = self._expect_denial_for(
            run_id, state, event, gate, "software-change-finding-ledger-invalid"
        )
        self._append_evidence_for(
            run_id, gate, state=state, record_prefix=record_prefix
        )
        second_denial = self._expect_denial_for(
            run_id, state, event, gate, "software-change-finding-ledger-invalid"
        )
        prior_denials = second_denial.get("details", {}).get("prior_denials", [])
        if not isinstance(prior_denials, list) or not any(
            item.get("code") == first_denial.get("code") for item in prior_denials
            if isinstance(item, dict)
        ):
            raise JourneyFailure(
                f"provider did not receive ordered prior denial lineage: {second_denial}",
                state=state,
                event=event,
            )
        # bookends:LE-31 — the second checked request observes the prior denial in durable order.
        self._append_finding_ledger_for(
            run_id, gate, state=state, record_prefix=record_prefix
        )
        final = self._expect_allow_for(run_id, state, event, target)
        shown = self._show_for(run_id, state=target, event=event + "-latest")
        latest = [
            evaluation
            for evaluation in shown.get("latest_evaluations", [])
            if evaluation.get("transition", {}).get("source") == state
            and evaluation.get("transition", {}).get("event") == event
        ]
        # bookends:LE-29 — a durable denial and later allow remain visible across fresh actor processes.
        # bookends:LE-33 — externally supplied evidence changes only provider authorization, not engine routing.
        # bookends:LE-34 — the denied result has feedback, while the later allow carries no feedback payload.
        # bookends:LE-47 — complete configured evidence allows the policy gate.
        # bookends:LE-50 — show projects the successful evaluation as the latest result.
        if (
            final.get("result", {}).get("run", {}).get("current_state") != target
            or len(latest) != 1
            or latest[0].get("result", {}).get("result") != "allow"
            or "feedback" in latest[0].get("result", {})
        ):
            raise JourneyFailure(
                f"successful review edge was not projected as latest allow: {shown}",
                state=state,
                event=event,
            )

    def _pass_review(self, gate: str, event: str, target: str) -> None:
        self._pass_review_for(
            self.run_id, self.state, gate, event, target, record_prefix=""
        )
        self.state = target

    def _prepare_successor_state(self, run_id: str, target: str) -> None:
        self._start_run(run_id)
        self._invoke_bound_slot(run_id, state="explore")
        prefix = f"{run_id}-"
        self._expect_allow_for(run_id, "explore", "intent-ready", "intent-review")
        self._pass_review_for(
            run_id,
            "intent-review",
            "intent-review",
            "approved",
            "intent-adversarial-review",
            record_prefix=prefix,
        )
        self._pass_review_for(
            run_id,
            "intent-adversarial-review",
            "intent-adversarial-review",
            "approved",
            "design",
            record_prefix=prefix,
        )
        self._expect_allow_for(run_id, "design", "design-ready", "design-review")
        if target == "design-review":
            return

        self._pass_review_for(
            run_id,
            "design-review",
            "design-review",
            "approved",
            "design-adversarial-review",
            record_prefix=prefix,
        )
        self._pass_review_for(
            run_id,
            "design-adversarial-review",
            "design-adversarial-review",
            "approved",
            "plan",
            record_prefix=prefix,
        )
        self._expect_allow_for(run_id, "plan", "plan-ready", "plan-review")
        if target == "plan-review":
            return

        self._pass_review_for(
            run_id,
            "plan-review",
            "plan-review",
            "approved",
            "plan-adversarial-review",
            record_prefix=prefix,
        )
        self._pass_review_for(
            run_id,
            "plan-adversarial-review",
            "plan-adversarial-review",
            "approved",
            "implement",
            record_prefix=prefix,
        )
        self._expect_allow_for(
            run_id, "implement", "implementation-ready", "implementation-review"
        )
        if target == "implementation-review":
            return

        self._pass_review_for(
            run_id,
            "implementation-review",
            "implementation-review",
            "approved",
            "implementation-adversarial-review",
            record_prefix=prefix,
        )
        self._pass_review_for(
            run_id,
            "implementation-adversarial-review",
            "implementation-adversarial-review",
            "approved",
            "validation",
            record_prefix=prefix,
        )
        self._expect_allow_for(
            run_id, "validation", "validation-ready", "validation-review"
        )
        if target != "validation-review":
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
            # bookends:LE-9 — the committed route is accompanied by its durable transition history.
            # bookends:LE-27 — one route request creates exactly one aggregate transition entry.
            # bookends:LE-30 — the history assertion identifies the exact source/event/target edge.
            matching = [
                entry
                for entry in history.get("result", [])
                if entry.get("action", {}).get("kind") == "transition"
                and entry["action"].get("transition", {}).get("source") == source
                and entry["action"]["transition"].get("event") == event
                and entry["action"]["transition"].get("target") == target
                and entry["action"].get("outcome", {}).get("outcome") == "committed"
            ]
            if len(matching) != 1:
                raise JourneyFailure(
                    f"history expected one committed {source}/{event}/{target} route, got {len(matching)}",
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

    def _prepare_real_repository(self) -> Path:
        """Create the driver-selected repository used by checkpoint gates."""
        assert self.run_dir is not None
        repository = self.run_dir / "checkpoint-repository"
        repository.mkdir(parents=True, exist_ok=True)
        for name, contents in (
            ("tracked.txt", "tracked baseline\n"),
            ("second.txt", "second baseline\n"),
            ("status.txt", "status baseline\n"),
            ("rename.txt", "rename baseline\n"),
            ("delete.txt", "delete baseline\n"),
            ("bytes.txt", "bytes baseline\n"),
        ):
            (repository / name).write_text(contents, encoding="utf-8")
        for git_args in (
            ["init", "-q"],
            ["config", "user.name", "software-change journey"],
            ["config", "user.email", "journey@example.invalid"],
            ["config", "commit.gpgsign", "false"],
            ["add", "-A"],
            ["commit", "-qm", "checkpoint baseline"],
        ):
            completed = subprocess.run(
                ["git", *git_args],
                cwd=repository,
                text=True,
                capture_output=True,
                check=False,
            )
            if completed.returncode != 0:
                raise JourneyFailure(
                    f"checkpoint repository git {' '.join(git_args)} failed: "
                    f"{completed.stderr.strip() or completed.stdout.strip()}"
                )
        self.repository_root = repository
        self.command_cwd = repository
        return repository

    def _create_checkpoint(self, phase: str) -> Dict[str, Any]:
        assert self.artifact_root is not None
        assert self.repository_root is not None
        return self._create_checkpoint_at(
            self.provider, phase, self.artifact_root, self.repository_root
        )

    @staticmethod
    def _repository_snapshot(repository: Path) -> tuple[Any, ...]:
        """Capture the public Git/worktree boundary before and after checkpoint CLI."""
        environment = {**os.environ, "GIT_OPTIONAL_LOCKS": "0"}

        def git_bytes(*args: str) -> bytes:
            completed = subprocess.run(
                ["git", *args],
                cwd=repository,
                env=environment,
                capture_output=True,
                check=False,
            )
            if completed.returncode != 0:
                raise JourneyFailure(
                    f"checkpoint snapshot git {' '.join(args)} failed: "
                    f"{completed.stderr.decode('utf-8', 'replace')}"
                )
            return completed.stdout

        tracked = set(filter(None, git_bytes("ls-files", "-z", "--cached").split(b"\0")))
        untracked = set(
            filter(
                None,
                git_bytes(
                    "ls-files", "-z", "--others", "--exclude-standard"
                ).split(b"\0"),
            )
        )
        entries = []
        for raw_path in sorted(tracked | untracked):
            try:
                relative = raw_path.decode("utf-8")
            except UnicodeDecodeError as error:
                raise JourneyFailure(
                    f"checkpoint snapshot found a non-UTF-8 path: {raw_path!r}"
                ) from error
            path = repository / relative
            if path.is_symlink():
                entry = (
                    relative,
                    raw_path in tracked,
                    "symlink",
                    path.stat().st_mode & 0o777,
                    os.readlink(path),
                )
            elif path.is_file():
                entry = (
                    relative,
                    raw_path in tracked,
                    "regular",
                    path.stat().st_mode & 0o777,
                    path.read_bytes(),
                )
            elif not path.exists():
                entry = (relative, raw_path in tracked, "missing", None, None)
            else:
                entry = (
                    relative,
                    raw_path in tracked,
                    "other",
                    path.stat().st_mode & 0o777,
                    None,
                )
            entries.append(entry)
        return (
            git_bytes("rev-parse", "HEAD"),
            git_bytes("ls-files", "--stage", "-z"),
            git_bytes(
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=all",
                "--ignored=no",
            ),
            tuple(entries),
        )

    @staticmethod
    def _create_checkpoint_at(
        provider: Path, phase: str, artifact_root: Path, repository: Path
    ) -> Dict[str, Any]:
        before = Journey._repository_snapshot(repository)
        completed = subprocess.run(
            [
                str(provider),
                "checkpoint",
                "--phase",
                phase,
                "--artifact-root",
                str(artifact_root),
                "--working-directory",
                str(repository),
            ],
            cwd=repository,
            text=True,
            capture_output=True,
            check=False,
        )
        if completed.returncode != 0:
            raise JourneyFailure(
                f"checkpoint {phase} failed: "
                f"{completed.stderr.strip() or completed.stdout.strip()}"
            )
        after = Journey._repository_snapshot(repository)
        if after != before:
            raise JourneyFailure(
                f"checkpoint {phase} CLI mutated the Git/worktree boundary"
            )
        try:
            result = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise JourneyFailure(
                f"checkpoint {phase} returned non-JSON: {completed.stdout!r}"
            ) from error
        if not isinstance(result, dict):
            raise JourneyFailure(f"checkpoint {phase} result is not an object: {result}")
        repository_identity = result.get("repository")
        if not isinstance(repository_identity, dict) or not repository_identity.get("state_sha256"):
            raise JourneyFailure(f"checkpoint {phase} omitted repository identity: {result}")
        Journey._assert_checkpoint_payload(result, phase, artifact_root, repository)
        return result

    @staticmethod
    def _assert_checkpoint_payload(
        checkpoint: Dict[str, Any],
        phase: str,
        artifact_root: Path,
        repository: Path,
    ) -> None:
        expected_files = {"schema_version", "phase", "report", "documents", "repository"}
        if set(checkpoint) != expected_files or checkpoint.get("schema_version") != "1":
            raise JourneyFailure(f"checkpoint {phase} did not use the closed schema: {checkpoint}")
        report = checkpoint.get("report")
        documents = checkpoint.get("documents")
        repository_value = checkpoint.get("repository")
        if not isinstance(report, dict) or set(report) != {"file", "revision", "sha256"}:
            raise JourneyFailure(f"checkpoint {phase} report fields changed: {checkpoint}")
        if not isinstance(documents, dict) or set(documents) != {
            "intent_revision",
            "design_revision",
            "plan_revision",
        }:
            raise JourneyFailure(f"checkpoint {phase} document fields changed: {checkpoint}")
        if not isinstance(repository_value, dict) or set(repository_value) != {
            "head",
            "index_sha256",
            "status_sha256",
            "entries",
            "state_sha256",
        }:
            raise JourneyFailure(f"checkpoint {phase} repository fields changed: {checkpoint}")
        expected_report = (
            "implementation-report.json"
            if phase == "implementation"
            else "validation-report.json"
        )
        if checkpoint.get("phase") != phase or report.get("file") != expected_report:
            raise JourneyFailure(f"checkpoint {phase} named the wrong phase/report: {checkpoint}")
        def is_digest(value: Any) -> bool:
            return (
                isinstance(value, str)
                and len(value) == 71
                and value.startswith("sha256:")
                and all(character in "0123456789abcdef" for character in value[7:])
            )
        for name in ("sha256",):
            if not is_digest(report.get(name)):
                raise JourneyFailure(f"checkpoint {phase} report digest is malformed: {checkpoint}")
        for name in ("index_sha256", "status_sha256", "state_sha256"):
            if not is_digest(repository_value.get(name)):
                raise JourneyFailure(f"checkpoint {phase} repository digest is malformed: {checkpoint}")
        head = repository_value.get("head")
        if not isinstance(head, str) or len(head) not in (40, 64) or any(
            character not in "0123456789abcdef" for character in head
        ):
            raise JourneyFailure(f"checkpoint {phase} HEAD identity is malformed: {checkpoint}")
        report_bytes = (artifact_root / expected_report).read_bytes()
        if report.get("sha256") != "sha256:" + hashlib.sha256(report_bytes).hexdigest():
            raise JourneyFailure(f"checkpoint {phase} report digest did not hash report bytes")
        def git_bytes(*args: str) -> bytes:
            completed = subprocess.run(
                ["git", *args],
                cwd=repository,
                env={**os.environ, "GIT_OPTIONAL_LOCKS": "0"},
                capture_output=True,
                check=False,
            )
            if completed.returncode != 0:
                raise JourneyFailure(
                    f"checkpoint {phase} digest source git {' '.join(args)} failed: "
                    f"{completed.stderr.decode('utf-8', 'replace')}"
                )
            return completed.stdout
        index_bytes = git_bytes("ls-files", "--stage", "-z")
        status_bytes = git_bytes(
            "status", "--porcelain=v2", "-z", "--untracked-files=all", "--ignored=no"
        )
        if repository_value["index_sha256"] != "sha256:" + hashlib.sha256(index_bytes).hexdigest():
            raise JourneyFailure(f"checkpoint {phase} index digest source changed")
        if repository_value["status_sha256"] != "sha256:" + hashlib.sha256(status_bytes).hexdigest():
            raise JourneyFailure(f"checkpoint {phase} status digest source changed")
        without_state = {
            "head": repository_value["head"],
            "index_sha256": repository_value["index_sha256"],
            "status_sha256": repository_value["status_sha256"],
            "entries": repository_value["entries"],
        }
        serialized = json.dumps(without_state, separators=(",", ":"), ensure_ascii=False).encode()
        if repository_value["state_sha256"] != "sha256:" + hashlib.sha256(serialized).hexdigest():
            raise JourneyFailure(f"checkpoint {phase} state digest source changed")
        checkpoint_path = artifact_root / (
            "implementation-checkpoint.json"
            if phase == "implementation"
            else "validation-checkpoint.json"
        )
        if json.loads(checkpoint_path.read_text(encoding="utf-8")) != checkpoint:
            raise JourneyFailure(f"checkpoint {phase} CLI output differed from persisted JSON")

    @staticmethod
    def _checkpoint_case_mutate(repository: Path, mutation: str, round_number: int) -> str:
        if mutation == "head":
            path = repository / f"head-{round_number}.txt"
            path.write_text(f"HEAD mutation {round_number}\n", encoding="utf-8")
            subprocess.run(["git", "add", path.name], cwd=repository, check=True)
            subprocess.run(
                ["git", "commit", "-qm", f"HEAD mutation {round_number}"],
                cwd=repository,
                check=True,
            )
            return "repository HEAD changed"
        if mutation == "add":
            path = repository / f"added-{round_number}.txt"
            path.write_text(f"untracked addition {round_number}\n", encoding="utf-8")
            return f"repository entry added at `{path.name}`"
        if mutation == "delete":
            path = repository / f"deleted-{round_number}.txt"
            path.unlink()
            return f"repository deleted changed at `{path.name}`"
        if mutation == "rename":
            old = repository / f"rename-{round_number}.txt"
            new = repository / f"renamed-{round_number}.txt"
            subprocess.run(["git", "mv", old.name, new.name], cwd=repository, check=True)
            return f"repository entry added at `{new.name}`"
        if mutation == "status":
            path = repository / f"status-{round_number}.txt"
            os.chmod(path, 0o755)
            return f"repository status changed at `{path.name}`"
        if mutation == "type":
            path = repository / f"type-{round_number}.txt"
            path.unlink()
            path.symlink_to("head-1.txt")
            return f"repository file type changed at `{path.name}`"
        if mutation == "bytes":
            path = repository / f"bytes-{round_number}.txt"
            path.write_text(f"changed bytes {round_number}\n", encoding="utf-8")
            return f"repository bytes changed at `{path.name}`"
        raise JourneyFailure(f"unknown checkpoint mutation {mutation}")

    def _run_checkpoint_case(self, mutation: str) -> None:
        """Exercise one real repository mutation against both checkpoint gates."""
        assert self.run_dir is not None
        assert self.fixture_root is not None
        case_dir = self.run_dir / "checkpoint-cases" / mutation
        repository = case_dir / "repository"
        artifacts = case_dir / "artifacts"
        profile_path = case_dir / "minimal.json"
        provider_config = case_dir / "providers.toml"
        database = case_dir / "loop.sqlite"
        case_dir.mkdir(parents=True, exist_ok=True)
        repository.mkdir()
        artifacts.mkdir()
        for name, contents in (
            ("head-1.txt", "head baseline 1\n"),
            ("head-2.txt", "head baseline 2\n"),
            ("deleted-1.txt", "delete baseline 1\n"),
            ("deleted-2.txt", "delete baseline 2\n"),
            ("rename-1.txt", "rename baseline 1\n"),
            ("rename-2.txt", "rename baseline 2\n"),
            ("status-1.txt", "status baseline 1\n"),
            ("status-2.txt", "status baseline 2\n"),
            ("type-1.txt", "type baseline 1\n"),
            ("type-2.txt", "type baseline 2\n"),
            ("bytes-1.txt", "bytes baseline 1\n"),
            ("bytes-2.txt", "bytes baseline 2\n"),
        ):
            (repository / name).write_text(contents, encoding="utf-8")
            if name.startswith("status-"):
                os.chmod(repository / name, 0o644)
        for git_args in (
            ["init", "-q"],
            ["config", "user.name", "software-change journey"],
            ["config", "user.email", "journey@example.invalid"],
            ["config", "commit.gpgsign", "false"],
            ["config", "core.filemode", "true"],
            ["add", "-A"],
            ["commit", "-qm", f"{mutation} baseline"],
        ):
            completed = subprocess.run(
                ["git", *git_args],
                cwd=repository,
                text=True,
                capture_output=True,
                check=False,
            )
            if completed.returncode != 0:
                raise JourneyFailure(
                    f"{mutation} fixture git {' '.join(git_args)} failed: "
                    f"{completed.stderr.strip() or completed.stdout.strip()}"
                )
        for subject, fixture in SUBJECTS.items():
            shutil.copy2(self.fixture_root / fixture, artifacts / subject)
        profile = self._read_json(
            self.data_root / STITCHED_PROFILE_SUBPATH, f"{mutation} minimal profile"
        )
        profile["artifact_root"] = str(artifacts)
        profile.pop("work_slot_bindings", None)
        _write_json(profile_path, profile)
        self._write_scenario_provider_config(provider_config, str(self.provider), [])
        run_id = f"checkpoint-{mutation}"

        def call(operation: Sequence[str]) -> Dict[str, Any]:
            return self._scenario_engine_call(database, operation, cwd=repository)

        started = call(
            [
                "--config",
                str(provider_config),
                "--timeout-ms",
                "30000",
                "start",
                "--id",
                run_id,
                "software-change",
                "@" + str(profile_path),
                f"checkpoint {mutation}",
            ]
        )
        self._expect_status(started, "completed", event="start", state="explore")
        initial_observed = call(["show", run_id])
        self._expect_status(initial_observed, "completed", event="show", state="explore")
        for event, target in (
            ("intent-ready", "design"),
            ("design-ready", "plan"),
            ("plan-ready", "implement"),
        ):
            response = call(["event", run_id, event])
            self._expect_status(response, "completed", event=event, state=target)
            observed = call(["show", run_id])
            self._expect_status(observed, "completed", event="show", state=target)
            if response.get("result", {}).get("run", {}).get("current_state") != target:
                raise JourneyFailure(f"{mutation} did not reach {target}: {response}")

        # bookends:LE-94 — a truthful implementation report without the provider-generated checkpoint is rejected on the public event path.
        report_only = call(["event", run_id, "implementation-ready"])
        self._expect_status(report_only, "rejected", event="implementation-ready", state="implement")
        if report_only.get("code") != "software-change-checkpoint-invalid" or "report-only" not in str(report_only.get("details", {}).get("diagnostic", "")):
            raise JourneyFailure(f"{mutation} report-only completion was not denied: {report_only}")

        self._create_checkpoint_at(self.provider, "implementation", artifacts, repository)
        expected_implementation = self._checkpoint_case_mutate(repository, mutation, 1)
        stale_implementation = call(["event", run_id, "implementation-ready"])
        self._expect_status(stale_implementation, "rejected", event="implementation-ready", state="implement")
        diagnostic = str(stale_implementation.get("details", {}).get("diagnostic", ""))
        # bookends:LE-95 — each named HEAD/add/delete/rename/status/type/bytes mutation is named by the stale implementation denial.
        if expected_implementation not in diagnostic:
            raise JourneyFailure(
                f"{mutation} implementation checkpoint denial omitted {expected_implementation!r}: {stale_implementation}"
            )
        self._create_checkpoint_at(self.provider, "implementation", artifacts, repository)
        ready = call(["event", run_id, "implementation-ready"])
        self._expect_status(ready, "completed", event="implementation-ready", state="implement")
        if ready.get("result", {}).get("run", {}).get("current_state") != "validation":
            raise JourneyFailure(f"{mutation} implementation recovery missed validation: {ready}")

        self._create_checkpoint_at(self.provider, "validation", artifacts, repository)
        validation_ready = call(["event", run_id, "validation-ready"])
        self._expect_status(validation_ready, "completed", event="validation-ready", state="validation")
        validation_revision = self._fixture_revision("validation-report.json")
        evidence = {
            "gate": "validation-review",
            "policy_id": "intent-delivered",
            "result": "pass",
            "findings": "",
            "author": {"name": f"checkpoint-reviewer-{mutation}", "kind": "script"},
            "subject": "validation-report.json",
            "subject_revision": validation_revision,
            "config_version": "minimal-6",
        }
        evidence_result = call(
            [
                "append",
                f"--record-id={run_id}-evidence-1",
                run_id,
                "review-evidence",
                json.dumps(evidence, separators=(",", ":")),
            ]
        )
        self._expect_status(evidence_result, "completed", event="append", state="validation-review")
        ledger = {
            "schema_version": "1",
            "gate": "validation-review",
            "subject": "validation-report.json",
            "subject_revision": validation_revision,
            "author": {"name": f"checkpoint-driver-{mutation}", "kind": "agent"},
            "repository_state": self._create_checkpoint_at(
                self.provider, "validation", artifacts, repository
            )["repository"]["state_sha256"],
            "findings": [],
        }
        ledger_result = call(
            [
                "append",
                f"--record-id={run_id}-ledger-1",
                run_id,
                "finding-ledger",
                json.dumps(ledger, separators=(",", ":")),
            ]
        )
        self._expect_status(ledger_result, "completed", event="append", state="validation-review")

        expected_validation = self._checkpoint_case_mutate(repository, mutation, 2)
        stale_validation = call(["event", run_id, "passed"])
        self._expect_status(stale_validation, "rejected", event="passed", state="validation-review")
        validation_diagnostic = str(stale_validation.get("details", {}).get("diagnostic", ""))
        # bookends:LE-95 — the same current passing review evidence cannot rescue a stale validation checkpoint after each named repository-state mutation.
        if expected_validation not in validation_diagnostic:
            raise JourneyFailure(
                f"{mutation} validation checkpoint denial omitted {expected_validation!r}: {stale_validation}"
            )

        recovery = call(["event", run_id, "revise-implementation"])
        self._expect_status(recovery, "completed", event="revise-implementation", state="validation-review")
        if recovery.get("result", {}).get("run", {}).get("current_state") != "implement":
            raise JourneyFailure(f"{mutation} validation recovery missed implement: {recovery}")
        implementation_report_path = artifacts / "implementation-report.json"
        implementation_report = self._read_json(
            implementation_report_path, f"{mutation} recovered implementation report"
        )
        implementation_report["revision"] = (
            str(implementation_report["revision"]) + "-recovered"
        )
        implementation_report_path.write_text(
            json.dumps(implementation_report, indent=2) + "\n", encoding="utf-8"
        )
        self._create_checkpoint_at(self.provider, "implementation", artifacts, repository)
        implementation_recovered = call(["event", run_id, "implementation-ready"])
        self._expect_status(implementation_recovered, "completed", event="implementation-ready", state="implement")
        self._create_checkpoint_at(self.provider, "validation", artifacts, repository)
        validation_recovered = call(["event", run_id, "validation-ready"])
        self._expect_status(validation_recovered, "completed", event="validation-ready", state="validation")
        fresh_evidence = dict(evidence)
        fresh_evidence["author"] = {"name": f"checkpoint-reviewer-{mutation}-fresh", "kind": "script"}
        fresh_result = call(
            [
                "append",
                f"--record-id={run_id}-evidence-2",
                run_id,
                "review-evidence",
                json.dumps(fresh_evidence, separators=(",", ":")),
            ]
        )
        self._expect_status(fresh_result, "completed", event="append", state="validation-review")
        fresh_ledger = dict(ledger)
        fresh_ledger["author"] = {"name": f"checkpoint-driver-{mutation}-fresh", "kind": "agent"}
        fresh_ledger["repository_state"] = self._create_checkpoint_at(
            self.provider, "validation", artifacts, repository
        )["repository"]["state_sha256"]
        fresh_ledger_result = call(
            [
                "append",
                f"--record-id={run_id}-ledger-2",
                run_id,
                "finding-ledger",
                json.dumps(fresh_ledger, separators=(",", ":")),
            ]
        )
        self._expect_status(fresh_ledger_result, "completed", event="append", state="validation-review")
        final = call(["event", run_id, "passed"])
        self._expect_status(final, "completed", event="passed", state="validation-review")
        shown = call(["show", run_id])
        self._expect_status(shown, "completed", event="show", state="end")
        result = shown.get("result", {})
        if result.get("current_state") != "end" or result.get("lifecycle") != "final":
            raise JourneyFailure(f"{mutation} did not finish after checkpoint recovery: {shown}")

    def _run_checkpoint_scenarios(self) -> None:
        """Use separate CLI processes and real temporary Git repositories."""
        for mutation in CHECKPOINT_MUTATIONS:
            self._run_checkpoint_case(mutation)
        # bookends:LE-96 — validation exposes stale proof, takes the check-free revise-implementation route, and final approval succeeds only after both checkpoints are regenerated.
        print(
            "checkpoint source scenarios passed: report-only denial, seven implementation/validation "
            "state invalidations, validation recovery, and current-tree final proof"
        )

    def _run_checked_prefix(self) -> None:
        if self.mode == "source":
            self._expect_denial("intent-ready", "intent", "software-change-schema-invalid")
            assert self.artifact_root is not None
            assert self.fixture_root is not None
            shutil.copy2(
                self.fixture_root / SUBJECTS["intent.json"],
                self.artifact_root / "intent.json",
            )
        self._expect_allow("intent-ready", "intent-review")
        if self.mode == "packaged":
            # Packaged smoke intentionally ends after one checked production
            # transition; the source adapter owns full graph traversal.
            self._assert_show("intent-review", "packaged-prefix-end")

    def _run_full_source(self) -> None:
        self._prepare_real_repository()
        self._expect_denial("intent-ready", "intent", "software-change-schema-invalid")
        assert self.artifact_root is not None
        assert self.fixture_root is not None
        shutil.copy2(
            self.fixture_root / SUBJECTS["intent.json"],
            self.artifact_root / "intent.json",
        )
        intent_context = self._read_json(
            self.artifact_root / "intent.json", "operating-context intent"
        ).get("operating_context")
        # bookends:LE-91 — the public run exposes one frozen operating context before the first checked transition and later worker/reviewer commissions inspect it.
        if (
            not isinstance(intent_context, dict)
            or set(intent_context) != {
                "operators",
                "environment",
                "threat_boundary",
                "accepted_risks",
                "outside_obligations",
            }
            or not all(
                isinstance(intent_context.get(name), (list, dict))
                and intent_context.get(name)
                for name in intent_context
            )
        ):
            raise JourneyFailure(
                f"intent did not expose the closed operating_context: {intent_context}",
                state=self.state,
                event="intent-ready",
            )
        self._assert_show("explore", "operating-context-show")
        # bookends:LE-83 — this checked edge is requested only after the public bound invocation has succeeded with the matching digest and visit subject.
        self._expect_allow("intent-ready", "intent-review")
        self._pass_review("intent-review", "approved", "intent-adversarial-review")
        self._pass_review("intent-adversarial-review", "approved", "design")
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

        self._pass_review("design-review", "approved", "design-adversarial-review")
        self._pass_review("design-adversarial-review", "approved", "plan")

        self._expect_allow("plan-ready", "plan-review")
        self._pass_review("plan-review", "approved", "plan-adversarial-review")
        self._pass_review("plan-adversarial-review", "approved", "implement")

        # bookends:LE-94 — implementation-ready refuses report-only completion until the public checkpoint command binds the report to the selected Git tree.
        self._create_checkpoint("implementation")
        self._expect_allow("implementation-ready", "implementation-review")
        self._pass_review(
            "implementation-review", "approved", "implementation-adversarial-review"
        )
        self._pass_review(
            "implementation-adversarial-review", "approved", "validation"
        )

        # Overwriting both mutable checkpoint files after implementation review
        # must not let validation accept repository bytes that no implementation
        # reviewer saw. The immutable implementation-review ledger is the anchor.
        assert self.repository_root is not None
        unreviewed = self.repository_root / "unreviewed-after-implementation-review.txt"
        unreviewed.write_text("not reviewed\n", encoding="utf-8")
        self._create_checkpoint("implementation")
        self._create_checkpoint("validation")
        late_ledger = {
            "schema_version": "1",
            "gate": "implementation-adversarial-review",
            "subject": "implementation-report.json",
            "subject_revision": self._fixture_revision("implementation-report.json"),
            "author": {"name": "late-validation-driver", "kind": "agent"},
            "repository_state": self._checkpoint_state_for_subject(
                "implementation-report.json"
            ),
            "findings": [],
        }
        late_append = self._engine(
            [
                "append",
                "--record-id=late-implementation-ledger-after-review",
                self.run_id,
                "finding-ledger",
                json.dumps(late_ledger, separators=(",", ":")),
            ],
            state="validation",
            event="append",
        )
        self._expect_status(late_append, "completed", event="append", state="validation")
        assert self.artifact_root is not None
        history_entries = list(
            (self.artifact_root / "implementation-proof-history").glob("*.json")
        )
        if len(history_entries) != 1:
            raise JourneyFailure(
                f"implementation proof history had {len(history_entries)} entries before overwrite test",
                state=self.state,
                event="validation-ready",
            )
        accepted_path = history_entries[0]
        accepted_bytes = accepted_path.read_bytes()
        accepted_path.write_bytes(
            (self.artifact_root / "implementation-checkpoint.json").read_bytes()
        )
        unreviewed_denial = self._expect_denial(
            "validation-ready", "validation", "software-change-checkpoint-invalid"
        )
        accepted_path.write_bytes(accepted_bytes)
        unreviewed_diagnostic = str(
            unreviewed_denial.get("details", {}).get("diagnostic", "")
        )
        if "does not match its content digest" not in unreviewed_diagnostic:
            raise JourneyFailure(
                "validation accepted an overwritten immutable implementation-proof entry",
                state=self.state,
                event="validation-ready",
            )

        history = accepted_path.parent
        missing_history = history.with_name("implementation-proof-history-missing-test")
        history.rename(missing_history)
        missing_denial = self._expect_denial(
            "validation-ready", "validation", "software-change-checkpoint-invalid"
        )
        missing_history.rename(history)
        if "validation requires accepted implementation proof" not in str(
            missing_denial.get("details", {}).get("diagnostic", "")
        ):
            raise JourneyFailure(
                "validation accepted missing implementation-proof history",
                state=self.state,
                event="validation-ready",
            )

        accepted = json.loads(accepted_bytes)
        differing = copy.deepcopy(accepted)
        differing["report"]["sha256"] = "sha256:" + hashlib.sha256(
            b"different accepted implementation report"
        ).hexdigest()
        differing_bytes = json.dumps(differing, separators=(",", ":")).encode("utf-8")
        differing_path = history / (hashlib.sha256(differing_bytes).hexdigest() + ".json")
        accepted_backup = self.artifact_root / "accepted-implementation-proof.backup"
        accepted_path.rename(accepted_backup)
        differing_path.write_bytes(differing_bytes)
        differing_denial = self._expect_denial(
            "validation-ready", "validation", "software-change-checkpoint-invalid"
        )
        differing_path.unlink()
        accepted_backup.rename(accepted_path)
        if "checkpoint mismatch:" not in str(
            differing_denial.get("details", {}).get("diagnostic", "")
        ):
            raise JourneyFailure(
                "validation did not reject a unique differing implementation proof",
                state=self.state,
                event="validation-ready",
            )

        differing_path.write_bytes(differing_bytes)
        ambiguous_denial = self._expect_denial(
            "validation-ready", "validation", "software-change-checkpoint-invalid"
        )
        differing_path.unlink()
        if "is ambiguous for report revision" not in str(
            ambiguous_denial.get("details", {}).get("diagnostic", "")
        ):
            raise JourneyFailure(
                "validation selected an ambiguous implementation proof",
                state=self.state,
                event="validation-ready",
            )
        unreviewed.unlink()
        self._create_checkpoint("implementation")
        self._create_checkpoint("validation")
        self._expect_allow("validation-ready", "validation-review")
        self._pass_review("validation-review", "approved", "validation-adversarial-review")
        self._pass_review("validation-adversarial-review", "passed", "end")
        shown = self._assert_show("end", "terminal-show")
        if shown.get("lifecycle") != "final":
            raise JourneyFailure("full journey did not reach final lifecycle", state=self.state, event="passed")
        if shown.get("requestable_events") != []:
            raise JourneyFailure("final journey exposed requestable events", state=self.state, event="show")
        intent = self._read_json(self.artifact_root / "intent.json", "completed intent")
        design = self._read_json(self.artifact_root / "design.json", "completed design")
        plan = self._read_json(self.artifact_root / "plan.json", "completed plan")
        validation = self._read_json(
            self.artifact_root / "validation-report.json", "completed validation report"
        )
        # bookends:LE-97 — _run_full_source requires the plan's operator outcome/black-box policy and validates each final citation against a real production-journey scenario with executable CLI assertions; token-only or activity-only proof is refused.
        scenario_path = self.data_root / COMPANION_SCENARIO_SUBPATH
        try:
            scenario_source = scenario_path.read_text(encoding="utf-8")
        except OSError as error:
            raise JourneyFailure(
                f"could not read cited public outcome scenarios: {error}",
                state=self.state,
                event="passed",
            ) from error
        try:
            assert_semantic_outcome_proof_contract(plan, validation, scenario_source)
        except JourneyFailure as error:
            raise JourneyFailure(
                f"final validation report did not provide semantic outcome proof: {error}",
                state=self.state,
                event="passed",
            ) from error
        # bookends:LE-40 — the same completed public run consumes already-known intent, design, and plan artifacts with their revision links intact.
        if (
            len(shown.get("context", [])) < 4
            or intent.get("operating_context") != intent_context
            or design.get("intent_revision") != intent.get("revision")
            or plan.get("design_revision") != design.get("revision")
        ):
            raise JourneyFailure(
                "full journey did not retain substantial durable intent/design/plan context",
                state=self.state,
                event="show",
            )
        print(
            "full software-change journey passed: parent and challenge reviews walked, last-hop passed"
        )

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
        self._run_stitched_source()
        self._run_engine_boundary_scenarios()
        self._run_dummy_worker_proofs()
        self._run_checkpoint_scenarios()
        self._run_bookends_enabled_source()

    def _scenario_engine_call(
        self,
        database: Path,
        operation: Sequence[str],
        *,
        cwd: Optional[Path] = None,
    ) -> Dict[str, Any]:
        """Run one boundary-scenario CLI process and parse its envelope."""
        command = [str(self.engine), "--database", str(database), "--json", *operation]
        completed = subprocess.run(
            command,
            text=True,
            capture_output=True,
            check=False,
            cwd=str(cwd) if cwd is not None else None,
        )
        try:
            response = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise JourneyFailure(
                f"boundary scenario returned non-JSON (exit={completed.returncode}): "
                f"{error}; stderr={completed.stderr.strip()!r}"
            ) from error
        if not isinstance(response, dict):
            raise JourneyFailure(f"boundary scenario response is not an object: {response}")
        if (
            operation
            and operation[0] in {"event", "invoke", "terminate"}
            and len(operation) > 1
            and response.get("status") in {"completed", "rejected"}
        ):
            # Keep the scenario callers on the same public path as a resumed
            # actor: every completed mutation is followed by a fresh show.
            self._scenario_show(database, operation[1])
        return response

    def _scenario_start_and_ready(
        self,
        database: Path,
        provider_config: Path,
        input_path: Path,
        run_id: str,
    ) -> None:
        started = self._scenario_start(database, provider_config, run_id, input_path)
        self._expect_status(started, "completed", event="start", state="explore")
        ready = self._scenario_engine_call(
            database, ["event", run_id, "intent-ready"], cwd=self.data_root
        )
        self._expect_status(ready, "completed", event="intent-ready", state="explore")
        self._scenario_show(database, run_id)

    def _scenario_event_call(
        self, database: Path, run_id: str, event: str
    ) -> Dict[str, Any]:
        command = [
            str(self.engine),
            "--database",
            str(database),
            "--json",
            "event",
            run_id,
            event,
        ]
        process = subprocess.run(
            command,
            text=True,
            capture_output=True,
            check=False,
            cwd=self.data_root,
        )
        try:
            value = json.loads(process.stdout)
        except json.JSONDecodeError as error:
            raise JourneyFailure(
                f"concurrent event returned non-JSON: {error}; stderr={process.stderr!r}"
            ) from error
        if not isinstance(value, dict):
            raise JourneyFailure(f"concurrent event response is not an object: {value}")
        return value

    @staticmethod
    def _write_scenario_provider_config(
        path: Path, command: str, args: Sequence[str]
    ) -> None:
        path.write_text(
            "[providers.software-change]\n"
            f"command = {json.dumps(command)}\n"
            f"args = {json.dumps(list(args))}\n",
            encoding="utf-8",
        )

    @staticmethod
    def _scenario_provider_call(command: Sequence[str], request: Dict[str, Any]) -> Dict[str, Any]:
        completed = subprocess.run(
            list(command),
            input=json.dumps(request),
            text=True,
            capture_output=True,
            check=False,
        )
        if completed.returncode != 0:
            raise JourneyFailure(
                "boundary provider call failed: "
                + (completed.stderr.strip() or f"exit {completed.returncode}")
            )
        try:
            response = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise JourneyFailure(
                f"boundary provider returned non-JSON: {error}; stderr={completed.stderr.strip()!r}"
            ) from error
        if not isinstance(response, dict):
            raise JourneyFailure(f"boundary provider response is not an object: {response}")
        return response

    def _write_mutating_provider(self, path: Path) -> None:
        """Create a temporary delegate used only to change production output between calls."""
        path.write_text(
            """#!/usr/bin/env python3
import json
import pathlib
import subprocess
import sys
import time

real_provider = sys.argv[1]
mode_path = pathlib.Path(sys.argv[2])
request_bytes = sys.stdin.read()
request = json.loads(request_bytes)
mode = mode_path.read_text(encoding="utf-8").strip()

if request.get("operation") == "evaluate" and mode == "unsupported":
    mode_path.with_suffix(".request").write_text(request_bytes, encoding="utf-8")
    print(json.dumps({"result": "unsupported"}, separators=(",", ":")))
    raise SystemExit(0)
if request.get("operation") == "evaluate" and mode == "allow":
    print(json.dumps({"result": "allow"}, separators=(",", ":")))
    raise SystemExit(0)
if request.get("operation") == "evaluate" and mode == "allow-other-target":
    print(json.dumps({"result": "allow", "target": "explore"}, separators=(",", ":")))
    raise SystemExit(0)
if request.get("operation") == "evaluate" and mode == "failure":
    raise SystemExit(7)
if request.get("operation") == "evaluate" and mode == "deny":
    print(json.dumps({
        "result": "deny",
        "feedback": {"code": "scenario-denied", "message": "scenario denial"},
    }, separators=(",", ":")))
    raise SystemExit(0)
if request.get("operation") == "evaluate" and mode in {"sleep", "sleep-deny"}:
    mode_path.with_suffix(".started").write_text("started\\n", encoding="utf-8")
    time.sleep(0.25)
    if mode == "sleep-deny":
        mode_path.with_suffix(".result").write_text("deny\\n", encoding="utf-8")
        print(json.dumps({
            "result": "deny",
            "feedback": {"code": "scenario-denied", "message": "scenario denial"},
        }, separators=(",", ":")))
    else:
        mode_path.with_suffix(".result").write_text("allow\\n", encoding="utf-8")
        print(json.dumps({"result": "allow"}, separators=(",", ":")))
    raise SystemExit(0)

completed = subprocess.run(
    [real_provider], input=request_bytes, text=True, capture_output=True, check=False
)
if completed.stderr:
    sys.stderr.write(completed.stderr)
if completed.returncode != 0:
    sys.stdout.write(completed.stdout)
    raise SystemExit(completed.returncode)

if request.get("operation") == "describe" and mode in {"invalid", "changed", "final-outgoing", "initial-final"}:
    workflow = json.loads(completed.stdout)
    if mode == "invalid":
        workflow["initial_state"] = "provider-missing-state"
    elif mode == "final-outgoing":
        workflow["transitions"].append({
            "source": "end",
            "event": "escape",
            "target": "explore",
            "kind": "check-free",
        })
    elif mode == "initial-final":
        workflow["initial_state"] = "end"
    else:
        workflow["states"].append({
            "id": "provider-changed-state",
            "title": "Provider changed",
            "instructions": "changed provider instructions",
            "final": False,
        })
        for transition in workflow["transitions"]:
            if transition["source"] == "explore" and transition["event"] == "intent-ready":
                transition["target"] = "provider-changed-state"
                break
        workflow["states"][0]["instructions"] += " [changed provider instructions]"
    print(json.dumps(workflow, separators=(",", ":")))
else:
    sys.stdout.write(completed.stdout)
""",
            encoding="utf-8",
        )
        path.chmod(0o755)

    def _scenario_start(
        self,
        database: Path,
        provider_config: Path,
        run_id: str,
        input_path: Path,
    ) -> Dict[str, Any]:
        response = self._scenario_engine_call(
            database,
            [
                "--config",
                str(provider_config),
                "--timeout-ms",
                "30000",
                "start",
                "--id",
                run_id,
                "software-change",
                "@" + str(input_path),
                "boundary scenario",
            ],
            cwd=self.data_root,
        )
        if response.get("status") == "completed":
            self._scenario_show(database, run_id)
        return response

    def _scenario_show(self, database: Path, run_id: str) -> Dict[str, Any]:
        response = self._scenario_engine_call(
            database, ["show", run_id], cwd=self.data_root
        )
        self._expect_status(response, "completed", event="show", state="boundary")
        return response["result"]

    def _run_le2_topology_scenario(
        self,
        scenario_dir: Path,
        provider_command: Sequence[str],
        input_path: Path,
    ) -> None:
        invalid_mode = scenario_dir / "le2-invalid.mode"
        invalid_mode.write_text("invalid\n", encoding="utf-8")
        invalid_config = scenario_dir / "le2-invalid.toml"
        self._write_scenario_provider_config(
            invalid_config, provider_command[0], provider_command[1:-1] + [str(invalid_mode)]
        )
        invalid_database = scenario_dir / "le2-invalid.sqlite"
        invalid = self._scenario_start(
            invalid_database, invalid_config, "le2-invalid-run", input_path
        )
        missing_after_invalid = self._scenario_engine_call(
            invalid_database,
            ["show", "le2-invalid-run"],
            cwd=self.data_root,
        )

        unusual_mode = scenario_dir / "le2-unusual.mode"
        unusual_mode.write_text("original\n", encoding="utf-8")
        unusual_config = scenario_dir / "le2-unusual.toml"
        self._write_scenario_provider_config(
            unusual_config,
            provider_command[0],
            provider_command[1:-1] + [str(unusual_mode)],
        )
        unusual_database = scenario_dir / "le2-unusual.sqlite"
        unusual = self._scenario_start(
            unusual_database, unusual_config, "le2-unusual-run", input_path
        )
        workflow = unusual.get("result", {}).get("run", {}).get("workflow", {})
        states = workflow.get("states", [])
        transitions = workflow.get("transitions", [])
        state_ids = {state.get("id") for state in states if isinstance(state, dict)}
        has_cycle = any(
            transition.get("source") == "intent-review"
            and transition.get("event") == "revise"
            and transition.get("target") == "explore"
            for transition in transitions
            if isinstance(transition, dict)
        )
        # bookends:LE-2 — the real start path rejects an uninterpretable production-provider graph and accepts its structurally valid cyclic graph.
        if (
            invalid.get("status") != "error"
            or invalid.get("code") != "undefined-initial-state"
            or missing_after_invalid.get("status") != "error"
            or missing_after_invalid.get("code") != "run-not-found"
        ):
            raise JourneyFailure(
                f"LE-2 malformed workflow was not rejected before persistence: {invalid}; "
                f"follow-up={missing_after_invalid}"
            )
        if (
            unusual.get("status") != "completed"
            or "end" not in state_ids
            or not has_cycle
        ):
            raise JourneyFailure(
                f"LE-2 structurally valid unusual topology was not accepted: {unusual}"
            )
        self.engine_boundary_proof.extend(
            [
                "LE-2 malformed workflow rejected before run creation",
                "LE-2 structurally valid cyclic production topology accepted",
            ]
        )
        print("LE-2 topology scenarios passed: malformed rejected, cyclic topology accepted")

    def _run_le13_final_state_outgoing_scenario(
        self,
        scenario_dir: Path,
        provider_command: Sequence[str],
        input_path: Path,
    ) -> None:
        """Reject a production-provider graph that gives a final state an edge."""
        mode_path = scenario_dir / "le13-final-outgoing.mode"
        mode_path.write_text("final-outgoing\n", encoding="utf-8")
        provider_config = scenario_dir / "le13-final-outgoing.toml"
        self._write_scenario_provider_config(
            provider_config,
            provider_command[0],
            provider_command[1:-1] + [str(mode_path)],
        )
        database = scenario_dir / "le13-final-outgoing.sqlite"
        run_id = "le13-final-outgoing-run"
        started = self._scenario_start(database, provider_config, run_id, input_path)
        missing = self._scenario_engine_call(
            database, ["show", run_id], cwd=self.data_root
        )
        # bookends:LE-13 — the public start rejects a production-provider final state with an outgoing transition before creating a run.
        if (
            started.get("status") != "error"
            or started.get("code") != "transition-from-final-state"
            or missing.get("status") != "error"
            or missing.get("code") != "run-not-found"
        ):
            raise JourneyFailure(
                f"LE-13 final state with outgoing transition was accepted or persisted: "
                f"start={started}; follow-up={missing}"
            )
        self.engine_boundary_proof.append(
            "LE-13 final-state outgoing transition rejected before run creation"
        )
        print(
            "LE-13 final-state scenario passed: outgoing transition rejected before run creation"
        )

    def _run_le14_initially_final_scenario(
        self,
        scenario_dir: Path,
        provider_command: Sequence[str],
        input_path: Path,
    ) -> tuple[Path, str]:
        """Create a public run whose production-provider initial state is final."""
        mode_path = scenario_dir / "le14-initial-final.mode"
        mode_path.write_text("initial-final\n", encoding="utf-8")
        provider_config = scenario_dir / "le14-initial-final.toml"
        self._write_scenario_provider_config(
            provider_config,
            provider_command[0],
            provider_command[1:-1] + [str(mode_path)],
        )
        database = scenario_dir / "le14-initial-final.sqlite"
        run_id = "le14-initial-final-run"
        started = self._scenario_start(database, provider_config, run_id, input_path)
        shown = self._scenario_show(database, run_id)
        # bookends:LE-14 — the public start and fresh show both observe an initially-final run as final at the final state.
        if (
            started.get("status") != "completed"
            or started.get("result", {}).get("run", {}).get("current_state") != "end"
            or started.get("result", {}).get("run", {}).get("lifecycle") != "final"
            or shown.get("current_state") != "end"
            or shown.get("lifecycle") != "final"
        ):
            raise JourneyFailure(
                f"LE-14 initially-final run was not created final: start={started}; show={shown}"
            )
        self.engine_boundary_proof.append(
            "LE-14 initially-final run created with final lifecycle"
        )
        print("LE-14 initially-final scenario passed: run created final")
        return database, run_id

    def _run_le15_terminal_mutation_scenario(
        self,
        database: Path,
        run_id: str,
    ) -> None:
        """Reject every primary terminal mutation without adding history."""
        before_show = self._scenario_show(database, run_id)
        before_history = self._scenario_engine_call(
            database, ["history", run_id], cwd=self.data_root
        )
        self._expect_status(before_history, "completed", event="history", state="end")
        append = self._scenario_engine_call(
            database,
            [
                "append",
                "--record-id=le15-terminal-append",
                run_id,
                "terminal-marker",
                "{}",
            ],
            cwd=self.data_root,
        )
        event = self._scenario_engine_call(
            database, ["event", run_id, "anything"], cwd=self.data_root
        )
        terminate = self._scenario_engine_call(
            database, ["terminate", run_id], cwd=self.data_root
        )
        after_show = self._scenario_show(database, run_id)
        after_history = self._scenario_engine_call(
            database, ["history", run_id], cwd=self.data_root
        )
        self._expect_status(after_history, "completed", event="history", state="end")
        # bookends:LE-15 — public append, event, and terminate all reject the terminal run, while show and history remain unchanged.
        if (
            any(
                response.get("status") != "rejected"
                or response.get("code") != "run-not-active"
                for response in (append, event, terminate)
            )
            or before_show.get("current_state") != "end"
            or before_show.get("lifecycle") != "final"
            or before_show.get("requestable_events") != []
            or after_show != before_show
            or after_history.get("result") != before_history.get("result")
        ):
            raise JourneyFailure(
                f"LE-15 terminal mutation changed state or semantic history: "
                f"append={append}; event={event}; terminate={terminate}; "
                f"before_show={before_show}; after_show={after_show}; "
                f"before_history={before_history}; after_history={after_history}"
            )
        self.engine_boundary_proof.append(
            "LE-15 terminal append/event/terminate rejected without history change"
        )
        print(
            "LE-15 terminal-mutation scenario passed: append/event/terminate rejected without history change"
        )

    def _run_le11_frozen_topology_scenario(
        self,
        scenario_dir: Path,
        provider_command: Sequence[str],
    ) -> None:
        mode_path = scenario_dir / "le11.mode"
        mode_path.write_text("original\n", encoding="utf-8")
        provider_config = scenario_dir / "le11.toml"
        self._write_scenario_provider_config(
            provider_config,
            provider_command[0],
            provider_command[1:-1] + [str(mode_path)],
        )
        input_value = self._read_json(
            self.data_root / STITCHED_PROFILE_SUBPATH, "LE-11 minimal profile"
        )
        input_path = scenario_dir / "le11-input.json"
        input_path.write_text(json.dumps(input_value, indent=2) + "\n", encoding="utf-8")
        database = scenario_dir / "le11.sqlite"
        run_id = "le11-frozen-run"
        started = self._scenario_start(database, provider_config, run_id, input_path)
        self._expect_status(started, "completed", event="start", state="explore")
        original_show = self._scenario_show(database, run_id)
        original_event = next(
            item
            for item in original_show["requestable_events"]
            if item.get("event") == "intent-ready"
        )
        original_instructions = original_show["current_state_instructions"]

        mode_path.write_text("changed\n", encoding="utf-8")
        changed_workflow = self._scenario_provider_call(
            provider_command[:-1] + [str(mode_path)],
            {"operation": "describe", "initial_input": input_value},
        )
        changed_event = next(
            item
            for item in changed_workflow["transitions"]
            if item.get("source") == "explore" and item.get("event") == "intent-ready"
        )
        changed_instructions = changed_workflow["states"][0]["instructions"]
        frozen_show = self._scenario_show(database, run_id)
        frozen_event = next(
            item
            for item in frozen_show["requestable_events"]
            if item.get("event") == "intent-ready"
        )
        # bookends:LE-11 — after the provider's current describe changes, the public show assertion retains the stored edge and exact instructions.
        if (
            changed_event.get("target") == original_event.get("target")
            or "changed provider instructions" not in changed_instructions
            or changed_instructions == original_instructions
        ):
            raise JourneyFailure(
                f"LE-11 provider describe did not change as expected: {changed_workflow}"
            )
        if (
            frozen_event != original_event
            or frozen_show["current_state_instructions"] != original_instructions
            or "changed provider instructions" in frozen_show["current_state_instructions"]
        ):
            raise JourneyFailure(
                f"LE-11 active run did not retain its stored topology/instructions: {frozen_show}"
            )
        self.engine_boundary_proof.append(
            "LE-11 show retained frozen topology and instructions after describe change"
        )
        print("LE-11 frozen-run scenario passed: changed describe did not alter show")

    def _run_le12_unsupported_action_scenario(
        self,
        scenario_dir: Path,
        provider_command: Sequence[str],
    ) -> None:
        mode_path = scenario_dir / "le12.mode"
        mode_path.write_text("original\n", encoding="utf-8")
        provider_config = scenario_dir / "le12.toml"
        self._write_scenario_provider_config(
            provider_config,
            provider_command[0],
            provider_command[1:-1] + [str(mode_path)],
        )
        input_value = self._read_json(
            self.data_root / STITCHED_PROFILE_SUBPATH, "LE-12 minimal profile"
        )
        input_path = scenario_dir / "le12-input.json"
        input_path.write_text(json.dumps(input_value, indent=2) + "\n", encoding="utf-8")
        database = scenario_dir / "le12.sqlite"
        run_id = "le12-unsupported-run"
        started = self._scenario_start(database, provider_config, run_id, input_path)
        self._expect_status(started, "completed", event="start", state="explore")
        before = self._scenario_show(database, run_id)
        for record_id, kind, data in (
            (
                "le19-first",
                "user-steering",
                '{"text":"preserve the first context record"}',
            ),
            (
                "le19-second",
                "observation",
                '{"text":"preserve the second context record"}',
            ),
        ):
            appended = self._scenario_engine_call(
                database,
                ["append", f"--record-id={record_id}", run_id, kind, data],
                cwd=self.data_root,
            )
            self._expect_status(appended, "completed", event="append", state="explore")
        mode_path.write_text("unsupported\n", encoding="utf-8")
        failed = self._scenario_engine_call(
            database,
            ["event", run_id, "intent-ready"],
            cwd=self.data_root,
        )
        after = self._scenario_show(database, run_id)
        history = self._scenario_engine_call(
            database, ["history", run_id], cwd=self.data_root
        )
        self._expect_status(history, "completed", event="history", state="explore")
        transition_history = [
            entry
            for entry in history["result"]
            if entry.get("action", {}).get("kind") == "transition"
        ]
        request_path = mode_path.with_suffix(".request")
        try:
            evaluate_request = json.loads(request_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise JourneyFailure(f"LE-12 did not capture the stored evaluate request: {error}") from error
        # bookends:LE-19 — the public evaluate request carries every accumulated context record in append order.
        if [item.get("id") for item in evaluate_request.get("context", [])] != [
            "le19-first",
            "le19-second",
        ]:
            raise JourneyFailure(
                f"LE-19 evaluate context was missing or out of order: {evaluate_request}"
            )
        # bookends:LE-12 — an unavailable stored action returns explicit evaluate failure, while show and history prove no advancement or lineage.
        if (
            failed.get("status") != "error"
            or failed.get("code") != "provider-unsupported"
            or failed.get("message", "").find("intent-ready") < 0
        ):
            raise JourneyFailure(
                f"LE-12 unavailable stored action was not an explicit error: {failed}"
            )
        mode_path.write_text("failure\n", encoding="utf-8")
        provider_failed = self._scenario_engine_call(
            database,
            ["event", run_id, "intent-ready"],
            cwd=self.data_root,
        )
        after_provider_failure = self._scenario_show(database, run_id)
        history_after_provider_failure = self._scenario_engine_call(
            database, ["history", run_id], cwd=self.data_root
        )
        self._expect_status(
            history_after_provider_failure,
            "completed",
            event="history",
            state="explore",
        )
        # bookends:LE-8 — the public unsupported and provider-failure errors preserve the same state as the checked rejection above.
        # bookends:LE-26 — unavailable events, reads, unsupported evaluations, and provider failures add no semantic history.
        # bookends:LE-32 — unsupported and failed evaluations, including this uncommitted result, do not enter lineage.
        # bookends:LE-35 — the captured public evaluate request carries no raw run history.
        if (
            before.get("current_state") != "explore"
            or after.get("current_state") != "explore"
            or after.get("latest_evaluations") != []
            or provider_failed.get("status") != "error"
            or after_provider_failure.get("current_state") != "explore"
            or after_provider_failure.get("latest_evaluations") != []
            or transition_history
            or "history" in evaluate_request
            or history_after_provider_failure.get("result")
            != history.get("result")
        ):
            raise JourneyFailure(
                f"LE-12 unsupported/provider failure advanced or polluted the run: "
                f"unsupported={failed}, provider_failed={provider_failed}, "
                f"after={after}, after_failure={after_provider_failure}, "
                f"history={history_after_provider_failure}"
            )
        self.engine_boundary_proof.append(
            "LE-12 unsupported stored action and provider failure failed without state or history advancement"
        )
        print("LE-12 unsupported-action scenario passed: explicit and operational errors preserved state")

    def _run_review_revision_scenario(
        self,
        scenario_dir: Path,
        provider_command: Sequence[str],
    ) -> None:
        mode_path = scenario_dir / "review-revision.mode"
        mode_path.write_text("allow\n", encoding="utf-8")
        provider_config = scenario_dir / "review-revision.toml"
        self._write_scenario_provider_config(
            provider_config,
            provider_command[0],
            provider_command[1:-1] + [str(mode_path)],
        )
        input_path = scenario_dir / "review-revision-input.json"
        review_input = self._read_json(
            self.data_root / PROFILE_SUBPATH, "LE-23 review-revision profile"
        )
        input_path.write_text(json.dumps(review_input, indent=2) + "\n", encoding="utf-8")
        database = scenario_dir / "review-revision.sqlite"
        run_id = "review-revision-run"
        started = self._scenario_start(database, provider_config, run_id, input_path)
        self._expect_status(started, "completed", event="start", state="explore")
        ready = self._scenario_engine_call(
            database, ["event", run_id, "intent-ready"], cwd=self.data_root
        )
        self._expect_status(ready, "completed", event="intent-ready", state="explore")
        evidence = self._scenario_engine_call(
            database,
            [
                "append",
                "--record-id=review-revision-evidence",
                run_id,
                "review-evidence",
                '{"gate":"intent-review","policy_id":"review-revision","result":"pass"}',
            ],
            cwd=self.data_root,
        )
        self._expect_status(evidence, "completed", event="append", state="intent-review")
        mode_path.write_text("deny\n", encoding="utf-8")
        denied = self._scenario_engine_call(
            database, ["event", run_id, "approved"], cwd=self.data_root
        )
        mode_path.write_text("unsupported\n", encoding="utf-8")
        unsupported_request = mode_path.with_suffix(".request")
        unsupported_request.unlink(missing_ok=True)
        revised = self._scenario_engine_call(
            database, ["event", run_id, "revise"], cwd=self.data_root
        )
        revised_show = self._scenario_show(database, run_id)
        # bookends:LE-5 — the public check-free revision commits without invoking the production provider.
        if revised.get("status") != "completed" or unsupported_request.exists():
            raise JourneyFailure(
                f"check-free revision invoked unavailable provider: revised={revised}; "
                f"request={unsupported_request}"
            )
        latest_after_revision = [
            item
            for item in revised_show.get("latest_evaluations", [])
            if item.get("transition", {}).get("source") == "intent-review"
            and item.get("transition", {}).get("event") == "approved"
        ]
        # bookends:LE-23 — a fresh public show after revision retains the latest exact-transition denial.
        # bookends:LE-49 — that same show carries frozen policies, appended evidence, and actionable feedback without reading history.
        # bookends:LE-48 — this public review run observes a checked denial and then takes the owning check-free revision edge.
        # bookends:LE-54 — the same check-free revision succeeds while the provider is explicitly unable to evaluate.
        if (
            denied.get("status") != "rejected"
            or denied.get("code") != "scenario-denied"
            or revised_show.get("current_state") != "explore"
            or not isinstance(revised_show.get("initial_input", {}).get("review_policies"), dict)
            or not any(
                record.get("id") == "review-revision-evidence"
                for record in revised_show.get("context", [])
                if isinstance(record, dict)
            )
            or len(latest_after_revision) != 1
            or latest_after_revision[0].get("result", {}).get("result") != "deny"
            or latest_after_revision[0].get("result", {}).get("feedback", {}).get("code")
            != "scenario-denied"
        ):
            raise JourneyFailure(
                f"review denial/revision scenario regressed: denied={denied}, "
                f"revised={revised}, show={revised_show}"
            )
        lineage_database = scenario_dir / "review-lineage.sqlite"
        lineage_run_id = "review-lineage-run"
        mode_path.write_text("allow\n", encoding="utf-8")
        lineage_started = self._scenario_start(
            lineage_database, provider_config, lineage_run_id, input_path
        )
        self._expect_status(lineage_started, "completed", event="start", state="explore")
        lineage_ready = self._scenario_engine_call(
            lineage_database, ["event", lineage_run_id, "intent-ready"], cwd=self.data_root
        )
        self._expect_status(lineage_ready, "completed", event="intent-ready", state="explore")
        allowed = self._scenario_engine_call(
            lineage_database, ["event", lineage_run_id, "approved"], cwd=self.data_root
        )
        self._expect_status(allowed, "completed", event="approved", state="intent-review")
        revised_lineage = self._scenario_engine_call(
            lineage_database, ["event", lineage_run_id, "revise"], cwd=self.data_root
        )
        self._expect_status(revised_lineage, "completed", event="revise", state="intent-adversarial-review")
        lineage_ready_again = self._scenario_engine_call(
            lineage_database, ["event", lineage_run_id, "intent-ready"], cwd=self.data_root
        )
        self._expect_status(
            lineage_ready_again, "completed", event="intent-ready", state="explore"
        )
        mode_path.write_text("deny\n", encoding="utf-8")
        denied_after_allow = self._scenario_engine_call(
            lineage_database, ["event", lineage_run_id, "approved"], cwd=self.data_root
        )
        lineage_show = self._scenario_show(lineage_database, lineage_run_id)
        latest_lineage = [
            item
            for item in lineage_show.get("latest_evaluations", [])
            if item.get("transition", {}).get("source") == "intent-review"
            and item.get("transition", {}).get("event") == "approved"
        ]
        # bookends:LE-24 — a later provider denial supersedes an earlier allow on the same exact checked edge.
        if (
            allowed.get("result", {}).get("run", {}).get("current_state")
            != "intent-adversarial-review"
            or denied_after_allow.get("status") != "rejected"
            or len(latest_lineage) != 1
            or latest_lineage[0].get("result", {}).get("result") != "deny"
            or latest_lineage[0].get("result", {}).get("feedback", {}).get("code")
            != "scenario-denied"
        ):
            raise JourneyFailure(
                f"allow-to-deny lineage did not supersede on the exact edge: "
                f"allowed={allowed}; denied={denied_after_allow}; show={lineage_show}"
            )
        target_database = scenario_dir / "provider-target.sqlite"
        target_run_id = "provider-target-run"
        mode_path.write_text("allow\n", encoding="utf-8")
        target_started = self._scenario_start(
            target_database, provider_config, target_run_id, input_path
        )
        self._expect_status(target_started, "completed", event="start", state="explore")
        target_ready = self._scenario_engine_call(
            target_database, ["event", target_run_id, "intent-ready"], cwd=self.data_root
        )
        self._expect_status(target_ready, "completed", event="intent-ready", state="explore")
        mode_path.write_text("allow-other-target\n", encoding="utf-8")
        target_response = self._scenario_engine_call(
            target_database, ["event", target_run_id, "approved"], cwd=self.data_root
        )
        target_show = self._scenario_show(target_database, target_run_id)
        target_lineage = [
            item
            for item in target_show.get("latest_evaluations", [])
            if item.get("transition", {}).get("source") == "intent-review"
            and item.get("transition", {}).get("event") == "approved"
        ]
        # bookends:LE-7 — an attempted provider-selected target is rejected by the public provider protocol and cannot alter the stored graph route.
        if (
            target_response.get("status") != "error"
            or target_show.get("current_state") != "intent-review"
            or target_lineage
        ):
            raise JourneyFailure(
                f"provider target injection changed routing or lineage: "
                f"response={target_response}; show={target_show}"
            )
        direct_state = self._scenario_engine_call(
            target_database,
            ["event", target_run_id, "approved", "end"],
            cwd=self.data_root,
        )
        direct_state_show = self._scenario_show(target_database, target_run_id)
        # bookends:LE-1 — the public event grammar accepts an event request, not a caller-supplied state, and the extra state token cannot advance the run.
        if (
            direct_state.get("status") == "completed"
            or direct_state_show.get("current_state") != "intent-review"
        ):
            raise JourneyFailure(
                f"caller-supplied state altered current state: "
                f"response={direct_state}; show={direct_state_show}"
            )
        print("review-revision scenario passed: denial, both-way lineage, target isolation, and provider-free repair")

    def _run_concurrency_scenarios(
        self,
        scenario_dir: Path,
        provider_command: Sequence[str],
    ) -> None:
        mode_path = scenario_dir / "concurrency.mode"
        mode_path.write_text("allow\n", encoding="utf-8")
        provider_config = scenario_dir / "concurrency.toml"
        self._write_scenario_provider_config(
            provider_config,
            provider_command[0],
            provider_command[1:-1] + [str(mode_path)],
        )
        input_path = scenario_dir / "concurrency-input.json"
        input_path.write_text(
            json.dumps({"objective": "concurrency"}, indent=2) + "\n",
            encoding="utf-8",
        )

        race_database = scenario_dir / "le36.sqlite"
        self._scenario_start_and_ready(
            race_database, provider_config, input_path, "le36-race-run"
        )
        processes = []
        for _ in range(2):
            processes.append(
                subprocess.Popen(
                    [
                        str(self.engine),
                        "--database",
                        str(race_database),
                        "--json",
                        "event",
                        "le36-race-run",
                        "revise",
                    ],
                    text=True,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    cwd=self.data_root,
                )
            )
        race_results = []
        for process in processes:
            stdout, stderr = process.communicate()
            try:
                result = json.loads(stdout)
            except json.JSONDecodeError as error:
                raise JourneyFailure(
                    f"LE-36 concurrent event returned non-JSON: {error}; stderr={stderr!r}"
                ) from error
            race_results.append(result)
        race_show = self._scenario_show(race_database, "le36-race-run")
        race_history = self._scenario_engine_call(
            race_database, ["history", "le36-race-run"], cwd=self.data_root
        )
        committed_races = [result for result in race_results if result.get("status") == "completed"]
        race_transitions = [
            entry
            for entry in race_history.get("result", [])
            if entry.get("action", {}).get("kind") == "transition"
            and entry.get("action", {}).get("transition", {}).get("event") == "revise"
        ]
        # bookends:LE-36 — two real CLI event attempts against one pre-mutation state produce one commit and one non-commit.
        if (
            len(committed_races) != 1
            or len(race_transitions) != 1
            or race_show.get("current_state") != "explore"
        ):
            raise JourneyFailure(
                f"LE-36 concurrent events conflicted incorrectly: results={race_results}, "
                f"show={race_show}, history={race_history}"
            )

        def run_stale_case(
            database: Path,
            run_id: str,
            mode: str,
            expected_provider_result: str,
        ) -> tuple[Dict[str, Any], Dict[str, Any], Dict[str, Any], Dict[str, Any]]:
            self._scenario_start_and_ready(database, provider_config, input_path, run_id)
            baseline_show = self._scenario_show(database, run_id)
            mode_path.write_text(mode + "\n", encoding="utf-8")
            started_marker = mode_path.with_suffix(".started")
            result_marker = mode_path.with_suffix(".result")
            started_marker.unlink(missing_ok=True)
            result_marker.unlink(missing_ok=True)
            checked = subprocess.Popen(
                [
                    str(self.engine),
                    "--database",
                    str(database),
                    "--json",
                    "event",
                    run_id,
                    "approved",
                ],
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                cwd=self.data_root,
            )
            deadline = time.monotonic() + 5
            while not started_marker.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            if not started_marker.exists():
                checked.kill()
                checked.communicate()
                raise JourneyFailure("LE-37 provider did not enter the in-flight evaluation")
            revised = self._scenario_event_call(database, run_id, "revise")
            checked_stdout, checked_stderr = checked.communicate()
            try:
                checked_result = json.loads(checked_stdout)
            except json.JSONDecodeError as error:
                raise JourneyFailure(
                    f"LE-37 stale event returned non-JSON: {error}; stderr={checked_stderr!r}"
                ) from error
            stale_show = self._scenario_show(database, run_id)
            stale_history = self._scenario_engine_call(
                database, ["history", run_id], cwd=self.data_root
            )
            stale_approved = [
                entry
                for entry in stale_history.get("result", [])
                if entry.get("action", {}).get("transition", {}).get("event")
                == "approved"
            ]
            stale_latest_approved = [
                evaluation
                for evaluation in stale_show.get("latest_evaluations", [])
                if evaluation.get("transition", {}).get("event") == "approved"
            ]
            provider_result = result_marker.read_text(encoding="utf-8").strip()
            # bookends:LE-37 — both allow and deny results made stale by a public revision produce no transition or lineage.
            if (
                revised.get("status") != "completed"
                or checked_result.get("status") != "error"
                or provider_result != expected_provider_result
                or stale_show.get("current_state") != "explore"
                or stale_show.get("context") != baseline_show.get("context")
                or stale_approved
                or stale_latest_approved
            ):
                raise JourneyFailure(
                    f"LE-37 stale evaluation produced a semantic effect: "
                    f"provider={provider_result}, checked={checked_result}, "
                    f"revised={revised}, show={stale_show}, history={stale_history}"
                )
            return checked_result, revised, stale_show, stale_history

        allow_stale = run_stale_case(
            scenario_dir / "le37-allow.sqlite",
            "le37-stale-allow-run",
            "sleep",
            "allow",
        )
        deny_stale = run_stale_case(
            scenario_dir / "le37-deny.sqlite",
            "le37-stale-deny-run",
            "sleep-deny",
            "deny",
        )
        # bookends:LE-38 — the state-race cases intentionally do not make a guarantee about concurrent context appends; they only assert that this fixture did not mutate context.
        if any(
            not isinstance(value, dict)
            for case in (allow_stale, deny_stale)
            for value in case
        ):
            raise JourneyFailure("LE-37 stale cases did not return public objects")
        print("concurrency scenarios passed: one commit and stale allow/deny evaluations fail-closed")

    def _run_binding_start_validation_scenario(
        self,
        scenario_dir: Path,
        provider_command: Sequence[str],
    ) -> None:
        """Exercise start's frozen binding admission through real CLI processes."""
        mode_path = scenario_dir / "binding-validation.mode"
        mode_path.write_text("original\n", encoding="utf-8")
        provider_config = scenario_dir / "binding-validation.toml"
        self._write_scenario_provider_config(
            provider_config,
            provider_command[0],
            provider_command[1:-1] + [str(mode_path)],
        )
        base = self._read_json(
            self.data_root / STITCHED_PROFILE_SUBPATH,
            "LE-76 minimal profile",
        )

        def start_variant(name: str, value: Dict[str, Any]) -> Dict[str, Any]:
            input_path = scenario_dir / f"{name}.json"
            input_path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
            database = scenario_dir / f"{name}.sqlite"
            return self._scenario_start(database, provider_config, name, input_path)

        omitted = start_variant("le76-omitted", dict(base))
        empty_value = dict(base)
        empty_value["work_slot_bindings"] = {}
        empty = start_variant("le76-empty", empty_value)
        if omitted.get("status") != "completed" or empty.get("status") != "completed":
            raise JourneyFailure(
                f"LE-76 omitted/empty start failed: omitted={omitted}; empty={empty}"
            )
        omitted_show = self._scenario_show(
            scenario_dir / "le76-omitted.sqlite", "le76-omitted"
        )
        empty_show = self._scenario_show(
            scenario_dir / "le76-empty.sqlite", "le76-empty"
        )

        valid_argv = dict(base)
        valid_argv["work_slot_bindings"] = {
            "intent-draft": {"command": "echo", "args": ["fan-out"]},
            "implement": {"command": "echo", "args": ["run-plan-graph"]},
        }
        argv_start = start_variant("le76-argv", valid_argv)

        invalid_variants = {
            "le76-unknown-slot": {
                **base,
                "work_slot_bindings": {
                    "not-a-catalog-slot": {"command": "echo", "args": []}
                },
            },
            "le76-unknown-field": {
                **base,
                "work_slot_bindings": {
                    "intent-draft": {
                        "command": "echo",
                        "args": [],
                        "unexpected": True,
                    }
                },
            },
            "le76-map-not-object": {**base, "work_slot_bindings": []},
            "le76-binding-not-object": {
                **base,
                "work_slot_bindings": {"intent-draft": []},
            },
        }
        invalid_results = {
            name: start_variant(name, value)
            for name, value in invalid_variants.items()
        }
        invalid_followups = {
            name: self._scenario_engine_call(
                scenario_dir / f"{name}.sqlite",
                ["show", name],
                cwd=self.data_root,
            )
            for name in invalid_variants
        }
        # bookends:LE-76 — real start/show calls prove omitted and empty bindings are unbound, valid fan-out/run-plan-graph argv is frozen without parsing, and malformed binding maps are rejected before persistence.
        if (
            omitted.get("status") != "completed"
            or empty.get("status") != "completed"
            or argv_start.get("status") != "completed"
            or omitted_show.get("initial_input", {}).get("work_slot_bindings") is not None
            or empty_show.get("initial_input", {}).get("work_slot_bindings") != {}
            or omitted_show.get("work_slot_invocations") != []
            or empty_show.get("work_slot_invocations") != []
            or any(result.get("status") != "rejected" for result in invalid_results.values())
            or any(result.get("status") != "error" for result in invalid_followups.values())
        ):
            raise JourneyFailure(
                f"LE-76 binding admission regressed: omitted={omitted}; empty={empty}; "
                f"argv={argv_start}; invalid={invalid_results}; followups={invalid_followups}"
            )
        print("LE-76 binding-start scenarios passed: omitted/empty, argv freeze, and invalid maps")

    def _run_engine_boundary_scenarios(self) -> None:
        """Drive focused workflow-boundary cases through real CLI processes."""
        if self.mode != "source":
            raise JourneyFailure("engine boundary scenarios are source-only", state=self.state)
        assert self.run_dir is not None
        assert self.data_root is not None
        scenario_dir = self.run_dir / "engine-boundary-scenarios"
        scenario_dir.mkdir(parents=True, exist_ok=True)
        provider_wrapper = scenario_dir / "mutating-production-provider.py"
        self._write_mutating_provider(provider_wrapper)
        wrapper_command = [
            sys.executable,
            str(provider_wrapper),
            str(self.provider),
            "unused-mode-file",
        ]
        input_path = scenario_dir / "le2-input.json"
        input_path.write_text(
            json.dumps({"objective": "boundary topology"}, indent=2) + "\n",
            encoding="utf-8",
        )
        self._run_le2_topology_scenario(scenario_dir, wrapper_command, input_path)
        self._run_le13_final_state_outgoing_scenario(
            scenario_dir, wrapper_command, input_path
        )
        terminal_database, terminal_run_id = self._run_le14_initially_final_scenario(
            scenario_dir, wrapper_command, input_path
        )
        self._run_le15_terminal_mutation_scenario(terminal_database, terminal_run_id)
        self._run_le11_frozen_topology_scenario(scenario_dir, wrapper_command)
        self._run_le12_unsupported_action_scenario(scenario_dir, wrapper_command)
        self._run_review_revision_scenario(scenario_dir, wrapper_command)
        self._run_binding_start_validation_scenario(scenario_dir, wrapper_command)
        self._run_concurrency_scenarios(scenario_dir, wrapper_command)

    def _run_bookends_enabled_source(self) -> None:
        """Drive the opt-in overlay through fresh engine/provider processes."""
        if self.mode != "source":
            raise JourneyFailure("Bookends overlay proof is source-only", state=self.state)
        assert self.run_dir is not None
        assert self.fixture_root is not None
        assert self.provider_config is not None

        scenario_dir = self.run_dir / "bookends-enabled"
        checkout = scenario_dir / "checkout"
        artifacts = scenario_dir / "artifacts"
        database = scenario_dir / "run.sqlite"
        profile_path = scenario_dir / "minimal-bookends.json"
        scenario_dir.mkdir(parents=True, exist_ok=True)

        saved = {
            "run_id": self.run_id,
            "database": self.database,
            "artifact_root": self.artifact_root,
            "profile_path": self.profile_path,
            "profile_source": self.profile_source,
            "profile": self.profile,
            "state": self.state,
            "command_cwd": self.command_cwd,
            "repository_root": self.repository_root,
            "command_env": self.command_env,
        }
        try:
            shutil.copytree(
                self.data_root,
                checkout,
                ignore=shutil.ignore_patterns(".git", "target", "__pycache__", "*.pyc"),
            )
            for git_args in (
                ["init", "-q"],
                ["config", "user.name", "software-change journey"],
                ["config", "user.email", "journey@example.invalid"],
                ["config", "commit.gpgsign", "false"],
                ["add", "-A"],
                ["commit", "-qm", "bookends journey baseline"],
            ):
                completed = subprocess.run(
                    ["git", *git_args],
                    cwd=checkout,
                    text=True,
                    capture_output=True,
                    check=False,
                )
                if completed.returncode != 0:
                    raise JourneyFailure(
                        f"Bookends scenario git {' '.join(git_args)} failed: "
                        f"{completed.stderr.strip() or completed.stdout.strip()}"
                    )

            prd_path = checkout / "docs/PRD.md"
            prd = prd_path.read_text(encoding="utf-8")
            prd_path.write_text(
                prd.rstrip()
                + f"\n\n### {BOOKENDS_TOMBSTONE_ID}: Retired journey fixture\n"
                + "- Status: tombstone\n",
                encoding="utf-8",
            )
            artifacts.mkdir()
            for subject, fixture in SUBJECTS.items():
                shutil.copy2(self.fixture_root / fixture, artifacts / subject)
            shipped_profile = self.data_root / BOOKENDS_SCENARIO_PROFILE
            shutil.copy2(shipped_profile, profile_path)
            scenario_profile = self._read_json(profile_path, "Bookends scenario profile")
            scenario_profile["artifact_root"] = str(artifacts)
            scenario_profile["extra"] = {"bookends": {"enabled": True}}
            profile_path.write_text(
                json.dumps(scenario_profile, indent=2) + "\n", encoding="utf-8"
            )

            self.run_id = BOOKENDS_SCENARIO_RUN_ID
            self.database = database
            self.artifact_root = artifacts
            self.profile_path = profile_path
            self.profile_source = shipped_profile
            self.profile = scenario_profile
            self.state = "not-started"
            self.command_cwd = checkout
            self.repository_root = checkout
            # An explicit empty value prevents a caller's ambient bypass from
            # changing the normal scenario while still allowing the next case.
            self.command_env = {"BOOKENDS_BYPASS": ""}
            self._write_provider_config()
            self._start()
            self._create_checkpoint("implementation")

            shown = self._assert_show("explore", "bookends-enabled-show")
            initial_input = shown.get("initial_input", {})
            if initial_input.get("extra", {}).get("bookends", {}).get("enabled") is not True:
                raise JourneyFailure(
                    "enabled Bookends option was not frozen in initial_input",
                    state=self.state,
                    event="show",
                )
            instructions = shown.get("current_state_instructions", "")
            for fragment in (
                "Bookends overlay",
                "requirement_ids",
                "durable e2e/journey",
                "Never mint an ID",
                "".join(("bookends", ":LE-", "<n>")),
            ):
                if fragment not in instructions:
                    raise JourneyFailure(
                        f"enabled Bookends instructions omitted {fragment!r}",
                        state=self.state,
                        event="show",
                    )

            def write_artifact(subject: str, ids: Optional[list[str]]) -> None:
                value = self._read_json(
                    self.fixture_root / SUBJECTS[subject],
                    f"Bookends fixture {subject}",
                )
                if ids is not None:
                    value["requirement_ids"] = ids
                (artifacts / subject).write_text(
                    json.dumps(value, indent=2) + "\n", encoding="utf-8"
                )

            def require_rule(response: Dict[str, Any], rule: str) -> None:
                violations = response.get("details", {}).get("violations", [])
                if not any(item.get("rule") == rule for item in violations):
                    raise JourneyFailure(
                        f"Bookends schema denial omitted {rule}: {response}",
                        state=self.state,
                        event="intent-ready",
                    )

            # Missing field, empty collection, and a tombstoned ID each travel
            # through the public event path and are refused as schema-invalid.
            write_artifact("intent.json", None)
            missing = self._expect_denial(
                "intent-ready", "intent", "software-change-schema-invalid"
            )
            require_rule(missing, "required")

            write_artifact("intent.json", [])
            empty = self._expect_denial(
                "intent-ready", "intent", "software-change-schema-invalid"
            )
            require_rule(empty, "minItems")

            write_artifact("intent.json", [BOOKENDS_TOMBSTONE_ID])
            tombstoned = self._expect_denial(
                "intent-ready", "intent", "software-change-schema-invalid"
            )
            require_rule(tombstoned, "requirement-ids-live")

            for subject in ("intent.json", "design.json", "plan.json", "validation-report.json"):
                write_artifact(subject, ["LE-1"])
            self._expect_allow("intent-ready", "design")
            self._expect_allow("design-ready", "plan")
            self._expect_allow("plan-ready", "implement")
            green_prd = (checkout / "docs/PRD.md").read_text(encoding="utf-8")
            (checkout / "docs/PRD.md").write_text(
                green_prd
                + "\n### LE-9001: Uncovered RED fixture\n"
                + "- Status: live\n- Coverage: e2e/journey\n",
                encoding="utf-8",
            )
            self._create_checkpoint("implementation")
            self._expect_allow("implementation-ready", "validation")
            self._create_checkpoint("validation")
            self._expect_allow("validation-ready", "validation-review")

            red = self._expect_denial(
                "passed", "bookends", "software-change-bookends-red"
            )
            red_details = red.get("details", {})
            if red_details.get("status") != "red":
                raise JourneyFailure(
                    f"real RED checker result was not surfaced by provider: {red}",
                    state=self.state,
                    event="passed",
                )

            self.command_env = {"BOOKENDS_BYPASS": "journey:public-proof"}
            bypass = self._expect_denial(
                "passed", "bookends", "software-change-bookends-red"
            )
            bypass_details = bypass.get("details", {})
            if (
                bypass_details.get("status") != "bypass"
                or bypass_details.get("bypass_class") != "journey"
                or bypass_details.get("bypass_reason") != "public-proof"
            ):
                raise JourneyFailure(
                    f"real BYPASS checker result was not surfaced by provider: {bypass}",
                    state=self.state,
                    event="passed",
                )

            (checkout / "docs/PRD.md").write_text(green_prd, encoding="utf-8")
            self.command_env = {"BOOKENDS_BYPASS": ""}
            self._expect_allow("revise-implementation", "implement")
            implementation_path = artifacts / "implementation-report.json"
            implementation = self._read_json(
                implementation_path, "Bookends recovered implementation report"
            )
            implementation["revision"] = str(implementation["revision"]) + "-recovered"
            implementation_path.write_text(
                json.dumps(implementation, indent=2) + "\n", encoding="utf-8"
            )
            self._create_checkpoint("implementation")
            self._expect_allow("implementation-ready", "validation")
            self._create_checkpoint("validation")
            self._expect_allow("validation-ready", "validation-review")
            validation_revision = self._fixture_revision("validation-report.json")
            for index, axis in enumerate(
                ("intent-delivered", "ids-grounded", "bypass-not-green")
            ):
                evidence = {
                    "gate": "validation-review",
                    "policy_id": axis,
                    "result": "pass",
                    "findings": "",
                    "author": {"name": f"bookends-{axis}", "kind": "script"},
                    "subject": "validation-report.json",
                    "subject_revision": validation_revision,
                    "config_version": scenario_profile["config_version"],
                }
                response = self._engine(
                    [
                        "append",
                        f"--record-id=bookends-evidence-{index}",
                        self.run_id,
                        "review-evidence",
                        json.dumps(evidence, separators=(",", ":")),
                    ],
                    state=self.state,
                    event="append",
                    axis=axis,
                )
                self._expect_status(
                    response,
                    "completed",
                    event="append",
                    axis=axis,
                    state=self.state,
                )
            self._append_finding_ledger_for(
                self.run_id, "validation-review", state=self.state, record_prefix="bookends-"
            )
            self._expect_allow("passed", "end")
            final = self._assert_show("end", "bookends-enabled-terminal")
            if final.get("lifecycle") != "final" or final.get("requestable_events") != []:
                raise JourneyFailure(
                    f"Bookends enabled scenario did not finish terminal: {final}",
                    state=self.state,
                    event="show",
                )

            proof_path = scenario_dir / "bookends-enabled-proof.json"
            proof_path.write_text(
                json.dumps(
                    {
                        "result": "passed",
                        "run_id": BOOKENDS_SCENARIO_RUN_ID,
                        "profile": str(shipped_profile),
                        "candidate_checkout": str(checkout),
                        "cases": [
                            "enabled instructions visible through show",
                            "missing requirement_ids refused as schema-invalid",
                            "empty requirement_ids refused as schema-invalid",
                            "tombstoned requirement ID refused as schema-invalid",
                            "validation passed refused on real RED checker",
                            "validation passed refused on real BYPASS checker",
                            "green checker allowed terminal validation after evidence",
                        ],
                        "database": str(database),
                        "artifact_root": str(artifacts),
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )
            self.bookends_proof = proof_path
            print(
                "bookends-enabled external scenario passed: "
                "show instructions, missing/empty/tombstoned IDs, RED, and BYPASS"
            )
        finally:
            self.run_id = saved["run_id"]
            self.database = saved["database"]
            self.artifact_root = saved["artifact_root"]
            self.profile_path = saved["profile_path"]
            self.profile_source = saved["profile_source"]
            self.profile = saved["profile"]
            self.state = saved["state"]
            self.command_cwd = saved["command_cwd"]
            self.repository_root = saved["repository_root"]
            self.command_env = saved["command_env"]

    def _run_stitched_source(self) -> None:
        """Walk the live graph produced from shipped minimal.json on a second run."""
        assert self.run_dir is not None
        assert self.fixture_root is not None
        source = self.data_root / STITCHED_PROFILE_SUBPATH
        if not source.is_file():
            raise JourneyFailure(f"stitched profile is missing: {source}")

        saved_run_id = self.run_id
        saved_artifact_root = self.artifact_root
        saved_profile_path = self.profile_path
        saved_profile = self.profile
        saved_state = self.state
        saved_bindings = self.work_slot_bindings
        saved_source = self.profile_source

        stitched_dir = self.run_dir / "stitched"
        artifact_root = stitched_dir / "artifacts"
        artifact_root.mkdir(parents=True)

        self.profile_source = source
        self.profile_path = stitched_dir / "minimal.json"
        self.artifact_root = artifact_root
        self.run_id = STITCHED_RUN_ID
        self.stitched_run_id = STITCHED_RUN_ID
        self.state = "not-started"

        try:
            self._prepare_profile()
            self._assert_stitched_profile(self.profile)

            for subject, fixture in SUBJECTS.items():
                shutil.copy2(self.fixture_root / fixture, artifact_root / subject)

            self._start_run(self.run_id)
            shown = self._assert_show("explore", "stitched-start")
            # bookends:LE-43 — the same production provider mechanism starts a materially different minimal topology profile.
            if (
                shown.get("workflow_id") != "software-change"
                or shown.get("initial_input", {}).get("config_version") != "minimal-6"
                or shown.get("initial_input", {}).get("review_policies")
                == saved_profile.get("review_policies")
            ):
                raise JourneyFailure(
                    "stitched run did not use the same provider workflow with distinct review policies",
                    state=self.state,
                    event="stitched-start",
                )
            try:
                work_slot_journey.assert_catalog(
                    shown,
                    STITCHED_SLOT_IDS,
                    stdin_context_kinds=_review_stdin_kinds(STITCHED_SLOT_IDS),
                )
            except work_slot_journey.WorkSlotJourneyFailure as error:
                raise JourneyFailure(
                    str(error), state=self.state, event="stitched-start"
                ) from error
            routes = [
                (item.get("event"), item.get("target"))
                for item in shown.get("requestable_events", [])
                if isinstance(item, dict)
            ]
            if ("intent-ready", "design") not in routes:
                raise JourneyFailure(
                    f"stitched explore omitted intent-ready→design; got {routes}",
                    state=self.state,
                    event="stitched-start",
                )
            self._invoke_bound_slot(self.run_id, state="explore")
            for expected_state, event, target in STITCHED_HOPS:
                if self.state != expected_state:
                    raise JourneyFailure(
                        f"stitched hop expected {expected_state}, at {self.state}",
                        state=self.state,
                        event=event,
                    )
                if event == "implementation-ready":
                    self._create_checkpoint("implementation")
                if event == "validation-ready":
                    self._create_checkpoint("validation")
                self._expect_allow(event, target)
            self._pass_review("validation-review", "passed", "end")
            shown = self._assert_show("end", "stitched-terminal-show")
            # bookends:LE-39 — the minimal software-change idea is driven to a final public run state.
            if shown.get("lifecycle") != "final":
                raise JourneyFailure(
                    "stitched journey did not reach final lifecycle",
                    state=self.state,
                    event="passed",
                )
            if shown.get("requestable_events") != []:
                raise JourneyFailure(
                    "stitched final journey exposed requestable events",
                    state=self.state,
                    event="show",
                )
            print(
                "stitched software-change journey passed: empty review lists omitted, last-hop passed"
            )
        finally:
            self.run_id = saved_run_id
            self.artifact_root = saved_artifact_root
            self.profile_path = saved_profile_path
            self.profile = saved_profile
            self.state = saved_state
            self.work_slot_bindings = saved_bindings
            self.profile_source = saved_source

    @staticmethod
    def _assert_stitched_profile(profile: Dict[str, Any]) -> None:
        policies = profile.get("review_policies")
        if not isinstance(policies, dict):
            raise JourneyFailure("stitched profile review_policies must be an object")
        for omitted in (
            "intent-review",
            "design-review",
            "plan-review",
            "implementation-review",
        ):
            if policies.get(omitted) != []:
                raise JourneyFailure(
                    f"stitched profile must omit {omitted} with an empty list",
                    event="stitched-start",
                )
        validation = policies.get("validation-review")
        if not isinstance(validation, list) or not validation:
            raise JourneyFailure(
                "stitched profile must keep a nonempty validation-review list",
                event="stitched-start",
            )
        for gate, axes in policies.items():
            if "adversarial" in str(gate) and axes:
                raise JourneyFailure(
                    f"stitched profile must not enable {gate}",
                    event="stitched-start",
                )

    def _run_dummy_worker_proofs(self) -> None:
        """Prove heartbeat, capture isolation, preview fail-closed, and sandbox argv."""
        assert self.run_dir is not None
        assert self.profile_source is not None
        assert self.fixture_root is not None
        assert_worker_data_skill_and_root_policy()
        proof_root = self.run_dir / "dummy-worker-proofs"
        try:
            # bookends:LE-85 — shipped profiles leave driver-performed slots unbound.
            # bookends:LE-86 — the same public binding contract is exercised for this provider.
            work_slot_journey.prove_shipped_software_change_profiles(self.data_root)
            # bookends:LE-92 — proposal-only data is inert and only the driver ledger routes exact implementation tasks.
            # bookends:LE-102 — the public run-plan-graph command refuses missing prerequisites and summarizes the resulting tree.
            work_slot_journey.prove_graph_runner(
                provider=self.provider,
                work_dir=proof_root / "graph-runner",
            )
            work_slot_journey.prove_engine_standing_join(
                engine=self.engine,
                provider=self.provider,
                work_dir=proof_root / "engine-standing-join",
            )
            # bookends:LE-89 — review fan-out remains entered through invoke and frozen workers.
            # bookends:LE-90 — nested worker stdin/output and Dagu graph shape are asserted.
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
            # bookends:LE-91 — the public bound-worker scenario reads the frozen operating context from artifact_root in a fresh CLI process.
            work_slot_journey.prove_bound_fan_out_heartbeat(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "bound-fan-out-heartbeat",
            )
            # bookends:LE-84 — the public overrun scenario proves immediate show, retry admission, and capture inspection.
            work_slot_journey.prove_bound_fan_out_overrun(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "bound-fan-out-overrun",
            )
            # bookends:LE-79 — waiter completion and captured inner worker status are inspected.
            # bookends:LE-80 — the public worker path exercises stdin-exec without shell framing.
            work_slot_journey.prove_bound_contracted_fan_out_failure(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "bound-contracted-fan-out-failure",
            )
            # bookends:LE-90 — the public fan-out scenarios cover both legacy key-presence and additive full-schema worker contracts.
            # bookends:LE-93 — the public fan-out scenario preserves both bounded same-worker conformance attempts.
            work_slot_journey.prove_full_schema_retry(
                engine=self.engine,
                work_dir=proof_root / "full-schema-retry",
            )
            # bookends:LE-92 — selected retry output remains candidate data until the driver links and dispositions it.
            # bookends:LE-98 — every guarded mutation refuses until the current state was observed.
            work_slot_journey.prove_observation_before_mutation(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "observation-before-mutation",
            )
            # bookends:LE-99 — the selected-attempt journey exposes durable assignment identity, digest, path, and coverage gap.
            # bookends:LE-100 — the selected invocation's public show asserts deterministic change-report dimensions.
            # bookends:LE-105 — the public provider gate refuses content disagreement with selected bytes.
            work_slot_journey.prove_selected_attempt_ledger_linkage(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "selected-attempt-ledger",
            )
            # A38 — subset re-execution, explicit remainder carry, and a later checked transition.
            work_slot_journey.prove_subset_carry_checked(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "subset-carry-checked",
            )
            # bookends:LE-101 — invoke subset starts only the selected fan-out assignment.
            work_slot_journey.prove_invoke_subset(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "invoke-subset",
            )
            work_slot_journey.prove_stdin_exec(
                provider=self.provider,
                work_dir=proof_root / "stdin-exec",
            )
            work_slot_journey.prove_bound_graph_runner_heartbeat(
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                checkout_root=self.data_root,
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
            "overrun retry, bounded reviewer retry/exhaustion, selected-attempt linkage, observation guard, "
            "subset invoke, change report, carry acts, and content-agreement refusal, stdin-exec, graph working-directory cwd/marker proof, implementation finding routing, "
            "bound operating-context inspection, overlay-running invocation-progress, "
            "omitted vs set --max-active, "
            "progress-query overlay-untouched"
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
    schema_field: str = "output_schema",
) -> None:
    if worker.get("command") != pi_command:
        raise JourneyFailure(f"worker command {worker.get('command')!r} != {pi_command!r}")
    expected_schema = schema
    if schema_field == "full_output_schema":
        expected_schema = copy.deepcopy(schema)
        expected_schema["properties"]["axis"]["const"] = policy["id"]
        expected_schema["properties"]["author"]["const"] = {
            "name": roster_entry["author"],
            "kind": "agent",
        }
    if worker.get(schema_field) != expected_schema:
        raise JourneyFailure(
            f"worker {schema_field} {worker.get(schema_field)} != {expected_schema}"
        )
    other_schema_field = (
        "output_schema" if schema_field == "full_output_schema" else "full_output_schema"
    )
    if other_schema_field in worker:
        raise JourneyFailure(
            f"worker unexpectedly emitted both output contracts: {worker}"
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
    repository: Path,
    bindings: Dict[str, Any],
    workers: Sequence[Dict[str, Any]],
    *,
    schema_field: str = "output_schema",
) -> None:
    if len(workers) == 0:
        raise JourneyFailure("constructor preview input had no workers")
    full_preambles = []
    for worker in workers:
        if "preamble" not in worker or schema_field not in worker:
            raise JourneyFailure(f"preview input omitted preamble/schema: {worker}")
        required = (worker.get(schema_field) or {}).get("required")
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
        required = (preview_worker.get(schema_field) or {}).get("required")
        if required != ["axis", "author", "result", "findings"]:
            raise JourneyFailure(
                f"preview-bindings omitted {schema_field}.required: {preview_worker}"
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
    full_review_schema = {
        "type": "object",
        "additionalProperties": False,
        "required": ["axis", "author", "result", "findings"],
        "properties": {
            "axis": {"type": "string", "minLength": 1},
            "author": {
                "type": "object",
                "additionalProperties": False,
                "required": ["name", "kind"],
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "kind": {"type": "string", "enum": ["human", "agent", "script"]},
                },
            },
            "result": {"type": "string", "enum": ["pass", "fail"]},
            "findings": {"type": "string"},
        },
        "oneOf": [
            {"properties": {"result": {"const": "pass"}, "findings": {"const": ""}}},
            {"properties": {"result": {"const": "fail"}, "findings": {"type": "string", "minLength": 1}}},
        ],
    }

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
    for clause in (
        "operating_context",
        "extra mechanism, unlisted requirements, and hypothetical-future",
        "Confirmation consumes the durable finding-ledger history",
        "does not search again except for fix-introduced holes",
        "Bound workers do not use previously overlooked",
    ):
        if clause.lower() not in sc_preamble.lower():
            raise JourneyFailure(f"review-worker preamble omitted {clause!r}")
    high_rigor = repository / "crates/software-change-provider/data/configs/high-rigor.json"
    high_rigor_schema = _load_json(high_rigor).get("artifact_schemas", {}).get("intent.json", {})
    operating_context_schema = high_rigor_schema.get("properties", {}).get("operating_context", {})
    if (
        high_rigor_schema.get("additionalProperties") is not False
        or set(operating_context_schema.get("required", []))
        != {
            "operators",
            "environment",
            "threat_boundary",
            "accepted_risks",
            "outside_obligations",
        }
        or operating_context_schema.get("additionalProperties") is not False
    ):
        raise JourneyFailure(
            "high-rigor intent schema did not lock the closed operating_context contract"
        )
    engine_skill = (
        repository / "skills/using-loop-engine/SKILL.md"
    ).read_text(encoding="utf-8")
    contract_text = sc_skill + "\n" + engine_skill
    for contract_clause in (
        "full_output_schema",
        "attempts.json",
        "finding-ledger",
        "revise-implementation",
        "never stages, commits, branches, pushes",
    ):
        if contract_clause.lower() not in contract_text.lower():
            raise JourneyFailure(
                f"software-change skill omitted constructor/workflow contract {contract_clause!r}"
            )
    pd_preamble = pd_preamble_path.read_text(encoding="utf-8")
    research_preamble = research_preamble_path.read_text(encoding="utf-8")
    sc_schema = _load_json(sc_schema_path)
    pd_schema = _load_json(pd_schema_path)
    research_schema = _load_json(research_schema_path)
    if sc_schema != full_review_schema:
        raise JourneyFailure("software-change complete output schema bytes are unsupported")
    if pd_schema != schema_required or research_schema != schema_required:
        raise JourneyFailure("provider output_schema bytes do not require axis/author/result/findings")

    sc_jq = _extract_jq_after(sc_skill, '--slurpfile roster "$ROSTER" ')
    pd_jq = _extract_heredoc_jq(pd_skill)
    research_validate_jq = _extract_jq_after(research_skill, '--argjson roster "$ROSTER_JSON" ')
    research_jq = _extract_jq_after(
        research_skill, '--slurpfile output_schema "$OUTPUT_SCHEMA_PATH" '
    )
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
                schema_field="full_output_schema",
            )
        _assert_preview_visibility(
            repository, bindings, workers, schema_field="full_output_schema"
        )
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
                schema_field="full_output_schema",
            )
        _assert_preview_visibility(
            repository,
            plan_result["work_slot_bindings"],
            plan_workers,
            schema_field="full_output_schema",
        )
        _assert_hash_guard(plan_profile)

        intent_profile = root / "high-rigor-intent-review.json"
        shutil.copy2(high_rigor, intent_profile)
        intent_source = _load_json(intent_profile)
        intent_result = run_sc(intent_profile, "intent-review", roster_path)
        intent_bindings = intent_result.get("work_slot_bindings")
        if not isinstance(intent_bindings, dict) or "intent-review" not in intent_bindings:
            raise JourneyFailure("intent-review constructor omitted work_slot_bindings")
        for draft in (
            "intent-draft",
            "design-draft",
            "plan-draft",
            "implement",
            "validation-draft",
        ):
            if draft in intent_bindings:
                raise JourneyFailure(f"constructor emitted draft binding {draft}")
        if "intent-adversarial-review" in intent_bindings:
            raise JourneyFailure(
                "intent-review constructor mixed adversarial fan-out into the parent slot"
            )
        intent_workers = _fan_out_workers(
            intent_bindings["intent-review"], engine=dummy_engine
        )
        intent_expected = _policy_author_pairs(
            intent_source["review_policies"]["intent-review"], roster
        )
        if len(intent_workers) != len(intent_expected):
            raise JourneyFailure(
                f"intent-review worker count {len(intent_workers)} != {len(intent_expected)}"
            )
        for worker, (policy, entry) in zip(intent_workers, intent_expected):
            _assert_worker_assignment(
                worker,
                policy=policy,
                roster_entry=entry,
                base_preamble=sc_preamble,
                schema=sc_schema,
                pi_command=dummy_pi,
                fragments=(
                    "software-change",
                    "intent-review",
                    "artifact_root",
                    f"required_author_claim: {entry['author']}",
                ),
                schema_field="full_output_schema",
            )

        adv_profile = root / "high-rigor-design-adversarial.json"
        shutil.copy2(high_rigor, adv_profile)
        adv_source = _load_json(adv_profile)
        adv_result = run_sc(adv_profile, "design-adversarial-review", roster_path)
        adv_bindings = adv_result.get("work_slot_bindings")
        if not isinstance(adv_bindings, dict) or "design-adversarial-review" not in adv_bindings:
            raise JourneyFailure(
                "design-adversarial-review constructor omitted work_slot_bindings"
            )
        if "design-review" in adv_bindings:
            raise JourneyFailure(
                "adversarial constructor mixed parent fan-out into the adversarial slot"
            )
        adv_workers = _fan_out_workers(
            adv_bindings["design-adversarial-review"], engine=dummy_engine
        )
        adv_expected = _policy_author_pairs(
            adv_source["review_policies"]["design-adversarial-review"], roster
        )
        if len(adv_workers) != len(adv_expected):
            raise JourneyFailure(
                f"design-adversarial-review worker count {len(adv_workers)} != {len(adv_expected)}"
            )
        for worker, (policy, entry) in zip(adv_workers, adv_expected):
            if entry["author"] != roster[0]["author"]:
                raise JourneyFailure(
                    "adversarial constructor required a second roster or disjoint author"
                )
            _assert_worker_assignment(
                worker,
                policy=policy,
                roster_entry=entry,
                base_preamble=sc_preamble,
                schema=sc_schema,
                pi_command=dummy_pi,
                fragments=(
                    "software-change",
                    "design-adversarial-review",
                    "artifact_root",
                ),
                schema_field="full_output_schema",
            )

        for draft in (
            "intent-draft",
            "design-draft",
            "plan-draft",
            "implement",
            "validation-draft",
        ):
            draft_profile = root / f"sc-draft-{draft}.json"
            shutil.copy2(high_rigor, draft_profile)
            _expect_constructor_closed(
                lambda draft=draft, draft_profile=draft_profile: run_sc(
                    draft_profile, draft, roster_path
                ),
                needle="constructor does not emit draft bindings",
                context=f"software-change draft slot {draft}",
            )

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
            needle="unsupported review slot",
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
    provider_policy = (repository / "crates/software-change-provider/AGENTS.md").read_text(
        encoding="utf-8"
    )
    policy_fragments = (
        "Fan-out spawn/capture/conformance mechanics belong to the engine.",
        "Providers/callers own role framing and output content",
        "Reviewers produce judgments only.",
        "Drivers run deterministic checks, `show`, capture triage, `append`, `event`, and progression.",
        "Exit 0 alone does not establish deliverable validity.",
        "overrun re-show and zero-axis review-binding rules",
        "[skills/using-loop-engine/SKILL.md](skills/using-loop-engine/SKILL.md)",
        "[docs/agent-usage.md](docs/agent-usage.md)",
        "before the commit introducing `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, that PRD is authoritative.",
        "This AGENTS summary is subordinate and referential, not a second product policy",
        "observed ordinary-use failure and why a smaller mechanism using existing durable state, history, capture, or driver judgment is insufficient",
        "Keep driver-authored metadata small, trust explicit materiality and carry declarations except for cheap mechanical identity mismatches",
        "prefer the narrowest honest correction, and preserve rich engine-generated history",
        "[`docs/PRD.md`](docs/PRD.md) LE-107",
    )
    provider_policy_fragments = (
        "before the commit introducing engine `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, it is authoritative.",
        "subordinate to that engine PRD and this provider PRD, and is referential rather than a second authority",
        "Apply LE-107's observed-ordinary-failure/smaller-mechanism burden",
        "retain R8 and R13 freshness and subject/identity checks, but do not repeat mechanically available invocation, attempt, digest, path, or coverage facts in driver-authored records",
        "Preserve R16's independent-author aggregation and visible verdict history",
        "R21's retained review, materiality, triage, source-visibility, and no-waiver rules remain",
        "stable-reference and carry runtime redesign is separate work",
    )
    # bookends:LE-107 — the public self-test checks referential root/provider operational summaries against the PRD authority boundary.
    for label, text, fragments in (
        ("root", policy, policy_fragments),
        ("software-change provider", provider_policy, provider_policy_fragments),
    ):
        for fragment in fragments:
            if fragment not in text:
                raise JourneyFailure(f"{label} AGENTS.md omitted policy fragment {fragment!r}")
        lowered = text.lower()
        for reversal in (
            "agents.md is authoritative",
            "agents.md is the requirement authority",
            "this summary is authoritative",
            "this summary is a product policy",
        ):
            if reversal in lowered:
                raise JourneyFailure(f"{label} AGENTS.md reverses PRD authority with {reversal!r}")
    assert_operator_contract_surfaces()
    assert_focused_boundary_scenarios()
    print("worker-data skill/root policy assertions passed")


def assert_operator_contract_surfaces() -> None:
    """Assert the existing public procedure exposes the accepted operator contract."""
    repository = Path(__file__).resolve().parent.parent
    root_policy = (repository / "AGENTS.md").read_text(encoding="utf-8")
    engine_skill = (repository / "skills/using-loop-engine/SKILL.md").read_text(
        encoding="utf-8"
    )
    provider_skill = (
        repository / "crates/software-change-provider/skills/using-software-change-provider/SKILL.md"
    ).read_text(encoding="utf-8")
    protocol = (
        repository / "crates/software-change-provider/data/reviewer-protocol.md"
    ).read_text(encoding="utf-8")
    contract = "\n".join((root_policy, engine_skill, provider_skill, protocol))
    for clause in (
        "Exact profile and external-fleet preflight",
        "config_version",
        "live review states",
        "normalized `required_authors`",
        "Bookends enabled/disabled state",
        "PROFILE_SHA256",
        "rehash that same file immediately",
        "two separate authorities",
        "role-to-model manifest",
        "separate owner confirmation",
        "pi --list-models",
        "preserve launch evidence",
        "never fall back",
        "validation-report-only",
        "revise-implementation",
        "revise-plan",
        "revise-design",
        "revise-intent",
        "captured ad hoc repair",
        "override-carry",
        "challenge review",
        "meaningfully falsify",
        "current supplied evidence",
        "concrete consequence",
    ):
        if clause.lower() not in contract.lower():
            raise JourneyFailure(f"operator procedure omitted {clause!r}")

    # Shipped profile bytes are the immutable source of profile-derived facts;
    # comparing against HEAD catches accidental edits without hard-coding any
    # run-specific model or profile values into the journey.
    for profile in sorted((repository / "crates/software-change-provider/data/configs").glob("*.json")):
        relative = profile.relative_to(repository).as_posix()
        try:
            committed = subprocess.check_output(
                ["git", "show", f"HEAD:{relative}"], cwd=repository
            )
        except (OSError, subprocess.CalledProcessError) as error:
            raise JourneyFailure(f"could not read committed profile bytes for {relative}: {error}") from error
        if profile.read_bytes() != committed:
            raise JourneyFailure(f"shipped profile/config bytes changed: {relative}")

    workflow_source = (
        repository / "crates/software-change-provider/src/workflow.rs"
    ).read_text(encoding="utf-8")
    for identifier in (
        "intent-adversarial-review",
        "design-adversarial-review",
        "plan-adversarial-review",
        "implementation-adversarial-review",
        "validation-adversarial-review",
    ):
        if identifier not in workflow_source:
            raise JourneyFailure(f"machine review identifier was removed: {identifier}")
    for clause in (
        "challenge review",
        "meaningfully falsify",
        "current supplied evidence",
        "concrete consequence",
    ):
        if clause not in workflow_source:
            raise JourneyFailure(f"runtime challenge wording omitted {clause!r}")



def assert_focused_boundary_scenarios() -> None:
    """Keep the focused citations beside their public assertions."""
    repository = Path(__file__).resolve().parent.parent
    fixture_root = repository / FIXTURE_SUBPATH
    scenario_path = repository / COMPANION_SCENARIO_SUBPATH
    scenario_source = scenario_path.read_text(encoding="utf-8")
    plan = _load_json(fixture_root / "plan-good.json")
    good_validation = _load_json(fixture_root / "validation-report-good.json")
    assert_semantic_outcome_proof_contract(plan, good_validation, scenario_source)
    for invalid_validation in (
        _load_json(fixture_root / "validation-report-defective.json"),
        {"outcome": "done", "requirements": [{"requirement": "LE-97", "proof": "LE-97"}]},
    ):
        try:
            assert_semantic_outcome_proof_contract(plan, invalid_validation, scenario_source)
        except JourneyFailure:
            pass
        else:
            raise JourneyFailure(
                "activity/token-only validation proof unexpectedly satisfied LE-97"
            )

    source = Path(__file__).read_text(encoding="utf-8")
    citation_prefix = "".join(("bookends", ":LE-"))
    probe_start = source.index("    def _probe_startup")
    probe_end = source.find("    def ", probe_start + len("    def _probe_startup"))
    probe = source[probe_start:probe_end if probe_end >= 0 else len(source)]
    if any(f"{citation_prefix}{number} —" in probe for number in (2, 11, 12)):
        raise JourneyFailure("LE-2/LE-11/LE-12 citations returned to the ordinary describe probe")

    full_start = source.index("    def _run_full_source")
    full_end = source.find("    def ", full_start + len("    def _run_full_source"))
    full_source = source[full_start:full_end if full_end >= 0 else len(source)]
    if "self._run_engine_boundary_scenarios()" not in full_source:
        raise JourneyFailure("focused engine boundary scenarios are not in the full source journey")

    focused_functions = {
        2: "_run_le2_topology_scenario",
        11: "_run_le11_frozen_topology_scenario",
        12: "_run_le12_unsupported_action_scenario",
        13: "_run_le13_final_state_outgoing_scenario",
        14: "_run_le14_initially_final_scenario",
        15: "_run_le15_terminal_mutation_scenario",
    }
    for number, function_name in focused_functions.items():
        token = f"{citation_prefix}{number} —"
        if source.count(token) != 1:
            raise JourneyFailure(f"{token} must have exactly one public scenario citation")
        function_start = source.index(f"    def {function_name}")
        function_end = source.find("    def ", function_start + len(f"    def {function_name}"))
        function_source = source[function_start:function_end if function_end >= 0 else len(source)]
        if token not in function_source:
            raise JourneyFailure(f"{token} is not beside {function_name}'s assertions")

    for marker in (
        "LE-2 topology scenarios passed:",
        "LE-11 frozen-run scenario passed:",
        "LE-12 unsupported-action scenario passed:",
        "LE-13 final-state scenario passed:",
        "LE-14 initially-final scenario passed:",
        "LE-15 terminal-mutation scenario passed:",
    ):
        if marker not in source:
            raise JourneyFailure(f"focused public scenario marker missing: {marker}")

    requirement_scenarios = {
        90: (
            "_run_dummy_worker_proofs",
            ("prove_fan_out", "prove_full_schema_retry"),
        ),
        91: (
            "_run_full_source",
            ("operating_context", "operating-context-show"),
        ),
        92: (
            "_run_dummy_worker_proofs",
            ("prove_graph_runner", "prove_selected_attempt_ledger_linkage"),
        ),
        93: (
            "_run_dummy_worker_proofs",
            ("prove_full_schema_retry", "bound-contracted-fan-out-failure"),
        ),
        94: ("_run_checkpoint_case", ("report-only", "checkpoint")),
        95: (
            "_run_checkpoint_case",
            ("expected_implementation", "expected_validation"),
        ),
        96: (
            "_run_checkpoint_scenarios",
            ("revise-implementation", "current-tree final proof"),
        ),
        97: (
            "_run_full_source",
            (
                "assert_semantic_outcome_proof_contract",
                "executable CLI assertions",
                "token-only or activity-only proof is refused",
            ),
        ),
    }
    for number, (function_name, assertions) in requirement_scenarios.items():
        token = f"{citation_prefix}{number} —"
        if token not in source:
            raise JourneyFailure(f"{token} has no public source-journey citation")
        function_start = source.index(f"    def {function_name}")
        function_end = source.find("\n    def ", function_start + len(f"    def {function_name}"))
        function_source = source[function_start:function_end + 1 if function_end >= 0 else len(source)]
        if token not in function_source:
            raise JourneyFailure(f"{token} is not beside {function_name}'s assertions")
        for assertion in assertions:
            if assertion not in function_source:
                raise JourneyFailure(
                    f"{token} scenario omitted observable assertion marker {assertion!r}"
                )

    print("focused citation scenarios remain bound to public assertions")


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
        if not raw_argv or raw_argv == ["--self-test"]:
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
