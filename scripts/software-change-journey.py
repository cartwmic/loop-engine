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

``--self-test`` executes the software-change setup utility and the two
remaining provider skill constructors against shipped profiles (software-change
high-rigor policy/stage setup, policy-document semantic policies/target/mode,
research verify and synthesize), asserts root AGENTS rules, and prints
``worker-data skill/root policy assertions passed`` only after all pass. Source full mode binds deterministic stdin-capturing workers that emit
conforming JSON or exit-0 refusal text; after overlay failure, persisted
summary/captures, and the compact one-key ``artifact_root`` stdin proof, it
prints ``contracted fan-out failure``. It also drives the real engine/provider
boundary cases for structural workflow rejection, final-state topology,
an initially-final run, terminal mutation rejection, a changed provider
``describe``, and an unavailable stored evaluation. The full source run also
executes the named Package 7b ``review-candidates`` pipe proof before it starts
a second run from shipped minimal.json and walks the stitched hops (empty
review lists omitted, last-hop ``passed`` on the live validation review).
Finally it runs the reduced criterion-spine scenarios: overlay-off proves AC-N
without PRD metadata, while overlay-on proves one disposition per criterion,
candidate blocking, and the non-waiver of not-applicable. The source tail also
runs isolated v11 reconciliation cases through the real provider/engine path,
inspecting document bytes, state ordering, Bookends mode, pending owner status,
and requirement-coverage contrast fixtures.
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
from typing import Any, Dict, List, Mapping, Optional, Sequence

import work_slot_journey

PROFILE_SUBPATH = Path("crates/software-change-provider/data/configs/high-rigor.json")
STITCHED_PROFILE_SUBPATH = Path(
    "crates/software-change-provider/data/configs/minimal.json"
)
BOOKENDS_CANDIDATE_ID = "LE-9001"
BOOKENDS_SCENARIO_RUN_ID = "bookends-enabled-journey"
BOOKENDS_NOT_APPLICABLE_RUN_ID = "bookends-not-applicable-journey"
BOOKENDS_SCENARIO_PROFILE = Path(
    "crates/software-change-provider/data/configs/high-rigor.json"
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
RECONCILIATION_SUBJECT = "reconciliation.json"
# Durable integrated citation spelling is exactly `bookends:LE-142`.
RECONCILIATION_REQUIREMENT_ID = "LE-142"
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
    "reconciliation-draft",
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
    "reconciliation-draft",
    "validation-draft",
    "validation-review",
)
STITCHED_HOPS = (
    ("explore", "intent-ready", "design"),
    ("design", "design-ready", "plan"),
    ("plan", "plan-ready", "implement"),
    ("implement", "implementation-ready", "reconciliation"),
    ("reconciliation", "reconciliation-ready", "validation"),
    ("validation", "validation-ready", "validation-review"),
)
CHECKPOINT_MUTATIONS = ("head", "add", "delete", "rename", "status", "type", "bytes")
COMPANION_SCENARIO_SUBPATH = Path(
    "crates/software-change-provider/data/calibration/companions/"
    "fictional-repo/scripts/production-journey.py"
)
RECOVERY_COMPLETION_MARKERS = {
    "dispositions": "recovery dispositions scenario passed:",
    "steering": "recovery steering passed:",
    "execution-controls": "recovery execution-controls scenario passed",
    "cancellation": "recovery cancellation scenario passed",
    "backtracking": "recovery backtracking scenario passed:",
    "override": "recovery override scenario passed:",
    "batched-review": "recovery batched-review scenario passed:",
    "criteria": "recovery criterion proof passed:",
    "composed-recovery": "composed recovery journey passed:",
}
ENGINE_BOUNDARY_PROOF = [
    "LE-2 malformed workflow rejected before run creation",
    "LE-2 structurally valid cyclic production topology accepted",
    "LE-13 final-state outgoing transition rejected before run creation",
    "LE-14 initially-final run created with final lifecycle",
    "LE-15 terminal append/event/terminate rejected without history change",
    "LE-11 show retained frozen topology and instructions after describe change",
    "LE-12 unsupported stored action and provider failure failed without state or history advancement",
]
PACKAGE_7B_PROOF = [
    "selected retry output is ready and exposes normalized result/findings",
    "exhausted assignment is a non-judgmental diagnostic",
    "raw attempts and captures remain unchanged",
    "worker attempt sentinels remain unchanged across repeated inspection",
    "repeated candidate inspection is byte-identical",
    "distinct durable invocations remain ordered without deduplication",
    "candidate inspection is inert before driver records",
    "foreign workflow identity is rejected before projection",
    "driver triage accepts ready and rejects exhausted before ordinary append",
    "driver-authored review-evidence and finding-ledger permit checked progression",
]


def _review_stdin_kinds(slot_ids: Sequence[str]) -> dict[str, list[str]]:
    return {
        slot_id: ["finding-ledger", "review-evidence", "evidence-applicability",
                  "user-steering", "steering-incorporation"] + (
                      ["command-evidence", "validation-command", "criterion-verdict", "goal-verdict", "criterion-revalidation"]
                      if slot_id == "implement" or slot_id.startswith("validation") else [])
        for slot_id in slot_ids
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
    "provider constructors omit unused extension pairs and validate supplied paths",
    "PATH stub pi default argv --print --no-skills --no-extensions without --no-context-files or --tools",
    "bound fan-out show heartbeat overlay_meaning elapsed remaining capture_dir inner_workers",
    "contracted fan-out exit-0 conformance summary and failed-overlay capture persistence",
    "same-reviewer invalid-then-valid retry and invalid-twice exhaustion preserve raw attempts",
    "selected retry attempt links into a driver ledger before review evidence",
    "show exposes durable change report and provider content-agreement refusal",
    "observation-before-mutation refuses all four guarded acts",
    "invoke subset starts only selected assignments",
    "stable evidence references and one applicability declaration preserve explicit provenance",
    "public negatives reject missing/cross-run invocation or assignment and stale subject/revision/checkpoint references",
    "public negatives reject missing evidence, unknown policy/task, and missing finding references before progression",
    "plan-graph subset refuses missing prerequisites and summarizes resulting tree",
    "overrun overlay show/retry and distinct captures",
    "stdin-exec sidecar/propagate, spawn failure, and session directory",
    "bound run-plan-graph inner workers in task order plus capture isolation",
    "graph-level run-plan-graph working_dir reaches every task and summarizer",
    "symlink-selected checkout cwd receipts are filesystem-equivalent and see .git",
    "dummy plan-graph summarizer writes implementation-report.json; ordinary dummy tasks do not",
    "implementation ledger routing enriches exact tasks, keeps proposal inert, and preserves task stdin envelope",
    "bound ad-hoc repair exact-input preflight refuses malformed, stale, routed, and non-current findings before Dagu",
    "ad-hoc repair no-context and frozen-task-flag refusals preserve proof",
    "ad-hoc repair captures one generic assignment with exact findings and pre/post report/state identities",
    "ad-hoc repair changes Git, refreshes implementation proof, and leaves plan-task-results untouched",
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
        self.args = args
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
        self.package_7b_proof: List[str] = []
        self.engine_boundary_proof: List[str] = []
        self.bookends_proof: Optional[Path] = None
        self.criterion_overlay_proof: Dict[str, Path] = {}
        self.reconciliation_proof: Optional[Path] = None
        self._operational_ux_outcomes: Dict[str, Any] = {}
        self.command_cwd: Optional[Path] = None
        self.command_env: Dict[str, str] = {}
        self.run_id = "journey-production-run"
        self.stitched_run_id: Optional[str] = None
        self.state = "not-started"

    def preflight(self) -> None:
        """Reject bad inputs before creating or mutating any run state."""
        if getattr(self.args, "jobs", 2) < 1 or getattr(self.args, "job_timeout", 1200) <= 0:
            raise JourneyFailure("jobs and job-timeout must be positive")
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

    def _run_operational_ux_cases(
        self, *, global_jobs: Optional[List[Dict[str, Any]]] = None
    ) -> None:
        """Run independent operational cases under the existing bounded pool."""
        assert self.run_dir is not None
        binary_dir = self.engine.parent
        for name in ("loop-engine", "software-change", "policy-document", "research", "bookends-check"):
            binary = binary_dir / name
            if not binary.is_file() or not os.access(binary, os.X_OK):
                raise JourneyFailure(f"operational UX binary missing or not executable: {binary}")
        if self.provider != binary_dir / "software-change":
            raise JourneyFailure("operational UX requires the selected provider beside the engine")
        # bookends:LE-143 — monitor_case checks changed source-backed update
        # packets and unchanged-poll silence; guidance_case checks the assistant's
        # posting duty. Actual conversation delivery remains external evidence.
        cases = (
            "monitor",  # bookends:LE-120 — monitor_case: live notifications, unknown judgments, no cancellation.
            "capture",  # bookends:LE-121 — capture_case: real exit 7, immutable receipts, stale-resume refusal.
            "summary",  # bookends:LE-122 — summary_case: selected sources, retained failures and budget exhaustion.
            "guidance",  # bookends:LE-123 — guidance_case: constructors and emitted Git checkpoint guidance, not human approval.
            "delivery",  # bookends:LE-124 — delivery_case: pending/matched pointers and content mismatch refusal.
            # bookends:LE-125 — bookends_case: full/shallow continuity and missing-parent refusal.
            # bookends:LE-126 — bookends_case: durable bypass receipts and recording-failure refusal.
            "bookends",
        )
        output = self.run_dir / "operational-ux"
        output.mkdir()
        script = Path(__file__).with_name("operational-ux-journey.py")
        if global_jobs is None and getattr(self.args, "jobs", 2) == 1:
            for case in cases:
                argv = [
                    sys.executable, str(script), "--case", case,
                    "--binary-dir", str(binary_dir),
                    # These cases do not read released_root. This existing source
                    # directory satisfies only the CLI parser, not release validation.
                    "--released-root", str(self.data_root), "--output-root", str(output),
                ]
                (output / f"{case}.argv.json").write_text(json.dumps(argv) + "\n")
                case_environment = os.environ.copy()
                case_environment["SOFTWARE_CHANGE_JOURNEY_ENGINE"] = str(self.engine)
                case_environment["SOFTWARE_CHANGE_JOURNEY_PROVIDER"] = str(self.provider)
                with (output / f"{case}.stdout").open("wb") as stdout, (output / f"{case}.stderr").open("wb") as stderr:
                    result = subprocess.run(
                        argv, stdout=stdout, stderr=stderr, env=case_environment, check=False
                    )
                (output / f"{case}.exit.json").write_text(json.dumps({"exit_code": result.returncode}) + "\n")
                if result.returncode != 0:
                    raise JourneyFailure(f"operational UX {case} failed ({result.returncode}); captures: {output}")
                print(f"operational UX {case} passed; captures: {output}")
            return

        import proof_pool

        env_binary = shutil.which("env") or "/usr/bin/env"
        outcomes = {}
        # guidance invokes the journey self-test, which owns its own proof pool.
        # Keep that genuinely nested workflow outside this pool rather than
        # multiplying budgets or weakening the existing self-test.
        serial_case = "guidance"
        serial_output = output / serial_case
        serial_argv = [
            env_binary,
            f"SOFTWARE_CHANGE_JOURNEY_ENGINE={self.engine}",
            f"SOFTWARE_CHANGE_JOURNEY_PROVIDER={self.provider}",
            "PYTHONUNBUFFERED=1",
            sys.executable,
            str(script),
            "--case", serial_case,
            "--binary-dir", str(binary_dir),
            "--released-root", str(self.data_root),
            "--output-root", str(serial_output),
        ]
        serial_environment = os.environ.copy()
        serial_environment["SOFTWARE_CHANGE_JOURNEY_ENGINE"] = str(self.engine)
        serial_environment["SOFTWARE_CHANGE_JOURNEY_PROVIDER"] = str(self.provider)
        serial_environment["PYTHONUNBUFFERED"] = "1"
        serial = subprocess.run(
            serial_argv,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=serial_environment,
            check=False,
        )
        (output / "guidance.argv.json").write_text(json.dumps(serial_argv) + "\n", encoding="utf-8")
        (output / "guidance.stdout").write_bytes(serial.stdout)
        (output / "guidance.stderr").write_bytes(serial.stderr)
        if serial.returncode != 0:
            raise JourneyFailure(
                f"operational UX guidance failed; inspect {output}",
                state="end",
                event="operational-ux",
            )
        try:
            guidance_outcome = json.loads(
                [line for line in serial.stdout.decode("utf-8", "replace").splitlines() if line][-1]
            )
        except (IndexError, json.JSONDecodeError) as error:
            raise JourneyFailure(
                f"operational UX guidance omitted its public outcome; inspect {output}",
                state="end",
                event="operational-ux",
            ) from error
        if guidance_outcome.get("case") != serial_case or guidance_outcome.get("status") != "passed":
            raise JourneyFailure(
                f"operational UX guidance reported an invalid outcome: {guidance_outcome}",
                state="end",
                event="operational-ux",
            )
        outcomes[serial_case] = guidance_outcome
        self._operational_ux_outcomes = outcomes
        print(f"operational UX guidance passed; captures: {guidance_outcome.get('artifact_root')}")

        parallel_cases = tuple(case for case in cases if case != serial_case)
        if global_jobs is not None:
            # The five non-guidance cases all touch the maintained checkout's
            # Git boundary.  Keep their measured safe cap of two by making two
            # sequential batches, while each batch still occupies one slot in
            # the caller's single global proof pool.
            assert self.run_dir is not None
            output = self.run_dir / "operational-ux"
            for index, batch in enumerate((parallel_cases[:3], parallel_cases[3:]), start=1):
                self._append_global_pool_job(
                    global_jobs,
                    name=f"operational-batch-{index}",
                    kind="operational-batch",
                    root=output / f"batch-{index}",
                    cases=list(batch),
                    binary_dir=str(binary_dir),
                    released_root=str(self.data_root),
                    script=str(script),
                )
            return

        jobs = []
        for case in parallel_cases:
            case_output = output / case
            argv = [
                env_binary,
                f"SOFTWARE_CHANGE_JOURNEY_ENGINE={self.engine}",
                f"SOFTWARE_CHANGE_JOURNEY_PROVIDER={self.provider}",
                "PYTHONUNBUFFERED=1",
                sys.executable,
                str(script),
                "--case", case,
                "--binary-dir", str(binary_dir),
                "--released-root", str(self.data_root),
                "--output-root", str(case_output),
            ]
            jobs.append({"name": case, "command": argv})
        # These cases all exercise the maintained checkout's Git boundary and
        # some also run their own bounded process fixtures. Two concurrent
        # cases is the measured safe cap; the caller's global budget remains an
        # upper bound and jobs=1 still takes the serial path above.
        operational_limit = min(self.args.jobs, 2)
        report = proof_pool.run(
            jobs,
            root=self.run_dir / "operational-ux-pool",
            limit=operational_limit,
            timeout=self.args.job_timeout,
        )
        report_path = self.run_dir / "operational-ux-pool-report.json"
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        expected_peak = min(operational_limit, len(parallel_cases))
        if report.get("status") != "passed" or report.get("peak_jobs") != expected_peak:
            raise JourneyFailure(
                f"operational UX pool failed; inspect {report_path}",
                state="end",
                event="operational-ux",
            )
        for case, row in zip(parallel_cases, report.get("jobs", [])):
            if row.get("name") != case or row.get("status") != "passed" or row.get("exit_code") != 0:
                raise JourneyFailure(
                    f"operational UX {case} did not complete; inspect {report_path}",
                    state="end",
                    event="operational-ux",
                )
            outcome = None
            for line in reversed(Path(row["stdout"]).read_text(encoding="utf-8", errors="replace").splitlines()):
                if not line:
                    continue
                try:
                    candidate = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if isinstance(candidate, dict) and candidate.get("case") == case:
                    outcome = candidate
                    break
            if not isinstance(outcome, dict) or outcome.get("status") != "passed":
                raise JourneyFailure(
                    f"operational UX {case} omitted its public outcome; inspect {report_path}",
                    state="end",
                    event="operational-ux",
                )
            if outcome.get("case") != case or outcome.get("status") != "passed":
                raise JourneyFailure(
                    f"operational UX {case} reported an invalid outcome: {outcome}",
                    state="end",
                    event="operational-ux",
                )
            outcomes[case] = outcome
            print(f"operational UX {case} passed; captures: {outcome.get('artifact_root')}")
        self._operational_ux_outcomes = outcomes
        (self.run_dir / "operational-ux-results.json").write_text(
            json.dumps(outcomes, indent=2) + "\n", encoding="utf-8"
        )

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

        if self.mode == "source" and self.depth == "full":
            self._prepare_real_repository()
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
            # Independent tail fixtures are scheduled only after the primary
            # same-run path has reached its terminal proof.  Their databases,
            # repositories, and artifact roots are isolated; the existing
            # proof_pool supplies the one effective budget for the whole tail.
            self._run_global_tail_proof()
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
                    "criterion_overlay_proof": {
                        name: str(path)
                        for name, path in self.criterion_overlay_proof.items()
                    },
                    "reconciliation_proof": (
                        str(self.reconciliation_proof)
                        if self.reconciliation_proof is not None
                        else None
                    ),
                    "successor_route_cases": successor_route_cases,
                    "work_slot_proof": WORK_SLOT_PROOF,
                    "dummy_worker_proof": self.dummy_worker_proof,
                    "package_7b_proof": self.package_7b_proof,
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

    def run_compact_worker_fixture(self) -> Path:
        """Drive one fresh setup/start/invoke compact-delivery fixture.

        This is intentionally a small public-path fixture, separate from the
        synthetic full journey.  Setup owns the profile and the engine owns
        the invocation/capture records; this method only prepares the caller
        inputs, drives the public commands, and retains read-only evidence.
        """
        if self.mode != "source" or self.args.compact_worker_fixture not in {
            "draft",
            "review",
            "negative-empty",
        }:
            raise JourneyFailure("compact worker fixtures require source mode and a supported fixture")
        # The compact fixture owns its supplied proof directory.  Unlike the
        # ordinary journey parent, its canonical producer command is expected
        # to work from a fresh absent leaf below the run proof root.
        if not self.work_root.exists():
            self.work_root.parent.mkdir(parents=True, exist_ok=True)
        self.preflight()
        if self.work_root.exists() and any(self.work_root.iterdir()):
            raise JourneyFailure(f"compact worker work-root is not fresh: {self.work_root}")
        self.work_root.mkdir(parents=True, exist_ok=True)
        fixture = self.args.compact_worker_fixture
        if fixture != "negative-empty" and not (
            self.args.worker_model and self.args.worker_thinking and self.args.worker_tools
        ):
            raise JourneyFailure(
                "positive compact worker fixtures require --worker-model, --worker-thinking, and --worker-tools"
            )
        assert self.profile_source is not None
        assert self.fixture_root is not None
        self.run_dir = self.work_root
        self.database = self.work_root / "loop.sqlite"
        self.provider_config = self.work_root / "providers.toml"
        self.profile_path = self.work_root / "setup-profile.json"
        self.artifact_root = self.work_root / "artifacts"
        self.artifact_root.mkdir()
        self.profile = self._read_json(self.profile_source, "compact source profile")
        self._validate_scenario_fixtures()
        for subject, fixture_name in SUBJECTS.items():
            shutil.copy2(self.fixture_root / fixture_name, self.artifact_root / subject)

        pi_command = shutil.which("pi")
        if fixture != "negative-empty" and not pi_command:
            raise JourneyFailure("compact positive fixture requires the pi executable on PATH")
        tools = self.args.worker_tools or "read,grep,find,ls"
        worker_args = [
            "--print",
            "--no-skills",
            "--no-extensions",
            "--tools",
            tools,
            "--model",
            self.args.worker_model or "",
            "--thinking",
            self.args.worker_thinking or "",
        ]

        seed_worker = self.work_root / "seed-intent-worker.py"
        seed_worker.write_text(
            "import json, pathlib, shutil, sys\n"
            "raw = sys.stdin.read()\n"
            "location = raw.split('---\\n\\n', 1)[0].splitlines()[-1]\n"
            "root = pathlib.Path(json.loads(location)['artifact_root'])\n"
            "shutil.copyfile(sys.argv[1], root / 'intent.json')\n"
            "print(json.dumps({'status':'completed','summary':'seed intent prepared'}), flush=True)\n",
            encoding="utf-8",
        )
        reject_worker = self.work_root / "reject-context-window-worker.py"
        reject_worker.write_text(
            "import sys\n"
            "sys.stdin.buffer.read()\n"
            "sys.stderr.write('context window rejected: intentionally empty worker delivery\\n')\n"
            "sys.stderr.flush()\n",
            encoding="utf-8",
        )
        draft_schema = {
            "type": "object",
            "additionalProperties": False,
            "required": ["status", "summary"],
            "properties": {
                "status": {"const": "completed"},
                "summary": {"type": "string", "minLength": 1},
            },
        }
        if fixture == "draft":
            draft_inner = {
                "command": pi_command,
                "args": worker_args,
                "preamble": (
                    "You are the intent-drafting worker. Read the compact location JSON and work only beneath "
                    "artifact_root. Read the existing intent.json and frozen operating context. Preserve the "
                    "closed intent schema and its meaningful outcomes; make a small valid improvement if needed. "
                    "Do not run workflow commands. End with exactly one JSON object containing status=completed "
                    "and a nonempty summary."
                ),
                "full_output_schema": draft_schema,
            }
        elif fixture == "review":
            draft_inner = {
                "command": pi_command,
                "args": worker_args,
                "preamble": (
                    "You are the intent-drafting worker. Read the compact location JSON and work only beneath "
                    "artifact_root. Preserve the existing valid intent schema and operating context. End with "
                    "exactly one JSON object containing status=completed and a nonempty summary."
                ),
                "full_output_schema": draft_schema,
            }
        else:
            draft_inner = {
                "command": sys.executable,
                "args": [str(reject_worker)],
                "full_output_schema": draft_schema,
            }
        draft_binding = {
            "command": str(self.engine),
            "args": [
                "fan-out",
                "--max-active",
                "1",
                "--worker",
                json.dumps(draft_inner, separators=(",", ":")),
            ],
        }
        draft_worker_path = self.work_root / "draft-worker.json"
        _write_json(draft_worker_path, draft_binding)

        if fixture == "negative-empty":
            review_command = sys.executable
            review_args = [str(seed_worker), str(self.fixture_root / SUBJECTS["intent.json"])]
        else:
            review_command = pi_command
            review_args = worker_args
        roster = [
            {"author": "sol-reviewer-a", "command": review_command, "args": review_args},
            {"author": "sol-reviewer-b", "command": review_command, "args": review_args},
        ]
        roster_path = self.work_root / "roster.json"
        _write_json(roster_path, roster)
        setup_command = [
            str(self.provider),
            "setup",
            "--rigor",
            "high",
            "--roster",
            str(roster_path),
            "--draft-worker",
            str(draft_worker_path),
            "--engine",
            str(self.engine),
            "--provider",
            str(self.provider),
            "--output",
            str(self.profile_path),
        ]
        setup = subprocess.run(
            setup_command,
            cwd=str(self.data_root),
            text=True,
            capture_output=True,
            check=False,
        )
        if setup.returncode != 0:
            raise JourneyFailure(
                f"compact setup failed: {setup.stderr.strip() or setup.stdout.strip()}"
            )
        try:
            setup_report = json.loads(setup.stdout)
        except json.JSONDecodeError as error:
            raise JourneyFailure(f"compact setup did not return JSON: {setup.stdout}") from error
        _write_json(self.work_root / "setup-report.json", setup_report)
        setup_profile = self._read_json(self.profile_path, "generated compact setup profile")
        if setup_profile.get("config_version") != "high-rigor-11":
            raise JourneyFailure("compact setup did not preserve the high-rigor-11 profile version")
        if setup_profile.get("criterion_policy") != {"required_authors": 2, "goal_required_authors": 2}:
            raise JourneyFailure("compact setup changed the high-rigor criterion/goal floors")
        for gate in ("intent-review", "intent-adversarial-review"):
            axes = setup_profile.get("review_policies", {}).get(gate, [])
            ids = {entry.get("id") for entry in axes if isinstance(entry, dict)}
            if not {"acceptance-granularity", "owner-comprehensible"}.issubset(ids):
                raise JourneyFailure(f"compact setup omitted shipped {gate} intent questions")
            for entry in axes:
                if entry.get("id") in {"acceptance-granularity", "owner-comprehensible"} and entry.get("review_stage", "aggregate") != "aggregate":
                    raise JourneyFailure(f"compact setup changed {gate} question stage")
        if setup_report.get("output_bytes") != self.profile_path.read_text(encoding="utf-8"):
            raise JourneyFailure("compact setup report did not retain exact profile bytes")
        if setup_report.get("output_sha256") != _sha256_file(self.profile_path):
            raise JourneyFailure("compact setup report hash does not match the generated profile")
        if setup_profile.get("work_slot_bindings", {}).get("intent-draft") != draft_binding:
            raise JourneyFailure("compact setup did not preserve the --draft-worker binding")
        review_binding = setup_profile.get("work_slot_bindings", {}).get("intent-review")
        if not isinstance(review_binding, dict):
            raise JourneyFailure("compact setup omitted the existing intent-review roster binding")
        if "--instructions" in review_binding.get("args", []) or "--instructions=" in " ".join(review_binding.get("args", [])):
            raise JourneyFailure("compact setup unexpectedly used an ad-hoc instructions packet")

        start_input = dict(setup_profile)
        start_input["artifact_root"] = str(self.artifact_root)
        start_input_path = self.work_root / "start-input.json"
        _write_json(start_input_path, start_input)
        self.provider_config.write_text(
            "[providers.software-change]\n"
            f"command = {json.dumps(str(self.provider))}\n"
            "args = []\n",
            encoding="utf-8",
        )
        _write_json(self.work_root / "accepted-artifacts-before.json", {
            subject: _sha256_file(self.artifact_root / subject) for subject in SUBJECTS
        })
        run_id = f"compact-worker-{fixture}"
        self.run_id = run_id

        def engine_call(operation: Sequence[str], *, start: bool = False) -> Dict[str, Any]:
            command = [
                str(self.engine),
                "--json",
                "--database",
                str(self.database),
                "--timeout-ms",
                "900000",
            ]
            if start:
                command.extend(["--config", str(self.provider_config)])
            command.extend(operation)
            completed = subprocess.run(
                command,
                cwd=str(self.data_root),
                text=True,
                capture_output=True,
                check=False,
            )
            if not completed.stdout.strip():
                raise JourneyFailure(
                    f"compact engine operation {operation[0]} returned no JSON: "
                    f"{completed.stderr.strip()}"
                )
            try:
                response = json.loads(completed.stdout)
            except json.JSONDecodeError as error:
                raise JourneyFailure(
                    f"compact engine operation {operation[0]} returned invalid JSON: {completed.stdout}"
                ) from error
            if not isinstance(response, dict):
                raise JourneyFailure(f"compact engine response was not an object: {response}")
            if response.get("status") not in {"completed", "rejected"}:
                raise JourneyFailure(f"compact engine operation failed: {response}")
            return response

        start_response = engine_call(
            [
                "start",
                "--id",
                run_id,
                "software-change",
                "@" + str(start_input_path),
                "compact worker fixture",
            ],
            start=True,
        )
        if start_response.get("status") != "completed":
            raise JourneyFailure(f"compact fixture start was rejected: {start_response}")
        _write_json(self.work_root / "start.json", start_response)
        start_full = engine_call(["show", run_id, "--view", "full"])
        _write_json(self.work_root / "show-full-start.json", start_full)
        if start_full.get("status") != "completed":
            raise JourneyFailure(f"compact fixture start show failed: {start_full}")

        steering = {
            "target": {"kind": "all"},
            "instruction": (
                "Compact fixture routed context: preserve the frozen intent and inspect the retained "
                "evidence before judging. " + ("x" * 1024)
            ),
        }
        engine_call(["show", run_id, "--view", "action"])
        steering_response = engine_call([
            "append",
            "--record-id=compact-worker-steering",
            run_id,
            "user-steering",
            json.dumps(steering, separators=(",", ":")),
        ])
        if steering_response.get("status") != "completed":
            raise JourneyFailure(f"compact fixture steering append failed: {steering_response}")
        _write_json(self.work_root / "steering-append.json", steering_response)

        def invoke_and_wait(slot_id: str, assignment_ids: Sequence[str], prefix: str) -> tuple[Dict[str, Any], Dict[str, Any], Dict[str, Any]]:
            before = engine_call(["show", run_id, "--view", "action"])
            if before.get("status") != "completed":
                raise JourneyFailure(f"compact action show before {slot_id} failed: {before}")
            invoke_args = ["invoke", run_id, slot_id]
            if assignment_ids:
                invoke_args.extend(["--assignments", ",".join(assignment_ids)])
            invoke_response = engine_call(invoke_args)
            _write_json(self.work_root / f"{prefix}-invoke.json", invoke_response)
            if invoke_response.get("status") != "completed":
                raise JourneyFailure(f"compact {slot_id} invocation admission failed: {invoke_response}")
            invocation_id = invoke_response.get("result", {}).get("invocation_id")
            if not isinstance(invocation_id, str) or not invocation_id:
                raise JourneyFailure(f"compact {slot_id} invoke omitted invocation_id")
            deadline = time.monotonic() + max(120.0, float(getattr(self.args, "job_timeout", 1200)))
            last_status: Optional[Dict[str, Any]] = None
            while time.monotonic() < deadline:
                last_status = engine_call(["show", run_id, "--view", "status"])
                status_result = last_status.get("result", {})
                execution = status_result.get("execution", {})
                rows = status_result.get("work_slot_invocations", [])
                row = next((item for item in rows if item.get("invocation_id") == invocation_id), None)
                if (
                    isinstance(row, dict)
                    and row.get("status") != "running"
                ) or (
                    isinstance(execution, dict)
                    and execution.get("invocation_id") == invocation_id
                    and execution.get("state") != "running"
                ):
                    break
                time.sleep(0.5)
            else:
                raise JourneyFailure(f"compact {slot_id} invocation did not finish: {last_status}")
            complete = engine_call(["show", run_id, "--view", "full"])
            _write_json(self.work_root / f"{prefix}-show-full-complete.json", complete)
            rows = complete.get("result", {}).get("work_slot_invocations", [])
            row = next((item for item in rows if item.get("invocation_id") == invocation_id), None)
            if not isinstance(row, dict):
                raise JourneyFailure(f"compact {slot_id} full show omitted invocation {invocation_id}")
            return invoke_response, complete, row

        slot_id = "intent-draft"
        draft_binding_args = setup_profile["work_slot_bindings"][slot_id]["args"]
        draft_assignment_ids = [
            f"worker-{index}"
            for index, token in enumerate(
                token for token in draft_binding_args if token == "--worker"
            )
        ]
        if not draft_assignment_ids:
            raise JourneyFailure("compact draft binding did not expose a worker assignment")
        draft_invoke, draft_complete, draft_invocation = invoke_and_wait(
            slot_id, draft_assignment_ids, "draft"
        )
        draft_capture = Path(draft_invocation["capture_dir"])
        if fixture == "negative-empty":
            if draft_invocation.get("status") != "failed":
                raise JourneyFailure(
                    f"negative compact fixture unexpectedly succeeded: {draft_invocation}"
                )
            final_state = draft_complete.get("result", {}).get("current_state")
        else:
            if draft_invocation.get("status") != "succeeded":
                raise JourneyFailure(f"positive compact draft invocation failed: {draft_invocation}")
            if fixture != "review":
                engine_call(["show", run_id, "--view", "action"])
                intent_ready = engine_call(["event", run_id, "intent-ready"])
                if intent_ready.get("status") != "completed":
                    raise JourneyFailure(f"compact intent-ready was rejected: {intent_ready}")
                _write_json(self.work_root / "intent-ready.json", intent_ready)
                final_state = intent_ready.get("result", {}).get("run", {}).get("current_state")
            else:
                final_state = draft_complete.get("result", {}).get("current_state")

        review_invoke = None
        review_complete = draft_complete
        review_invocation = None
        if fixture == "review":
            draft_output = next(
                (worker for worker in draft_complete["result"]["work_slot_invocations"] if worker.get("invocation_id") == draft_invocation["invocation_id"]),
                None,
            )
            if not isinstance(draft_output, dict):
                raise JourneyFailure("review compact fixture omitted its draft invocation")
            origin = {
                "kind": "selected-assignment-output",
                "id": draft_invocation["invocation_id"],
                "assignment_id": "worker-0",
            }
            intent_revision = self._fixture_revision("intent.json")
            review_context = {
                "gate": "intent-review",
                "policy_id": "outside-verifiable",
                "review_stage": "aggregate",
                "result": "pass",
                "findings": "",
                "author": {"name": "fixture-driver", "kind": "script"},
                "subject": "intent.json",
                "subject_revision": intent_revision,
                "config_version": setup_profile["config_version"],
                "origin": origin,
            }
            engine_call(["show", run_id, "--view", "action"])
            routed_append = engine_call([
                "append",
                "--record-id=compact-worker-routed-evidence",
                run_id,
                "review-evidence",
                json.dumps(review_context, separators=(",", ":")),
            ])
            if routed_append.get("status") != "completed":
                raise JourneyFailure(f"compact routed evidence append failed: {routed_append}")
            _write_json(self.work_root / "routed-evidence-append.json", routed_append)
            engine_call(["show", run_id, "--view", "action"])
            ready = engine_call(["event", run_id, "intent-ready"])
            if ready.get("status") != "completed":
                raise JourneyFailure(f"review compact intent-ready was rejected: {ready}")
            review_binding_args = setup_profile["work_slot_bindings"]["intent-review"]["args"]
            review_assignment_ids = [
                f"worker-{index}"
                for index, _token in enumerate(
                    token for token in review_binding_args if token == "--worker"
                )
            ]
            if not review_assignment_ids:
                raise JourneyFailure("compact review binding did not expose assignments")
            review_invoke, review_complete, review_invocation = invoke_and_wait(
                "intent-review", review_assignment_ids, "review"
            )
            if review_invocation.get("status") != "succeeded":
                raise JourneyFailure(f"compact review invocation failed: {review_invocation}")
            final_state = review_complete.get("result", {}).get("current_state")

        accepted_before = self._read_json(
            self.work_root / "accepted-artifacts-before.json", "compact accepted artifact hashes"
        )
        accepted_after = {
            subject: _sha256_file(self.artifact_root / subject) for subject in SUBJECTS
        }
        _write_json(self.work_root / "accepted-artifacts-after.json", accepted_after)
        if fixture == "negative-empty" and accepted_after != accepted_before:
            raise JourneyFailure(
                f"negative compact fixture changed accepted artifacts: before={accepted_before} after={accepted_after}"
            )
        final_invocation = review_invocation or draft_invocation
        final_complete = review_complete
        capture_dir = Path(final_invocation["capture_dir"])
        spec_path = capture_dir / "fan-out-spec.json"
        summary_path = capture_dir / "summary.json"
        if not spec_path.is_file() or not summary_path.is_file():
            raise JourneyFailure(f"compact capture omitted fan-out receipts: {capture_dir}")
        spec = _load_json(spec_path)
        summary = _load_json(summary_path)
        try:
            projection_metrics = work_slot_journey.assert_projected_fan_out_capture(final_invocation)
        except (AssertionError, KeyError, TypeError, OSError, UnicodeError, json.JSONDecodeError) as error:
            raise JourneyFailure(
                f"compact fixture did not retain a valid projected/full capture: {error}"
            ) from error
        routed_inputs = final_invocation.get("routed_inputs", [])
        routing = {
            "invocation_routed_inputs": routed_inputs,
            "capture_format": spec.get("capture_format"),
            "spec_workers": spec.get("workers", []),
            "summary_workers": summary.get("workers", []),
            "projection_metrics": projection_metrics,
        }
        _write_json(self.work_root / "routing.json", routing)
        verification = {
            "run_id": run_id,
            "slot_id": final_invocation.get("slot_id"),
            "invocation_id": final_invocation.get("invocation_id"),
            "overlay_status": final_invocation.get("status"),
            "overlay_exit_code": final_invocation.get("exit_code"),
            "inner_workers": final_invocation.get("inner_workers", []),
            "show_full": final_complete,
            "selected_outputs": [
                {
                    "assignment_id": worker.get("assignment_id"),
                    "selected_attempt": worker.get("selected_attempt"),
                    "selected_output_sha256": worker.get("selected_output_sha256"),
                    "selected_output_path": worker.get("selected_output_path"),
                    "attempts_path": worker.get("attempts_path"),
                }
                for worker in summary.get("workers", [])
            ],
            "projection_metrics": projection_metrics,
        }
        nonempty_stdout_count = sum(
            bool(Path(capture_dir / str(index) / "stdout").is_file() and (capture_dir / str(index) / "stdout").read_bytes())
            for index in range(len(summary.get("workers", [])))
        )
        if fixture != "negative-empty" and nonempty_stdout_count == 0:
            raise JourneyFailure("compact positive fixture retained no nonempty worker output")
        if fixture == "negative-empty" and nonempty_stdout_count != 0:
            raise JourneyFailure("compact empty-delivery fixture unexpectedly retained worker stdout")
        _write_json(self.work_root / "verification.json", verification)
        metadata = {
            "schema_version": 1,
            "fixture": fixture,
            "run_id": run_id,
            "state_after_fixture": final_state,
            "slot_id": final_invocation.get("slot_id"),
            "invocation_id": final_invocation.get("invocation_id"),
            "setup_report": str(self.work_root / "setup-report.json"),
            "setup_profile": str(self.profile_path),
            "setup_profile_sha256": _sha256_file(self.profile_path),
            "profile_contract": {
                "config_version": setup_profile["config_version"],
                "criterion_policy": setup_profile["criterion_policy"],
                "intent_review_stages": {
                    gate: sorted({entry.get("review_stage", "aggregate") for entry in setup_profile["review_policies"][gate]})
                    for gate in ("intent-review", "intent-adversarial-review")
                },
                "intent_questions": ["acceptance-granularity", "owner-comprehensible"],
            },
            "start_input": str(start_input_path),
            "start": str(self.work_root / "start.json"),
            "show_full_start": str(self.work_root / "show-full-start.json"),
            "invoke": str(self.work_root / ("review-invoke.json" if review_invoke else "draft-invoke.json")),
            "show_full_complete": str(self.work_root / ("review-show-full-complete.json" if review_invoke else "draft-show-full-complete.json")),
            "capture_dir": str(capture_dir),
            "fan_out_spec": str(spec_path),
            "summary": str(summary_path),
            "routing": str(self.work_root / "routing.json"),
            "verification": str(self.work_root / "verification.json"),
            "draft_worker": str(draft_worker_path),
            "roster": str(roster_path),
            "binding": setup_profile["work_slot_bindings"][final_invocation["slot_id"]],
            "routed_inputs": routed_inputs,
            "accepted_artifacts_before": accepted_before,
            "accepted_artifacts_after": accepted_after,
            "outcome": {
                "overlay_status": final_invocation.get("status"),
                "overlay_exit_code": final_invocation.get("exit_code"),
                "inner_workers": final_invocation.get("inner_workers", []),
                "worker_count": len(summary.get("workers", [])),
                "nonempty_stdout_count": nonempty_stdout_count,
            },
        }
        metadata_path = self.work_root / "compact-capture.json"
        _write_json(metadata_path, metadata)
        print(f"compact worker fixture passed: {fixture}; captures: {self.work_root}")
        return metadata_path

    def _write_reconciliation_result(
        self,
        *,
        revision: str,
        mode: str,
        branch: str,
        document_observations: List[Dict[str, str]],
        behavior_observations: List[Dict[str, str]],
        action: str,
        action_reason: str,
        authorization: str,
        application: str,
        commit: str,
        traceability: Dict[str, Any],
        proof_references: List[str],
        blockers: List[str],
        decision: str,
    ) -> Path:
        """Write the provider-owned reconciliation artifact for the current visit.

        The journey is a driver fixture, not a semantic reviewer.  Its result
        deliberately records the synthetic actor and leaves the owner/calibration
        limitation in the surrounding proof record.  The provider still validates
        the closed artifact and the mode/branch status through the public event.
        """
        assert self.artifact_root is not None
        value = {
            "revision": revision,
            "author": {"name": "reconciliation-journey", "kind": "script"},
            "mode": mode,
            "branch": branch,
            "document_observations": document_observations,
            "behavior_observations": behavior_observations,
            "action": action,
            "action_reason": action_reason,
            "authorization": authorization,
            "application": application,
            "commit": commit,
            "traceability": traceability,
            "proof_references": proof_references,
            "blockers": blockers,
            "decision": decision,
        }
        path = self.artifact_root / RECONCILIATION_SUBJECT
        _write_json(path, value)
        return path

    def _write_no_change_reconciliation(self, revision: str) -> Path:
        """Write the primary run's Bookends-off, justified no-change result."""
        return self._write_reconciliation_result(
            revision=revision,
            mode="bookends-disabled",
            branch="change-specific-proof",
            document_observations=[
                {"path": "tracked.txt", "status": "unrelated", "observation": "the change-specific fixture does not require a repository-document edit"},
            ],
            behavior_observations=[
                {"status": "change-specific", "observation": "the delivered fixture behavior is proved by this run and creates no permanent document obligation"},
            ],
            action="no-document-change",
            action_reason="The public behavior is change-specific and the relevant authoritative document set requires no edit.",
            authorization="not-required",
            application="not-required",
            commit="not-required",
            traceability={"status": "not-applicable", "references": []},
            proof_references=["journey:primary-reconciliation"],
            blockers=[],
            decision="complete",
        )

    def _committed_reconciliation_reference(self, repository: Optional[Path] = None) -> Dict[str, str]:
        """Return the live citation only after the exact PRD text is committed.

        The working tree may contain the owner-accepted wording before the
        separate Git action.  Looking at HEAD prevents a fixture from presenting
        a candidate as live.  Before integration, LE-1 is a real mechanical live
        ID and LE-142 remains explicitly pending in the journey receipt.
        """
        source = repository or self.data_root
        completed = subprocess.run(
            ["git", "show", "HEAD:docs/PRD.md"],
            cwd=source,
            capture_output=True,
            check=False,
        )
        if completed.returncode != 0:
            return {
                "reference": "bookends:LE-1",
                "requested": f"bookends:{RECONCILIATION_REQUIREMENT_ID}",
                "status": "pending-owner-integration",
            }
        text = completed.stdout.decode("utf-8", "replace")
        lines = text.splitlines()
        for index, line in enumerate(lines):
            if not line.startswith(f"### {RECONCILIATION_REQUIREMENT_ID}: "):
                continue
            end = next(
                (candidate for candidate in range(index + 1, len(lines)) if lines[candidate].startswith("### ")),
                len(lines),
            )
            body = lines[index + 1 : end]
            # The accepted integration may retain the exact owner-approved
            # `Proposed ...` title; committed HEAD plus a live record is the
            # authoritative integration signal, not title wording.
            if "- Status: live" in body:
                return {
                    "reference": f"bookends:{RECONCILIATION_REQUIREMENT_ID}",
                    "requested": f"bookends:{RECONCILIATION_REQUIREMENT_ID}",
                    "status": "committed-owner-integrated",
                }
            break
        return {
            "reference": "bookends:LE-1",
            "requested": f"bookends:{RECONCILIATION_REQUIREMENT_ID}",
            "status": "pending-owner-integration",
        }

    def _committed_requirement_status(self, requirement_id: str) -> Dict[str, Any]:
        """Report whether one owner-accepted requirement is live in committed HEAD.

        The journey may run before the driver's separately authorized PRD commit.
        Keep that state explicit instead of turning a working-tree candidate into
        a durable live citation.
        """
        completed = subprocess.run(
            ["git", "show", "HEAD:docs/PRD.md"],
            cwd=self.data_root,
            capture_output=True,
            check=False,
        )
        requested = f"bookends:{requirement_id}"
        if completed.returncode != 0:
            return {
                "requested": requested,
                "reference": None,
                "status": "pending-owner-integration",
            }
        lines = completed.stdout.decode("utf-8", "replace").splitlines()
        heading = f"### {requirement_id}: "
        for index, line in enumerate(lines):
            if not line.startswith(heading):
                continue
            end = next(
                (candidate for candidate in range(index + 1, len(lines)) if lines[candidate].startswith("### ")),
                len(lines),
            )
            live = "- Status: live" in lines[index + 1 : end]
            return {
                "requested": requested,
                "reference": requested if live else None,
                "status": "committed-owner-integrated" if live else "pending-owner-integration",
            }
        return {
            "requested": requested,
            "reference": None,
            "status": "pending-owner-integration",
        }

    def _commit_fixture_document(self, target: Path) -> str:
        """Commit an isolated fixture edit before downstream proof.

        This only mutates the temporary fixture repository, never the source
        checkout.  The result's ``commit: committed`` status therefore names an
        observed Git commit rather than a synthetic claim.
        """
        assert self.repository_root is not None
        relative = target.relative_to(self.repository_root).as_posix()
        for command in (
            ["git", "add", "--", relative],
            ["git", "commit", "-qm", f"reconciliation fixture: {relative}"],
        ):
            completed = subprocess.run(
                command,
                cwd=self.repository_root,
                text=True,
                capture_output=True,
                check=False,
            )
            if completed.returncode != 0:
                raise JourneyFailure(
                    f"fixture Git command {' '.join(command[1:])} failed: "
                    f"{completed.stderr.strip() or completed.stdout.strip()}"
                )
        status = subprocess.run(
            ["git", "status", "--porcelain", "--", relative],
            cwd=self.repository_root,
            text=True,
            capture_output=True,
            check=False,
        )
        if status.returncode != 0 or status.stdout.strip():
            raise JourneyFailure(
                f"fixture document remained dirty after reconciliation commit: {relative}"
            )
        head = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=self.repository_root,
            text=True,
            capture_output=True,
            check=False,
        )
        if head.returncode != 0 or not head.stdout.strip():
            raise JourneyFailure("fixture reconciliation commit omitted an observable HEAD")
        return head.stdout.strip()

    def _provider_document_observations(self) -> List[Dict[str, Any]]:
        """Capture authored provider guidance without claiming target-run completion."""
        observations: List[Dict[str, Any]] = []
        required = {
            "crates/software-change-provider/README.md": (
                "Reconciliation and document integration",
                "Bookends-disabled runs",
                "no-document-change",
            ),
            "crates/software-change-provider/AGENTS.md": (
                "reconciliation",
                "reconciliation-ready",
                "Bookends-on",
            ),
        }
        for relative, clauses in required.items():
            path = self.data_root / relative
            try:
                data = path.read_bytes()
                text = data.decode("utf-8")
            except (OSError, UnicodeDecodeError) as error:
                raise JourneyFailure(f"provider document observation failed for {path}: {error}") from error
            missing = [clause for clause in clauses if clause.lower() not in text.lower()]
            if missing:
                raise JourneyFailure(f"provider document {path} omitted required reconciliation guidance: {missing}")
            observations.append(
                {
                    "path": relative,
                    "sha256": hashlib.sha256(data).hexdigest(),
                    "status": "authored-input",
                    "checked_run_status": "pending-driver-owned-target-run",
                    "missing_guidance": [],
                }
            )
        return observations

    def _prepare_reconciliation_profile(
        self, case_dir: Path, artifacts: Path, *, bookends_enabled: bool
    ) -> Dict[str, Any]:
        """Construct a small v11 profile from shipped data for the public fixture."""
        profile = self._read_json(
            self.data_root / PROFILE_SUBPATH, "reconciliation source profile"
        )
        policies = profile.get("review_policies")
        if not isinstance(policies, dict):
            raise JourneyFailure("reconciliation source profile omitted review_policies")
        profile["config_version"] = "journey-reconciliation-11"
        profile["review_policies"] = {gate: [] for gate in policies}
        profile["review_policies"]["implementation-review"] = [{
            "id": "tasks-actually-done",
            "description": "Reconciliation journey implementation proof boundary.",
            "example_prompt": "Judge tasks-actually-done only.",
            "review_stage": "aggregate",
            "required_authors": 1,
        }]
        profile["artifact_root"] = str(artifacts)
        profile["work_slot_bindings"] = {}
        if bookends_enabled:
            profile["extra"] = {"bookends": {"enabled": True}}
        else:
            profile.pop("extra", None)
        profile_path = case_dir / "reconciliation-profile.json"
        _write_json(profile_path, profile)
        self.profile_path = profile_path
        self.profile = profile
        self.provider_config = case_dir / "providers.toml"
        self._write_provider_config_at(self.provider_config)
        return profile

    def _initialize_reconciliation_case(
        self, case_dir: Path, case_name: str, *, bookends_enabled: bool
    ) -> None:
        """Create one isolated fixture repository and fresh public run shell."""
        assert self.fixture_root is not None
        case_dir.mkdir(parents=True, exist_ok=True)
        self.run_dir = case_dir
        self.database = case_dir / "loop.sqlite"
        self.artifact_root = case_dir / "artifacts"
        self.artifact_root.mkdir(exist_ok=True)
        checkout = case_dir / "checkout"
        if bookends_enabled:
            shutil.copytree(
                self.data_root,
                checkout,
                ignore=shutil.ignore_patterns(
                    ".git", "target", "__pycache__", "*.pyc", ".pi-subagents", ".loop-engine", "fan-out-adhoc",
                ),
            )
        else:
            checkout.mkdir()
            (checkout / "docs").mkdir()
            (checkout / "docs" / "PRD.md").write_bytes(
                (self.data_root / "docs" / "PRD.md").read_bytes()
            )
            (checkout / "docs" / "reconciliation-target.md").write_text(
                "# Reconciliation target\n\nThe fixture documents the delivered behavior.\n",
                encoding="utf-8",
            )
            (checkout / "README.md").write_text("# Reconciliation fixture\n", encoding="utf-8")
        self._initialize_overlay_checkout(checkout)
        self.repository_root = checkout
        self.command_cwd = checkout
        self.command_env = {"BOOKENDS_BYPASS": ""}
        self.profile_source = self.data_root / PROFILE_SUBPATH
        profile = self._prepare_reconciliation_profile(case_dir, self.artifact_root, bookends_enabled=bookends_enabled)
        if bookends_enabled:
            self._write_overlay_artifacts(self.artifact_root, candidate=False, unfulfilled=False)
        else:
            for subject, fixture in SUBJECTS.items():
                shutil.copy2(self.fixture_root / fixture, self.artifact_root / subject)
            self._prepare_fixture_proof_commands(self.artifact_root)
        self.run_id = f"reconciliation-{case_name}"
        self.state = "not-started"
        self._start()
        shown = self._assert_show("explore", f"{case_name}-start")
        if shown.get("initial_input") != profile:
            raise JourneyFailure(f"{case_name} start changed the frozen reconciliation profile")
        for event, target in (
            ("intent-ready", "design"),
            ("design-ready", "plan"),
            ("plan-ready", "implement"),
        ):
            self._expect_allow(event, target)

    def _write_requirement_coverage_contrasts(self, root: Path) -> Dict[str, Any]:
        """Retain the three semantic-coverage contrasts without making judgments."""
        assert self.fixture_root is not None
        companion = self.data_root / "crates/software-change-provider/data/calibration/companions/fictional-repo/docs/requirement-coverage.md"
        companion_text = companion.read_text(encoding="utf-8")
        cases: List[Dict[str, Any]] = []
        expected = {
            "sufficient": "sufficient-existing-wording",
            "related-insufficient": "missing-or-changed-enduring-meaning",
            "implementation-defect": "implementation-defect",
        }
        for name, branch in expected.items():
            fixture = self._read_json(
                self.fixture_root / f"requirement-coverage-{name}.json",
                f"requirement coverage fixture {name}",
            )
            references = fixture.get("requirement_references")
            if not isinstance(references, list) or len(references) != 1:
                raise JourneyFailure(f"requirement coverage fixture {name} omitted one reference")
            reference = references[0]
            authoritative = str(reference.get("authoritative_text", ""))
            normalized_authoritative = " ".join(authoritative.split()).lower()
            if not normalized_authoritative or "process exit" not in normalized_authoritative:
                raise JourneyFailure(f"requirement coverage fixture {name} omitted meaningful authoritative wording")
            cross_references = reference.get("explicit_cross_references")
            if not isinstance(cross_references, list) or not cross_references:
                raise JourneyFailure(f"requirement coverage fixture {name} omitted its explicit cross-reference")
            inspected_cross_references: List[Dict[str, Any]] = []
            for label in cross_references:
                if not isinstance(label, str) or not label.startswith("fictional-repo/"):
                    raise JourneyFailure(f"requirement coverage fixture {name} used a non-fictional cross-reference: {label!r}")
                relative = label.removeprefix("fictional-repo/")
                cross_path = self.data_root / "crates/software-change-provider/data/calibration/companions/fictional-repo" / relative
                try:
                    cross_bytes = cross_path.read_bytes()
                except OSError as error:
                    raise JourneyFailure(f"requirement coverage fixture {name} cross-reference is unreadable: {cross_path}: {error}") from error
                cross_text = cross_bytes.decode("utf-8")
                normalized_cross = " ".join(cross_text.split()).lower()
                if "req-coverage-1" not in normalized_cross or "process exit by itself is not acceptance" not in normalized_cross:
                    raise JourneyFailure(f"requirement coverage fixture {name} cross-reference omitted the accepted status wording: {cross_path}")
                inspected_cross_references.append({
                    "label": label,
                    "sha256": hashlib.sha256(cross_bytes).hexdigest(),
                    "bytes": len(cross_bytes),
                    "authoritative_text_present": all(
                        phrase in normalized_cross
                        for phrase in ("running", "succeeded", "failed", "unknown", "process exit")
                    ),
                })
            normalized_companion = " ".join(companion_text.split()).lower()
            for phrase in ("running", "succeeded", "failed", "unknown", "process exit by itself is not acceptance"):
                if phrase not in normalized_companion:
                    raise JourneyFailure(f"coverage fixture did not match its named document bytes: {phrase}")
            if name == "sufficient":
                if "operator-visible job status" not in normalized_companion:
                    raise JourneyFailure("sufficient coverage fixture did not identify the named status requirement")
            if name == "related-insufficient":
                if "owner-facing" not in str(fixture.get("promised_outcome", "")):
                    raise JourneyFailure("related-insufficient fixture lost its distinct owner-facing outcome")
                if "proactive owner chat" not in normalized_companion or "new notification channel" not in normalized_companion:
                    raise JourneyFailure("related-insufficient fixture did not inspect the cross-reference's owner-chat boundary")
                proposal = fixture.get("proposed_requirement", {})
                if proposal.get("owner_acceptance") != "pending" or proposal.get("commit") != "not-committed":
                    raise JourneyFailure("related-insufficient fixture lost pending owner status")
            if name == "implementation-defect":
                if fixture.get("delivered_behavior", {}).get("status_view") != "unknown":
                    raise JourneyFailure("implementation-defect fixture lost its observed stale status")
                if fixture.get("reclassification", {}).get("new_requirement_needed") is not False:
                    raise JourneyFailure("implementation-defect fixture incorrectly proposes a new requirement")
                if fixture.get("reclassification", {}).get("correction") != "implementation correction is required before proof can complete":
                    raise JourneyFailure("implementation-defect fixture lost its correction classification")
            cases.append({
                "fixture": f"data/calibration/fixtures/requirement-coverage-{name}.json",
                "named_requirement": reference.get("id"),
                "branch": branch,
                "citation_contrast": (
                    "sufficient-wording"
                    if name == "sufficient"
                    else "related-topic-but-authoritative-text-is-insufficient"
                    if name == "related-insufficient"
                    else "sufficient-wording-with-implementation-defect"
                ),
                "unrelated_or_insufficient_citation_cannot_close_gap": name == "related-insufficient",
                "semantic_judgment": "pending-owner-review",
                "owner_acceptance": "pending",
                "cross_references": inspected_cross_references,
                "cross_reference_bytes_inspected": bool(inspected_cross_references),
                "token_or_related_id_alone_rejected": name != "sufficient",
            })
        result = {
            "status": "mechanics-captured; semantic-calibration-pending",
            "companion": str(companion),
            "companion_sha256": hashlib.sha256(companion.read_bytes()).hexdigest(),
            "cases": cases,
            "synthetic_limit": "Fixture classifications do not establish owner approval or semantic truth.",
        }
        path = root / "requirement-coverage-contrasts.json"
        _write_json(path, result)
        return result

    def _run_reconciliation_case(self, case_name: str, *, bookends_enabled: bool) -> Dict[str, Any]:
        """Drive one reconciliation branch through real engine/provider processes."""
        self._initialize_reconciliation_case(self.run_dir / case_name, case_name, bookends_enabled=bookends_enabled)
        assert self.artifact_root is not None
        assert self.repository_root is not None
        mode = "bookends-enabled" if bookends_enabled else "bookends-disabled"
        # Use the source checkout's committed HEAD for liveness. The isolated
        # fixture commits copied working-tree bytes and must not turn a
        # provisional candidate into an accepted live citation.
        citation = self._committed_reconciliation_reference()
        target = self.repository_root / "docs" / ("PRD.md" if bookends_enabled else "reconciliation-target.md")
        before = target.read_bytes() if target.exists() else b""
        edit = case_name in {"bookends-enabled-edit", "bookends-disabled-edit", "bookends-enabled-missing-authorization"}
        if edit:
            marker = f"\n<!-- reconciliation journey {case_name}: authorized fixture edit -->\n"
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(before + marker.encode("utf-8"))
        after = target.read_bytes() if target.exists() else b""
        fixture_commit: Optional[str] = None
        if case_name in {"bookends-enabled-edit", "bookends-disabled-edit"}:
            fixture_commit = self._commit_fixture_document(target)
        # The provider-owned state is entered through the checked implementation
        # handoff before its result can be authored or evaluated.  Keep this
        # transition in every branch, including the blocked outcomes, so the
        # denial proves the real reconciliation boundary rather than an
        # unavailable event from `implement`.
        self._expect_allow("implementation-ready", "reconciliation")
        if case_name == "bookends-enabled-unresolved":
            missing_document = self.repository_root / "docs" / "missing-authoritative-document.md"
            if missing_document.exists():
                raise JourneyFailure(
                    "unresolved reconciliation fixture unexpectedly supplied its required cross-reference"
                )
            result_path = self._write_reconciliation_result(
                revision="reconciliation-unresolved-r1",
                mode=mode,
                branch="sufficient-existing-wording",
                document_observations=[
                    {"path": "docs/PRD.md", "status": "unchanged", "observation": "accepted text was inspected"},
                    {"path": "docs/missing-authoritative-document.md", "status": "missing", "observation": "required cross-reference is absent"},
                ],
                behavior_observations=[{"status": "unknown", "observation": "the discrepancy cannot be resolved from the supplied document"}],
                action="blocked",
                action_reason="The authoritative cross-reference is missing and the durable discrepancy remains unresolved.",
                authorization="pending",
                application="pending",
                commit="pending",
                traceability={"status": "blocked", "references": [citation["reference"]]},
                proof_references=[citation["reference"]],
                blockers=["missing authoritative cross-reference", "unresolved document discrepancy"],
                decision="blocked",
            )
            denial = self._expect_denial("reconciliation-ready", "reconciliation", "software-change-reconciliation-blocked")
            shown = self._assert_show("reconciliation", "unresolved-blocked-show")
            if shown.get("current_state") != "reconciliation" or target.read_bytes() != before:
                raise JourneyFailure("unresolved reconciliation changed state or document bytes")
            return {"case": case_name, "mode": mode, "decision": "blocked", "denial": denial.get("code"), "artifact": str(result_path), "state": shown.get("current_state"), "citation": citation}
        if case_name == "bookends-enabled-missing-authorization":
            result_path = self._write_reconciliation_result(
                revision="reconciliation-authorization-r1",
                mode=mode,
                branch="missing-or-changed-enduring-meaning",
                document_observations=[{"path": "docs/PRD.md", "status": "updated", "observation": "fixture amendment bytes are present but not authorized"}],
                behavior_observations=[{"status": "matches-intent", "observation": "delivered behavior requires the documented amendment"}],
                action="amendment-application",
                action_reason="The fixture demonstrates that a required amendment cannot proceed without owner authorization.",
                authorization="pending",
                application="pending",
                commit="pending",
                traceability={"status": "pending", "references": [citation["reference"]]},
                proof_references=[citation["reference"]],
                blockers=["exact owner acceptance is pending", "separate Git authorization and commit are pending"],
                decision="blocked",
            )
            denial = self._expect_denial("reconciliation-ready", "reconciliation", "software-change-reconciliation-blocked")
            shown = self._assert_show("reconciliation", "authorization-blocked-show")
            status = subprocess.run(
                ["git", "status", "--porcelain", "--", target.relative_to(self.repository_root).as_posix()],
                cwd=self.repository_root,
                text=True,
                capture_output=True,
                check=False,
            )
            if shown.get("current_state") != "reconciliation" or target.read_bytes() == before:
                raise JourneyFailure("missing authorization did not retain the edited bytes and blocked state")
            if status.returncode != 0 or not status.stdout.strip():
                raise JourneyFailure("missing authorization fixture unexpectedly committed its document edit")
            return {"case": case_name, "mode": mode, "decision": "blocked", "denial": denial.get("code"), "artifact": str(result_path), "state": shown.get("current_state"), "document_status": "updated-but-uncommitted", "citation": citation}

        if case_name == "bookends-disabled-no-change":
            branch = "sufficient-existing-wording"
            action = "no-document-change"
            action_reason = "The relevant repository document already matches the approved fixture behavior."
            document_status = "unchanged"
            behavior_status = "matches-intent"
            traceability = {"status": "not-applicable", "references": []}
            proof = ["journey:reconciliation-bookends-disabled-no-change"]
            authorization = application = commit = "not-required"
        elif case_name == "bookends-disabled-edit":
            branch = "missing-or-changed-enduring-meaning"
            action = "document-edit"
            action_reason = "The fixture applies the authorized repository-document correction without Bookends machinery."
            document_status = "updated"
            behavior_status = "matches-intent"
            traceability = {"status": "not-applicable", "references": []}
            proof = ["journey:reconciliation-bookends-disabled-edit"]
            authorization = "accepted"
            application = "applied"
            commit = "committed"
        else:
            branch = "missing-or-changed-enduring-meaning"
            action = "amendment-application"
            action_reason = "The fixture applies the exact accepted amendment before downstream proof."
            document_status = "updated"
            behavior_status = "matches-intent"
            traceability = {"status": "updated", "references": [citation["reference"]]}
            proof = [citation["reference"]]
            authorization = "accepted"
            application = "applied"
            commit = "committed"
        if fixture_commit is not None:
            proof.append(f"git:{fixture_commit}")
        result_path = self._write_reconciliation_result(
            revision=f"reconciliation-{case_name}-r1",
            mode=mode,
            branch=branch,
            document_observations=[{"path": target.relative_to(self.repository_root).as_posix(), "status": document_status, "observation": "actual fixture bytes were inspected before and after reconciliation" + (f"; fixture commit {fixture_commit}" if fixture_commit else "")}],
            behavior_observations=[{"status": behavior_status, "observation": "the public fixture behavior matches the approved branch"}],
            action=action,
            action_reason=action_reason,
            authorization=authorization,
            application=application,
            commit=commit,
            traceability=traceability,
            proof_references=proof,
            blockers=[],
            decision="complete",
        )
        self._expect_allow("reconciliation-ready", "implementation-review")
        checkpoint_path = self.artifact_root / "implementation-checkpoint.json"
        if checkpoint_path.exists():
            raise JourneyFailure("implementation checkpoint existed before reconciliation downstream proof")
        checkpoint = self._create_checkpoint("implementation")
        if not checkpoint_path.is_file():
            raise JourneyFailure("post-reconciliation implementation checkpoint was not retained")
        if edit and after == before:
            raise JourneyFailure(f"{case_name} claimed an edit but document bytes did not change")
        if edit and fixture_commit is None:
            raise JourneyFailure(f"{case_name} claimed a successful edit without an observed fixture commit")
        if not edit and after != before:
            raise JourneyFailure(f"{case_name} claimed no change but document bytes changed")
        if bookends_enabled:
            self._pass_overlay_review("implementation-review", "approved", "validation")
        else:
            self._pass_review("implementation-review", "approved", "validation")
        shown = self._assert_show("validation", f"{case_name}-after-review")
        history = self._engine(["history", self.run_id], state="validation", event="history")
        self._expect_status(history, "completed", event="history", state="validation")
        transitions = [
            entry.get("action", {}).get("transition", {})
            for entry in history.get("result", [])
            if entry.get("action", {}).get("kind") == "transition"
            and entry.get("action", {}).get("outcome", {}).get("outcome") == "committed"
        ]
        required_edges = [
            ("implement", "implementation-ready", "reconciliation"),
            ("reconciliation", "reconciliation-ready", "implementation-review"),
        ]
        if any(edge not in [(item.get("source"), item.get("event"), item.get("target")) for item in transitions] for edge in required_edges):
            raise JourneyFailure(f"{case_name} did not retain reconciliation ordering: {transitions}")
        return {
            "case": case_name,
            "mode": mode,
            "decision": "complete",
            "artifact": str(result_path),
            "checkpoint": str(checkpoint_path),
            "checkpoint_state_sha256": checkpoint.get("repository", {}).get("state_sha256"),
            "document": str(target),
            "before_sha256": hashlib.sha256(before).hexdigest(),
            "after_sha256": hashlib.sha256(after).hexdigest(),
            "document_commit": fixture_commit,
            "state_after_review": shown.get("current_state"),
            "state_order": ["implement", "reconciliation", "implementation-review", "validation"],
            "citation": citation,
        }

    def _run_reconciliation_scenarios(self) -> Path:
        """Exercise all supported reconciliation outcomes in isolated public runs."""
        assert self.run_dir is not None
        parent_run_dir = self.run_dir
        root = parent_run_dir / "reconciliation-journey"
        root.mkdir(parents=True, exist_ok=True)
        outcomes = []
        for case_name, bookends_enabled in (
            ("bookends-disabled-no-change", False),
            ("bookends-disabled-edit", False),
            ("bookends-enabled-edit", True),
            ("bookends-enabled-unresolved", True),
            ("bookends-enabled-missing-authorization", True),
        ):
            # Each case owns a sibling database, artifact root and checkout;
            # the previous case mutates this Journey shell while it runs.
            self.run_dir = parent_run_dir
            outcomes.append(
                self._run_reconciliation_case(case_name, bookends_enabled=bookends_enabled)
            )
        contrasts = self._write_requirement_coverage_contrasts(root)
        documents = self._provider_document_observations()
        requirement_citations = {
            requirement_id: self._committed_requirement_status(requirement_id)
            for requirement_id in ("LE-141", "LE-142", "LE-143", "LE-144")
        }
        reconciliation_citation = outcomes[2]["citation"]
        if requirement_citations[RECONCILIATION_REQUIREMENT_ID]["status"] == "committed-owner-integrated":
            expected_reference = f"bookends:{RECONCILIATION_REQUIREMENT_ID}"
            if reconciliation_citation.get("reference") != expected_reference:
                raise JourneyFailure(
                    "integrated reconciliation proof did not use its exact live Bookends citation"
                )
        elif reconciliation_citation.get("status") != "pending-owner-integration":
            raise JourneyFailure(
                "pre-integration reconciliation proof did not retain pending owner status"
            )
        proof = {
            "schema_version": 1,
            "status": "passed-mechanics; semantic-owner-review-pending",
            "cases": outcomes,
            "requirement_coverage_contrasts": contrasts,
            "provider_document_observations": documents,
            "accepted_requirement_citation": {
                "requested": f"bookends:{RECONCILIATION_REQUIREMENT_ID}",
                "status": outcomes[2]["citation"]["status"],
                "public_results_use_exact_requested_citation_after_committed_integration": outcomes[2]["citation"]["status"] == "committed-owner-integrated",
            },
            "required_owner_citations": requirement_citations,
            "calibration": {
                "status": "pending-owner-attestation",
                "manifest": "crates/software-change-provider/data/calibration/manifest.json",
                "capture_root": None,
                "note": "The driver-owned supplied-material captures and owner judgments are not manufactured by this synthetic journey.",
            },
            "provider_target_runs": [
                {
                    "key": "provider-readme",
                    "status": "pending-driver-owned-target-run",
                    "target": "crates/software-change-provider/README.md",
                },
                {
                    "key": "provider-agents",
                    "status": "pending-driver-owned-target-run",
                    "target": "crates/software-change-provider/AGENTS.md",
                },
            ],
            "synthetic_limit": "Fixture actors and deterministic provider checks prove mechanics only; they do not establish owner approval, semantic calibration, or checked completion of the separate README/AGENTS target runs.",
        }
        path = root / "reconciliation-journey-proof.json"
        _write_json(path, proof)
        self.reconciliation_proof = path
        print("reconciliation journey passed: successful edit, justified no-change, unresolved discrepancy, missing authorization, and Bookends on/off")
        print("reconciliation synthetic limit: owner approval, semantic calibration, and provider-document target-run completion remain pending")
        return path

    def _run_recovery_inventory(
        self, *, global_jobs: Optional[List[Dict[str, Any]]] = None
    ) -> None:
        # Every focused scenario is mandatory in full source mode. Each
        # scenario owns a separate fixture catalog, so the existing bounded
        # process pool can run them independently without sharing a database.
        from recovery_journey import SCENARIOS, dispatch
        completed = []
        if global_jobs is not None:
            assert self.run_dir is not None
            case_root = self.run_dir / "recovery-inventory"
            # Keep the shipped inventory order in the generated record.  The
            # single pool may interleave these jobs with every other isolated
            # tail proof; no selector or coverage row is removed.
            for name in SCENARIOS:
                # Keep the existing public focused-selector command as the
                # pool member.  In particular, its timeout/descendant cleanup
                # remains owned directly by proof_pool rather than adding an
                # intermediate Python facade around cancellation cases.
                global_jobs.append(
                    {
                        "name": f"recovery-{name}",
                        "command": [
                            sys.executable,
                            str(Path(__file__).resolve()),
                            "--mode",
                            "source",
                            "--engine",
                            str(self.engine),
                            "--provider",
                            str(self.provider),
                            "--data-root",
                            str(self.data_root),
                            "--work-root",
                            str(case_root / name),
                            "--profile",
                            str(self.profile_source),
                            "--traversal-depth",
                            "full",
                            "--jobs",
                            str(self.args.jobs),
                            "--job-timeout",
                            str(self.args.job_timeout),
                            "--scenario",
                            name,
                        ],
                    }
                )
            return
        if self.args.jobs == 1:
            for name in SCENARIOS:
                if name == "override":
                    self._run_recovery_override()
                elif name == "criteria":
                    self._run_recovery_criteria()
                else:
                    dispatch(name, self)
                completed.append(name)
        else:
            import proof_pool

            assert self.run_dir is not None
            pool_root = self.run_dir / "recovery-inventory-pool"
            case_root = self.run_dir / "recovery-inventory"
            jobs = []
            for name in SCENARIOS:
                jobs.append({
                    "name": name,
                    "command": [
                        sys.executable,
                        str(Path(__file__).resolve()),
                        "--mode", "source",
                        "--engine", str(self.engine),
                        "--provider", str(self.provider),
                        "--data-root", str(self.data_root),
                        "--work-root", str(case_root / name),
                        "--profile", str(self.profile_source),
                        "--traversal-depth", "full",
                        "--jobs", str(self.args.jobs),
                        "--job-timeout", str(self.args.job_timeout),
                        "--scenario", name,
                    ],
                })
            try:
                report = proof_pool.run(
                    jobs,
                    root=pool_root,
                    limit=self.args.jobs,
                    timeout=self.args.job_timeout,
                )
            except proof_pool.PoolFailure as error:
                raise JourneyFailure(f"recovery scenario pool failed: {error}") from error
            report_path = self.run_dir / "recovery-inventory-pool-report.json"
            report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
            expected_peak = min(self.args.jobs, len(SCENARIOS))
            if report.get("status") != "passed" or report.get("peak_jobs") != expected_peak:
                raise JourneyFailure(
                    f"recovery scenario pool failed; inspect {report_path}",
                    state="end",
                    event="recovery-inventory",
                )
            for name, row in zip(SCENARIOS, report.get("jobs", [])):
                if row.get("name") != name or row.get("status") != "passed" or row.get("exit_code") != 0:
                    raise JourneyFailure(
                        f"recovery scenario {name} did not complete; inspect {report_path}",
                        state="end",
                        event="recovery-inventory",
                    )
                stdout = Path(row["stdout"]).read_text(encoding="utf-8", errors="replace")
                marker = RECOVERY_COMPLETION_MARKERS[name]
                if marker not in stdout:
                    raise JourneyFailure(
                        f"recovery scenario {name} omitted its completion marker; inspect {report_path}",
                        state="end",
                        event="recovery-inventory",
                    )
                completed.append(name)
        if completed != list(SCENARIOS):
            raise JourneyFailure("incomplete recovery scenario inventory")
        (self.run_dir / "recovery-inventory.json").write_text(
            json.dumps({"status": "passed", "scenarios": completed}, indent=2) + "\n")
        print("full recovery inventory passed: " + ", ".join(completed))

    # bookends:LE-113 — public override reaches visibly exceptional completion,
    # preserves failures/skipped checks and rejects stale/live-work requests.
    def _run_recovery_override(self) -> None:
        # bookends:LE-4 — public exceptional progression still resolves only the
        # named available edge; malformed/unavailable requests leave history intact.
        from recovery_journey import dispatch
        dispatch("override", self)

    # bookends:LE-116 — named captured commands, exact criterion/goal coverage,
    # fixed checkpoint, affected repair and explicit carry reach normal completion.
    def _run_recovery_criteria(self) -> None:
        # Criterion proof asserts real named command captures, checkpoint-bound
        # independent judgments, focused repair/carry and normal terminal state.
        from recovery_journey import dispatch
        dispatch("criteria", self)

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
        required = {
            "contract_version",
            "config_version",
            "criterion_policy",
            "review_policies",
            "artifact_schemas",
            "revision_links",
        }
        missing = sorted(required.difference(profile))
        if missing:
            raise JourneyFailure(f"high-rigor profile is missing fields: {', '.join(missing)}")
        if profile.get("contract_version") != 3:
            raise JourneyFailure(
                f"journey requires contract_version 3, got {profile.get('contract_version')!r}"
            )
        if profile.get("config_version") != "high-rigor-11":
            raise JourneyFailure(
                f"journey requires high-rigor-11, got {profile.get('config_version')!r}"
            )
        criterion_policy = profile.get("criterion_policy")
        if criterion_policy != {"required_authors": 2, "goal_required_authors": 2}:
            raise JourneyFailure(
                f"high-rigor profile has the wrong independent criterion/goal floors: {criterion_policy!r}"
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
            {"from": "validation-report.json", "field": "implementation_revision", "to": "implementation-report.json"},
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
        # Keep the existing sparse dummy overlay. Full source additionally
        # binds implement to the provider's graph runner before the profile is
        # frozen; review slots remain unbound.
        bindings = work_slot_journey.bindings_for([BOUND_SLOT_ID])
        if (
            self.mode == "source"
            and self.depth == "full"
            and self.run_id == "journey-production-run"
        ):
            assert self.repository_root is not None
            assert self.run_dir is not None
            implementation_receipts = self.run_dir / "implementation-receipts"
            repair_revision_file = self.run_dir / "repair-report-revision.txt"
            repair_effect_file = self.repository_root / "ad-hoc-repair-effect.txt"
            implementation_worker = work_slot_journey.stdin_worker_cli(
                implementation_receipts,
                (
                    "--stdout",
                    '{"repository_effect":{"kind":"dummy"}}',
                    "--repair-revision-file",
                    str(repair_revision_file),
                    "--repair-effect-file",
                    str(repair_effect_file),
                ),
            )
            bindings["implement"] = work_slot_journey.implement_graph_runner_binding(
                provider=self.provider,
                task_worker=implementation_worker,
                working_directory=self.repository_root,
            )
        self.work_slot_bindings = bindings
        profile["work_slot_bindings"] = self.work_slot_bindings
        self.profile_path.write_text(json.dumps(profile, indent=2) + "\n", encoding="utf-8")
        # All five artifact files are the shipped good calibration shapes. The
        # full source run keeps its early intent/design/plan context in place;
        # _run_full_source temporarily removes intent for the negative check.
        assert self.fixture_root is not None
        for subject, fixture in SUBJECTS.items():
            # The completed source run must begin with its intent, design, and
            # plan already available.  Negative artifact-read coverage is
            # exercised by temporarily removing the already-present intent
            # after start; it must not be authored at the end of the run.
            shutil.copy2(self.fixture_root / fixture, self.artifact_root / subject)
        if self.mode == "source" and self.depth == "full":
            early = self.run_dir / "early-context-presence.json"
            early.write_text(
                json.dumps(
                    {
                        "status": "present-before-start",
                        "subjects": {
                            subject: str(self.artifact_root / subject)
                            for subject in ("intent.json", "design.json", "plan.json")
                        },
                        "revisions": {
                            subject: self._fixture_revision(subject)
                            for subject in ("intent.json", "design.json", "plan.json")
                        },
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )
        self._prepare_fixture_proof_commands(self.artifact_root)

    def _prepare_fixture_proof_commands(self, artifacts: Path) -> None:
        """Freeze executable protocol assertions in the runtime plan copy only.

        The calibration's fictional Cargo workspace does not exist in the
        checkpoint repository. These commands prove fixture protocol behavior,
        not workspace correctness or semantic review quality.
        """
        plan_path = artifacts / "plan.json"
        plan = self._read_json(plan_path, "runtime fixture plan")
        cases = [
            ("fixture-topology", {"operation": "describe"},
             "assert p.returncode == 0 and not p.stderr; "
             "v=json.loads(p.stdout); "
             "assert v['id']=='software-change' and v['initial_state']=='explore'; "
             "assert {'explore','implement','validation','end'} <= {s['id'] for s in v['states']}; "
             "assert any(t['source']=='validation' and t['event']=='validation-ready' for t in v['transitions'])",
             "Assert public describe exposes the expected workflow, phases and validation-ready transition."),
            ("fixture-unknown-operation", {"operation": "not-a-provider-operation"},
             "assert p.returncode == 2 and not p.stdout and p.stderr",
             "Assert unknown public operations refuse with protocol exit 2, diagnostics and no response."),
            ("fixture-malformed-json", None,
             "assert p.returncode == 2 and not p.stdout and p.stderr",
             "Assert malformed public JSON refuses with protocol exit 2, diagnostics and no response."),
            ("fixture-unknown-field", {"operation": "describe", "unexpected": True},
             "assert p.returncode == 2 and not p.stdout and p.stderr",
             "Assert closed public describe parsing rejects unknown envelope fields without a response."),
        ]
        plan["proof_commands"] = []
        for name, request, assertion, obligation in cases:
            raw = "{" if request is None else json.dumps(request)
            script = (
                "import json,subprocess; "
                f"p=subprocess.run([{str(self.provider)!r}],input={raw!r},text=True,capture_output=True); "
                "print(json.dumps({'exit':p.returncode,'stdout':p.stdout,'stderr':p.stderr}),flush=True); "
                + assertion
            )
            plan["proof_commands"].append({"id": name, "command": sys.executable,
                "args": ["-c", script], "owner": "driver", "obligation": obligation})
        _write_json(plan_path, plan)

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
            # bookends:LE-139 — this is the shared public-boundary sparse-binding scenario.
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
        start_profile = self.profile_path
        if run_id != self.run_id and "implement" in self.work_slot_bindings:
            assert self.run_dir is not None
            start_profile = self.run_dir / "high-rigor-secondary.json"
            if not start_profile.exists():
                secondary = self._read_json(self.profile_path, "primary started profile")
                secondary["work_slot_bindings"].pop("implement", None)
                _write_json(start_profile, secondary)
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
                "@" + str(start_profile),
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
        expected_initial_input = self._read_json(start_profile, "started profile")
        if result.get("run", {}).get("initial_input") != expected_initial_input:
            raise JourneyFailure(
                "start did not freeze the caller input", state="explore", event="start"
            )
        # Observation-before-mutation: arm the newly created state visit before
        # the journey invokes its bound work slot.
        self._show_for(run_id, state="explore", event="start-observation")

    def _show_for(self, run_id: str, *, state: str, event: str) -> Dict[str, Any]:
        response = self._engine_for(
            run_id, ["show", "--view", "full", run_id], state=state, event=event
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
        if kind == "user-steering":
            data = json.dumps({"target": {"kind": "all"},
                "instruction": "Preserve the caller's durable direction in this deterministic fixture."})
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
        # bookends:LE-136 — caller-selected run and context-record identities
        # survive separate append/show/history processes unchanged.
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
        if (marker_records["journey-marker-equals"].get("data", {}).get("semantic_verdict_quality") != "not tested"
            or marker_records["journey-marker-separate"].get("data") != {
                "target": {"kind": "all"},
                "instruction": "Preserve the caller's durable direction in this deterministic fixture."}):
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
        self,
        run_id: str,
        state: str,
        event: str,
        target: str,
        *,
        verify_latest: bool = False,
    ) -> Dict[str, Any]:
        response = self._event_for(run_id, event, state=state)
        self._expect_status(response, "completed", event=event, state=state)
        if response.get("result", {}).get("run", {}).get("current_state") != target:
            raise JourneyFailure(
                f"event {event} did not reach {target}", state=state, event=event
            )
        shown = self._assert_show_for(run_id, target, event)
        if verify_latest:
            latest = [
                evaluation
                for evaluation in shown.get("latest_evaluations", [])
                if evaluation.get("transition", {}).get("source") == state
                and evaluation.get("transition", {}).get("event") == event
            ]
            if (
                len(latest) != 1
                or latest[0].get("result", {}).get("result") != "allow"
                or "feedback" in latest[0].get("result", {})
            ):
                raise JourneyFailure(
                    f"successful review edge was not projected as latest allow: {shown}",
                    state=state,
                    event=event,
                )
        return response

    def _expect_allow(self, event: str, target: str) -> Dict[str, Any]:
        response = self._expect_allow_for(self.run_id, self.state, event, target)
        self.state = target
        return response

    def _append_evidence(
        self,
        gate: str,
        *,
        record_prefix: str = "",
        subject_revision: Optional[str] = None,
    ) -> None:
        self._append_evidence_for(
            self.run_id,
            gate,
            state=self.state,
            record_prefix=record_prefix,
            subject_revision=subject_revision,
        )

    def _append_evidence_for(
        self,
        run_id: str,
        gate: str,
        *,
        state: str,
        record_prefix: str = "",
        subject_revision: Optional[str] = None,
    ) -> None:
        subject = GATE_SUBJECT[gate]
        revision = subject_revision or self._fixture_revision(subject)
        axes = self.profile["review_policies"][gate]
        for entry in axes:
            axis = entry["id"]
            required_authors = int(entry.get("required_authors", 1))
            # Two authors are used for every axis, including N=1 axes.  This
            # makes independence explicit while keeping the fixture synthetic.
            for index, suffix in enumerate(("a", "b")):
                if index >= max(2, required_authors):
                    break
                stage = entry.get("review_stage", "aggregate")
                author = f"synthetic-{gate}-{axis}-{suffix}"
                record_id = f"{record_prefix}evidence-{gate}-{stage}-{axis}-{suffix}"
                data = {
                    "gate": gate,
                    "policy_id": axis,
                    "review_stage": stage,
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
        self,
        run_id: str,
        gate: str,
        *,
        state: str,
        record_prefix: str = "",
        subject_revision: Optional[str] = None,
    ) -> None:
        subject = GATE_SUBJECT[gate]
        revision = subject_revision or self._fixture_revision(subject)
        record_id = f"{record_prefix}finding-ledger-{gate}"
        data = {
            "schema_version": "1",
            "gate": gate,
            "subject": subject,
            "subject_revision": revision,
            "author": {"name": "journey-driver", "kind": "agent"},
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

    def _append_ledger_snapshot_for(
        self,
        run_id: str,
        snapshot: Mapping[str, Any],
        *,
        record_id: str,
        state: str,
        axis: str = "none",
    ) -> None:
        response = self._engine_for(
            run_id,
            [
                "append",
                f"--record-id={record_id}",
                run_id,
                "finding-ledger",
                json.dumps(snapshot, separators=(",", ":")),
            ],
            state=state,
            event="append",
            axis=axis,
        )
        self._expect_status(response, "completed", event="append", axis=axis, state=state)
        if response.get("result", {}).get("context", {}).get("id") != record_id:
            raise JourneyFailure(
                f"finding-ledger record ID was changed for {record_id}",
                state=state,
                event="append",
                axis=axis,
            )

    @staticmethod
    def _artifact_tree_snapshot(root: Path) -> Dict[str, bytes]:
        return {
            path.relative_to(root).as_posix(): path.read_bytes()
            for path in root.rglob("*")
            if path.is_file()
        }

    def _invoke_expected_failure(
        self,
        invocation_input: Any,
        *,
        label: str,
        expect_worker: bool = False,
        observed: Optional[Mapping[str, Any]] = None,
    ) -> Dict[str, Any]:
        """Invoke the bound slot and prove its failed overlay did not mutate proof."""
        assert self.artifact_root is not None
        assert self.repository_root is not None
        assert self.run_dir is not None
        before_artifacts = self._artifact_tree_snapshot(self.artifact_root)
        before_invocations = (
            observed
            if observed is not None
            else self._show_for(self.run_id, state="implement", event=f"{label}-before")
        ).get("work_slot_invocations", [])
        response = self._engine(
            [
                "invoke",
                self.run_id,
                "implement",
                "--input",
                json.dumps(invocation_input, separators=(",", ":")),
            ],
            state="implement",
            event="invoke",
            axis=label,
        )
        if response.get("status") == "error":
            # Shared preview/launch preparation now refuses malformed selection
            # before admission, with no worker, invocation or proof mutation.
            if response.get("code") != "provider-execution-failed" or not any(
                term in response.get("message", "") for term in ("invocation_input", "repair selection", "checkpoint mismatch:")):
                raise JourneyFailure(f"{label} unexpected preparation error: {response}")
            after = self._show_for(self.run_id, state="implement", event=f"{label}-refused")
            if (after.get("work_slot_invocations", []) != before_invocations
                or self._artifact_tree_snapshot(self.artifact_root) != before_artifacts):
                raise JourneyFailure(f"{label} early refusal mutated execution or proof")
            return response
        self._expect_status(response, "completed", event="invoke", state="implement", axis=label)
        invocation_id = response.get("result", {}).get("invocation_id")
        if not isinstance(invocation_id, str) or not invocation_id:
            raise JourneyFailure(f"{label} invocation omitted invocation_id: {response}")
        deadline = time.monotonic() + 20.0
        match: Optional[Dict[str, Any]] = None
        while time.monotonic() < deadline:
            shown = self._show_for(self.run_id, state="implement", event=f"{label}-poll")
            invocations = shown.get("work_slot_invocations", [])
            candidate = next(
                (
                    item
                    for item in invocations
                    if isinstance(item, dict)
                    and item.get("invocation_id") == invocation_id
                ),
                None,
            )
            if candidate is not None:
                match = candidate
                if candidate.get("status") == "failed" and candidate.get("completed_at") is not None:
                    break
            time.sleep(0.05)
        if match is None or match.get("status") != "failed":
            raise JourneyFailure(f"{label} did not produce a durable failed overlay: {match}")
        if not expect_worker and match.get("inner_workers"):
            raise JourneyFailure(f"{label} started a worker before refusing: {match}")
        after_artifacts = self._artifact_tree_snapshot(self.artifact_root)
        # Owned admission now retains even a refused attempt's packet/exits.
        # Existing proof must be byte-identical; only this invocation's new
        # capture files may appear, with no primary worker admitted above.
        capture_prefix = Path(match["capture_dir"]).relative_to(self.artifact_root).as_posix() + "/"
        if (any(after_artifacts.get(path) != body for path, body in before_artifacts.items())
            or any(not path.startswith(capture_prefix) for path in after_artifacts.keys() - before_artifacts.keys())):
            raise JourneyFailure(f"{label} mutated artifact proof before refusal")
        receipt_root = self.run_dir / "implementation-receipts"
        if any(receipt_root.glob("*.stdin")):
            raise JourneyFailure(f"{label} started the bound worker: {sorted(receipt_root.glob('*.stdin'))}")
        return match

    def _invoke_direct_repair_failure(
        self,
        binding: Mapping[str, Any],
        invocation_input: Any,
        *,
        label: str,
        include_context: bool = False,
        extra_args: Sequence[str] = (),
    ) -> None:
        """Exercise provider packet/argv refusals without the engine envelope."""
        assert self.artifact_root is not None
        assert self.repository_root is not None
        packet: Dict[str, Any] = {
            "run_id": self.run_id,
            "slot_id": "implement",
            "artifact_root": str(self.artifact_root),
            "instruction_body": "Implement",
            "capture_dir": str(self.run_dir / f"{label}-capture") if self.run_dir else "",
            "invocation_input": invocation_input,
        }
        if include_context:
            packet["context"] = self._show_for(
                self.run_id, state="implement", event=f"{label}-context"
            ).get("context", [])
        command = [str(self.provider), *list(binding["args"]), *extra_args]
        before = self._artifact_tree_snapshot(self.artifact_root)
        completed = subprocess.run(
            command,
            input=json.dumps(packet, separators=(",", ":")),
            text=True,
            capture_output=True,
            check=False,
            cwd=self.repository_root,
            env={**os.environ, **self.command_env},
        )
        if completed.returncode == 0:
            raise JourneyFailure(f"{label} unexpectedly succeeded: {completed.stdout}")
        if self._artifact_tree_snapshot(self.artifact_root) != before:
            raise JourneyFailure(f"{label} mutated artifact proof before refusal")

    def _invoke_expected_collision(
        self,
        invocation_input: Dict[str, Any],
        *,
        label: str,
        expected_revision: str,
    ) -> Dict[str, Any]:
        """Prove a worker-executed report-revision collision stays failed."""
        assert self.artifact_root is not None
        assert self.run_dir is not None
        before_plan_results = self.artifact_root / "plan-task-results.json"
        before_plan_bytes = before_plan_results.read_bytes() if before_plan_results.exists() else None
        response = self._engine(
            [
                "invoke",
                self.run_id,
                "implement",
                "--input",
                json.dumps(invocation_input, separators=(",", ":")),
            ],
            state="implement",
            event="invoke",
            axis=label,
        )
        self._expect_status(response, "completed", event="invoke", state="implement", axis=label)
        invocation_id = response.get("result", {}).get("invocation_id")
        if not isinstance(invocation_id, str) or not invocation_id:
            raise JourneyFailure(f"{label} invocation omitted invocation_id: {response}")
        deadline = time.monotonic() + 120.0
        match: Optional[Dict[str, Any]] = None
        while time.monotonic() < deadline:
            shown = self._show_for(self.run_id, state="implement", event=f"{label}-poll")
            match = next(
                (
                    item
                    for item in shown.get("work_slot_invocations", [])
                    if isinstance(item, dict)
                    and item.get("invocation_id") == invocation_id
                ),
                None,
            )
            if match is not None and match.get("status") == "failed" and match.get("completed_at") is not None:
                break
            time.sleep(0.05)
        if match is None or match.get("status") != "failed":
            raise JourneyFailure(f"{label} did not preserve a failed collision overlay: {match}")
        report = self._read_json(
            self.artifact_root / "implementation-report.json",
            f"{label} collision report",
        )
        if report.get("revision") != expected_revision:
            raise JourneyFailure(f"{label} worker did not write the collision revision: {report}")
        if (self.artifact_root / "implementation-checkpoint.json").exists():
            raise JourneyFailure(f"{label} wrote a post-collision implementation checkpoint")
        if before_plan_bytes is not None and before_plan_results.read_bytes() != before_plan_bytes:
            raise JourneyFailure(f"{label} changed plan-task-results.json")
        capture_dir = match.get("capture_dir")
        if not isinstance(capture_dir, str) or not capture_dir:
            raise JourneyFailure(f"{label} omitted capture_dir: {match}")
        summary = self._read_json(Path(capture_dir) / "summary.json", f"{label} capture summary")
        if len(summary.get("workers", [])) != 1 or summary.get("repair", {}).get("post_report_revision") is not None:
            raise JourneyFailure(f"{label} did not preserve worker-executed collision metadata: {summary}")
        return match

    def _run_ad_hoc_repair_proof(
        self,
        plan_revision: str,
        frozen_binding: Mapping[str, Any],
    ) -> str:
        """Prove the no-honest-task repair route on the public bound slot."""
        assert self.artifact_root is not None
        assert self.repository_root is not None
        assert self.run_dir is not None
        assert self.state == "implementation-review"

        report_path = self.artifact_root / "implementation-report.json"
        checkpoint_path = self.artifact_root / "implementation-checkpoint.json"
        pre_checkpoint = self._read_json(checkpoint_path, "pre-repair implementation checkpoint")
        pre_report_revision = pre_checkpoint.get("report", {}).get("revision")
        pre_state = pre_checkpoint.get("repository", {}).get("state_sha256")
        if not isinstance(pre_report_revision, str) or not pre_report_revision:
            raise JourneyFailure("pre-repair checkpoint omitted report revision")
        if not isinstance(pre_state, str) or not pre_state:
            raise JourneyFailure("pre-repair checkpoint omitted repository state")

        policy_id = self.profile["review_policies"]["implementation-review"][0]["id"]
        finding = {
            "id": "F-no-honest-task-repair",
            "source": {
                "kind": "context-record",
                "id": "ad-hoc-failing-evidence",
            },
            "policy_id": policy_id,
            "statement": "The accepted implementation defect has no honest frozen plan task owner.",
            "disposition": "accepted",
            "reason": "The driver accepted the finding and selected the narrow bound repair route.",
            "owner_phase": "implementation",
            "task_ids": [],
            "review_axes": [policy_id],
            "status": "unresolved",
        }
        failing_evidence = {
            "gate": "implementation-review",
            "policy_id": policy_id,
            "review_stage": self.profile["review_policies"]["implementation-review"][0].get("review_stage", "aggregate"),
            "result": "fail",
            "findings": finding["statement"],
            "author": {
                "name": f"synthetic-implementation-review-{policy_id}-a",
                "kind": "script",
            },
            "subject": "implementation-report.json",
            "subject_revision": pre_report_revision,
            "config_version": self.profile["config_version"],
        }
        self._assert_show("implementation-review", "ad-hoc-failing-evidence")
        evidence_response = self._engine(
            [
                "append",
                "--record-id=ad-hoc-failing-evidence",
                self.run_id,
                "review-evidence",
                json.dumps(failing_evidence, separators=(",", ":")),
            ],
            state="implementation-review",
            event="append",
            axis=policy_id,
        )
        self._expect_status(
            evidence_response,
            "completed",
            event="append",
            state="implementation-review",
            axis=policy_id,
        )
        unresolved_ledger = {
            "schema_version": "1",
            "gate": "implementation-review",
            "subject": "implementation-report.json",
            "subject_revision": pre_report_revision,
            "author": {"name": "no-honest-task-driver", "kind": "agent"},
            "findings": [finding],
        }
        self._assert_show("implementation-review", "ad-hoc-unresolved-ledger")
        self._append_ledger_snapshot_for(
            self.run_id,
            unresolved_ledger,
            record_id="ad-hoc-unresolved-ledger",
            state="implementation-review",
            axis=policy_id,
        )
        self._assert_show("implementation-review", "ad-hoc-finding-denial")
        denied = self._event("approved", policy_id)
        self._expect_status(
            denied,
            "rejected",
            event="approved",
            state="implementation-review",
            axis=policy_id,
        )
        if (denied.get("code") != "software-change-finding-ledger-invalid"
            or denied.get("details", {}).get("status") != "accepted_unresolved"):
            raise JourneyFailure(
                f"no-honest-task finding did not block implementation review: {denied}",
                state="implementation-review",
                event="approved",
                axis=policy_id,
            )
        self._expect_allow("revise", "implement")

        repair_input = {"repair_finding_ids": [finding["id"]]}
        # Invalid input/currentness requests run with an executable Dagu
        # sentinel first in PATH. They must fail before that sentinel or the
        # bound worker is reached and must leave proof bytes unchanged.
        fake_dagu_dir = self.run_dir / "repair-fail-if-dagu"
        fake_dagu_dir.mkdir()
        fake_dagu = fake_dagu_dir / "dagu"
        fake_dagu.write_text(
            "#!/bin/sh\nprintf 'called\\n' >> \"$(dirname \"$0\")/called\"\nexit 97\n",
            encoding="utf-8",
        )
        fake_dagu.chmod(0o755)
        saved_env = dict(self.command_env)
        self.command_env = {
            "PATH": str(fake_dagu_dir)
            + os.pathsep
            + os.environ.get("PATH", "")
        }
        try:
            for label, invalid_input in (
                ("repair-null", None),
                ("repair-empty", {"repair_finding_ids": []}),
                ("repair-blank", {"repair_finding_ids": ["  "]}),
                (
                    "repair-duplicate",
                    {"repair_finding_ids": [finding["id"], finding["id"]]},
                ),
                ("repair-unknown", {"repair_finding_ids": ["F-unknown-repair"]}),
                (
                    "repair-extra",
                    {"repair_finding_ids": [finding["id"]], "extra": True},
                ),
                (
                    "repair-mixed",
                    {
                        "plan_revision": plan_revision,
                        "task_roots": ["checked-transition-evaluator"],
                        "repair_finding_ids": [finding["id"]],
                    },
                ),
                ("repair-wrong-type", {"repair_finding_ids": [7]}),
            ):
                observed = self._assert_show("implement", f"{label}-show")
                self._invoke_expected_failure(invalid_input, label=label, observed=observed)
                if (fake_dagu_dir / "called").exists():
                    raise JourneyFailure(f"invalid repair selection probed Dagu during {label}")

            self._invoke_direct_repair_failure(
                frozen_binding,
                repair_input,
                label="repair-no-context",
            )
            self._invoke_direct_repair_failure(
                frozen_binding,
                repair_input,
                label="repair-frozen-task-flags",
                extra_args=("--task", "checked-transition-evaluator"),
            )
            if (fake_dagu_dir / "called").exists():
                raise JourneyFailure("invalid repair selection probed Dagu during direct argv checks")

            invalid_variants = (
                ("repair-stale-subject", {"subject_revision": "stale-report"}),
                ("repair-stale-checkpoint", {"checkpoint_stale": True}),
                ("repair-resolved", {"status": "resolved"}),
                (
                    "repair-rejected",
                    {
                        "disposition": "rejected",
                        "status": "recorded",
                        "owner_phase": None,
                        "review_axes": [],
                    },
                ),
                (
                    "repair-advisory",
                    {
                        "disposition": "advisory",
                        "status": "recorded",
                        "owner_phase": None,
                        "review_axes": [],
                    },
                ),
                ("repair-wrong-phase", {"owner_phase": "plan"}),
                (
                    "repair-task-routed",
                    {"task_ids": ["checked-transition-evaluator"]},
                ),
            )
            finding_fields = {
                "disposition",
                "reason",
                "owner_phase",
                "task_ids",
                "review_axes",
                "status",
            }
            stale_checkpoint_marker = self.repository_root / "repair-stale-checkpoint.txt"
            for index, (label, changes) in enumerate(invalid_variants, start=1):
                invalid_finding = copy.deepcopy(finding)
                invalid_finding.update(
                    {
                        key: copy.deepcopy(value)
                        for key, value in changes.items()
                        if key in finding_fields
                    }
                )
                invalid_snapshot = copy.deepcopy(unresolved_ledger)
                invalid_snapshot["subject_revision"] = changes.get(
                    "subject_revision", pre_report_revision
                )
                invalid_snapshot["findings"] = [invalid_finding]
                checkpoint_was_staled = bool(changes.get("checkpoint_stale"))
                if checkpoint_was_staled:
                    stale_checkpoint_marker.write_text("checkpoint drift\n", encoding="utf-8")
                self._assert_show("implement", f"{label}-before-ledger")
                self._append_ledger_snapshot_for(
                    self.run_id,
                    invalid_snapshot,
                    record_id=f"{label}-ledger-{index}",
                    state="implement",
                    axis=policy_id,
                )
                observed = self._assert_show("implement", f"{label}-show")
                self._invoke_expected_failure(repair_input, label=label, observed=observed)
                if (fake_dagu_dir / "called").exists():
                    raise JourneyFailure(f"invalid repair selection probed Dagu during {label}")
                if checkpoint_was_staled:
                    stale_checkpoint_marker.unlink()
                self._assert_show("implement", f"{label}-restore-ledger")
                self._append_ledger_snapshot_for(
                    self.run_id,
                    unresolved_ledger,
                    record_id=f"{label}-valid-ledger-{index}",
                    state="implement",
                    axis=policy_id,
                )
        finally:
            self.command_env = saved_env
        if (fake_dagu_dir / "called").exists():
            raise JourneyFailure("invalid repair selection probed Dagu")
        repair_revision_file = self.run_dir / "repair-report-revision.txt"
        repair_revision = pre_report_revision + "-ad-hoc-repair"
        repair_revision_file.write_text(repair_revision + "\n", encoding="utf-8")
        self._assert_show("implement", "ad-hoc-repair-before-invoke")
        pre_report_bytes = report_path.read_bytes()
        pre_checkpoint_bytes = checkpoint_path.read_bytes()
        plan_results_path = self.artifact_root / "plan-task-results.json"
        plan_results_bytes = plan_results_path.read_bytes() if plan_results_path.exists() else None
        pre_repository = self._repository_snapshot(self.repository_root)
        try:
            repair_invocation = work_slot_journey.invoke_until_succeeded(
                self._engine_call(self.run_id, state="implement"),
                self.run_id,
                "implement",
                timeout_s=120.0,
                invoke_args=[
                    "--input",
                    json.dumps(repair_input, separators=(",", ":")),
                ],
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state="implement", event="invoke") from error
        if (
            repair_invocation.get("binding") != frozen_binding
            or repair_invocation.get("invocation_input") != repair_input
            or [
                worker.get("assignment_id")
                for worker in repair_invocation.get("inner_workers", [])
            ]
            != ["ad-hoc-repair"]
            or repair_invocation.get("change_report", {}).get("plan_task_results") != []
        ):
            raise JourneyFailure(
                f"bound repair widened into plan execution or changed its frozen binding: {repair_invocation}",
                state="implement",
                event="invoke",
            )
        capture_dir = Path(repair_invocation["capture_dir"])
        summary = self._read_json(capture_dir / "summary.json", "ad-hoc repair capture summary")
        if set(summary) != {"workers", "repair"}:
            raise JourneyFailure(f"ad-hoc repair summary changed shape: {summary}")
        workers = summary.get("workers")
        if not isinstance(workers, list) or len(workers) != 1:
            raise JourneyFailure(f"ad-hoc repair did not capture exactly one worker: {summary}")
        worker = workers[0]
        if (
            worker.get("assignment_id") != "ad-hoc-repair"
            or "task_definition" in worker
            or "repository_effect" in worker
            or worker.get("routed_inputs") != [finding]
        ):
            raise JourneyFailure(f"ad-hoc repair worker was not a generic routed assignment: {worker}")
        task_packet = worker.get("task_packet")
        if not isinstance(task_packet, str):
            raise JourneyFailure(f"ad-hoc repair worker omitted task packet: {worker}")
        location_raw, assignment_raw = task_packet.split("\n---\n\n", 1)
        if json.loads(location_raw) != {"artifact_root": str(self.artifact_root)}:
            raise JourneyFailure(f"ad-hoc repair location was not compact artifact_root JSON: {task_packet}")
        assignment = json.loads(assignment_raw)
        if set(assignment) != {
            "kind",
            "plan_revision",
            "pre_report_revision",
            "pre_repository_state_sha256",
            "findings",
            "instruction",
        } or assignment.get("kind") != "ad-hoc-repair":
            raise JourneyFailure(f"ad-hoc repair assignment shape changed: {assignment}")
        if (
            assignment.get("plan_revision") != plan_revision
            or assignment.get("pre_report_revision") != pre_report_revision
            or assignment.get("pre_repository_state_sha256") != pre_state
            or assignment.get("findings") != [finding]
            or "fresh report revision" not in assignment.get("instruction", "")
        ):
            raise JourneyFailure(f"ad-hoc repair assignment omitted exact frozen inputs: {assignment}")
        summary_repair = summary["repair"]
        if (
            summary_repair.get("repair_finding_ids") != [finding["id"]]
            or summary_repair.get("pre_report_revision") != pre_report_revision
            or summary_repair.get("post_report_revision") != repair_revision
            or summary_repair.get("pre_repository_state_sha256") != pre_state
        ):
            raise JourneyFailure(f"ad-hoc repair summary omitted pre/post report identity: {summary}")
        post_checkpoint = self._read_json(checkpoint_path, "post-repair implementation checkpoint")
        post_state = post_checkpoint.get("repository", {}).get("state_sha256")
        if post_checkpoint.get("report", {}).get("revision") != repair_revision:
            raise JourneyFailure(f"post-repair checkpoint omitted fresh report revision: {post_checkpoint}")
        if not isinstance(post_state, str) or post_state == pre_state:
            raise JourneyFailure(f"ad-hoc repair did not create a distinct repository state: {post_checkpoint}")
        if summary_repair.get("post_repository_state_sha256") != post_state:
            raise JourneyFailure(f"ad-hoc repair summary omitted post repository identity: {summary}")
        report = self._read_json(report_path, "post-repair report")
        if report.get("revision") != repair_revision or report.get("plan_revision") != plan_revision:
            raise JourneyFailure(f"ad-hoc repair worker report was not fresh and plan-linked: {report}")
        if plan_results_bytes is not None and plan_results_path.read_bytes() != plan_results_bytes:
            raise JourneyFailure("ad-hoc repair changed plan-task-results.json")
        if self._repository_snapshot(self.repository_root) == pre_repository:
            raise JourneyFailure("ad-hoc repair worker did not create a repository effect")
        selected_path = worker.get("selected_output_path")
        selected_digest = worker.get("selected_output_sha256")
        if (
            not isinstance(selected_path, str)
            or not isinstance(selected_digest, str)
            or selected_digest != "sha256:" + hashlib.sha256(Path(selected_path).read_bytes()).hexdigest()
        ):
            raise JourneyFailure(f"ad-hoc repair capture omitted selected output identity: {worker}")
        history_revisions = {
            self._read_json(path, "implementation proof history entry")
            .get("report", {})
            .get("revision")
            for path in (self.artifact_root / "implementation-proof-history").glob("*.json")
        }
        if pre_report_revision not in history_revisions:
            raise JourneyFailure(f"ad-hoc repair did not preserve the historical pre-repair proof: {history_revisions}")
        implementation_receipts = self.run_dir / "implementation-receipts"
        if not (implementation_receipts / "ad-hoc-repair.stdin").is_file() or any(
            (implementation_receipts / f"{task}.stdin").exists()
            for task in (*[result.get("assignment_id") for result in workers], "summarizer")
            if task != "ad-hoc-repair"
        ):
            raise JourneyFailure("ad-hoc repair started an ordinary task or summarizer")
        valid_report_bytes = report_path.read_bytes()
        valid_checkpoint_bytes = checkpoint_path.read_bytes()
        repair_effect = self.repository_root / "ad-hoc-repair-effect.txt"
        if not repair_effect.is_file():
            raise JourneyFailure("ad-hoc repair dummy omitted its controlled repository effect")
        valid_effect_bytes = repair_effect.read_bytes()
        # The original failure remains the immutable source. Reuse it for the
        # fresh implementation revision through the one explicit applicability
        # declaration instead of copying its engine/checkpoint coordinates.
        repair_applicability = {
            "origin": {"kind": "context-record", "id": "ad-hoc-failing-evidence"},
            "target": {
                "subject": "implementation-report.json",
                "revision": repair_revision,
                "checkpoint": {
                    "phase": "implementation",
                    "report_revision": repair_revision,
                },
            },
            "attesting_driver": {"name": "no-honest-task-driver", "kind": "agent"},
            "reason": "The accepted implementation finding remains applicable after the repair proof revision.",
        }
        self._assert_show("implement", "ad-hoc-applicability")
        applicability_result = self._engine(
            [
                "append",
                "--record-id=ad-hoc-repair-applicability",
                self.run_id,
                "evidence-applicability",
                json.dumps(repair_applicability, separators=(",", ":")),
            ],
            state="implement",
            event="append",
            axis=policy_id,
        )
        self._expect_status(
            applicability_result,
            "completed",
            event="append",
            state="implement",
            axis=policy_id,
        )
        current_unresolved = copy.deepcopy(unresolved_ledger)
        current_unresolved["subject_revision"] = repair_revision
        current_unresolved["findings"] = [finding]
        self._assert_show("implement", "ad-hoc-collision-ledger")
        self._append_ledger_snapshot_for(
            self.run_id,
            current_unresolved,
            record_id="ad-hoc-collision-unresolved-ledger",
            state="implement",
            axis=policy_id,
        )
        collision_input = repair_input
        for label, collision_revision in (
            ("ad-hoc-immediate-collision", repair_revision),
            ("ad-hoc-historical-collision", pre_report_revision),
        ):
            repair_revision_file.write_text(collision_revision + "\n", encoding="utf-8")
            self._assert_show("implement", f"{label}-before-invoke")
            collision = self._invoke_expected_collision(
                collision_input,
                label=label,
                expected_revision=collision_revision,
            )
            if collision.get("binding") != frozen_binding or collision.get("invocation_input") != collision_input:
                raise JourneyFailure(f"{label} did not preserve frozen invocation identity: {collision}")
            inner = collision.get("inner_workers", [])
            if [item.get("assignment_id") for item in inner] != ["ad-hoc-repair"] or inner[0].get("exit_code") != 0:
                raise JourneyFailure(f"{label} did not retain the successful worker capture: {collision}")
            report_path.write_bytes(valid_report_bytes)
            repair_effect.write_bytes(valid_effect_bytes)
            checkpoint_path.write_bytes(valid_checkpoint_bytes)
        repair_receipts = self.run_dir / "ad-hoc-repair-receipts"
        repair_receipts.mkdir(exist_ok=True)
        for path in (self.run_dir / "implementation-receipts").glob("ad-hoc-repair*"):
            path.rename(repair_receipts / path.name)
        repair_revision_file.write_text(repair_revision + "\n", encoding="utf-8")
        resolved_finding = copy.deepcopy(finding)
        resolved_finding["status"] = "resolved"
        resolved_ledger = copy.deepcopy(current_unresolved)
        resolved_ledger["author"] = {"name": "no-honest-task-driver-confirmation", "kind": "agent"}
        resolved_ledger["findings"] = [resolved_finding]
        self._assert_show("implement", "ad-hoc-resolved-ledger")
        self._append_ledger_snapshot_for(
            self.run_id,
            resolved_ledger,
            record_id="ad-hoc-resolved-ledger",
            state="implement",
            axis=policy_id,
        )
        # Re-enter the reconciliation state after the repair and require a
        # fresh document decision before independent implementation review.
        self._write_no_change_reconciliation("primary-reconciliation-ad-hoc-repair")
        self._expect_allow("implementation-ready", "reconciliation")
        self._assert_show("reconciliation", "ad-hoc-reconciliation")
        self._expect_allow("reconciliation-ready", "implementation-review")
        self._append_evidence(
            "implementation-review",
            record_prefix="ad-hoc-",
            subject_revision=repair_revision,
        )
        self._assert_show("implementation-review", "ad-hoc-fresh-review")
        self._expect_allow("approved", "implementation-adversarial-review")
        print(
            "ad hoc repair journey passed: exact bound selection, fail-before-Dagu refusals, "
            "worker capture, collision failures, fresh proof, and independent re-review"
        )
        return repair_revision

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
        subject_revision: Optional[str] = None,
    ) -> None:
        # bookends:LE-6 — this provider-denied checked approval does not advance without allow.
        # bookends:LE-44 — the driver appends externally produced review evidence.
        # bookends:LE-137 — the provider validates evidence; it does not author a review.
        # bookends:LE-52 — prior evidence and denial lineage are carried into the next check.
        first_denial = self._expect_denial_for(
            run_id, state, event, gate, "software-change-finding-ledger-invalid"
        )
        self._append_evidence_for(
            run_id,
            gate,
            state=state,
            record_prefix=record_prefix,
            subject_revision=subject_revision,
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
            run_id,
            gate,
            state=state,
            record_prefix=record_prefix,
            subject_revision=subject_revision,
        )
        final = self._expect_allow_for(
            run_id, state, event, target, verify_latest=True
        )
        # bookends:LE-29 — a durable denial and later allow remain visible across fresh actor processes.
        # bookends:LE-33 — externally supplied evidence changes only provider authorization, not engine routing.
        # bookends:LE-34 — the denied result has feedback, while the later allow carries no feedback payload.
        # bookends:LE-47 — complete configured evidence allows the policy gate.
        # bookends:LE-50 — show projects the successful evaluation as the latest result.
        if final.get("result", {}).get("run", {}).get("current_state") != target:
            raise JourneyFailure(
                f"successful review edge did not reach target {target}: {final}",
                state=state,
                event=event,
            )

    def _pass_review(
        self,
        gate: str,
        event: str,
        target: str,
        *,
        subject_revision: Optional[str] = None,
        record_prefix: str = "",
    ) -> None:
        self._pass_review_for(
            self.run_id,
            self.state,
            gate,
            event,
            target,
            record_prefix=record_prefix,
            subject_revision=subject_revision,
        )
        self.state = target

    def _prepare_successor_state(
        self,
        run_id: str,
        target: str,
        *,
        implementation_revision: Optional[str] = None,
        isolated: bool = False,
    ) -> None:
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
        self._write_no_change_reconciliation(f"route-{run_id}-reconciliation")
        self._expect_allow_for(
            run_id, "implement", "implementation-ready", "reconciliation"
        )
        if isolated:
            self._create_checkpoint("implementation")
        self._expect_allow_for(
            run_id, "reconciliation", "reconciliation-ready", "implementation-review"
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
            subject_revision=implementation_revision,
        )
        self._pass_review_for(
            run_id,
            "implementation-adversarial-review",
            "implementation-adversarial-review",
            "approved",
            "validation",
            record_prefix=prefix,
            subject_revision=implementation_revision,
        )
        if isolated:
            assert self.repository_root is not None
            self._create_validation_fixture(
                lambda operations: self._engine_for(
                    run_id,
                    operations,
                    state="validation",
                    event=operations[0] if operations else "none",
                ),
                run_id,
                self.repository_root,
                f"successor-{run_id}",
            )
        else:
            # These route fixtures share the exact immutable target and
            # artifact location; explicitly supply the selected external fixture
            # judgments and real command captures rather than inventing a report.
            report = self._read_json(self.artifact_root / "validation-report.json", "route validation index")
            ids = set(report["command_evidence_ids"] + report["goal_verdict_ids"])
            ids.update(id for row in report["criteria"] for id in row["verdict_ids"])
            original = self._show_for(self.run_id, state=self.state, event="route-retained-proof")
            for row in original["context"]:
                if row["id"] in ids:
                    self._show_for(run_id, state="validation", event="route-proof-append")
                    result = self._engine_for(run_id, ["append", "--record-id=" + row["id"],
                        run_id, row["kind"], json.dumps(row["data"])], state="validation", event="append")
                    self._expect_status(result, "completed", event="append", state="validation")
        self._expect_allow_for(
            run_id, "validation", "validation-ready", "validation-review"
        )
        if target != "validation-review":
            raise JourneyFailure(f"unsupported successor route source state: {target}")

    def _run_successor_route_case(
        self,
        index: int,
        source: str,
        event: str,
        target: str,
        *,
        implementation_revision: Optional[str] = None,
        isolated: bool = False,
    ) -> None:
        """Check one route with its own catalog when running in the pool."""
        run_id = f"successor-route-{index:02d}-{event}"
        self._prepare_successor_state(
            run_id,
            source,
            implementation_revision=implementation_revision,
            isolated=isolated,
        )
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
        print(f"successor route case passed: {run_id} {source}/{event}/{target}")

    def _run_successor_route_proof(
        self,
        *,
        implementation_revision: Optional[str] = None,
        global_jobs: Optional[List[Dict[str, Any]]] = None,
    ) -> None:
        """Run independent route fixtures under the existing bounded pool."""
        assert self.run_dir is not None
        jobs = []
        job_root = self.run_dir / "successor-route-cases"
        spec_root = self.run_dir / "successor-route-jobs"
        spec_root.mkdir()
        for index, (source, event, target) in enumerate(SUCCESSOR_ROUTE_CASES, start=1):
            name = f"route-{index:02d}-{event}"
            spec_path = spec_root / f"{name}.json"
            spec_path.write_text(
                json.dumps(
                    {
                        "kind": "successor-route",
                        "root": str(job_root / name),
                        "args": self._pool_args(),
                        "index": index,
                        "source": source,
                        "event": event,
                        "target": target,
                        "implementation_revision": implementation_revision,
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )
            jobs.append({"name": name, "spec_path": spec_path})
        if global_jobs is not None:
            for job in jobs:
                global_jobs.append(
                    {
                        "name": job["name"],
                        "command": [
                            sys.executable,
                            str(self._pool_worker_path()),
                            str(job["spec_path"]),
                        ],
                    }
                )
            return
        if getattr(self.args, "jobs", 2) == 1:
            for job in jobs:
                _run_pool_job(str(job["spec_path"]))
        else:
            import proof_pool

            worker = self._pool_worker_path()
            pool_jobs = [
                {"name": job["name"], "command": [sys.executable, str(worker), str(job["spec_path"])]}
                for job in jobs
            ]
            report = proof_pool.run(
                pool_jobs,
                root=self.run_dir / "successor-route-pool",
                limit=self.args.jobs,
                timeout=self.args.job_timeout,
            )
            report_path = self.run_dir / "successor-route-pool-report.json"
            report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
            expected_peak = min(self.args.jobs, len(SUCCESSOR_ROUTE_CASES))
            if report.get("status") != "passed" or report.get("peak_jobs") != expected_peak:
                raise JourneyFailure(
                    f"successor route pool failed; inspect {report_path}",
                    state="end",
                    event="successor-routes",
                )
            if any(
                row.get("status") != "passed" or row.get("exit_code") != 0
                for row in report.get("jobs", [])
            ):
                raise JourneyFailure(
                    f"successor route pool omitted a completed case; inspect {report_path}",
                    state="end",
                    event="successor-routes",
                )
        print(f"successor route proof passed: {len(SUCCESSOR_ROUTE_CASES)} fresh runs")

    def _fixture_revision(self, subject: str) -> str:
        if subject == "validation-report.json" and self.artifact_root is not None:
            current = self.artifact_root / subject
            if current.exists():
                return self._read_json(current, "current validation index")["revision"]
        assert self.fixture_root is not None
        fixture = self._read_json(
            self.fixture_root / SUBJECTS[subject], f"fixture {subject}"
        )
        revision = fixture.get("revision")
        if not isinstance(revision, str) or not revision:
            raise JourneyFailure(f"fixture {subject} has no revision")
        return revision

    def _pool_args(self, *, work_root: Optional[Path] = None) -> Dict[str, Any]:
        """Serialize only the fixed inputs needed by an isolated proof child."""
        return {
            "mode": self.mode,
            "traversal_depth": self.depth,
            "engine": str(self.engine),
            "provider": str(self.provider),
            "data_root": str(self.data_root),
            "work_root": str(work_root or self.work_root),
            "profile": str(self.profile_source or self.profile_arg),
            "jobs": 1,
            "job_timeout": getattr(self.args, "job_timeout", 1200),
            "scenario": None,
        }

    def _append_global_pool_job(
        self,
        jobs: List[Dict[str, Any]],
        *,
        name: str,
        kind: str,
        root: Path,
        args: Optional[Dict[str, Any]] = None,
        **payload: Any,
    ) -> None:
        """Add one isolated case to the existing single proof-pool visit."""
        assert self.run_dir is not None
        spec_root = self.run_dir / "global-proof-jobs"
        spec_root.mkdir(parents=True, exist_ok=True)
        spec_path = spec_root / f"{name}.json"
        spec = {
            "kind": kind,
            "root": str(root),
            "args": args or self._pool_args(),
            **payload,
        }
        spec_path.write_text(
            json.dumps(spec, indent=2, default=str) + "\n", encoding="utf-8"
        )
        jobs.append(
            {
                "name": name,
                "command": [
                    sys.executable,
                    str(self._pool_worker_path()),
                    str(spec_path),
                ],
            }
        )

    def _pool_worker_path(self) -> Path:
        """Create the run-local adapter used by the existing proof pool."""
        assert self.run_dir is not None
        path = self.run_dir / "proof-pool-worker.py"
        if not path.exists():
            module_path = Path(__file__).resolve()
            path.write_text(
                "import importlib.util, sys\n"
                f"sys.path.insert(0, {str(module_path.parent)!r})\n"
                f"spec = importlib.util.spec_from_file_location('software_change_journey_pool', {str(module_path)!r})\n"
                "module = importlib.util.module_from_spec(spec)\n"
                "sys.modules[spec.name] = module\n"
                "spec.loader.exec_module(module)\n"
                "module._run_pool_job(sys.argv[1])\n",
                encoding="utf-8",
            )
        return path

    def _initialize_pool_case(
        self,
        root: Path,
        *,
        run_id: str = "pool-run",
        prepare_profile: bool = False,
        repository: bool = False,
    ) -> None:
        """Build a fresh case shell; no catalog or database is copied."""
        root.mkdir(parents=True, exist_ok=True)
        self.run_dir = root
        self.work_root = root
        self.database = root / "loop.sqlite"
        self.provider_config = root / "providers.toml"
        self.profile_path = root / "high-rigor.json"
        self.artifact_root = root / "artifacts"
        self.artifact_root.mkdir(exist_ok=True)
        self.profile_source = Path(self.profile_arg).expanduser().resolve()
        self.fixture_root = self.data_root / FIXTURE_SUBPATH
        self.profile = self._read_json(self.profile_source, "pool profile")
        self.run_id = run_id
        self.state = "not-started"
        self.command_env = {}
        self.command_cwd = None
        self.repository_root = None
        if repository:
            self._prepare_real_repository()
        if prepare_profile:
            self._prepare_profile()
            self._write_provider_config()

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
        if phase == "validation":
            number = getattr(self, "_validation_fixture_number", 0) + 1
            self._validation_fixture_number = number
            result = self._create_validation_fixture(
                lambda ops: self._engine(ops, state=self.state, event=ops[0]),
                self.run_id, self.repository_root, f"journey-v2-{number}")
            return result["checkpoint"]
        return self._create_checkpoint_at(
            self.provider, phase, self.artifact_root, self.repository_root
        )

    def _create_validation_fixture(self, call, run_id, repository, revision):
        # Real command captures and a fixed index precede synthetic independent
        # judgments. This test helper never substitutes report prose for proof.
        import importlib.util
        helper = Path(__file__).resolve().parents[1] / "tests/fixtures/prepare-validation.py"
        spec = importlib.util.spec_from_file_location("validation_fixture", helper)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        shown = call(["show", "--view", "full", run_id])
        self._expect_status(shown, "completed", event="show", state="validation")
        result = module.prepare(self.provider, self.engine, repository, shown, revision)
        ledgers = [r for r in shown["result"]["context"] if r["kind"] == "finding-ledger"
            and r["data"].get("gate") == "validation-review"]
        if ledgers:
            ledger = copy.deepcopy(ledgers[-1]["data"])
            ledger["subject_revision"] = revision
            result["records"].insert(0, {"record_id": f"validation-index-ledger-{revision}",
                "kind": "finding-ledger", "data": ledger})
        # The first full observation arms this unchanged validation state visit.
        # Preparation runs only provider/capture/checkpoint commands; it does
        # not transition the engine state, so repeating a full show before every
        # append only reparses the same context.  Keep one fresh show after the
        # append batch to prove the resulting context instead.
        for row in result["records"]:
            response = call(["append", "--record-id=" + row["record_id"], run_id,
                row["kind"], json.dumps(row["data"])])
            self._expect_status(response, "completed", event="append", state="validation")
        final = call(["show", "--view", "full", run_id])
        self._expect_status(final, "completed", event="show", state="validation")
        context_ids = {row["id"] for row in final["result"].get("context", [])}
        missing = [row["record_id"] for row in result["records"] if row["record_id"] not in context_ids]
        if missing:
            raise JourneyFailure(
                f"validation append batch omitted context records: {missing}",
                state="validation",
                event="show",
            )
        return result

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
        self._prepare_fixture_proof_commands(artifacts)
        profile = self._read_json(
            self.data_root / STITCHED_PROFILE_SUBPATH, f"{mutation} minimal profile"
        )
        # This focused checkpoint fixture retains the pre-reconciliation v3
        # graph so its report/checkpoint invalidation assertions remain about
        # that boundary rather than adding an unrelated document decision.
        profile["config_version"] = "minimal-10"
        # This checkpoint fixture is deliberately a focused v3 graph: draft
        # phases advance directly, while validation retains one aggregate
        # review so the existing final evidence/checkpoint assertions remain
        # meaningful without manufacturing the other nine gates' records.
        policies = profile.get("review_policies")
        if not isinstance(policies, dict):
            raise JourneyFailure(f"{mutation} minimal profile omitted review_policies")
        for gate in policies:
            policies[gate] = []
        policies["validation-review"] = [{
            "id": "intent-delivered",
            "description": "Checkpoint journey validation obligation",
            "example_prompt": "Judge intent-delivered only.",
            "review_stage": "aggregate",
            "required_authors": 1,
        }]
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
        initial_observed = call(["show", "--view", "full", run_id])
        self._expect_status(initial_observed, "completed", event="show", state="explore")
        for event, target in (
            ("intent-ready", "design"),
            ("design-ready", "plan"),
            ("plan-ready", "implement"),
        ):
            response = call(["event", run_id, event])
            self._expect_status(response, "completed", event=event, state=target)
            observed = call(["show", "--view", "full", run_id])
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

        proof = self._create_validation_fixture(call, run_id, repository, "checkpoint-v2-1")
        validation_ready = call(["event", run_id, "validation-ready"])
        self._expect_status(validation_ready, "completed", event="validation-ready", state="validation")
        validation_revision = proof["report"]["revision"]
        evidence = {
            "gate": "validation-review",
            "policy_id": "intent-delivered",
            "review_stage": "aggregate",
            "result": "pass",
            "findings": "",
            "author": {"name": f"checkpoint-reviewer-{mutation}", "kind": "script"},
            "subject": "validation-report.json",
            "subject_revision": validation_revision,
            "config_version": profile["config_version"],
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
        proof = self._create_validation_fixture(call, run_id, repository, "checkpoint-v2-2")
        validation_recovered = call(["event", run_id, "validation-ready"])
        self._expect_status(validation_recovered, "completed", event="validation-ready", state="validation")
        fresh_evidence = dict(evidence)
        fresh_evidence["subject_revision"] = proof["report"]["revision"]
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
        fresh_ledger["subject_revision"] = proof["report"]["revision"]
        fresh_ledger["author"] = {"name": f"checkpoint-driver-{mutation}-fresh", "kind": "agent"}
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
        shown = call(["show", "--view", "full", run_id])
        self._expect_status(shown, "completed", event="show", state="end")
        result = shown.get("result", {})
        if result.get("current_state") != "end" or result.get("lifecycle") != "final":
            raise JourneyFailure(f"{mutation} did not finish after checkpoint recovery: {shown}")

    def _run_checkpoint_scenarios(
        self, *, global_jobs: Optional[List[Dict[str, Any]]] = None
    ) -> None:
        """Run independent mutation fixtures under the existing bounded pool."""
        if global_jobs is None and getattr(self.args, "jobs", 2) == 1:
            for mutation in CHECKPOINT_MUTATIONS:
                self._run_checkpoint_case(mutation)
        else:
            assert self.run_dir is not None
            import proof_pool

            jobs = []
            job_root = self.run_dir / "checkpoint-pool-cases"
            spec_root = self.run_dir / "checkpoint-pool-jobs"
            spec_root.mkdir()
            worker = self._pool_worker_path()
            for mutation in CHECKPOINT_MUTATIONS:
                spec_path = spec_root / f"{mutation}.json"
                spec_path.write_text(
                    json.dumps(
                        {
                            "kind": "checkpoint",
                            "root": str(job_root / mutation),
                            "args": self._pool_args(),
                            "mutation": mutation,
                        },
                        indent=2,
                    )
                    + "\n",
                    encoding="utf-8",
                )
                jobs.append({"name": mutation, "command": [sys.executable, str(worker), str(spec_path)]})
            if global_jobs is not None:
                for job in jobs:
                    global_jobs.append(
                        {
                            "name": f"checkpoint-{job['name']}",
                            "command": job["command"],
                        }
                    )
                return
            report = proof_pool.run(
                jobs,
                root=self.run_dir / "checkpoint-pool",
                limit=self.args.jobs,
                timeout=self.args.job_timeout,
            )
            report_path = self.run_dir / "checkpoint-pool-report.json"
            report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
            expected_peak = min(self.args.jobs, len(CHECKPOINT_MUTATIONS))
            if report.get("status") != "passed" or report.get("peak_jobs") != expected_peak:
                raise JourneyFailure(
                    f"checkpoint pool failed; inspect {report_path}",
                    state="end",
                    event="checkpoint-scenarios",
                )
            if any(
                row.get("status") != "passed" or row.get("exit_code") != 0
                for row in report.get("jobs", [])
            ):
                raise JourneyFailure(
                    f"checkpoint pool omitted a completed case; inspect {report_path}",
                    state="end",
                    event="checkpoint-scenarios",
                )
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
        assert self.artifact_root is not None
        assert self.fixture_root is not None
        # The initial profile already contains all three early artifacts.  Keep
        # the existing missing-artifact negative case, but remove and restore
        # only the previously present intent before the first checked hop.
        intent_path = self.artifact_root / "intent.json"
        intent_bytes = intent_path.read_bytes()
        intent_path.unlink()
        self._expect_denial("intent-ready", "intent", "software-change-schema-invalid")
        intent_path.write_bytes(intent_bytes)
        # The primary full run is overlay-off.  Keep its authored artifact
        # set on the AC-N-only spine even though the shared historical
        # validation fixture contains one old Bookends citation in prose.
        validation_path = self.artifact_root / "validation-report.json"
        validation = self._replace_overlay_off_citation(
            self._read_json(validation_path, "overlay-off validation report")
        )
        validation_path.write_text(
            json.dumps(validation, indent=2) + "\n", encoding="utf-8"
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

        assert self.run_dir is not None
        assert self.repository_root is not None
        implementation_receipts = self.run_dir / "implementation-receipts"
        frozen_implement_binding = copy.deepcopy(self.work_slot_bindings["implement"])
        plan_document = self._read_json(self.artifact_root / "plan.json", "accepted plan")
        if any(
            not isinstance(task.get("criterion_ids"), list) or not task["criterion_ids"]
            for task in plan_document.get("tasks", [])
            if isinstance(task, dict)
        ):
            raise JourneyFailure(
                "full plan contains an implementation task without criterion_ids",
                state="plan",
                event="plan-ready",
            )
        plan_revision = str(plan_document["revision"])
        pre_repair_revision = self._fixture_revision("implementation-report.json")
        full_task_ids = [task["id"] for task in plan_document["tasks"]]
        self._assert_show("implement", "full-implementation-invoke")
        try:
            full_invocation = work_slot_journey.invoke_until_succeeded(
                self._engine_call(self.run_id, state="implement"),
                self.run_id,
                "implement",
                timeout_s=120.0,
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state="implement", event="invoke") from error
        if (
            full_invocation.get("binding") != frozen_implement_binding
            or "invocation_input" in full_invocation
            or [worker.get("assignment_id") for worker in full_invocation.get("inner_workers", [])]
            != full_task_ids
        ):
            raise JourneyFailure(
                f"full bound implementation did not preserve the frozen full-plan path: {full_invocation}",
                state="implement",
                event="invoke",
            )
        full_selection = self._read_json(
            Path(full_invocation["capture_dir"]) / "selection.json",
            "full implementation selection",
        )
        if full_selection.get("requested") is not None or full_selection.get("tasks") != full_task_ids:
            raise JourneyFailure(
                f"omitted invocation input did not record full-plan execution: {full_selection}",
                state="implement",
                event="invoke",
            )
        for task_id in (*full_task_ids, "summarizer"):
            if not (implementation_receipts / f"{task_id}.stdin").is_file():
                raise JourneyFailure(
                    f"full implementation omitted worker receipt {task_id}",
                    state="implement",
                    event="invoke",
                )
        full_receipts = self.run_dir / "implementation-receipts-full"
        implementation_receipts.rename(full_receipts)
        implementation_receipts.mkdir()

        implementation_report_path = self.artifact_root / "implementation-report.json"
        implementation_report = self._read_json(
            implementation_report_path, "full implementation report"
        )
        implementation_report["revision"] = self._fixture_revision(
            "implementation-report.json"
        )
        implementation_report_path.write_text(
            json.dumps(implementation_report, indent=2) + "\n", encoding="utf-8"
        )

        # Reconciliation is a checked provider boundary before the report/checkpoint/review proof.
        self._write_no_change_reconciliation("primary-reconciliation-r1")
        self._expect_allow("implementation-ready", "reconciliation")
        self._assert_show("reconciliation", "primary-reconciliation")
        self._expect_allow("reconciliation-ready", "implementation-review")
        # bookends:LE-94 — implementation-ready refuses report-only completion until the public checkpoint command binds the report to the selected Git tree.
        self._create_checkpoint("implementation")
        # First complete the ordinary implementation route so the accepted
        # pre-repair checkpoint is present in immutable proof history. Re-enter
        # implementation through the public check-free owning-phase route, then
        # exercise the no-honest-task finding at a fresh implementation review.
        self._pass_review(
            "implementation-review", "approved", "implementation-adversarial-review"
        )
        self._pass_review(
            "implementation-adversarial-review", "approved", "validation"
        )
        self._create_checkpoint("validation")
        self._expect_allow("validation-ready", "validation-review")
        self._expect_allow("revise-implementation", "implement")
        # A bound implementation-ready hop after backtracking requires a new
        # succeeded invocation for the new state visit. Preserve Package 3a's
        # full execution contract while preparing the repair proof.
        self._assert_show("implement", "repair-reentry-full-invoke")
        try:
            reentry_invocation = work_slot_journey.invoke_until_succeeded(
                self._engine_call(self.run_id, state="implement"),
                self.run_id,
                "implement",
                timeout_s=120.0,
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state="implement", event="invoke") from error
        if (
            reentry_invocation.get("binding") != frozen_implement_binding
            or "invocation_input" in reentry_invocation
            or [worker.get("assignment_id") for worker in reentry_invocation.get("inner_workers", [])]
            != full_task_ids
        ):
            raise JourneyFailure(
                f"repair re-entry widened or changed the frozen full-plan path: {reentry_invocation}",
                state="implement",
                event="invoke",
            )
        for task_id in (*full_task_ids, "summarizer"):
            if not (implementation_receipts / f"{task_id}.stdin").is_file():
                raise JourneyFailure(
                    f"repair re-entry omitted worker receipt {task_id}",
                    state="implement",
                    event="invoke",
                )
        reentry_receipts = self.run_dir / "implementation-receipts-reentry"
        implementation_receipts.rename(reentry_receipts)
        implementation_receipts.mkdir()
        implementation_report = self._read_json(
            implementation_report_path, "repair re-entry implementation report"
        )
        implementation_report["revision"] = self._fixture_revision(
            "implementation-report.json"
        )
        implementation_report_path.write_text(
            json.dumps(implementation_report, indent=2) + "\n", encoding="utf-8"
        )
        self._write_no_change_reconciliation("primary-reconciliation-r2")
        self._expect_allow("implementation-ready", "reconciliation")
        self._assert_show("reconciliation", "primary-reconciliation-reentry")
        self._expect_allow("reconciliation-ready", "implementation-review")
        self._create_checkpoint("implementation")
        # bookends:LE-108 — the public bound run refuses invalid no-task selections, captures one ad-hoc repair with no plan-task replay, refreshes proof, and continues through independent review and validation to terminal end.
        repair_revision = self._run_ad_hoc_repair_proof(
            plan_revision, frozen_implement_binding
        )
        self._pass_review(
            "implementation-adversarial-review",
            "approved",
            "validation",
            subject_revision=repair_revision,
            record_prefix="ad-hoc-",
        )
        accepted_history = {
            self._read_json(path, "accepted implementation proof history entry")
            .get("report", {})
            .get("revision")
            for path in (self.artifact_root / "implementation-proof-history").glob("*.json")
        }
        if not {pre_repair_revision, repair_revision}.issubset(accepted_history):
            raise JourneyFailure(
                f"implementation review did not record both pre/post accepted proofs: {accepted_history}",
                state=self.state,
                event="approved",
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
            "subject_revision": repair_revision,
            "author": {"name": "late-validation-driver", "kind": "agent"},
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
        if len(history_entries) != 2:
            raise JourneyFailure(
                f"implementation proof history had {len(history_entries)} entries before overwrite test",
                state=self.state,
                event="validation-ready",
            )
        accepted_candidates = [
            path
            for path in history_entries
            if self._read_json(path, "accepted implementation proof history entry")
            .get("report", {})
            .get("revision")
            == repair_revision
        ]
        if len(accepted_candidates) != 1:
            raise JourneyFailure(
                f"implementation proof history did not select the fresh repair proof: {accepted_candidates}",
                state=self.state,
                event="validation-ready",
            )
        accepted_path = accepted_candidates[0]
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

        validation_revision = self._fixture_revision("validation-report.json")
        repair_policy = self.profile["review_policies"]["validation-review"][0]["id"]
        repair_author = f"synthetic-validation-review-{repair_policy}-a"
        repair_finding = {
            "id": "F-focused-implementation-repair",
            "source": {
                "kind": "context-record",
                "id": "focused-repair-fail-evidence",
            },
            "policy_id": repair_policy,
            "statement": "The checked-transition task requires focused rework.",
            "disposition": "accepted",
            "reason": "The driver accepted the current validation finding and routed it to its implementation owner.",
            "owner_phase": "implementation",
            "task_ids": ["checked-transition-evaluator"],
            "review_axes": [repair_policy],
            "status": "unresolved",
        }
        failing_evidence = {
            "gate": "validation-review",
            "policy_id": repair_policy,
            "review_stage": self.profile["review_policies"]["validation-review"][0].get("review_stage", "aggregate"),
            "result": "fail",
            "findings": repair_finding["statement"],
            "author": {"name": repair_author, "kind": "script"},
            "subject": "validation-report.json",
            "subject_revision": validation_revision,
            "config_version": self.profile["config_version"],
        }
        self._assert_show("validation-review", "focused-repair-fail-evidence")
        failed_append = self._engine(
            [
                "append",
                "--record-id=focused-repair-fail-evidence",
                self.run_id,
                "review-evidence",
                json.dumps(failing_evidence, separators=(",", ":")),
            ],
            state="validation-review",
            event="append",
            axis=repair_policy,
        )
        self._expect_status(
            failed_append,
            "completed",
            event="append",
            axis=repair_policy,
            state="validation-review",
        )
        unresolved_ledger = {
            "schema_version": "1",
            "gate": "validation-review",
            "subject": "validation-report.json",
            "subject_revision": validation_revision,
            "author": {"name": "focused-repair-driver", "kind": "agent"},
            "findings": [repair_finding],
        }
        self._assert_show("validation-review", "focused-repair-ledger")
        ledger_append = self._engine(
            [
                "append",
                "--record-id=focused-repair-ledger-unresolved",
                self.run_id,
                "finding-ledger",
                json.dumps(unresolved_ledger, separators=(",", ":")),
            ],
            state="validation-review",
            event="append",
            axis=repair_policy,
        )
        self._expect_status(
            ledger_append,
            "completed",
            event="append",
            axis=repair_policy,
            state="validation-review",
        )
        self._assert_show("validation-review", "focused-repair-denial")
        repair_denial = self._event("approved", repair_policy)
        self._expect_status(
            repair_denial,
            "rejected",
            event="approved",
            axis=repair_policy,
            state="validation-review",
        )
        if repair_denial.get("code") == "software-change-finding-ledger-invalid":
            raise JourneyFailure(
                f"accepted focused repair finding produced an invalid ledger: {repair_denial}",
                state="validation-review",
                event="approved",
                axis=repair_policy,
            )
        self._assert_show("validation-review", "focused-repair-revise")
        self._expect_allow("revise-implementation", "implement")

        focused_input = {
            "plan_revision": plan_revision,
            "task_roots": ["checked-transition-evaluator"],
        }
        focused_task_ids = [
            "checked-transition-evaluator",
            "acceptance-proof",
            "authoritative-doc-integration",
        ]
        pre_focused_show = self._assert_show(
            "implement", "focused-implementation-invoke"
        )
        standing_before_focused = {
            result.get("assignment_id")
            for result in pre_focused_show.get("change_report", {}).get(
                "plan_task_results", []
            )
            if result.get("standing") is True
        }
        required_standing = {
            dependency
            for task in plan_document["tasks"]
            if task["id"] in focused_task_ids
            for dependency in task.get("dependencies", [])
            if dependency not in focused_task_ids
        }
        if not required_standing or not required_standing.issubset(
            standing_before_focused
        ):
            raise JourneyFailure(
                "focused invocation prerequisites were not visibly standing before invoke: "
                f"required={sorted(required_standing)} standing={sorted(standing_before_focused)}",
                state="implement",
                event="show",
            )
        try:
            focused_invocation = work_slot_journey.invoke_until_succeeded(
                self._engine_call(self.run_id, state="implement"),
                self.run_id,
                "implement",
                timeout_s=120.0,
                invoke_args=[
                    "--input",
                    json.dumps(focused_input, separators=(",", ":")),
                ],
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error), state="implement", event="invoke") from error
        if (
            focused_invocation.get("binding") != frozen_implement_binding
            or focused_invocation.get("invocation_input") != focused_input
            or [
                worker.get("assignment_id")
                for worker in focused_invocation.get("inner_workers", [])
            ]
            != focused_task_ids
        ):
            raise JourneyFailure(
                f"focused bound implementation widened or changed its frozen binding: {focused_invocation}",
                state="implement",
                event="invoke",
            )
        focused_selection = self._read_json(
            Path(focused_invocation["capture_dir"]) / "selection.json",
            "focused implementation selection",
        )
        if (
            focused_selection.get("requested")
            != ["checked-transition-evaluator"]
            or focused_selection.get("tasks") != focused_task_ids
        ):
            raise JourneyFailure(
                f"focused selection record did not identify the selected closure: {focused_selection}",
                state="implement",
                event="invoke",
            )
        focused_receipt_ids = sorted(
            path.name.removesuffix(".stdin")
            for path in implementation_receipts.glob("*.stdin")
        )
        if focused_receipt_ids != sorted([*focused_task_ids, "summarizer"]):
            raise JourneyFailure(
                f"focused invocation started unrelated or omitted workers: {focused_receipt_ids}",
                state="implement",
                event="invoke",
            )
        focused_packet = work_slot_journey.parse_graph_runner_stdin(
            (
                implementation_receipts
                / "checked-transition-evaluator.stdin"
            ).read_text(encoding="utf-8")
        )["task"]
        routed_finding_ids = [
            finding.get("id")
            for finding in focused_packet.get("finding_context", [])
            if isinstance(finding, dict)
        ]
        if repair_finding["id"] not in routed_finding_ids:
            raise JourneyFailure(
                f"focused task packet omitted the routed finding: {focused_packet}",
                state="implement",
                event="invoke",
            )
        durable_invocations = self._assert_show(
            "implement", "focused-implementation-durable"
        ).get("work_slot_invocations", [])
        durable_by_id = {
            invocation.get("invocation_id"): invocation
            for invocation in durable_invocations
            if isinstance(invocation, dict)
        }
        durable_full = durable_by_id.get(full_invocation["invocation_id"])
        durable_focused = durable_by_id.get(focused_invocation["invocation_id"])
        if (
            not isinstance(durable_full, dict)
            or "invocation_input" in durable_full
            or not isinstance(durable_focused, dict)
            or durable_focused.get("invocation_input") != focused_input
            or durable_full.get("capture_dir") == durable_focused.get("capture_dir")
            or durable_full.get("binding") != durable_focused.get("binding")
        ):
            raise JourneyFailure(
                "durable show did not distinguish full and focused invocations while preserving the binding",
                state="implement",
                event="show",
            )
        full_task_results = durable_full.get("change_report", {}).get(
            "plan_task_results", []
        )
        focused_task_results = durable_focused.get("change_report", {}).get(
            "plan_task_results", []
        )
        if [result.get("assignment_id") for result in full_task_results] != full_task_ids:
            raise JourneyFailure(
                f"durable full invocation omitted or reordered plan-task results: {full_task_results}",
                state="implement",
                event="show",
            )
        if [result.get("assignment_id") for result in focused_task_results] != focused_task_ids:
            raise JourneyFailure(
                "durable focused invocation included unrelated or omitted plan-task results: "
                f"{focused_task_results}",
                state="implement",
                event="show",
            )
        for label, task_results in (
            ("full", full_task_results),
            ("focused", focused_task_results),
        ):
            for result in task_results:
                effect = result.get("dimensions", {}).get("repository_effect", {})
                if (
                    effect.get("changed") is not False
                    or effect.get("recorded") != {"kind": "dummy"}
                    or effect.get("current") != {"kind": "dummy"}
                ):
                    raise JourneyFailure(
                        f"durable {label} plan-task result omitted its repository effect: {result}",
                        state="implement",
                        event="show",
                    )

        focused_report_revision = (
            self._fixture_revision("implementation-report.json") + "-focused"
        )
        focused_report = self._read_json(
            implementation_report_path, "focused implementation report"
        )
        focused_report["revision"] = focused_report_revision
        focused_report["summary"] = "dummy focused repair summarizer wrote this report"
        implementation_report_path.write_text(
            json.dumps(focused_report, indent=2) + "\n", encoding="utf-8"
        )
        self._write_no_change_reconciliation("primary-reconciliation-focused")
        self._expect_allow("implementation-ready", "reconciliation")
        self._assert_show("reconciliation", "primary-reconciliation-focused")
        self._expect_allow("reconciliation-ready", "implementation-review")
        self._create_checkpoint("implementation")
        self._pass_review(
            "implementation-review",
            "approved",
            "implementation-adversarial-review",
            subject_revision=focused_report_revision,
            record_prefix="focused-",
        )
        self._pass_review(
            "implementation-adversarial-review",
            "approved",
            "validation",
            subject_revision=focused_report_revision,
            record_prefix="focused-",
        )

        # Completed focused work discharges the exact historical source before
        # the new index is admitted; a revision bump cannot omit its finding.
        repaired_finding = {**repair_finding, "status": "resolved",
            "reason": "Focused task and dependent captures completed; independent implementation reviews reconfirmed the repair."}
        self._append_ledger_snapshot_for(self.run_id,
            {**unresolved_ledger, "findings": [repaired_finding]},
            record_id="focused-repair-resolution-before-validation", state="validation", axis=repair_policy)
        self._create_checkpoint("validation")
        self._expect_allow("validation-ready", "validation-review")
        self._assert_show("validation-review", "focused-repair-pass-evidence")
        self._append_evidence("validation-review")
        resolved_finding = dict(repair_finding)
        resolved_finding["status"] = "resolved"
        resolved_ledger = {
            **unresolved_ledger,
            "subject_revision": self._fixture_revision("validation-report.json"),
            "author": {"name": "focused-repair-driver-confirmation", "kind": "agent"},
            "findings": [resolved_finding],
        }
        self._assert_show("validation-review", "focused-repair-resolved-ledger")
        resolved_append = self._engine(
            [
                "append",
                "--record-id=focused-repair-ledger-resolved",
                self.run_id,
                "finding-ledger",
                json.dumps(resolved_ledger, separators=(",", ":")),
            ],
            state="validation-review",
            event="append",
            axis=repair_policy,
        )
        self._expect_status(
            resolved_append,
            "completed",
            event="append",
            axis=repair_policy,
            state="validation-review",
        )
        self._assert_show("validation-review", "focused-repair-reviewed")
        self._expect_allow("approved", "validation-adversarial-review")
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
            # The predecessor outcome prose is now retained companion evidence,
            # not an executable report. Keep its token-only negative proof and
            # additionally require the live v2 index to resolve genuine records.
            companion = self._read_json(self.fixture_root / "validation-evidence-2026-08-12.json", "retained outcome evidence")
            assert_semantic_outcome_proof_contract(plan, companion, scenario_source)
            context = {row["id"]: row for row in shown["context"]}
            assert {row["criterion_id"] for row in validation["criteria"]} == {row["id"] for row in intent["acceptance"]}
            for row in validation["criteria"]:
                for record_id in row["verdict_ids"]:
                    verdict = context[record_id]
                    assert verdict["kind"] == "criterion-verdict"
                    assert verdict["data"]["criterion_id"] == row["criterion_id"]
                    assert verdict["data"]["subject_revision"] == validation["revision"]
                    assert verdict["data"]["result"] == "pass"
                    assert verdict["data"]["author"] != validation["author"]
            assert all(context[id]["kind"] == "goal-verdict" for id in validation["goal_verdict_ids"])
            assert all(context[id]["kind"] == "command-evidence" for id in validation["command_evidence_ids"])
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

        # The completed primary run is the dependency barrier for the
        # independent tail fixtures.  The tail is scheduled by run() so these
        # same-run checks remain serial while unrelated isolated cases can use
        # the existing bounded proof pool.

    @staticmethod
    def _emit_global_pool_failure_diagnostics(
        report: Any, report_path: Path, expected_names: Sequence[str]
    ) -> None:
        """Keep the original pool failure while exposing its retained evidence."""
        def render(value: Any) -> str:
            try:
                return json.dumps(value, sort_keys=True, default=str)
            except Exception as error:  # pragma: no cover - defensive diagnostics only
                return f"<unrenderable: {error}>"

        def capture_tail(path: Any) -> str:
            if not path:
                return "<missing capture: no capture path recorded>"
            try:
                capture = Path(path)
                with capture.open("rb") as stream:
                    stream.seek(0, os.SEEK_END)
                    size = stream.tell()
                    start = max(0, size - 4096)
                    stream.seek(start)
                    value = stream.read(4096)
                text = value.decode("utf-8", "replace")
                if start:
                    text = "<capture tail truncated to 4096 bytes>\n" + text
                return text or "<empty capture>"
            except (OSError, TypeError, ValueError) as error:
                return f"<missing/unreadable capture {path!r}: {error}>"

        rows = report.get("jobs", []) if isinstance(report, dict) else []
        rows = [row for row in rows if isinstance(row, dict)] if isinstance(rows, list) else []
        actual_names = {
            row.get("name") for row in rows if isinstance(row.get("name"), str)
        }
        missing_names = [name for name in expected_names if name not in actual_names]
        failed_rows = [
            row
            for row in rows
            if row.get("status") != "passed" or row.get("exit_code") != 0
        ]
        failed_names = [row.get("name") for row in failed_rows]
        print(f"global full-source proof pool report: {report_path}", file=sys.stderr)
        print(f"global full-source proof pool failed jobs: {render(failed_names)}", file=sys.stderr)
        print(f"global full-source proof pool missing jobs: {render(missing_names)}", file=sys.stderr)
        if isinstance(report, dict):
            print(
                "global full-source proof pool status/cleanup/error: "
                + render({
                    "status": report.get("status"),
                    "cleanup": report.get("cleanup"),
                    "error": report.get("error"),
                }),
                file=sys.stderr,
            )

        for row in failed_rows:
            result = row.get("result")
            error = row.get("error")
            if error is None and isinstance(result, dict):
                error = result.get("error")
            name = row.get("name")
            print(
                "global full-source proof pool row: "
                + render({
                    "name": name,
                    "status": row.get("status"),
                    "exit_code": row.get("exit_code"),
                    "cleanup": row.get("cleanup"),
                    "error": error,
                }),
                file=sys.stderr,
            )
            for stream_name in ("stdout", "stderr"):
                path = row.get(stream_name)
                print(
                    f"global full-source proof pool {name!r} {stream_name} tail "
                    f"(capture={path!r}):",
                    file=sys.stderr,
                )
                tail = capture_tail(path)
                print(tail, file=sys.stderr, end="")
                if not tail.endswith("\n"):
                    print(file=sys.stderr)

        for name in missing_names:
            print(
                f"global full-source proof pool missing row: {name!r}; "
                "stdout/stderr capture unavailable because the job was not recorded",
                file=sys.stderr,
            )

    def _run_global_tail_proof(self) -> None:
        """Run independent full-journey fixtures through one bounded pool."""
        assert self.mode == "source" and self.depth == "full"
        assert self.run_dir is not None
        assert self.database is not None
        assert self.provider_config is not None
        assert self.profile_path is not None
        assert self.profile_source is not None
        assert self.artifact_root is not None
        import proof_pool
        from recovery_journey import SCENARIOS

        global_jobs: List[Dict[str, Any]] = []
        # Guidance owns a self-test pool of its own. Run it before the shared
        # pool, then enqueue its known-long first batch before the other
        # independent tail cases so the existing FIFO pool starts that work
        # immediately. The batch still preserves the measured operational cap.
        self._run_operational_ux_cases(global_jobs=global_jobs)
        # bookends:LE-127 — recovery_batch calls work_slot_journey's
        # assert_projected_fan_out_capture for compact stdin and full evidence;
        # the inventory below retains the invalid-evidence refusal and all
        # recovery cases while the full source traversal keeps its graph proof.
        # Recovery scenarios are independent fixture roots. Keep their public
        # --scenario entry points and full inventory; the same pool interleaves
        # them with every other independent tail case.
        self._run_recovery_inventory(global_jobs=global_jobs)
        self._run_successor_route_proof(
            implementation_revision=self._fixture_revision("implementation-report.json"),
            global_jobs=global_jobs,
        )
        # The stitched run intentionally reads the completed primary database.
        # It is the only cross-run tail case; the parent is quiescent while the
        # pool runs, so it cannot race a primary mutation.
        self._append_global_pool_job(
            global_jobs,
            name="stitched",
            kind="stitched",
            root=self.run_dir / "stitched-pool-case",
            parent_run_dir=self.run_dir,
            database=self.database,
            provider_config=self.provider_config,
            artifact_root=self.artifact_root,
            profile_path=self.profile_path,
            profile_source=self.profile_source,
            repository_root=self.repository_root,
            run_id=self.run_id,
            work_slot_bindings=self.work_slot_bindings,
        )
        self._append_global_pool_job(
            global_jobs,
            name="engine-boundary",
            kind="engine-boundary",
            root=self.run_dir / "engine-boundary-pool-case",
        )
        self._append_global_pool_job(
            global_jobs,
            name="reconciliation",
            kind="reconciliation",
            root=self.run_dir / "reconciliation-pool-case",
        )
        self._run_dummy_worker_proofs(global_jobs=global_jobs)
        self._append_global_pool_job(
            global_jobs,
            name="package-7b",
            kind="package-7b",
            root=self.run_dir / "package-7b-pool-case",
        )
        self._run_checkpoint_scenarios(global_jobs=global_jobs)
        self._run_bookends_enabled_source(global_jobs=global_jobs)

        expected_names = {job["name"] for job in global_jobs}
        if len(expected_names) != len(global_jobs):
            raise JourneyFailure("global full-source proof inventory contains duplicate names")
        report_root = self.run_dir / "global-proof-pool"
        report_path = self.run_dir / "global-proof-pool-report.json"
        try:
            report = proof_pool.run(
                global_jobs,
                root=report_root,
                limit=self.args.jobs,
                timeout=self.args.job_timeout,
            )
        except proof_pool.PoolFailure as error:
            self._emit_global_pool_failure_diagnostics(
                {"status": "failed", "jobs": [], "error": str(error)},
                report_path,
                sorted(expected_names),
            )
            raise JourneyFailure(f"global full-source proof pool failed: {error}") from error
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        rows = report.get("jobs", [])
        by_name = {
            row.get("name"): row
            for row in rows
            if isinstance(row, dict) and isinstance(row.get("name"), str)
        }
        if (
            report.get("status") != "passed"
            or set(by_name) != expected_names
            or any(
                by_name[name].get("status") != "passed"
                or by_name[name].get("exit_code") != 0
                for name in expected_names
            )
        ):
            self._emit_global_pool_failure_diagnostics(report, report_path, sorted(expected_names))
            raise JourneyFailure(
                f"global full-source proof pool did not complete its inventory; inspect {report_path}",
                state="end",
                event="global-proof-pool",
            )

        def group_report(path: Path, prefix: str, inventory: Sequence[str]) -> None:
            selected = [row for row in rows if str(row.get("name", "")).startswith(prefix)]
            path.write_text(
                json.dumps(
                    {
                        "status": "passed",
                        "inventory": list(inventory),
                        "jobs": selected,
                        "global_pool": str(report_path),
                        "global_pool_peak_jobs": report.get("peak_jobs"),
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )

        group_report(
            self.run_dir / "recovery-inventory-pool-report.json",
            "recovery-",
            SCENARIOS,
        )
        group_report(
            self.run_dir / "successor-route-pool-report.json",
            "route-",
            [f"route-{index:02d}-{event}" for index, (_, event, _) in enumerate(SUCCESSOR_ROUTE_CASES, 1)],
        )
        group_report(
            self.run_dir / "checkpoint-pool-report.json",
            "checkpoint-",
            [f"checkpoint-{mutation}" for mutation in CHECKPOINT_MUTATIONS],
        )
        group_report(
            self.run_dir / "criterion-overlay-pool-report.json",
            "criterion-overlay-",
            [
                "criterion-overlay-off",
                "criterion-overlay-on-candidate",
                "criterion-overlay-on-not-applicable",
            ],
        )

        for name in SCENARIOS:
            row = by_name[f"recovery-{name}"]
            stdout = Path(row["stdout"]).read_text(encoding="utf-8", errors="replace")
            marker = RECOVERY_COMPLETION_MARKERS[name]
            if marker not in stdout:
                raise JourneyFailure(
                    f"recovery scenario {name} omitted its completion marker; inspect {report_path}",
                    state="end",
                    event="recovery-inventory",
                )
        (self.run_dir / "recovery-inventory.json").write_text(
            json.dumps({"status": "passed", "scenarios": list(SCENARIOS)}, indent=2) + "\n",
            encoding="utf-8",
        )

        reconciliation_row = by_name["reconciliation"]
        reconciliation_stdout = Path(reconciliation_row["stdout"]).read_text(
            encoding="utf-8", errors="replace"
        )
        if "reconciliation journey passed:" not in reconciliation_stdout:
            raise JourneyFailure(
                f"reconciliation public scenarios omitted their completion marker; inspect {report_path}",
                state="end",
                event="reconciliation",
            )
        reconciliation_path = (
            self.run_dir
            / "reconciliation-pool-case"
            / "reconciliation-journey"
            / "reconciliation-journey-proof.json"
        )
        if not reconciliation_path.is_file():
            raise JourneyFailure(
                f"reconciliation public scenarios omitted proof artifact {reconciliation_path}",
                state="end",
                event="reconciliation",
            )
        self.reconciliation_proof = reconciliation_path

        overlay_paths = getattr(self, "_global_overlay_paths", None)
        if not isinstance(overlay_paths, dict) or any(
            not isinstance(path, Path) or not path.is_file()
            for path in overlay_paths.values()
        ):
            raise JourneyFailure(
                f"criterion overlay omitted proof artifacts; inspect {report_path}",
                state="end",
                event="criterion-overlay",
            )

        operational_cases = ("monitor", "capture", "summary", "guidance", "delivery", "bookends")
        outcomes = dict(self._operational_ux_outcomes)
        operational_rows = []
        for batch_name, cases in (
            ("operational-batch-1", ("monitor", "capture", "summary")),
            ("operational-batch-2", ("delivery", "bookends")),
        ):
            stdout = Path(by_name[batch_name]["stdout"]).read_text(
                encoding="utf-8", errors="replace"
            )
            found: Dict[str, Dict[str, Any]] = {}
            for line in stdout.splitlines():
                try:
                    value = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if isinstance(value, dict) and value.get("case") in cases:
                    found[value["case"]] = value
            for case in cases:
                outcome = found.get(case)
                if not isinstance(outcome, dict) or outcome.get("status") != "passed":
                    raise JourneyFailure(
                        f"operational UX {case} omitted its public outcome; inspect {report_path}",
                        state="end",
                        event="operational-ux",
                    )
                outcomes[case] = outcome
                operational_rows.append(
                    {
                        "name": case,
                        "status": "passed",
                        "batch": batch_name,
                        "stdout": by_name[batch_name]["stdout"],
                        "stderr": by_name[batch_name]["stderr"],
                    }
                )
                print(f"operational UX {case} passed; captures: {outcome.get('artifact_root')}")
        if set(outcomes) != set(operational_cases):
            raise JourneyFailure(
                f"operational UX inventory incomplete: {sorted(outcomes)}",
                state="end",
                event="operational-ux",
            )
        (self.run_dir / "operational-ux-pool-report.json").write_text(
            json.dumps(
                {
                    "status": "passed",
                    "inventory": list(operational_cases),
                    "jobs": operational_rows,
                    "global_pool": str(report_path),
                    "global_pool_peak_jobs": min(self.args.jobs, 2),
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        (self.run_dir / "operational-ux-results.json").write_text(
            json.dumps(outcomes, indent=2) + "\n", encoding="utf-8"
        )
        self._operational_ux_outcomes = outcomes

        self.stitched_run_id = STITCHED_RUN_ID
        self.engine_boundary_proof = list(ENGINE_BOUNDARY_PROOF)
        self.dummy_worker_proof = list(DUMMY_WORKER_PROOF)
        self.package_7b_proof = list(PACKAGE_7B_PROOF)
        self.criterion_overlay_proof = dict(overlay_paths)
        self.bookends_proof = overlay_paths["overlay_on_candidate"]
        self.proof_pool_report = report

        print(f"successor route proof passed: {len(SUCCESSOR_ROUTE_CASES)} fresh runs")
        print(
            "stitched software-change journey passed: same topology, distinct frozen policies, wrong-run evidence denied"
        )
        engine_markers = (
            "LE-2 topology scenarios passed:",
            "LE-13 final-state scenario passed:",
            "LE-14 initially-final scenario passed:",
            "LE-15 terminal-mutation scenario passed:",
            "LE-11 frozen-run scenario passed:",
            "LE-12 unsupported-action scenario passed:",
            "review-revision scenario passed:",
            "LE-76 binding-start scenarios passed:",
            "concurrency scenarios passed:",
        )
        engine_stdout = Path(by_name["engine-boundary"]["stdout"]).read_text(
            encoding="utf-8", errors="replace"
        )
        for line in engine_stdout.splitlines():
            if line.startswith(engine_markers):
                print(line)
        print(
            "dummy worker proofs passed: shipped profiles, graph-runner, fan-out, "
            "preview-bindings fail-closed, missing -e warning, default sandbox argv, bound heartbeats, "
            "overrun wait/cancel/retry, bounded reviewer retry/exhaustion, selected-attempt linkage, observation guard, "
            "subset invoke, change report, applicability, and content-agreement refusal, stdin-exec, graph working-directory cwd/marker proof, implementation finding routing, "
            "bound operating-context inspection, overlay-running invocation-progress, "
            "omitted vs set --max-active, progress-query overlay-untouched"
        )
        print("contracted fan-out failure")
        print(
            "Package 7b review-candidates scenario passed: selected retry, exhausted assignment, "
            "raw capture preservation, deterministic repeated inspection, inert-before-records, "
            "and driver-action-afterward progression"
        )
        print(
            "checkpoint source scenarios passed: report-only denial, seven implementation/validation "
            "state invalidations, validation recovery, and current-tree final proof"
        )
        print(
            "overlay-off criterion spine scenario passed: AC-N only; no PRD disposition, "
            "candidate, liveness, citation, or Green claim"
        )
        print(
            "overlay-on criterion scenarios passed: one disposition per criterion, "
            "candidate blocks Bookends-enabled final completion, not-applicable does not waive or "
            "fulfill its criterion"
        )
        print("reconciliation journey passed: successful edit, justified no-change, unresolved discrepancy, missing authorization, and Bookends on/off")
        print("full recovery inventory passed: " + ", ".join(SCENARIOS))

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
            database, ["show", "--view", "full", run_id], cwd=self.data_root
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
            ["show", "--view", "full", "le2-invalid-run"],
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
            database, ["show", "--view", "full", run_id], cwd=self.data_root
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
        # Sample time is observation metadata, not a terminal-run mutation.
        if "observed_at" in after_show:
            after_show["observed_at"] = before_show["observed_at"]
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
        # The state-race cases intentionally cover state/lifecycle staleness;
        # context-only append semantics are covered by the public engine tests.
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
                ["show", "--view", "full", name],
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

    def _run_package_7b_review_candidates_scenario(self) -> None:
        """Prove the read-only bound-review candidate pipe end to end."""
        if self.mode != "source":
            raise JourneyFailure("Package 7b candidate scenario is source-only", state=self.state)
        assert self.run_dir is not None
        assert self.provider is not None
        assert self.profile_source is not None
        assert self.fixture_root is not None
        scenario_dir = self.run_dir / "package-7b-review-candidates"
        scenario_dir.mkdir(parents=True, exist_ok=True)

        axis = "package-7b-selected"
        ready_author = "package-7b-ready-reviewer"
        exhausted_author = "package-7b-exhausted-reviewer"
        profile = copy.deepcopy(self.profile)
        policies = profile.get("review_policies")
        if not isinstance(policies, dict):
            raise JourneyFailure("Package 7b profile omitted review_policies", state="design-review")
        for gate in policies:
            policies[gate] = []
        policies["design-review"] = [
            {
                "id": axis,
                "description": "Package 7b selected candidate proof",
                "example_prompt": "Judge package-7b-selected only.",
                "review_stage": "aggregate",
                "required_authors": 1,
            }
        ]
        profile_path = scenario_dir / "profile.json"
        _write_json(profile_path, profile)

        worker_script = scenario_dir / "review-worker.py"
        worker_script.write_text(
            "#!/usr/bin/env python3\n"
            "import argparse, json, sys\n"
            "from pathlib import Path\n"
            "parser = argparse.ArgumentParser()\n"
            "parser.add_argument('--mode', choices=('ready', 'exhausted'), required=True)\n"
            "parser.add_argument('--state', required=True)\n"
            "parser.add_argument('--author', required=True)\n"
            "args = parser.parse_args()\n"
            "sys.stdin.buffer.read()\n"
            "state = Path(args.state)\n"
            "count = int(state.read_text()) if state.exists() else 0\n"
            "state.write_text(str(count + 1))\n"
            "if args.mode == 'ready' and count == 0:\n"
            "    sys.stdout.write('{\\\"axis\\\":\\\"malformed-first-attempt\\\"}')\n"
            "else:\n"
            "    result = {'review_stage': 'aggregate', 'axis': 'package-7b-selected', 'author': {'name': args.author, 'kind': 'script'}, 'result': 'pass', 'findings': ''}\n"
            "    if args.mode == 'exhausted':\n"
            "        result['axis'] = 'always-invalid'\n"
            "    sys.stdout.write(json.dumps(result, separators=(',', ':')))\n",
            encoding="utf-8",
        )
        worker_script.chmod(0o755)

        def output_schema(author: str) -> Dict[str, Any]:
            return {
                "type": "object",
                "additionalProperties": False,
                "required": ["review_stage", "axis", "author", "result", "findings"],
                "properties": {
                    "review_stage": {"type": "string", "const": "aggregate"},
                    "axis": {"type": "string", "const": axis},
                    "author": {
                        "type": "object",
                        "additionalProperties": False,
                        "required": ["name", "kind"],
                        "properties": {
                            "name": {"type": "string"},
                            "kind": {"type": "string"},
                        },
                        "const": {"name": author, "kind": "script"},
                    },
                    "result": {"type": "string", "const": "pass"},
                    "findings": {"type": "string", "const": ""},
                },
            }

        workers = [
            {
                "command": sys.executable,
                "args": [
                    str(worker_script),
                    "--mode",
                    "ready",
                    "--state",
                    str(scenario_dir / "ready-attempt-count"),
                    "--author",
                    ready_author,
                ],
                "preamble": "Package 7b selected reviewer",
                "full_output_schema": output_schema(ready_author),
            },
            {
                "command": sys.executable,
                "args": [
                    str(worker_script),
                    "--mode",
                    "exhausted",
                    "--state",
                    str(scenario_dir / "exhausted-attempt-count"),
                    "--author",
                    exhausted_author,
                ],
                "preamble": "Package 7b exhausted reviewer",
                "full_output_schema": output_schema(exhausted_author),
            },
        ]
        binding = work_slot_journey.fan_out_binding(engine=self.engine, workers=workers)
        engine_call, artifact_root, frozen_profile = work_slot_journey._start_isolated_software_change(
            engine=self.engine,
            provider=self.provider,
            profile_source=profile_path,
            fixture_root=self.fixture_root,
            work_dir=scenario_dir / "run",
            run_id="package-7b-review-candidates",
            extra_bindings={"design-review": binding},
        )
        run_id = "package-7b-review-candidates"
        database = scenario_dir / "run" / "loop.sqlite"

        def wait_for_terminal(invocation_id: str) -> Dict[str, Any]:
            deadline = time.monotonic() + 30.0
            last: Optional[Dict[str, Any]] = None
            while time.monotonic() < deadline:
                response = engine_call(["show", "--view", "full", run_id])
                if response.get("status") != "completed":
                    raise JourneyFailure(
                        f"Package 7b show failed while waiting: {response}",
                        state="design-review",
                        event="show",
                    )
                projection = response.get("result", {})
                invocations = projection.get("work_slot_invocations", [])
                last = next(
                    (
                        item
                        for item in invocations
                        if isinstance(item, dict)
                        and item.get("invocation_id") == invocation_id
                    ),
                    None,
                )
                if last is not None and last.get("status") in {"succeeded", "failed"}:
                    if last.get("completed_at") is not None:
                        return last
                time.sleep(0.05)
            raise JourneyFailure(
                f"Package 7b invocation did not finish: {last}",
                state="design-review",
                event="invoke",
            )

        def show_only() -> tuple[bytes, Dict[str, Any]]:
            completed = subprocess.run(
                [str(self.engine), "--database", str(database), "--json", "show", "--view", "full", run_id],
                cwd=self.data_root,
                capture_output=True,
                check=False,
            )
            if completed.returncode != 0:
                raise JourneyFailure(
                    "Package 7b explicit full show failed: "
                    + completed.stderr.decode("utf-8", "replace"),
                    state="design-review",
                    event="show",
                )
            try:
                envelope = json.loads(completed.stdout)
            except json.JSONDecodeError as error:
                raise JourneyFailure(
                    f"Package 7b explicit full show returned non-JSON: {error}",
                    state="design-review",
                    event="show",
                ) from error
            if not isinstance(envelope, dict) or envelope.get("status") != "completed":
                raise JourneyFailure(
                    f"Package 7b explicit full show was not completed: {envelope}",
                    state="design-review",
                    event="show",
                )
            return completed.stdout, envelope

        def project_candidates(show_bytes: bytes) -> tuple[bytes, Dict[str, Any]]:
            # This is the public pipe: loop-engine --json show RUN | software-change review-candidates.
            completed = subprocess.run(
                [str(self.provider), "review-candidates"],
                input=show_bytes,
                capture_output=True,
                check=False,
            )
            if completed.returncode != 0:
                raise JourneyFailure(
                    "Package 7b review-candidates failed: "
                    + completed.stderr.decode("utf-8", "replace"),
                    state="design-review",
                    event="review-candidates",
                )
            if completed.stderr:
                raise JourneyFailure(
                    f"Package 7b review-candidates wrote stderr: {completed.stderr!r}",
                    state="design-review",
                    event="review-candidates",
                )
            try:
                document = json.loads(completed.stdout)
            except json.JSONDecodeError as error:
                raise JourneyFailure(
                    f"Package 7b candidate output returned non-JSON: {error}",
                    state="design-review",
                    event="review-candidates",
                ) from error
            if not isinstance(document, dict):
                raise JourneyFailure(
                    f"Package 7b candidate output was not an object: {document}",
                    state="design-review",
                    event="review-candidates",
                )
            return completed.stdout, document

        def state_context_view(envelope: Mapping[str, Any]) -> Dict[str, Any]:
            result = envelope.get("result")
            if not isinstance(result, dict):
                raise JourneyFailure(f"Package 7b show omitted result: {envelope}")
            return {
                name: result.get(name)
                for name in (
                    "current_state",
                    "lifecycle",
                    "initial_input",
                    "context",
                    "requestable_events",
                    "latest_evaluations",
                )
            }

        # Reach a single configured review axis through the real engine and provider.
        work_slot_journey.invoke_until_succeeded(
            engine_call, run_id, "intent-draft", timeout_s=20.0
        )
        work_slot_journey._expect_event_state(engine_call, run_id, "intent-ready", "design")
        work_slot_journey._expect_event_state(
            engine_call, run_id, "design-ready", "design-review"
        )
        started = engine_call(["invoke", run_id, "design-review"])
        if started.get("status") != "completed":
            raise JourneyFailure(
                f"Package 7b bound review invoke failed: {started}",
                state="design-review",
                event="invoke",
            )
        first_invocation_id = started.get("result", {}).get("invocation_id")
        first_capture_value = started.get("result", {}).get("capture_dir")
        if not isinstance(first_invocation_id, str) or not isinstance(first_capture_value, str):
            raise JourneyFailure(
                f"Package 7b invoke omitted invocation identity: {started}",
                state="design-review",
                event="invoke",
            )
        first_capture = Path(first_capture_value)
        first_overlay = wait_for_terminal(first_invocation_id)
        if first_overlay.get("status") != "failed":
            raise JourneyFailure(
                f"Package 7b exhausted assignment did not fail the overlay: {first_overlay}",
                state="design-review",
                event="invoke",
            )
        first_workers = first_overlay.get("inner_workers")
        if not isinstance(first_workers, list) or len(first_workers) != 2:
            raise JourneyFailure(
                f"Package 7b failed overlay omitted both assignment results: {first_overlay}",
                state="design-review",
                event="invoke",
            )
        attempt_count_paths = {
            "ready": scenario_dir / "ready-attempt-count",
            "exhausted": scenario_dir / "exhausted-attempt-count",
        }

        def read_attempt_counts(stage: str) -> Dict[str, bytes]:
            counts = {}
            for name, path in attempt_count_paths.items():
                try:
                    value = path.read_bytes()
                except OSError as error:
                    raise JourneyFailure(
                        f"Package 7b could not read {name} worker attempt sentinel {path} "
                        f"{stage}: {error}",
                        state="design-review",
                        event="review-candidates",
                    ) from error
                counts[name] = value
            return counts

        first_attempt_counts = read_attempt_counts("after the first bound invocation")
        if any(value != b"2" for value in first_attempt_counts.values()):
            raise JourneyFailure(
                "Package 7b bound workers did not produce exactly two attempts before inspection: "
                f"{first_attempt_counts}",
                state="design-review",
                event="review-candidates",
            )
        raw_before = self._artifact_tree_snapshot(first_capture)
        first_show_bytes, first_show = show_only()
        foreign_show = copy.deepcopy(first_show)
        foreign_result = foreign_show.get("result")
        if not isinstance(foreign_result, dict):
            raise JourneyFailure(
                f"Package 7b explicit full show omitted a mutable result: {first_show}",
                state="design-review",
                event="show",
            )
        foreign_result["workflow_id"] = "research"
        foreign_projection = subprocess.run(
            [str(self.provider), "review-candidates"],
            input=json.dumps(foreign_show, separators=(",", ":")).encode("utf-8"),
            capture_output=True,
            check=False,
        )
        if (
            foreign_projection.returncode != 2
            or foreign_projection.stdout
            or b"software-change" not in foreign_projection.stderr
        ):
            raise JourneyFailure(
                "Package 7b projected a foreign workflow instead of failing closed: "
                f"returncode={foreign_projection.returncode}, "
                f"stdout={foreign_projection.stdout!r}, "
                f"stderr={foreign_projection.stderr!r}",
                state="design-review",
                event="review-candidates",
            )
        candidate_counts_before = read_attempt_counts("before repeated candidate inspection")
        first_candidate_bytes, first_document = project_candidates(first_show_bytes)
        repeated_candidate_bytes, repeated_document = project_candidates(first_show_bytes)
        candidate_counts_after = read_attempt_counts("after repeated candidate inspection")
        if candidate_counts_before != candidate_counts_after:
            raise JourneyFailure(
                "Package 7b candidate inspection changed worker attempt sentinels: "
                f"before={candidate_counts_before}, after={candidate_counts_after}",
                state="design-review",
                event="review-candidates",
            )
        if first_candidate_bytes != repeated_candidate_bytes or first_document != repeated_document:
            raise JourneyFailure(
                "Package 7b repeated candidate inspections were not byte-identical",
                state="design-review",
                event="review-candidates",
            )
        after_candidate_bytes, after_candidate = show_only()
        del after_candidate_bytes
        if state_context_view(first_show) != state_context_view(after_candidate):
            raise JourneyFailure(
                "Package 7b candidate inspection changed run context or state",
                state="design-review",
                event="review-candidates",
            )
        if self._artifact_tree_snapshot(first_capture) != raw_before:
            raise JourneyFailure(
                "Package 7b candidate inspection changed raw capture files",
                state="design-review",
                event="review-candidates",
            )

        candidates = first_document.get("candidates")
        if first_document.get("schema_version") != "1" or not isinstance(candidates, list):
            raise JourneyFailure(
                f"Package 7b candidate document was not closed: {first_document}",
                state="design-review",
                event="review-candidates",
            )
        if [candidate.get("status") for candidate in candidates] != ["ready", "exhausted"]:
            raise JourneyFailure(
                f"Package 7b candidate statuses were wrong: {first_document}",
                state="design-review",
                event="review-candidates",
            )
        ready = candidates[0]
        exhausted = candidates[1]
        expected_first_origin = {
            "kind": "selected-assignment-output",
            "id": first_invocation_id,
            "assignment_id": "worker-0",
        }
        if (
            set(ready)
            != {"status", "origin", "review_stage", "axis", "author", "result", "findings"}
            or ready.get("review_stage") != "aggregate"
            or ready.get("origin") != expected_first_origin
            or ready.get("axis") != axis
            or ready.get("author") != {"name": ready_author, "kind": "script"}
            or ready.get("result") != "pass"
            or ready.get("findings") != ""
        ):
            raise JourneyFailure(
                f"Package 7b did not select the conforming retry output: {ready}",
                state="design-review",
                event="review-candidates",
            )
        expected_exhausted_origin = {
            "kind": "selected-assignment-output",
            "id": first_invocation_id,
            "assignment_id": "worker-1",
        }
        if (
            set(exhausted) != {"status", "origin", "diagnostic"}
            or exhausted.get("origin") != expected_exhausted_origin
            or not isinstance(exhausted.get("diagnostic"), str)
            or "exhausted" not in exhausted["diagnostic"]
        ):
            raise JourneyFailure(
                f"Package 7b exhausted assignment became judgment data: {exhausted}",
                state="design-review",
                event="review-candidates",
            )
        for index, expected_selected, expected_exhausted in (
            (0, 2, False),
            (1, None, True),
        ):
            manifest_path = first_capture / str(index) / "attempts.json"
            try:
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as error:
                raise JourneyFailure(
                    f"Package 7b could not read raw attempt manifest {manifest_path}: {error}",
                    state="design-review",
                    event="review-candidates",
                ) from error
            if (
                manifest.get("schema_version") != "1"
                or manifest.get("selected_attempt") != expected_selected
                or manifest.get("exhausted") is not expected_exhausted
                or len(manifest.get("attempts", [])) != 2
            ):
                raise JourneyFailure(
                    f"Package 7b raw attempt selection/exhaustion was wrong: {manifest}",
                    state="design-review",
                    event="review-candidates",
                )
            for number in (1, 2):
                if not (first_capture / str(index) / "attempts" / str(number) / "stdout").is_file():
                    raise JourneyFailure(
                        f"Package 7b omitted raw attempt {index}/{number}",
                        state="design-review",
                        event="review-candidates",
                    )

        # Candidate inspection alone cannot satisfy the bound edge or the provider gate.
        inert_before_driver = engine_call(["event", run_id, "approved"])
        inert_show = engine_call(["show", "--view", "full", run_id])
        if (
            inert_before_driver.get("status") != "rejected"
            or inert_before_driver.get("code") != "bound-slot-invocation-required"
            or inert_show.get("result", {}).get("current_state") != "design-review"
        ):
            raise JourneyFailure(
                f"Package 7b candidate inspection unexpectedly advanced the failed overlay: {inert_before_driver}",
                state="design-review",
                event="approved",
            )

        # Synthetic driver triage accepts the conforming ready candidate and rejects the
        # exhausted diagnostic; a deliberate assignment selection starts a new invocation.
        # The projection keeps both invocations rather than silently deduplicating them.
        second_overlay = work_slot_journey.invoke_until_succeeded(
            engine_call,
            run_id,
            "design-review",
            timeout_s=20.0,
            invoke_args=("--assignment", "worker-0"),
        )
        second_invocation_id = second_overlay.get("invocation_id")
        if not isinstance(second_invocation_id, str) or second_invocation_id == first_invocation_id:
            raise JourneyFailure(
                f"Package 7b selected retry reused the first invocation: {second_overlay}",
                state="design-review",
                event="invoke",
            )
        second_show_bytes, second_show = show_only()
        _, second_document = project_candidates(second_show_bytes)
        second_candidates = second_document.get("candidates")
        if (
            not isinstance(second_candidates, list)
            or [candidate.get("status") for candidate in second_candidates]
            != ["ready", "exhausted", "ready"]
            or [
                candidate.get("origin", {}).get("id")
                for candidate in second_candidates
                if candidate.get("status") == "ready"
            ]
            != [first_invocation_id, second_invocation_id]
        ):
            raise JourneyFailure(
                f"Package 7b projected durable invocations out of order or deduplicated them: {second_document}",
                state="design-review",
                event="review-candidates",
            )
        second_ready = second_candidates[-1]
        expected_target_candidates = [
            item
            for item in second_show.get("result", {}).get("requestable_events", [])
            if item.get("event") == "approved"
        ]
        if len(expected_target_candidates) != 1:
            raise JourneyFailure(
                f"Package 7b review state did not expose one approved event: {second_show}",
                state="design-review",
                event="show",
            )
        expected_target = expected_target_candidates[0].get("target")
        inert_without_records = engine_call(["event", run_id, "approved"])
        inert_without_records_show = engine_call(["show", "--view", "full", run_id])
        if (
            inert_without_records.get("status") != "rejected"
            or inert_without_records_show.get("result", {}).get("current_state") != "design-review"
        ):
            raise JourneyFailure(
                f"Package 7b candidate output satisfied the checked event without driver records: {inert_without_records}",
                state="design-review",
                event="approved",
            )

        subject = "design.json"
        subject_revision = self._fixture_revision(subject)
        evidence = {
            "gate": "design-review",
            "policy_id": axis,
            "review_stage": second_ready.get("review_stage", "aggregate"),
            "result": second_ready.get("result"),
            "findings": second_ready.get("findings"),
            "author": second_ready.get("author"),
            "subject": subject,
            "subject_revision": subject_revision,
            "config_version": frozen_profile["config_version"],
            "origin": second_ready.get("origin"),
        }
        evidence_result = engine_call(
            [
                "append",
                "--record-id=package-7b-review-evidence",
                run_id,
                "review-evidence",
                json.dumps(evidence, separators=(",", ":")),
            ]
        )
        if evidence_result.get("status") != "completed":
            raise JourneyFailure(
                f"Package 7b driver review-evidence append failed: {evidence_result}",
                state="design-review",
                event="append",
            )
        ledger = {
            "schema_version": "1",
            "gate": "design-review",
            "subject": subject,
            "subject_revision": subject_revision,
            "author": {"name": "package-7b-driver", "kind": "agent"},
            "findings": [],
        }
        ledger_result = engine_call(
            [
                "append",
                "--record-id=package-7b-finding-ledger",
                run_id,
                "finding-ledger",
                json.dumps(ledger, separators=(",", ":")),
            ]
        )
        if ledger_result.get("status") != "completed":
            raise JourneyFailure(
                f"Package 7b driver finding-ledger append failed: {ledger_result}",
                state="design-review",
                event="append",
            )
        approved = engine_call(["event", run_id, "approved"])
        final_show = engine_call(["show", "--view", "full", run_id])
        contexts = final_show.get("result", {}).get("context", [])
        context_ids = [record.get("id") for record in contexts if isinstance(record, dict)]
        if (
            approved.get("status") != "completed"
            or approved.get("result", {}).get("run", {}).get("current_state") != expected_target
            or expected_target == "design-review"
            or final_show.get("result", {}).get("current_state") != expected_target
            or "package-7b-review-evidence" not in context_ids
            or "package-7b-finding-ledger" not in context_ids
        ):
            raise JourneyFailure(
                f"Package 7b explicit driver records did not permit normal progression: {approved}",
                state="design-review",
                event="approved",
            )
        self.package_7b_proof = [
            "selected retry output is ready and exposes normalized result/findings",
            "exhausted assignment is a non-judgmental diagnostic",
            "raw attempts and captures remain unchanged",
            "worker attempt sentinels remain unchanged across repeated inspection",
            "repeated candidate inspection is byte-identical",
            "distinct durable invocations remain ordered without deduplication",
            "candidate inspection is inert before driver records",
            "foreign workflow identity is rejected before projection",
            "driver triage accepts ready and rejects exhausted before ordinary append",
            "driver-authored review-evidence and finding-ledger permit checked progression",
        ]
        # bookends:LE-109 — this named Package 7b source scenario pipes explicit full show into review-candidates, proves selected retry/exhaustion and raw preservation, denies inert inspection, and advances only after explicit driver records.
        print(
            "Package 7b review-candidates scenario passed: selected retry, exhausted assignment, "
            "raw capture preservation, deterministic repeated inspection, inert-before-records, "
            "and driver-action-afterward progression"
        )

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

    def _run_bookends_enabled_source(
        self, *, global_jobs: Optional[List[Dict[str, Any]]] = None
    ) -> None:
        """Drive the reduced AC-N spine with the optional Bookends overlay."""
        if self.mode != "source":
            raise JourneyFailure("criterion overlay proof is source-only", state=self.state)
        if global_jobs is not None:
            assert self.run_dir is not None
            job_root = self.run_dir / "criterion-overlay-pool-cases"
            for name, candidate in (
                ("overlay-off", None),
                ("overlay-on-candidate", True),
                ("overlay-on-not-applicable", False),
            ):
                self._append_global_pool_job(
                    global_jobs,
                    name=f"criterion-{name}",
                    kind="overlay-off" if candidate is None else "overlay-on",
                    root=job_root / name,
                    candidate=candidate,
                )
            self._global_overlay_paths = {
                "overlay_off": job_root / "overlay-off" / "criterion-overlay-off" / "overlay-off-proof.json",
                "overlay_on_candidate": job_root / "overlay-on-candidate" / "criterion-overlay-on-candidate" / "overlay-on-candidate-proof.json",
                "overlay_on_not_applicable": job_root / "overlay-on-not-applicable" / "criterion-overlay-on-not-applicable" / "overlay-on-not-applicable-proof.json",
            }
            return
        if getattr(self.args, "jobs", 2) == 1:
            overlay_off = self._run_overlay_off_source()
            overlay_candidate = self._run_overlay_on_source(candidate=True)
            overlay_not_applicable = self._run_overlay_on_source(candidate=False)
        else:
            assert self.run_dir is not None
            import proof_pool

            jobs = []
            job_root = self.run_dir / "criterion-overlay-pool-cases"
            spec_root = self.run_dir / "criterion-overlay-pool-jobs"
            spec_root.mkdir()
            worker = self._pool_worker_path()
            for name, kind, candidate in (
                ("overlay-off", "overlay-off", None),
                ("overlay-on-candidate", "overlay-on", True),
                ("overlay-on-not-applicable", "overlay-on", False),
            ):
                spec_path = spec_root / f"{name}.json"
                spec_path.write_text(
                    json.dumps(
                        {
                            "kind": kind,
                            "root": str(job_root / name),
                            "args": self._pool_args(),
                            "candidate": candidate,
                        },
                        indent=2,
                    )
                    + "\n",
                    encoding="utf-8",
                )
                jobs.append({"name": name, "command": [sys.executable, str(worker), str(spec_path)]})
            report = proof_pool.run(
                jobs,
                root=self.run_dir / "criterion-overlay-pool",
                limit=self.args.jobs,
                timeout=self.args.job_timeout,
            )
            report_path = self.run_dir / "criterion-overlay-pool-report.json"
            report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
            expected_peak = min(self.args.jobs, 3)
            if report.get("status") != "passed" or report.get("peak_jobs") != expected_peak:
                raise JourneyFailure(
                    f"criterion overlay pool failed; inspect {report_path}",
                    state="end",
                    event="criterion-overlay",
                )
            if any(
                row.get("status") != "passed" or row.get("exit_code") != 0
                for row in report.get("jobs", [])
            ):
                raise JourneyFailure(
                    f"criterion overlay pool omitted a completed case; inspect {report_path}",
                    state="end",
                    event="criterion-overlay",
                )
            overlay_off = job_root / "overlay-off" / "criterion-overlay-off" / "overlay-off-proof.json"
            overlay_candidate = job_root / "overlay-on-candidate" / "criterion-overlay-on-candidate" / "overlay-on-candidate-proof.json"
            overlay_not_applicable = job_root / "overlay-on-not-applicable" / "criterion-overlay-on-not-applicable" / "overlay-on-not-applicable-proof.json"
            for path in (overlay_off, overlay_candidate, overlay_not_applicable):
                if not path.is_file():
                    raise JourneyFailure(
                        f"criterion overlay omitted proof artifact {path}; inspect {report_path}",
                        state="end",
                        event="criterion-overlay",
                    )
        self.criterion_overlay_proof = {
            "overlay_off": overlay_off,
            "overlay_on_candidate": overlay_candidate,
            "overlay_on_not_applicable": overlay_not_applicable,
        }
        self.bookends_proof = overlay_candidate
        print(
            "overlay-off criterion spine scenario passed: AC-N only; no PRD disposition, "
            "candidate, liveness, citation, or Green claim"
        )
        print(
            "overlay-on criterion scenarios passed: one disposition per criterion, "
            "candidate blocks Bookends-enabled final completion, not-applicable does not waive or "
            "fulfill its criterion"
        )

    def _run_overlay_off_source(self) -> Path:
        """Prove the unannotated public path uses only the AC-N criterion spine."""
        assert self.run_dir is not None
        assert self.data_root is not None
        assert self.fixture_root is not None
        scenario_dir = self.run_dir / "criterion-overlay-off"
        artifacts = scenario_dir / "artifacts"
        database = scenario_dir / "run.sqlite"
        profile_path = scenario_dir / "high-rigor.json"
        provider_config = scenario_dir / "providers.toml"
        scenario_dir.mkdir(parents=True, exist_ok=True)
        artifacts.mkdir()
        for subject, fixture in SUBJECTS.items():
            value = self._read_json(
                self.fixture_root / fixture, f"overlay-off fixture {subject}"
            )
            # The supplied validation fixture contains an old historical
            # citation in prose.  This focused overlay-off artifact set is
            # deliberately citation-free, so replace that prose only in the
            # temporary scenario copy; shipped fixtures remain untouched.
            if subject == "validation-report.json":
                value = self._replace_overlay_off_citation(value)
            (artifacts / subject).write_text(
                json.dumps(value, indent=2) + "\n", encoding="utf-8"
            )
        self._prepare_fixture_proof_commands(artifacts)
        profile = self._read_json(
            self.data_root / PROFILE_SUBPATH, "overlay-off high-rigor profile"
        )
        # This focused overlay fixture predates the dedicated reconciliation
        # phase; the v11 public cases below own that boundary explicitly.
        profile["config_version"] = "high-rigor-10"
        profile["artifact_root"] = str(artifacts)
        _write_json(profile_path, profile)
        self._write_provider_config_at(provider_config)

        saved = {
            "run_id": self.run_id,
            "database": self.database,
            "provider_config": self.provider_config,
            "artifact_root": self.artifact_root,
            "profile_path": self.profile_path,
            "profile_source": self.profile_source,
            "profile": self.profile,
            "work_slot_bindings": self.work_slot_bindings,
            "state": self.state,
            "command_cwd": self.command_cwd,
            "repository_root": self.repository_root,
            "command_env": self.command_env,
        }
        proof_path = scenario_dir / "overlay-off-proof.json"
        try:
            self.run_id = "criterion-overlay-off-journey"
            self.database = database
            self.provider_config = provider_config
            self.artifact_root = artifacts
            self.profile_path = profile_path
            self.profile_source = self.data_root / PROFILE_SUBPATH
            self.profile = profile
            self.work_slot_bindings = {}
            self.state = "not-started"
            self.command_cwd = self.data_root
            self.repository_root = None
            self.command_env = {"BOOKENDS_BYPASS": ""}
            self._start()
            shown = self._assert_show("explore", "overlay-off-show")
            intent = self._read_json(artifacts / "intent.json", "overlay-off intent")
            criteria = self._assert_overlay_off_criterion_spine(intent)
            for subject in SUBJECTS:
                self._assert_no_overlay_off_metadata(
                    self._read_json(artifacts / subject, f"overlay-off {subject}"),
                    subject,
                )
            initial_input = shown.get("initial_input", {})
            if initial_input.get("extra", {}).get("bookends", {}).get("enabled") is True:
                raise JourneyFailure(
                    "overlay-off run unexpectedly enabled Bookends",
                    state=self.state,
                    event="show",
                )
            self._expect_allow("intent-ready", "intent-review")
            after = self._assert_show("intent-review", "overlay-off-ready-show")
            if after.get("initial_input") != initial_input:
                raise JourneyFailure(
                    "overlay-off checked transition changed frozen initial input",
                    state=self.state,
                    event="show",
                )
            proof_path.write_text(
                json.dumps(
                    {
                        "result": "passed",
                        "run_id": self.run_id,
                        "overlay_enabled": False,
                        "config_version": profile["config_version"],
                        "criterion_ids": criteria,
                        "state_after_intent_ready": self.state,
                        "assertions": [
                            "intent acceptance is a closed AC-N {id, statement} spine",
                            "intent-ready is accepted through the real engine/provider path",
                            "all five authored overlay-off artifacts carry no PRD disposition, candidate, liveness, citation, or Green claim",
                        ],
                        "database": str(database),
                        "artifact_root": str(artifacts),
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )
        finally:
            self.run_id = saved["run_id"]
            self.database = saved["database"]
            self.provider_config = saved["provider_config"]
            self.artifact_root = saved["artifact_root"]
            self.profile_path = saved["profile_path"]
            self.profile_source = saved["profile_source"]
            self.profile = saved["profile"]
            self.work_slot_bindings = saved["work_slot_bindings"]
            self.state = saved["state"]
            self.command_cwd = saved["command_cwd"]
            self.repository_root = saved["repository_root"]
            self.command_env = saved["command_env"]
        return proof_path

    def _assert_overlay_off_criterion_spine(self, intent: Dict[str, Any]) -> List[str]:
        acceptance = intent.get("acceptance")
        if not isinstance(acceptance, list) or not acceptance:
            raise JourneyFailure("overlay-off intent omitted acceptance criteria")
        ids: List[str] = []
        for index, criterion in enumerate(acceptance):
            if not isinstance(criterion, dict) or set(criterion) != {"id", "statement"}:
                raise JourneyFailure(
                    f"overlay-off criterion {index} is not the closed AC-N record: {criterion}"
                )
            criterion_id = criterion.get("id")
            statement = criterion.get("statement")
            if (
                not isinstance(criterion_id, str)
                or not criterion_id.startswith("AC-")
                or not criterion_id[3:].isdigit()
                or criterion_id[3:] == "0"
                or criterion_id in ids
                or not isinstance(statement, str)
                or not statement
            ):
                raise JourneyFailure(
                    f"overlay-off criterion identity is malformed: {criterion}"
                )
            ids.append(criterion_id)
        serialized = json.dumps(intent, separators=(",", ":"), ensure_ascii=False).lower()
        forbidden = (
            "prd_traceability",
            "requirement_ids",
            "bookends:",
            "candidate",
            "citation",
            "green",
            "live_ids",
        )
        found = [term for term in forbidden if term in serialized]
        if found:
            raise JourneyFailure(
                f"overlay-off artifact carried optional PRD metadata: {found}",
                state=self.state,
                event="show",
            )
        return ids

    @staticmethod
    def _replace_overlay_off_citation(value: Any) -> Any:
        if isinstance(value, dict):
            return {
                key: Journey._replace_overlay_off_citation(child)
                for key, child in value.items()
            }
        if isinstance(value, list):
            return [Journey._replace_overlay_off_citation(child) for child in value]
        if isinstance(value, str):
            return value.replace("bookends:LE-39", "named public requirement")
        return value

    @staticmethod
    def _assert_no_overlay_off_metadata(value: Dict[str, Any], subject: str) -> None:
        serialized = json.dumps(value, separators=(",", ":"), ensure_ascii=False).lower()
        forbidden = (
            "prd_traceability",
            "requirement_ids",
            "bookends:",
            "candidate",
            "citation",
            "green",
            "live_ids",
        )
        found = [term for term in forbidden if term in serialized]
        if found:
            raise JourneyFailure(
                f"overlay-off {subject} carried optional PRD metadata: {found}",
                state="explore",
                event="show",
            )

    def _write_provider_config_at(self, path: Path) -> None:
        assert self.provider is not None
        path.write_text(
            "[providers.software-change]\n"
            f"command = {json.dumps(str(self.provider))}\n"
            "args = []\n",
            encoding="utf-8",
        )

    def _overlay_axes(self, gate: str) -> List[Dict[str, Any]]:
        policies = self.profile.get("review_policies", {})
        configured = policies.get(gate)
        if not isinstance(configured, list):
            raise JourneyFailure(f"overlay scenario profile omitted policy gate {gate}")
        axes = [copy.deepcopy(entry) for entry in configured]
        if not axes:
            return axes

        contract_v3 = self.profile.get("contract_version") == 3
        stages: List[str] = []
        for entry in axes:
            stage = entry.get("review_stage", "aggregate") if isinstance(entry, dict) else "aggregate"
            if stage not in stages:
                stages.append(stage)
        extras = ["ids-grounded"]
        if gate in {"validation-review", "validation-adversarial-review"}:
            extras.append("bypass-not-green")
        for stage in stages:
            required_authors = max(
                int(entry.get("required_authors", 1))
                for entry in axes
                if isinstance(entry, dict)
                and entry.get("review_stage", "aggregate") == stage
            )
            for axis_id in extras:
                if any(
                    isinstance(entry, dict)
                    and entry.get("id") == axis_id
                    and (not contract_v3 or entry.get("review_stage", "aggregate") == stage)
                    for entry in axes
                ):
                    continue
                extra: Dict[str, Any] = {
                    "id": axis_id,
                    "required_authors": required_authors,
                }
                if contract_v3:
                    extra["review_stage"] = stage
                axes.append(extra)
        return axes

    def _append_overlay_evidence(
        self,
        gate: str,
        *,
        record_prefix: str = "",
        failing_axis: Optional[str] = None,
        failure_findings: str = "",
    ) -> Optional[str]:
        subject = GATE_SUBJECT[gate]
        revision = self._fixture_revision(subject)
        failure_record_id: Optional[str] = None
        for entry in self._overlay_axes(gate):
            axis = entry["id"]
            required = int(entry.get("required_authors", 1))
            count = 1 if axis == failing_axis else required
            if axis == failing_axis:
                if not failure_findings:
                    raise JourneyFailure("overlay failing evidence omitted findings")
            for index in range(count):
                result = "fail" if axis == failing_axis else "pass"
                findings = failure_findings if result == "fail" else ""
                stage = entry.get("review_stage", "aggregate")
                record_id = f"{record_prefix}evidence-{gate}-{stage}-{axis}-{index}"
                data = {
                    "gate": gate,
                    "policy_id": axis,
                    "review_stage": stage,
                    "result": result,
                    "findings": findings,
                    "author": {
                        "name": f"overlay-{gate}-{axis}-{index}",
                        "kind": "script",
                    },
                    "subject": subject,
                    "subject_revision": revision,
                    "config_version": self.profile["config_version"],
                }
                response = self._engine(
                    [
                        "append",
                        f"--record-id={record_id}",
                        self.run_id,
                        "review-evidence",
                        json.dumps(data, separators=(",", ":")),
                    ],
                    state=self.state,
                    event="append",
                    axis=axis,
                )
                self._expect_status(
                    response, "completed", event="append", state=self.state, axis=axis
                )
                if result == "fail":
                    failure_record_id = record_id
        return failure_record_id

    def _pass_overlay_review(self, gate: str, event: str, target: str) -> None:
        prefix = f"overlay-{gate}-"
        self._assert_show(self.state, f"{prefix}before")
        self._append_overlay_evidence(gate, record_prefix=prefix)
        self._append_finding_ledger_for(
            self.run_id, gate, state=self.state, record_prefix=prefix
        )
        self._expect_allow(event, target)

    def _write_overlay_artifacts(
        self, artifacts: Path, *, candidate: bool, unfulfilled: bool
    ) -> Dict[str, Any]:
        assert self.fixture_root is not None
        values = {
            subject: self._read_json(
                self.fixture_root / fixture, f"overlay fixture {subject}"
            )
            for subject, fixture in SUBJECTS.items()
        }
        dispositions = [
            {
                "type": "linked-live",
                "live_ids": ["LE-1"],
            },
            (
                {
                    "type": "candidate",
                    "proposed_id": BOOKENDS_CANDIDATE_ID,
                    "record_markdown": (
                        f"### {BOOKENDS_CANDIDATE_ID}: Proposed requirement\n"
                        "- Status: live\n- Coverage: e2e/journey\n\n"
                        "The software-change provider must preserve the proposed behavior in ordinary runs and expose proof that an owner can inspect before completion.\n"
                    ),
                }
                if candidate
                else {"type": "linked-live", "live_ids": ["LE-2"]}
            ),
            {
                "type": "not-applicable",
                "reason": "This criterion is change-specific and has no enduring PRD identity.",
            },
            {
                "type": "linked-live",
                "live_ids": ["LE-2"],
            },
        ]
        intent = values["intent.json"]
        for criterion, disposition in zip(intent["acceptance"], dispositions):
            criterion["prd_traceability"] = disposition

        design = values["design.json"]
        design["coverage"][0]["criterion_id"] = "AC-1"

        plan = values["plan.json"]
        plan["tasks"][0]["criterion_ids"] = ["AC-1"]

        implementation = values["implementation-report.json"]
        implementation["validation"][0]["criterion_id"] = "AC-1"

        # The v2 index covers AC-3 regardless of PRD disposition. The later
        # independent challenge finding, not report prose, declares it unfulfilled.
        assert any(row["criterion_id"] == "AC-3" for row in values["validation-report.json"]["criteria"])

        for subject, value in values.items():
            (artifacts / subject).write_text(
                json.dumps(value, indent=2) + "\n", encoding="utf-8"
            )
        self._prepare_fixture_proof_commands(artifacts)
        return {
            "criterion_types": [disposition["type"] for disposition in dispositions],
        }

    def _assert_overlay_artifacts(
        self, artifacts: Path, *, candidate: bool, unfulfilled: bool
    ) -> Dict[str, Any]:
        intent = self._read_json(artifacts / "intent.json", "overlay intent")
        criteria = intent.get("acceptance")
        if not isinstance(criteria, list) or len(criteria) != 4:
            raise JourneyFailure(f"overlay intent has unexpected criteria: {intent}")
        types = []
        for index, criterion in enumerate(criteria):
            if not isinstance(criterion, dict):
                raise JourneyFailure(f"overlay criterion {index} is not an object")
            disposition = criterion.get("prd_traceability")
            if not isinstance(disposition, dict):
                raise JourneyFailure(
                    f"overlay criterion {index} omitted its one disposition"
                )
            types.append(disposition.get("type"))
        expected = (
            ["linked-live", "candidate", "not-applicable", "linked-live"]
            if candidate
            else ["linked-live", "linked-live", "not-applicable", "linked-live"]
        )
        if types != expected or types.count("not-applicable") != 1:
            raise JourneyFailure(
                f"overlay dispositions were not exactly one per criterion: {types}"
            )
        if self._read_json(artifacts / "design.json", "overlay design")["coverage"][0].get("criterion_id") != "AC-1":
            raise JourneyFailure("overlay design omitted its AC-N reference")
        if self._read_json(artifacts / "plan.json", "overlay plan")["tasks"][0].get("criterion_ids") != ["AC-1"]:
            raise JourneyFailure("overlay plan omitted its AC-N reference")
        if {row["criterion_id"] for row in self._read_json(artifacts / "validation-report.json", "overlay validation")["criteria"]} != {"AC-1", "AC-2", "AC-3", "AC-4"}:
            raise JourneyFailure("overlay validation omitted its AC-N index coverage")
        return {"criterion_types": types}

    def _run_overlay_on_source(self, *, candidate: bool) -> Path:
        """Prove candidate blocking and the non-waiver of not-applicable."""
        assert self.run_dir is not None
        assert self.data_root is not None
        scenario_name = "candidate" if candidate else "not-applicable"
        scenario_dir = self.run_dir / f"criterion-overlay-on-{scenario_name}"
        checkout = scenario_dir / "checkout"
        artifacts = scenario_dir / "artifacts"
        database = scenario_dir / "run.sqlite"
        profile_path = scenario_dir / "high-rigor-bookends.json"
        provider_config = scenario_dir / "providers.toml"
        scenario_dir.mkdir(parents=True, exist_ok=True)
        # The overlay only needs tracked proof inputs and a fresh Git root.
        # Agent sessions and prior run/fan-out outputs are generated artifacts,
        # never Bookends inputs; copying them made each overlay setup duplicate
        # gigabytes without changing the checked tree.
        shutil.copytree(
            self.data_root,
            checkout,
            ignore=shutil.ignore_patterns(
                ".git", "target", "__pycache__", "*.pyc",
                ".pi-subagents", ".loop-engine", "fan-out-adhoc",
            ),
        )
        artifacts.mkdir()
        self._initialize_overlay_checkout(checkout)
        profile = self._read_json(
            self.data_root / BOOKENDS_SCENARIO_PROFILE,
            "overlay-on high-rigor profile",
        )
        # Keep this criterion-spine fixture on the frozen pre-reconciliation
        # graph; dedicated v11 scenarios prove the new state and ordering.
        profile["config_version"] = "high-rigor-10"
        profile["artifact_root"] = str(artifacts)
        extra = copy.deepcopy(profile.get("extra", {}))
        extra["bookends"] = {"enabled": True}
        profile["extra"] = extra
        _write_json(profile_path, profile)
        self._write_provider_config_at(provider_config)

        saved = {
            "run_id": self.run_id,
            "database": self.database,
            "provider_config": self.provider_config,
            "artifact_root": self.artifact_root,
            "profile_path": self.profile_path,
            "profile_source": self.profile_source,
            "profile": self.profile,
            "work_slot_bindings": self.work_slot_bindings,
            "state": self.state,
            "command_cwd": self.command_cwd,
            "repository_root": self.repository_root,
            "command_env": self.command_env,
        }
        proof_path = scenario_dir / f"overlay-on-{scenario_name}-proof.json"
        try:
            self.run_id = (
                BOOKENDS_SCENARIO_RUN_ID
                if candidate
                else BOOKENDS_NOT_APPLICABLE_RUN_ID
            )
            self.database = database
            self.provider_config = provider_config
            self.artifact_root = artifacts
            self.profile_path = profile_path
            self.profile_source = self.data_root / BOOKENDS_SCENARIO_PROFILE
            self.profile = profile
            self.work_slot_bindings = {}
            self.state = "not-started"
            self.command_cwd = checkout
            self.repository_root = checkout
            self.command_env = {"BOOKENDS_BYPASS": ""}
            dispositions = self._write_overlay_artifacts(
                artifacts, candidate=candidate, unfulfilled=not candidate
            )
            self._assert_overlay_artifacts(
                artifacts, candidate=candidate, unfulfilled=not candidate
            )
            self._start()
            shown = self._assert_show("explore", f"overlay-on-{scenario_name}-show")
            initial_input = shown.get("initial_input", {})
            if initial_input.get("extra", {}).get("bookends", {}).get("enabled") is not True:
                raise JourneyFailure(
                    "overlay-on option was not frozen in initial_input",
                    state=self.state,
                    event="show",
                )
            instructions = str(shown.get("current_state_instructions", ""))
            for fragment in ("prd_traceability", "linked-live", "candidate", "not-applicable"):
                if fragment not in instructions:
                    raise JourneyFailure(
                        f"overlay-on instructions omitted {fragment!r}",
                        state=self.state,
                        event="show",
                    )

            self._expect_allow("intent-ready", "intent-review")
            self._pass_overlay_review("intent-review", "approved", "intent-adversarial-review")
            self._pass_overlay_review("intent-adversarial-review", "approved", "design")
            self._expect_allow("design-ready", "design-review")
            self._pass_overlay_review("design-review", "approved", "design-adversarial-review")
            self._pass_overlay_review("design-adversarial-review", "approved", "plan")
            self._expect_allow("plan-ready", "plan-review")
            self._pass_overlay_review("plan-review", "approved", "plan-adversarial-review")
            self._pass_overlay_review("plan-adversarial-review", "approved", "implement")
            self._create_checkpoint("implementation")
            self._expect_allow("implementation-ready", "implementation-review")
            self._pass_overlay_review("implementation-review", "approved", "implementation-adversarial-review")
            self._pass_overlay_review("implementation-adversarial-review", "approved", "validation")
            self._create_checkpoint("validation")
            self._expect_allow("validation-ready", "validation-review")
            self._pass_overlay_review("validation-review", "approved", "validation-adversarial-review")

            if candidate:
                # bookends:LE-97 — the public overlay-on candidate assertion proves a current provisional PRD record blocks Bookends-enabled final completion after all other phase checks pass.
                denial = self._event("passed", "bookends-candidate")
                self._expect_status(
                    denial,
                    "rejected",
                    event="passed",
                    axis="bookends-candidate",
                    state=self.state,
                )
                if denial.get("code") != "software-change-bookends-candidate":
                    raise JourneyFailure(
                        f"current candidate did not block Bookends-enabled final completion: {denial}",
                        state=self.state,
                        event="passed",
                    )
                self._assert_show("validation-adversarial-review", "overlay-candidate-denied")
                case_assertions = [
                    "exactly one prd_traceability disposition is present on every current criterion",
                    "linked-live dispositions use only live LE-1 and LE-2 IDs",
                    "current candidate LE-9001 blocks final high-rigor passed",
                ]
            else:
                # bookends:LE-97 — the public overlay-on not-applicable assertion proves traceability classification does not waive an unfulfilled AC-N criterion.
                finding_id = self._append_overlay_evidence(
                    "validation-adversarial-review",
                    record_prefix="overlay-not-applicable-",
                    failing_axis="intent-delivered",
                    failure_findings=(
                        "AC-3 remains unfulfilled; not-applicable classifies PRD traceability only and does not satisfy the criterion."
                    ),
                )
                if finding_id is None:
                    raise JourneyFailure("not-applicable proof did not append a failing review")
                finding = {
                    "id": "F-not-applicable-criterion",
                    "source": {"kind": "context-record", "id": finding_id},
                    "policy_id": "intent-delivered",
                    "statement": (
                        "AC-3 remains unfulfilled; not-applicable classifies PRD traceability only "
                        "and does not satisfy the criterion."
                    ),
                    "disposition": "accepted",
                    "reason": "The driver accepted the current semantic failure and kept the criterion binding.",
                    "owner_phase": "validation",
                    "task_ids": [],
                    "review_axes": ["intent-delivered"],
                    "status": "unresolved",
                }
                ledger = {
                    "schema_version": "1",
                    "gate": "validation-adversarial-review",
                    "subject": "validation-report.json",
                    "subject_revision": self._fixture_revision("validation-report.json"),
                    "author": {"name": "overlay-not-applicable-driver", "kind": "agent"},
                    "findings": [finding],
                }
                self._append_ledger_snapshot_for(
                    self.run_id,
                    ledger,
                    record_id="overlay-not-applicable-ledger",
                    state=self.state,
                    axis="intent-delivered",
                )
                denial = self._event("passed", "not-applicable")
                self._expect_status(
                    denial,
                    "rejected",
                    event="passed",
                    axis="not-applicable",
                    state=self.state,
                )
                if (denial.get("code") != "software-change-finding-ledger-invalid"
                    or denial.get("details", {}).get("status") != "accepted_unresolved"):
                    raise JourneyFailure(
                        f"not-applicable unexpectedly waived the unfulfilled criterion: {denial}",
                        state=self.state,
                        event="passed",
                    )
                if "AC-3" not in json.dumps(denial, ensure_ascii=False):
                    raise JourneyFailure(
                        "not-applicable denial did not retain the unfulfilled criterion finding",
                        state=self.state,
                        event="passed",
                    )
                self._assert_show("validation-adversarial-review", "overlay-not-applicable-denied")
                case_assertions = [
                    "exactly one prd_traceability disposition is present on every current criterion",
                    "not-applicable is PRD traceability only and contributes no fulfillment claim",
                    "an external validation failure for AC-3 still blocks passed",
                ]

            proof_path.write_text(
                json.dumps(
                    {
                        "result": "passed",
                        "run_id": self.run_id,
                        "overlay_enabled": True,
                        "scenario": scenario_name,
                        "config_version": profile["config_version"],
                        "dispositions": dispositions,
                        "terminal_state": self.state,
                        "denial_code": denial.get("code"),
                        "candidate_id": BOOKENDS_CANDIDATE_ID if candidate else None,
                        "finding_id": "F-not-applicable-criterion" if not candidate else None,
                        "assertions": case_assertions,
                        "database": str(database),
                        "artifact_root": str(artifacts),
                        "candidate_checkout": str(checkout),
                    },
                    indent=2,
                )
                + "\n",
                encoding="utf-8",
            )
        finally:
            self.run_id = saved["run_id"]
            self.database = saved["database"]
            self.provider_config = saved["provider_config"]
            self.artifact_root = saved["artifact_root"]
            self.profile_path = saved["profile_path"]
            self.profile_source = saved["profile_source"]
            self.profile = saved["profile"]
            self.work_slot_bindings = saved["work_slot_bindings"]
            self.state = saved["state"]
            self.command_cwd = saved["command_cwd"]
            self.repository_root = saved["repository_root"]
            self.command_env = saved["command_env"]
        return proof_path

    @staticmethod
    def _initialize_overlay_checkout(checkout: Path) -> None:
        for git_args in (
            ["init", "-q"],
            ["config", "user.name", "software-change journey"],
            ["config", "user.email", "journey@example.invalid"],
            ["config", "commit.gpgsign", "false"],
            ["add", "-A"],
            ["commit", "-qm", "criterion overlay journey baseline"],
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
                    f"criterion overlay git {' '.join(git_args)} failed: "
                    f"{completed.stderr.strip() or completed.stdout.strip()}"
                )

    def _run_stitched_source(self) -> None:
        """Complete a same-topology run with a different frozen policy set."""
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

        primary_show = self._show_for(saved_run_id, state="end", event="stitched-primary-show")
        primary_input = primary_show.get("initial_input")
        if not isinstance(primary_input, dict):
            raise JourneyFailure("primary completed run omitted frozen initial input")
        primary_workflow = self._describe_initial_input(primary_input, "primary")

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
            self._prepare_fixture_proof_commands(artifact_root)

            self._start_run(self.run_id)
            shown = self._assert_show("explore", "stitched-start")
            secondary_input = shown.get("initial_input")
            if not isinstance(secondary_input, dict):
                raise JourneyFailure("stitched run omitted frozen initial input")
            secondary_workflow = self._describe_initial_input(secondary_input, "stitched")
            # bookends:LE-39 — the minimal-profile public run reaches the
            # terminal state through the same provider path.
            # bookends:LE-43 — completed runs compare identical topology while
            # retaining materially different frozen review obligations.
            # AC-12: the provider and every state/transition/work-slot row are
            # identical, while the frozen review and criterion obligations are
            # materially different.
            state_shape = lambda workflow: [
                {key: state.get(key) for key in ("id", "title", "final")}
                for state in workflow.get("states", [])
            ]
            if (
                state_shape(primary_workflow) != state_shape(secondary_workflow)
                or primary_workflow.get("transitions") != secondary_workflow.get("transitions")
                or primary_workflow.get("work_slots") != secondary_workflow.get("work_slots")
            ):
                raise JourneyFailure(
                    "completed-run comparison changed workflow topology",
                    state=self.state,
                    event="stitched-start",
                )
            if (
                secondary_workflow.get("id") != "software-change"
                or secondary_input.get("config_version") != "minimal-11"
                or secondary_input.get("review_policies") == primary_input.get("review_policies")
                or secondary_input.get("criterion_policy") == primary_input.get("criterion_policy")
            ):
                raise JourneyFailure(
                    "same-topology comparison did not freeze distinct review obligations",
                    state=self.state,
                    event="stitched-start",
                )
            try:
                work_slot_journey.assert_catalog(
                    shown,
                    SOFTWARE_CHANGE_SLOT_IDS,
                    stdin_context_kinds=_review_stdin_kinds(SOFTWARE_CHANGE_SLOT_IDS),
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
            if ("intent-ready", "intent-review") not in routes:
                raise JourneyFailure(
                    f"stitched explore omitted intent-ready→intent-review; got {routes}",
                    state=self.state,
                    event="stitched-start",
                )
            self._invoke_bound_slot(self.run_id, state="explore")
            self._expect_allow("intent-ready", "intent-review")

            # Deliberately import the completed high-rigor run's intent-review
            # records.  Its config identity is frozen differently, so this
            # evidence must deny until the secondary run receives its own
            # policy evidence; merely sharing axis names is not enough.
            foreign_context = [
                record
                for record in primary_show.get("context", [])
                if isinstance(record, dict)
                and record.get("kind") in {"review-evidence", "finding-ledger"}
                and record.get("data", {}).get("gate") == "intent-review"
            ]
            if not foreign_context:
                raise JourneyFailure("primary run had no intent-review evidence to compare")
            for record in foreign_context:
                self._assert_show("intent-review", "foreign-policy-record")
                response = self._engine(
                    [
                        "append",
                        "--record-id=foreign-" + str(record["id"]),
                        self.run_id,
                        str(record["kind"]),
                        json.dumps(record["data"], separators=(",", ":")),
                    ],
                    state="intent-review",
                    event="append",
                    axis="foreign-policy",
                )
                self._expect_status(response, "completed", event="append", state="intent-review")
            foreign_denial = self._event("approved", "foreign-policy")
            self._expect_status(
                foreign_denial,
                "rejected",
                event="approved",
                state="intent-review",
                axis="foreign-policy",
            )
            self._assert_show("intent-review", "foreign-policy-denied")
            if foreign_denial.get("code") != "software-change-review-incomplete":
                raise JourneyFailure(
                    f"wrong-run policy evidence produced the wrong denial: {foreign_denial}",
                    state="intent-review",
                    event="approved",
                )
            evidence_details = foreign_denial.get("details", {})
            diagnostics = [
                *evidence_details.get("diagnostics", []),
                *evidence_details.get("informational", []),
            ]
            if not any(
                diagnostic.get("category") == "stale_config"
                for axis in diagnostics
                if isinstance(axis, dict)
                for diagnostic in axis.get("diagnostics", [])
                if isinstance(diagnostic, dict)
            ):
                raise JourneyFailure(
                    f"wrong-run policy evidence was not exposed as stale config: {foreign_denial}",
                    state="intent-review",
                    event="approved",
                )
            self._append_evidence("intent-review", record_prefix="stitched-current-")
            self._expect_allow("approved", "intent-adversarial-review")
            self._pass_review(
                "intent-adversarial-review", "approved", "design", record_prefix="stitched-current-"
            )

            self._expect_allow("design-ready", "design-review")
            self._pass_review("design-review", "approved", "design-adversarial-review")
            self._pass_review("design-adversarial-review", "approved", "plan")
            self._expect_allow("plan-ready", "plan-review")
            self._pass_review("plan-review", "approved", "plan-adversarial-review")
            self._pass_review("plan-adversarial-review", "approved", "implement")
            self._write_no_change_reconciliation("stitched-reconciliation-r1")
            self._expect_allow("implementation-ready", "reconciliation")
            self._expect_allow("reconciliation-ready", "implementation-review")
            self._create_checkpoint("implementation")
            self._pass_review(
                "implementation-review", "approved", "implementation-adversarial-review"
            )
            self._pass_review(
                "implementation-adversarial-review", "approved", "validation"
            )
            self._create_checkpoint("validation")
            self._expect_allow("validation-ready", "validation-review")
            self._pass_review(
                "validation-review", "approved", "validation-adversarial-review"
            )
            self._pass_review("validation-adversarial-review", "passed", "end")
            shown = self._assert_show("end", "stitched-terminal-show")
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
                "stitched software-change journey passed: same topology, distinct frozen policies, wrong-run evidence denied"
            )
        finally:
            self.run_id = saved_run_id
            self.artifact_root = saved_artifact_root
            self.profile_path = saved_profile_path
            self.profile = saved_profile
            self.state = saved_state
            self.work_slot_bindings = saved_bindings
            self.profile_source = saved_source

    def _describe_initial_input(self, initial_input: Dict[str, Any], label: str) -> Dict[str, Any]:
        """Describe a frozen input through the public provider subprocess."""
        completed = subprocess.run(
            [str(self.provider)],
            input=json.dumps({"operation": "describe", "initial_input": initial_input}),
            text=True,
            capture_output=True,
            check=False,
        )
        if completed.returncode != 0:
            raise JourneyFailure(
                f"{label} provider describe failed: {completed.stderr.strip() or completed.returncode}"
            )
        try:
            value = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise JourneyFailure(f"{label} provider describe returned invalid JSON: {error}") from error
        if not isinstance(value, dict):
            raise JourneyFailure(f"{label} provider describe returned a non-object")
        return value

    @staticmethod
    def _assert_stitched_profile(profile: Dict[str, Any]) -> None:
        policies = profile.get("review_policies")
        if not isinstance(policies, dict) or set(policies) != set(GATE_SUBJECT):
            raise JourneyFailure(
                "stitched profile must configure every ordinary and challenge gate",
                event="stitched-start",
            )
        for gate, axes in policies.items():
            if not isinstance(axes, list) or not axes:
                raise JourneyFailure(
                    f"stitched profile must keep a nonempty {gate} policy list",
                    event="stitched-start",
                )
            if any(axis.get("review_stage", "aggregate") != "aggregate" for axis in axes):
                raise JourneyFailure(
                    f"stitched minimal profile unexpectedly includes staged individual review in {gate}",
                    event="stitched-start",
                )

    def _run_dummy_worker_proofs(
        self, *, global_jobs: Optional[List[Dict[str, Any]]] = None
    ) -> None:
        """Prove heartbeat, capture isolation, preview fail-closed, and sandbox argv."""
        assert self.run_dir is not None
        assert self.profile_source is not None
        assert self.fixture_root is not None
        import proof_pool
        setup_started = time.monotonic()
        assert_worker_data_skill_and_root_policy(
            engine_binary=self.engine,
            provider_binary=self.provider,
        )
        self.proof_setup = [{"name": "assert_worker_data_skill_and_root_policy",
                             "status": "passed", "stage": "setup",
                             "wall_seconds": time.monotonic() - setup_started}]
        proof_root = self.run_dir / "dummy-worker-proofs"
        jobs = []

        def enqueue(function, **kwargs):
            jobs.append({"name": function.__name__,
                         "kwargs": {key: str(value.resolve()) for key, value in kwargs.items()}})

        try:
            # bookends:LE-85 — shipped profiles leave driver-performed slots unbound.
            # bookends:LE-86 — the same public binding contract is exercised for this provider.
            # bookends:LE-132 — the explicit roster/setup path exposes the
            # effective worker policy before any run is started.
            setup_started = time.monotonic()
            assertions = work_slot_journey.prove_shipped_software_change_profiles(self.data_root)
            self.proof_setup.append({"name": "prove_shipped_software_change_profiles",
                "status": "passed", "stage": "setup", "returned_assertions": assertions,
                "wall_seconds": time.monotonic() - setup_started})
            # bookends:LE-140 — setup proves selected model-provider extensions
            # are optional and validates only explicitly supplied paths.
            # bookends:LE-92 — proposal-only data is inert and only the driver ledger routes exact implementation tasks.
            # bookends:LE-102 — the public run-plan-graph command refuses missing prerequisites and summarizes the resulting tree.
            enqueue(work_slot_journey.prove_graph_runner,
                provider=self.provider,
                work_dir=proof_root / "graph-runner",
            )
            enqueue(work_slot_journey.prove_engine_standing_join,
                engine=self.engine,
                provider=self.provider,
                work_dir=proof_root / "engine-standing-join",
            )
            # bookends:LE-140 — review fan-out remains entered through invoke and frozen workers.
            # bookends:LE-138 — nested worker stdin/output and Dagu graph shape are asserted.
            enqueue(work_slot_journey.prove_fan_out,
                engine=self.engine,
                work_dir=proof_root / "fan-out",
            )
            enqueue(work_slot_journey.prove_preview_fail_closed,
                engine=self.engine,
                work_dir=proof_root / "preview-fail-closed",
            )
            enqueue(work_slot_journey.prove_preview_pi_extension_warnings,
                engine=self.engine,
                work_dir=proof_root / "preview-pi-extension-warnings",
            )
            enqueue(work_slot_journey.prove_default_sandbox_argv,
                provider=self.provider,
                work_dir=proof_root / "default-sandbox-argv",
            )
            # bookends:LE-91 — the public bound-worker scenario reads the frozen operating context from artifact_root in a fresh CLI process.
            enqueue(work_slot_journey.prove_bound_fan_out_heartbeat,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "bound-fan-out-heartbeat",
            )
            # bookends:LE-110 — elapsed allowance requires wait/cancel and verified cleanup before distinct fresh captures (recovery amendment draft).
            enqueue(work_slot_journey.prove_bound_fan_out_overrun,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "bound-fan-out-overrun",
            )
            # bookends:LE-129 — waiter completion and captured inner worker status are inspected.
            # bookends:LE-80 — the public worker path exercises stdin-exec without shell framing.
            enqueue(work_slot_journey.prove_bound_contracted_fan_out_failure,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "bound-contracted-fan-out-failure",
            )
            # bookends:LE-138 — the public fan-out scenarios cover the worker contract boundary.
            # bookends:LE-93 — the public fan-out scenario preserves both bounded same-worker conformance attempts.
            enqueue(work_slot_journey.prove_full_schema_retry,
                engine=self.engine,
                work_dir=proof_root / "full-schema-retry",
            )
            # bookends:LE-92 — selected retry output remains candidate data until the driver links and dispositions it.
            # bookends:LE-98 — every guarded mutation refuses until the current state was observed.
            enqueue(work_slot_journey.prove_observation_before_mutation,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "observation-before-mutation",
            )
            # bookends:LE-99 — the selected-attempt journey exposes durable engine-owned assignment, capture, and output identity.
            # bookends:LE-100 — the selected invocation's public show asserts deterministic change-report dimensions.
            # bookends:LE-105 — the public provider gate refuses content disagreement with selected bytes.
            enqueue(work_slot_journey.prove_selected_attempt_ledger_linkage,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "selected-attempt-ledger",
            )
            # bookends:LE-107 — subset re-execution, concise evidence references, and one current applicability declaration.
            enqueue(work_slot_journey.prove_subset_applicability_checked,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "subset-carry-checked",
            )
            # bookends:LE-101 — invoke subset starts only the selected fan-out assignment.
            enqueue(work_slot_journey.prove_invoke_subset,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "invoke-subset",
            )
            enqueue(work_slot_journey.prove_stdin_exec,
                provider=self.provider,
                work_dir=proof_root / "stdin-exec",
            )
            enqueue(work_slot_journey.prove_bound_graph_runner_heartbeat,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                checkout_root=self.data_root,
                work_dir=proof_root / "bound-graph-runner-heartbeat",
            )
            enqueue(work_slot_journey.prove_overlay_running_bound_fan_out_progress,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "overlay-running-bound-fan-out",
            )
            enqueue(work_slot_journey.prove_overlay_running_bound_graph_runner_progress,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "overlay-running-bound-graph-runner",
            )
            enqueue(work_slot_journey.prove_max_active_bound_fan_out,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "max-active-bound-fan-out",
            )
            enqueue(work_slot_journey.prove_max_active_bound_graph_runner,
                engine=self.engine,
                provider=self.provider,
                profile_source=self.profile_source,
                fixture_root=self.fixture_root,
                work_dir=proof_root / "max-active-bound-graph-runner",
            )
            if global_jobs is not None:
                for job in jobs:
                    self._append_global_pool_job(
                        global_jobs,
                        name=f"dummy-{job['name']}",
                        kind="dummy-worker",
                        root=proof_root / job["name"],
                        function=job["name"],
                        kwargs=job["kwargs"],
                    )
                return
            # bookends:LE-117 — the complete isolated job inventory uses one
            # bounded pool with retained outcomes and verified cleanup; final
            # comparable performance/hosted proof remains separately driver-owned.
            self.proof_pool_report = proof_pool.run(jobs, root=proof_root / "pool",
                limit=getattr(self.args, "jobs", 2), timeout=getattr(self.args, "job_timeout", 1200))
            print(json.dumps(self.proof_pool_report, indent=2))
            if self.proof_pool_report["status"] != "passed":
                raise proof_pool.PoolFailure(f"proof pool failed; inspect {proof_root / 'pool/summary.json'}")
        except (work_slot_journey.WorkSlotJourneyFailure, proof_pool.PoolFailure) as error:
            raise JourneyFailure(
                str(error),
                state="end",
                event="dummy-worker-proofs",
            ) from error
        self.dummy_worker_proof = list(DUMMY_WORKER_PROOF)
        print(
            "dummy worker proofs passed: shipped profiles, graph-runner, fan-out, "
            "preview-bindings fail-closed, missing -e warning, default sandbox argv, bound heartbeats, "
            "overrun wait/cancel/retry, bounded reviewer retry/exhaustion, selected-attempt linkage, observation guard, "
            "subset invoke, change report, applicability, and content-agreement refusal, stdin-exec, graph working-directory cwd/marker proof, implementation finding routing, "
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


def _run_operational_batch(job: Dict[str, Any]) -> Dict[str, Any]:
    """Run a resource-capped operational batch inside one global-pool slot."""
    root = Path(job["root"]).resolve()
    output_root = root / "cases"
    output_root.mkdir(parents=True, exist_ok=True)
    binary_dir = Path(job["binary_dir"]).resolve()
    released_root = Path(job["released_root"]).resolve()
    script = Path(job["script"]).resolve()
    env_binary = str(job.get("env_binary") or shutil.which("env") or "/usr/bin/env")
    outcomes: Dict[str, Any] = {}
    for case in job["cases"]:
        case_output = output_root / case
        argv = [
            env_binary,
            f"SOFTWARE_CHANGE_JOURNEY_ENGINE={binary_dir / 'loop-engine'}",
            f"SOFTWARE_CHANGE_JOURNEY_PROVIDER={binary_dir / 'software-change'}",
            "PYTHONUNBUFFERED=1",
            sys.executable,
            str(script),
            "--case",
            case,
            "--binary-dir",
            str(binary_dir),
            "--released-root",
            str(released_root),
            "--output-root",
            str(case_output),
        ]
        environment = os.environ.copy()
        environment.update(
            {
                "SOFTWARE_CHANGE_JOURNEY_ENGINE": str(binary_dir / "loop-engine"),
                "SOFTWARE_CHANGE_JOURNEY_PROVIDER": str(binary_dir / "software-change"),
                "PYTHONUNBUFFERED": "1",
            }
        )
        completed = subprocess.run(
            argv,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
            check=False,
        )
        (root / f"{case}.stdout").write_bytes(completed.stdout)
        (root / f"{case}.stderr").write_bytes(completed.stderr)
        if completed.returncode != 0:
            raise JourneyFailure(
                f"operational UX {case} failed ({completed.returncode}); captures: {root}"
            )
        text = completed.stdout.decode("utf-8", "replace")
        for line in reversed(text.splitlines()):
            if not line:
                continue
            try:
                outcome = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(outcome, dict) and outcome.get("case") == case:
                outcomes[case] = outcome
                break
        if case not in outcomes or outcomes[case].get("status") != "passed":
            raise JourneyFailure(
                f"operational UX {case} omitted its public outcome; captures: {root}"
            )
        sys.stdout.write(text)
        sys.stdout.flush()
    return outcomes


def _run_pool_job(job_path: str) -> None:
    """Execute one isolated public proof from a proof-pool worker."""
    job = json.loads(Path(job_path).read_text(encoding="utf-8"))
    args = argparse.Namespace(**job["args"])
    journey = Journey(args)
    root = Path(job["root"]).resolve()
    kind = job["kind"]
    if kind == "successor-route":
        journey._initialize_pool_case(
            root,
            run_id=f"successor-route-parent-{job['index']:02d}",
            prepare_profile=True,
            repository=True,
        )
        revision = job.get("implementation_revision")
        if revision is not None:
            report_path = journey.artifact_root / "implementation-report.json"
            report = journey._read_json(report_path, "isolated route implementation report")
            report["revision"] = revision
            report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        result = journey._run_successor_route_case(
            job["index"],
            job["source"],
            job["event"],
            job["target"],
            implementation_revision=revision,
            isolated=True,
        )
    elif kind == "checkpoint":
        journey._initialize_pool_case(root)
        result = journey._run_checkpoint_case(job["mutation"])
    elif kind == "overlay-off":
        journey._initialize_pool_case(root)
        result = journey._run_overlay_off_source()
    elif kind == "overlay-on":
        journey._initialize_pool_case(root)
        result = journey._run_overlay_on_source(candidate=job["candidate"])
    elif kind == "engine-boundary":
        journey._initialize_pool_case(root)
        result = journey._run_engine_boundary_scenarios()
    elif kind == "reconciliation":
        journey._initialize_pool_case(root)
        result = journey._run_reconciliation_scenarios()
    elif kind == "package-7b":
        journey._initialize_pool_case(root)
        result = journey._run_package_7b_review_candidates_scenario()
    elif kind == "recovery":
        import recovery_journey

        recovery_journey.dispatch(job["scenario"], journey)
        result = None
    elif kind == "dummy-worker":
        kwargs = {
            key: Path(value)
            for key, value in job.get("kwargs", {}).items()
        }
        result = getattr(work_slot_journey, job["function"])(**kwargs)
    elif kind == "stitched":
        # This is the one intentional cross-run fixture: it reads the
        # completed primary database, while every other global job owns a
        # separate database/root.  The parent is quiescent during the pool.
        journey.run_dir = Path(job["parent_run_dir"]).resolve()
        journey.database = Path(job["database"]).resolve()
        journey.provider_config = Path(job["provider_config"]).resolve()
        journey.artifact_root = Path(job["artifact_root"]).resolve()
        journey.profile_path = Path(job["profile_path"]).resolve()
        journey.profile_source = Path(job["profile_source"]).resolve()
        journey.profile = journey._read_json(journey.profile_source, "primary stitched profile")
        journey.fixture_root = journey.data_root / FIXTURE_SUBPATH
        journey.repository_root = (
            Path(job["repository_root"]).resolve()
            if job.get("repository_root")
            else None
        )
        journey.command_cwd = journey.repository_root
        journey.state = "end"
        journey.run_id = job["run_id"]
        journey.work_slot_bindings = copy.deepcopy(job.get("work_slot_bindings", {}))
        result = journey._run_stitched_source()
    elif kind == "operational-batch":
        result = _run_operational_batch(job)
    else:
        raise JourneyFailure(f"unknown isolated proof-pool job kind: {kind}")
    output = job.get("output")
    if output:
        Path(output).write_text(
            json.dumps({"status": "passed", "result": str(result)}) + "\n",
            encoding="utf-8",
        )
    print(f"isolated proof-pool job passed: {kind}", flush=True)


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
        default="full",
        help="full source graph or checked software-change prefix",
    )
    parser.add_argument(
        "--compact-worker-fixture",
        choices=("draft", "review", "negative-empty"),
        help="run the fresh supported compact-worker setup/start/invoke fixture",
    )
    parser.add_argument("--worker-model", help="model ID for a positive compact-worker fixture")
    parser.add_argument("--worker-thinking", help="thinking level for a positive compact-worker fixture")
    parser.add_argument("--worker-tools", help="comma-separated tools for a positive compact-worker fixture")
    parser.add_argument("--jobs", type=int, default=2, help="independent proof processes (default 2; serial 1)")
    parser.add_argument("--job-timeout", type=float, default=1200, help="per-proof deadline in seconds")
    parser.add_argument("--scenario", help="focused implemented recovery scenario (source only)")
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
        if args[index] == "--then":
            index += 1
            continue
        if args[index] == "--max-active":
            index += 2
            continue
        if args[index].startswith("--max-active="):
            index += 1
            continue
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


def _policy_author_batches(policies, roster):
    stages = []
    for policy in policies:
        stage = policy.get("review_stage", "aggregate")
        if stage not in stages:
            stages.append(stage)
    batches = []
    for stage in stages:
        stage_policies = [
            policy
            for policy in policies
            if policy.get("review_stage", "aggregate") == stage
        ]
        if stage == "individual":
            # High-rigor individual assignments are deliberately singleton
            # axis reviews. Aggregate assignments remain one all-axis batch per
            # author so a correction can select only affected individuals.
            pairs = _policy_author_pairs(stage_policies, roster)
            for entry in roster:
                for policy, author in pairs:
                    if author == entry:
                        batches.append(([policy], entry))
        else:
            pairs = _policy_author_pairs(stage_policies, roster)
            batches.extend(
                (assigned, entry)
                for entry in roster
                if (assigned := [policy for policy, author in pairs if author == entry])
            )
    return batches


def _batch_schema(schema, policies, author):
    result = copy.deepcopy(schema)
    result["properties"]["author"]["const"] = author
    if policies:
        result["properties"]["review_stage"]["const"] = policies[0].get("review_stage", "aggregate")
    rows = result["properties"]["judgments"]
    rows["minItems"] = rows["maxItems"] = len(policies)
    for branch in rows["items"]["oneOf"]:
        branch["properties"]["axis"]["enum"] = [p["id"] for p in policies]
    rows["allOf"] = [{"contains": {"type": "object", "required": ["axis"],
        "properties": {"axis": {"const": p["id"]}}}} for p in policies]
    return result


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
    criterion_contract: bool = False,
) -> None:
    if worker.get("command") != pi_command:
        raise JourneyFailure(f"worker command {worker.get('command')!r} != {pi_command!r}")
    expected_schema = schema
    if schema_field == "full_output_schema":
        expected_schema = _batch_schema(schema, policy, {
            "name": roster_entry["author"], "kind": "agent"})
    if criterion_contract:
        expected_schema = copy.deepcopy(expected_schema)
        expected_schema["properties"]["validation_verdicts"] = {
            "type": "array", "items": {"type": "object", "additionalProperties": False,
                "required": ["record_id", "kind", "data"], "properties": {
                    "record_id": {"type": "string", "minLength": 1},
                    "kind": {"type": "string", "enum": ["criterion-verdict", "goal-verdict"]},
                    "data": {"type": "object"}}}}
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
    if schema_field == "full_output_schema":
        assigned = next(line.removeprefix("assigned_policies: ") for line in preamble.splitlines()
                        if line.startswith("assigned_policies: "))
        if json.loads(assigned) != policy:
            raise JourneyFailure("batch changed assigned policy order or exact prompts")
    else:
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
    engine_binary: Optional[Path] = None,
) -> None:
    if len(workers) == 0:
        raise JourneyFailure("constructor preview input had no workers")
    full_preambles = []
    for worker in workers:
        if "preamble" not in worker or schema_field not in worker:
            raise JourneyFailure(f"preview input omitted preamble/schema: {worker}")
        required = (worker.get(schema_field) or {}).get("required")
        expected_required = ["review_stage", "author", "judgments"] if schema_field == "full_output_schema" else ["axis", "author", "result", "findings"]
        if required != expected_required:
            raise JourneyFailure(f"preview input omitted required keys: {worker}")
        preamble = worker.get("preamble")
        if isinstance(preamble, str) and preamble:
            full_preambles.append(preamble)
    engine = engine_binary or (repository / "target/debug/loop-engine")
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
        if required != expected_required:
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


def assert_worker_data_skill_and_root_policy(
    *,
    engine_binary: Optional[Path] = None,
    provider_binary: Optional[Path] = None,
) -> None:
    """Execute provider constructors and assert root policy against revision-18 contracts."""
    repository = Path(__file__).resolve().parent.parent
    dummy_engine = "/tmp/loop-engine-constructor-proof"
    dummy_pi = "/tmp/pi-constructor-proof"
    dummy_cursor = str(repository / "crates/policy-document-provider/data/semantic-review-worker-preamble.md")
    dummy_bridge = str(repository / "crates/policy-document-provider/data/semantic-review-worker-output-schema.json")
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
        if name == "software-change":
            required = ("software-change setup", "--roster", "--draft-worker", "output_sha256", "preview-bindings")
        else:
            required = ("--rawfile base_preamble", "preview-bindings", "SHA-256", "validate_extension_path")
        for clause in required:
            if clause.lower() not in skill.lower():
                raise JourneyFailure(f"{name} constructor/setup omitted {clause!r}")

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
        "extra mechanisms, unlisted requirements and hypothetical-future",
        "latest driver finding-ledger disposition",
        "Confirmation revisits affected judgments and fix-introduced holes",
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
        "review-candidates",
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
    fresh_row = copy.deepcopy(full_review_schema)
    fresh_row["required"].remove("author")
    del fresh_row["properties"]["author"]
    # The batch retains the same pass/fail relation, with a separate closed carry row.
    expected_batch = {"type": "object", "additionalProperties": False,
        "required": ["review_stage", "author", "judgments"], "properties": {
        "review_stage": {"type": "string", "enum": ["individual", "aggregate"]},
        "author": full_review_schema["properties"]["author"],
        "judgments": {"type": "array", "minItems": 1, "items": {"oneOf": [fresh_row,
            {"type": "object", "additionalProperties": False, "required": ["axis", "reuse"],
             "properties": {"axis": {"type": "string", "minLength": 1}, "reuse": {"type": "string", "minLength": 1}}}]}}}}
    expected_batch["properties"]["judgments"]["items"]["oneOf"][0]["properties"]["result"].pop("type")
    expected_batch["properties"]["judgments"]["items"]["oneOf"][0]["oneOf"][1]["properties"]["findings"].pop("type")
    expected_batch["x-loop-engine-force-fresh"] = {
        "properties": {"judgments": {"items": {"required": ["result", "findings"]}}}}
    if sc_schema != expected_batch:
        raise JourneyFailure("software-change complete batch output schema bytes are unsupported")
    if pd_schema != schema_required or research_schema != schema_required:
        raise JourneyFailure("provider output_schema bytes do not require axis/author/result/findings")

    software_change_binary = provider_binary or (repository / "target/debug/software-change")
    loop_engine_binary = engine_binary or (repository / "target/debug/loop-engine")
    if not software_change_binary.is_file() or not loop_engine_binary.is_file():
        raise JourneyFailure(
            "software-change setup self-test requires target/debug/loop-engine and target/debug/software-change"
        )

    def run_sc(
        output: Path,
        roster_path: Path,
        rigor: str = "high",
        draft_worker_path: Optional[Path] = None,
    ) -> Dict[str, Any]:
        command = [
            str(software_change_binary),
            "setup",
            "--rigor",
            rigor,
            "--roster",
            str(roster_path),
            "--engine",
            str(loop_engine_binary),
            "--provider",
            str(software_change_binary),
        ]
        if draft_worker_path is not None:
            command.extend(["--draft-worker", str(draft_worker_path)])
        command.extend(["--output", str(output)])
        result = subprocess.run(
            command,
            cwd=str(output.parent),
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            raise ConstructorClosed(detail or f"software-change setup exited {result.returncode}")
        try:
            report = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise JourneyFailure(
                f"software-change setup stdout was not JSON: {result.stdout}"
            ) from error
        if not output.is_file():
            raise JourneyFailure("software-change setup did not write its output profile")
        return report

    def assert_sc_binding(
        profile: Dict[str, Any],
        source: Dict[str, Any],
        gate: str,
        roster_entries: Sequence[Dict[str, Any]],
    ) -> None:
        bindings = profile.get("work_slot_bindings")
        if not isinstance(bindings, dict) or gate not in bindings:
            raise JourneyFailure(f"software-change setup omitted {gate} binding")
        binding = bindings[gate]
        workers = _fan_out_workers(binding, engine=str(loop_engine_binary))
        expected = _policy_author_batches(source["review_policies"][gate], roster_entries)
        if len(workers) != len(expected):
            raise JourneyFailure(
                f"software-change setup {gate} worker count {len(workers)} != {len(expected)}"
            )
        if binding.get("context_filter") != {
            "command": str(software_change_binary),
            "args": ["commission"],
        }:
            raise JourneyFailure(f"software-change setup {gate} lost its commission filter")
        args = binding.get("args") or []
        if args[:3] != ["fan-out", "--max-active", "2"]:
            raise JourneyFailure(f"software-change setup {gate} lost review concurrency: {args}")
        for worker, (policies, entry) in zip(workers, expected):
            if worker.get("command") != entry["command"] or worker.get("args") != entry["args"]:
                raise JourneyFailure(
                    f"software-change setup changed worker command/args: {worker} != {entry}"
                )
            expected_schema = _batch_schema(
                sc_schema, policies, {"name": entry["author"], "kind": "agent"}
            )
            if (
                gate == "validation-review"
                and expected_schema["properties"]["review_stage"].get("const") == "aggregate"
            ):
                expected_schema["properties"]["validation_verdicts"] = {
                    "type": "array", "items": {"type": "object", "additionalProperties": False,
                    "required": ["record_id", "kind", "data"], "properties": {
                        "record_id": {"type": "string", "minLength": 1},
                        "kind": {"type": "string", "enum": ["criterion-verdict", "goal-verdict"]},
                        "data": {"type": "object"}}}}
            if worker.get("full_output_schema") != expected_schema:
                raise JourneyFailure(f"software-change setup changed worker schema: {worker}")
            assigned = next(
                line.removeprefix("assigned_policies: ")
                for line in worker.get("preamble", "").splitlines()
                if line.startswith("assigned_policies: ")
            )
            if json.loads(assigned) != policies:
                raise JourneyFailure("software-change setup changed assigned policy order or prompts")
            if not worker.get("preamble", "").startswith(sc_preamble):
                raise JourneyFailure("software-change setup did not retain shipped preamble bytes")
        _assert_preview_visibility(
            repository, {gate: binding}, workers, schema_field="full_output_schema",
            engine_binary=loop_engine_binary,
        )

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

    def pd_args(roster_path: Path, *, extensions: bool = True) -> List[str]:
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
            dummy_cursor if extensions else "",
            "--arg",
            "claude_bridge_extension",
            dummy_bridge if extensions else "",
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

    def research_args(
        slot_id: str, roster_json: str, *, extensions: bool = True
    ) -> List[str]:
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
            dummy_cursor if extensions else "",
            "--arg",
            "claude_bridge_extension",
            dummy_bridge if extensions else "",
            "--rawfile",
            "base_preamble",
            str(research_preamble_path),
            "--slurpfile",
            "output_schema",
            str(research_schema_path),
        ]

    def run_pd(
        profile: Path,
        roster_path: Path,
        *,
        slot_id: str = "semantic-review",
        extensions: bool = True,
    ) -> Dict[str, Any]:
        extra = pd_args(roster_path, extensions=extensions)
        extra[2] = slot_id
        stdout = _run_jq(pd_jq, profile, extra)
        profile.write_text(stdout, encoding="utf-8")
        return _load_json(profile)

    def run_research(
        profile: Path,
        slot_id: str,
        roster_json: str,
        *,
        extensions: bool = True,
    ) -> Dict[str, Any]:
        extra = research_args(slot_id, roster_json, extensions=extensions)
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

        setup_roster = [
            {"author": "reviewer-a", "command": "/tmp/software-change-worker-a", "args": ["--worker", "a"]},
            {"author": "reviewer-b", "command": "/tmp/software-change-worker-b", "args": ["--worker", "b"]},
        ]
        setup_roster_path = root / "software-change-roster.json"
        _write_json(setup_roster_path, setup_roster)
        draft_worker_path = root / "software-change-draft-worker.json"
        draft_worker = {"command": dummy_pi, "args": ["--draft"]}
        _write_json(draft_worker_path, draft_worker)
        setup_source = _load_json(high_rigor)
        setup_profile = root / "software-change-setup.json"
        setup_report = run_sc(
            setup_profile,
            setup_roster_path,
            draft_worker_path=draft_worker_path,
        )
        if setup_report.get("status") != "ready" or setup_report.get("started") is not False:
            raise JourneyFailure(f"software-change setup report was not an inert ready report: {setup_report}")
        if setup_report.get("output_bytes") != setup_profile.read_text(encoding="utf-8"):
            raise JourneyFailure("software-change setup report lost exact output bytes")
        if setup_report.get("output_sha256") != _sha256_file(setup_profile):
            raise JourneyFailure("software-change setup report lost output hash")
        setup_result = _load_json(setup_profile)
        if setup_result.get("review_policies") != setup_source.get("review_policies"):
            raise JourneyFailure("software-change setup changed shipped review policy bytes")
        if setup_result.get("work_slot_bindings", {}).get("intent-draft") != draft_worker:
            raise JourneyFailure(
                "software-change setup did not preserve the closed --draft-worker binding"
            )
        for gate in setup_source["review_policies"]:
            assert_sc_binding(setup_result, setup_source, gate, setup_roster)
        high_workers = _fan_out_workers(
            setup_result["work_slot_bindings"]["intent-review"], engine=str(loop_engine_binary)
        )
        if not any(
            "fresh independent reviewer session" in worker.get("preamble", "")
            and "individual-stage captures" in worker.get("preamble", "")
            for worker in high_workers
            if worker.get("full_output_schema", {}).get("properties", {}).get("review_stage", {}).get("const") == "aggregate"
        ):
            raise JourneyFailure("high-rigor setup omitted fresh aggregate exclusion framing")
        _assert_hash_guard(setup_profile)

        duplicate_roster_path = root / "software-change-duplicate-roster.json"
        _write_json(
            duplicate_roster_path,
            [setup_roster[0], {"author": setup_roster[0]["author"], "command": "/tmp/other", "args": []}],
        )
        _expect_constructor_closed(
            lambda: run_sc(root / "software-change-duplicate.json", duplicate_roster_path),
            needle="duplicate author",
            context="software-change setup duplicate author",
        )
        short_roster_path = root / "software-change-short-roster.json"
        _write_json(short_roster_path, [setup_roster[0]])
        _expect_constructor_closed(
            lambda: run_sc(root / "software-change-short.json", short_roster_path),
            needle="requires 2",
            context="software-change setup insufficient roster",
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
            _assert_preview_visibility(
                repository, pd_bindings, pd_workers, engine_binary=loop_engine_binary
            )
            _assert_hash_guard(dest)
            for worker in pd_workers:
                args = worker.get("args")
                if not isinstance(args, list) or args[args.index("-e") : args.index("-e") + 2] != ["-e", dummy_cursor]:
                    raise JourneyFailure(f"{label} constructor lost the supplied cursor extension")
                second_extension = args.index("-e", args.index("-e") + 1)
                if args[second_extension : second_extension + 2] != ["-e", dummy_bridge]:
                    raise JourneyFailure(f"{label} constructor lost the supplied bridge extension")

        pd_no_extensions = root / "pd-no-extensions.json"
        no_extension_profile = _load_json(readme_profile)
        no_extension_profile["target"]["path"] = str((root / "pd-no-extension-target.md").resolve())
        _write_json(pd_no_extensions, no_extension_profile)
        no_extension_result = run_pd(pd_no_extensions, roster_path, extensions=False)
        no_extension_workers = _fan_out_workers(
            no_extension_result["work_slot_bindings"]["semantic-review"], engine=dummy_engine
        )
        if not no_extension_workers or any(
            "-e" in (worker.get("args") or []) for worker in no_extension_workers
        ):
            raise JourneyFailure(
                "policy-document constructor emitted an extension pair for an omitted path"
            )

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
            _assert_preview_visibility(
                repository, research_bindings, research_workers, engine_binary=loop_engine_binary
            )
            _assert_hash_guard(research_dest)
            for worker in research_workers:
                args = worker.get("args")
                if not isinstance(args, list) or args[args.index("-e") : args.index("-e") + 2] != ["-e", dummy_cursor]:
                    raise JourneyFailure(f"research {slot_id} constructor lost the supplied cursor extension")
                second_extension = args.index("-e", args.index("-e") + 1)
                if args[second_extension : second_extension + 2] != ["-e", dummy_bridge]:
                    raise JourneyFailure(f"research {slot_id} constructor lost the supplied bridge extension")

        for slot_id in ("verify", "synthesize"):
            no_extension_dest = root / f"research-{slot_id}-no-extensions.json"
            shutil.copy2(research_profile, no_extension_dest)
            no_extension_result = run_research(
                no_extension_dest, slot_id, roster_json, extensions=False
            )
            no_extension_workers = _fan_out_workers(
                no_extension_result["work_slot_bindings"][slot_id], engine=dummy_engine
            )
            if not no_extension_workers or any(
                "-e" in (worker.get("args") or []) for worker in no_extension_workers
            ):
                raise JourneyFailure(
                    f"research {slot_id} emitted an extension pair for an omitted path"
                )

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
        "Keep driver-authored metadata small, trust explicit materiality and applicability declarations except for cheap mechanical identity mismatches",
        "prefer the narrowest honest correction, and preserve rich engine-generated history",
        "[`docs/PRD.md`](docs/PRD.md) LE-107",
    )
    provider_policy_fragments = (
        "before the commit introducing engine `LE-107`, the owner-accepted wording is a proposal; once it is present in committed `docs/PRD.md`, it is authoritative.",
        "subordinate to that engine PRD and this provider PRD, and is referential rather than a second authority",
        "Apply LE-107's observed-ordinary-failure/smaller-mechanism burden",
        "retain R8 and R13 freshness and subject/identity checks, but do not repeat mechanically available invocation, attempt, digest, path, or coverage facts in driver-authored records",
        "Preserve R16's independent-author aggregation and visible verdict history",
        "R21's retained review, materiality, triage and source-visibility rules remain",
        "normal disposition is exact-source driver judgment",
        "exceptional owner override is distinct engine history, never reviewer success",
        "Accepted-unresolved findings block across revisions",
        "retired-author needs recorded roster departure and replacement coverage",
        "Historical missing ownership is unsupported",
        "independent criterion/goal author floors of 1/2/2",
        "The separate deterministic `setup` helper assembles per-run bindings from shipped review data and explicit caller commands",
        "delivered stable references use context-record or invocation/assignment identities",
        "Frozen requirements this crate's acceptance suite traces to (R1–R29",
        "`software-change --help`/`-h` names `describe`, `evaluate`, `setup`",
        "`review-candidates` reads one completed `show --view full` envelope from stdin",
    )
    # LE-107 — the self-test checks referential root/provider operational summaries against the PRD authority boundary.
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
    assert_criterion_spine_docs()
    assert_reconciliation_documents_and_profiles()
    assert_focused_boundary_scenarios()
    print("worker-data skill/root policy assertions passed")


# bookends:LE-141 bookends:LE-144
# These assertions check every shipped profile's ordinary/challenge questions,
# stages and author floors; the full source traversal exercises their gates.
# Supplied-material semantic calibration remains separate external evidence.
def assert_reconciliation_documents_and_profiles() -> None:
    """Check shipped profile floors and the authored reconciliation contract."""
    repository = Path(__file__).resolve().parent.parent
    readme = (repository / "crates/software-change-provider/README.md").read_text(encoding="utf-8")
    agents = (repository / "crates/software-change-provider/AGENTS.md").read_text(encoding="utf-8")
    for label, text, clauses in (
        ("provider README", readme, ("Reconciliation and document integration", "no-document-change", "Bookends-disabled runs", "reconciliation-ready")),
        ("provider AGENTS", agents, ("reconciliation", "reconciliation-ready", "Bookends-on", "older v10 runs retain")),
    ):
        for clause in clauses:
            if clause.lower() not in text.lower():
                raise JourneyFailure(f"{label} omitted reconciliation contract clause {clause!r}")
    expected = {
        "minimal.json": ("minimal-11", 1, 1),
        "standard.json": ("standard-11", 2, 2),
        "high-rigor.json": ("high-rigor-11", 2, 2),
    }
    for name, (version, criterion_floor, goal_floor) in expected.items():
        profile = _load_json(repository / "crates/software-change-provider/data/configs" / name)
        if profile.get("config_version") != version:
            raise JourneyFailure(f"{name} profile version changed unexpectedly: {profile.get('config_version')!r}")
        if profile.get("criterion_policy") != {"required_authors": criterion_floor, "goal_required_authors": goal_floor}:
            raise JourneyFailure(f"{name} profile criterion/goal floors changed unexpectedly")
        policies = profile.get("review_policies", {})
        for gate in ("intent-review", "intent-adversarial-review"):
            axes = policies.get(gate, [])
            ids = {entry.get("id") for entry in axes if isinstance(entry, dict)}
            if not {"acceptance-granularity", "owner-comprehensible"}.issubset(ids):
                raise JourneyFailure(f"{name} {gate} omitted the shipped intent questions")
            for entry in axes:
                if entry.get("id") in {"acceptance-granularity", "owner-comprehensible"} and entry.get("review_stage", "aggregate") != "aggregate":
                    raise JourneyFailure(f"{name} changed the new intent-question stage")
    schema = _load_json(repository / "crates/software-change-provider/data/reconciliation-schema.json")
    if schema.get("additionalProperties") is not False or set(schema.get("required", [])) != {
        "revision", "author", "mode", "branch", "document_observations", "behavior_observations",
        "action", "action_reason", "authorization", "application", "commit", "traceability",
        "proof_references", "blockers", "decision",
    }:
        raise JourneyFailure("reconciliation schema is not the closed provider contract")
    fixture_root = repository / FIXTURE_SUBPATH
    for name in ("sufficient", "related-insufficient", "implementation-defect"):
        fixture = _load_json(fixture_root / f"requirement-coverage-{name}.json")
        if fixture.get("handoff", {}).get("proof_references") is None:
            raise JourneyFailure(f"requirement coverage fixture {name} omitted its proof handoff")
    notes = repository / "crates/software-change-provider/data/calibration/companions/fictional-repo/docs/requirement-coverage.md"
    if "proactive owner chat" not in notes.read_text(encoding="utf-8"):
        raise JourneyFailure("requirement coverage companion lost its unrelated-owner-chat contrast")


def assert_criterion_spine_docs() -> None:
    """Keep the provider-facing criterion and overlay instructions aligned."""
    repository = Path(__file__).resolve().parent.parent
    criterion_docs = (
        repository / "crates/software-change-provider/README.md",
        repository / "crates/software-change-provider/AGENTS.md",
        repository / "crates/software-change-provider/docs/prd.md",
        repository / "crates/software-change-provider/data/reviewer-protocol.md",
        repository / "crates/software-change-provider/data/calibration/PROCEDURE.md",
        repository / "crates/software-change-provider/skills/using-software-change-provider/SKILL.md",
        repository / "crates/software-change-provider/data/templates/intent.md",
        repository / "crates/software-change-provider/data/templates/design.md",
        repository / "crates/software-change-provider/data/templates/task-packet.md",
        repository / "crates/software-change-provider/data/templates/implementation-report.md",
        repository / "crates/software-change-provider/data/templates/validation-report.md",
    )
    for path in criterion_docs:
        text = path.read_text(encoding="utf-8").lower()
        if "ac-n" not in text:
            raise JourneyFailure(f"criterion-spine documentation {path} omitted AC-N")
    overlay_docs = criterion_docs[:6] + (
        criterion_docs[6],
        criterion_docs[-1],
    )
    for path in overlay_docs:
        text = path.read_text(encoding="utf-8").lower()
        for clause in ("prd_traceability", "candidate", "not-applicable"):
            if clause not in text:
                raise JourneyFailure(
                    f"criterion overlay documentation {path} omitted {clause!r}"
                )
        if "waive" not in text or "fulfill" not in text:
            raise JourneyFailure(
                f"criterion overlay documentation {path} omitted the non-waiver rule"
            )


def assert_operator_contract_surfaces() -> None:
    """Assert the existing public procedure exposes the accepted operator contract."""
    repository = Path(__file__).resolve().parent.parent
    root_readme = (repository / "README.md").read_text(encoding="utf-8")
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
    agent_usage = (repository / "docs/agent-usage.md").read_text(encoding="utf-8")
    contract = "\n".join(
        (root_readme, root_policy, engine_skill, provider_skill, protocol, agent_usage)
    )
    for clause in (
        "software-change setup",
        "config_version",
        "live review states",
        "normalized `required_authors`",
        "Bookends enabled/disabled state",
        "output_sha256",
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
        "evidence-applicability",
        "context-record",
        "engine-owned",
        "does not infer semantic applicability",
        "current target",
        "Historical completed-run records",
        "challenge review",
        "meaningfully falsify",
        "current supplied evidence",
        "concrete consequence",
        "review-candidates",
        "selected-assignment-output",
        "accept, edit, or reject",
        "not evidence or semantic review",
        "raw attempts",
        "deduplicate",
    ):
        if clause.lower() not in contract.lower():
            raise JourneyFailure(f"operator procedure omitted {clause!r}")

    # The ordinary procedures use only the stable-reference contract. Legacy
    # forms remain readable only through the explicit historical PRD note and
    # negative compatibility tests, not as driver instructions.
    legacy_fields = (
        "unchanged" + "-carry",
        "override" + "-carry",
        "originating" + "_output",
        "external" + "-artifact",
    )
    for legacy in legacy_fields:
        if any(
            legacy in text
            for text in (
                root_readme,
                root_policy,
                engine_skill,
                provider_skill,
                protocol,
                agent_usage,
            )
        ):
            raise JourneyFailure(f"ordinary operator guidance still exposes legacy field {legacy!r}")

    # Shipped profiles are intentionally changed by the criterion-spine
    # package.  Validate their current contract here instead of comparing the
    # worktree to HEAD: a source journey must run against the same bytes it
    # proves.
    expected_versions = {
        "minimal.json": "minimal-11",
        "standard.json": "standard-11",
        "high-rigor.json": "high-rigor-11",
    }
    for name, expected_version in expected_versions.items():
        profile = _load_json(repository / "crates/software-change-provider/data/configs" / name)
        if profile.get("config_version") != expected_version:
            raise JourneyFailure(
                f"criterion-spine profile {name} has {profile.get('config_version')!r}, "
                f"expected {expected_version!r}"
            )
        if profile.get("extra", {}).get("bookends") is not None:
            raise JourneyFailure(f"shipped profile unexpectedly enables Bookends: {name}")
        acceptance = (
            profile.get("artifact_schemas", {})
            .get("intent.json", {})
            .get("properties", {})
            .get("acceptance", {})
            .get("items", {})
        )
        if (
            acceptance.get("type") != "object"
            or set(acceptance.get("required", [])) != {"id", "statement"}
            or set(acceptance.get("properties", {})) != {"id", "statement"}
            or acceptance.get("additionalProperties") is not False
            or acceptance.get("properties", {}).get("id", {}).get("pattern")
            != "^AC-[1-9][0-9]*$"
        ):
            raise JourneyFailure(f"shipped profile lacks the closed AC-N acceptance shape: {name}")

    workflow_source = (
        repository / "crates/software-change-provider/src/workflow.rs"
    ).read_text(encoding="utf-8")
    if (
        "REVIEW_EVIDENCE_KIND.to_owned()" not in workflow_source
        or "EVIDENCE_APPLICABILITY_KIND.to_owned()" not in workflow_source
    ):
        raise JourneyFailure(
            "implementation slot stopped forwarding stable-reference source context"
        )
    work_slot_source = (repository / "scripts/work_slot_journey.py").read_text(
        encoding="utf-8"
    )
    for function_name in (
        "prove_selected_attempt_ledger_linkage",
        "prove_subset_applicability_checked",
    ):
        function_start = work_slot_source.index(f"def {function_name}(")
        function_end = work_slot_source.find("\ndef ", function_start + 1)
        function_source = work_slot_source[
            function_start : function_end if function_end >= 0 else len(work_slot_source)
        ]
        for clause in ("context-record", "evidence-applicability", '"origin"'):
            if clause not in function_source:
                raise JourneyFailure(
                    f"{function_name} omitted stable-reference clause {clause!r}"
                )
        legacy_fields = (
            "originating" + "_output",
            "external" + "-artifact",
            "unchanged" + "-carry",
            "override" + "-carry",
        )
        for legacy in legacy_fields:
            if legacy in function_source:
                raise JourneyFailure(
                    f"{function_name} retained legacy provenance path {legacy!r}"
                )
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
    # Outcome prose is retained evidence, no longer the live report index.
    good_validation = _load_json(fixture_root / "validation-evidence-2026-08-12.json")
    assert_semantic_outcome_proof_contract(plan, good_validation, scenario_source)
    for invalid_validation in (
        _load_json(fixture_root / "validation-evidence-2026-08-13.json"),
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
    if "self._run_global_tail_proof()" not in source:
        raise JourneyFailure("global tail proof is not in the full source journey")
    if "reconciliation journey passed:" not in source or "_run_reconciliation_scenarios" not in source:
        raise JourneyFailure("reconciliation public journey is not wired into the source boundary")
    for case_name in (
        "bookends-disabled-no-change",
        "bookends-disabled-edit",
        "bookends-enabled-edit",
        "bookends-enabled-unresolved",
        "bookends-enabled-missing-authorization",
    ):
        if case_name not in source:
            raise JourneyFailure(f"reconciliation journey omitted case {case_name}")
    if "bookends:LE-142" not in source or "pending-owner-integration" not in source:
        raise JourneyFailure("reconciliation journey omitted exact live-citation/pending-owner handling")
    reconciliation_start = source.index("    def _run_reconciliation_case")
    reconciliation_end = source.find("\n    def ", reconciliation_start + len("    def _run_reconciliation_case"))
    reconciliation_source = source[reconciliation_start:reconciliation_end if reconciliation_end >= 0 else len(source)]
    if (
        'self._expect_allow("implementation-ready", "reconciliation")' not in reconciliation_source
        or 'self._expect_allow("reconciliation-ready", "implementation-review")' not in reconciliation_source
        or 'self._commit_fixture_document(target)' not in reconciliation_source
    ):
        raise JourneyFailure("reconciliation journey omitted checked entry, exit, or committed document assertions")
    global_start = source.index("    def _run_global_tail_proof")
    global_end = source.find("    def ", global_start + len("    def _run_global_tail_proof"))
    global_source = source[global_start:global_end if global_end >= 0 else len(source)]
    if (
        'name="engine-boundary"' not in global_source
        or 'name="reconciliation"' not in global_source
        or 'name="package-7b"' not in global_source
        or "self._run_dummy_worker_proofs(global_jobs=global_jobs)" not in global_source
        or "prove_selected_attempt_ledger_linkage" not in source
        or "prove_subset_applicability_checked" not in source
    ):
        raise JourneyFailure("global tail or stable-reference applicability proof is not in the full source journey")

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
        138: (
            "_run_dummy_worker_proofs",
            ("prove_fan_out", "prove_full_schema_retry"),
        ),
        91: (
            "_run_full_source",
            ("operating_context", "operating-context-show"),
        ),
        92: (
            "_run_dummy_worker_proofs",
            (
                "prove_graph_runner",
                "prove_selected_attempt_ledger_linkage",
                "prove_subset_applicability_checked",
            ),
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
        107: (
            "_run_dummy_worker_proofs",
            (
                "prove_selected_attempt_ledger_linkage",
                "prove_subset_applicability_checked",
                "current applicability declaration",
            ),
        ),
        108: (
            "_run_full_source",
            (
                "self._run_ad_hoc_repair_proof(",
                '"implementation-adversarial-review"',
            ),
        ),
        109: (
            "_run_package_7b_review_candidates_scenario",
            (
                "review-candidates",
                "selected retry",
                "exhausted assignment",
                "raw capture",
                "inert-before-records",
                "attempt_count_paths",
                "candidate_counts_before",
                "candidate_counts_after",
                "foreign_projection",
                "driver-action-afterward",
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

    overlay_scenarios = {
        "_run_overlay_off_source": (
            "_assert_overlay_off_criterion_spine",
            "all five authored overlay-off artifacts",
        ),
        "_run_overlay_on_source": (
            "prd_traceability",
            "software-change-bookends-candidate",
            "not-applicable",
            "AC-3 remains unfulfilled",
        ),
    }
    for function_name, assertions in overlay_scenarios.items():
        function_start = source.index(f"    def {function_name}")
        function_end = source.find("\n    def ", function_start + len(f"    def {function_name}"))
        function_source = source[function_start:function_end + 1 if function_end >= 0 else len(source)]
        for assertion in assertions:
            if assertion not in function_source:
                raise JourneyFailure(
                    f"criterion overlay scenario {function_name} omitted {assertion!r}"
                )
    for marker in (
        "overlay-off criterion spine scenario passed:",
        "overlay-on criterion scenarios passed:",
    ):
        if marker not in source:
            raise JourneyFailure(f"criterion overlay marker missing: {marker}")

    repair_start = source.index("    def _run_ad_hoc_repair_proof")
    repair_end = source.find("\n    def ", repair_start + len("    def _run_ad_hoc_repair_proof"))
    repair_source = source[repair_start:repair_end + 1 if repair_end >= 0 else len(source)]
    for assertion in (
        "repair_finding_ids",
        "fail-before-Dagu refusals",
        "ad hoc repair journey passed:",
    ):
        if assertion not in repair_source:
            raise JourneyFailure(
                f"LE-108 repair scenario omitted observable assertion marker {assertion!r}"
            )

    print("focused citation scenarios remain bound to public assertions")


def self_test() -> int:
    import recovery_journey
    recovery_journey.self_test()
    source = Path(__file__).read_text(encoding="utf-8")
    assert "self._run_recovery_inventory()" in source
    assert "for name in SCENARIOS:" in source
    subprocess.run([sys.executable, str(Path(__file__).with_name("proof-pool-self-test.py"))], check=True)
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
    constructor_engine = os.environ.get("SOFTWARE_CHANGE_JOURNEY_ENGINE")
    constructor_provider = os.environ.get("SOFTWARE_CHANGE_JOURNEY_PROVIDER")
    assert_worker_data_skill_and_root_policy(
        engine_binary=Path(constructor_engine).expanduser().resolve()
        if constructor_engine else None,
        provider_binary=Path(constructor_provider).expanduser().resolve()
        if constructor_provider else None,
    )
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
        if args.scenario:
            # Cancellation asserts bounded cleanup and admission inhibition.
            # Backtracking asserts all three implement routes refuse live,
            # elapsed, or cleanup-pending work, then reach the exact owner
            # without a report, retaining captures/history and old graphs.
            import recovery_journey
            try:
                journey = Journey(args)
                if args.scenario == "override":
                    journey._run_recovery_override()
                elif args.scenario == "criteria":
                    journey._run_recovery_criteria()
                elif args.scenario == "execution-controls":
                    # bookends:LE-111 — real preview/amend/invoke retains original
                    # input and attempts and proves later corrected execution.
                    recovery_journey.dispatch(args.scenario, journey)
                elif args.scenario == "cancellation":
                    # bookends:LE-112 — real cancellation verifies resistant
                    # descendants/reaping, no later tasks, and delayed resumption.
                    recovery_journey.dispatch(args.scenario, journey)
                elif args.scenario == "steering":
                    # bookends:LE-114 — recipient-selected immutable steering
                    # changes observable later work through the public commission.
                    recovery_journey.dispatch(args.scenario, journey)
                elif args.scenario == "dispositions":
                    # bookends:LE-115 — exact-source discharge/retirement keeps
                    # raw failures and rejects unresolved/missing author coverage.
                    recovery_journey.dispatch(args.scenario, journey)
                elif args.scenario == "batched-review":
                    # bookends:LE-130 — shipped stage-aware construction drives
                    # per-axis individual and aggregate capture/candidates/triage.
                    recovery_journey.dispatch(args.scenario, journey)
                else:
                    recovery_journey.dispatch(args.scenario, journey)
            except ValueError as error:
                raise JourneyFailure(str(error)) from error
        else:
            journey = Journey(args)
            if args.compact_worker_fixture:
                journey.run_compact_worker_fixture()
            else:
                journey.run()
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
