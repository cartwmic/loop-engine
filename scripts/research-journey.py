#!/usr/bin/env python3
"""Separate-process public-boundary journey for the research provider."""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Optional, Sequence

import work_slot_journey

DUMPED_PROFILE = "crates/research-provider/data/configs/standard.json"
PACKAGED_PROFILE_NAME = "standard.json"
BOUND_SLOT_ID = "scope"
UNBOUND_INVOKE_SLOT_ID = "gather"
RESEARCH_SLOT_IDS = (
    "scope",
    "gather",
    "verify",
    "synthesize",
)
WORK_SLOT_PROOF = [
    "copied shipped profile omitted review work_slot_bindings",
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


class JourneyFailure(Exception):
    """Journey preflight or assertion failure."""


def call(engine: Path, database: Path, arguments: list[str]) -> dict[str, Any]:
    """Invoke one fresh CLI process and parse its public JSON response."""
    completed = subprocess.run(
        [str(engine), "--database", str(database), "--json", *arguments],
        text=True,
        capture_output=True,
        check=False,
    )
    if not completed.stdout.strip():
        raise JourneyFailure(f"CLI returned no JSON: {completed.stderr}")
    value = json.loads(completed.stdout)
    if value.get("status") not in (None, "completed", "rejected"):
        raise JourneyFailure(f"unexpected CLI envelope: {value}")
    return value


def expect_denial(response: dict[str, Any], code: str, phase: str) -> dict[str, Any]:
    if response.get("status") != "rejected":
        raise JourneyFailure(f"expected rejection, got {response}")
    if response.get("code") != code:
        raise JourneyFailure(f"expected denial {code}, got {response}")
    details = response.get("details") or {}
    if details.get("phase") != phase:
        raise JourneyFailure(f"expected phase {phase}, got {response}")
    return details


def show_state(engine: Path, database: Path, run_id: str, state: str) -> dict[str, Any]:
    shown = call(engine, database, ["show", run_id])
    if shown.get("status") != "completed":
        raise JourneyFailure(f"show failed: {shown}")
    result = shown["result"]
    if result.get("current_state") != state:
        raise JourneyFailure(f"expected state {state}, got {result.get('current_state')}")
    return result


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def author(name: str) -> dict[str, str]:
    return {"name": name, "kind": "human"}


def brief() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": author("owner"),
        "question": "What is the capital of France?",
        "scope": "One inspectable geography fact.",
        "acceptance": ["Name the capital city."],
        "constraints": ["Cite one source extract."],
        "non_goals": ["Travel advice."],
    }


def sources() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": author("owner"),
        "brief_revision": "1",
        "sources": [
            {
                "id": "src-1",
                "title": "Atlas",
                "locator": "https://example.invalid/paris",
                "extract": "Paris is the capital of France.",
            }
        ],
    }


def verification() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": author("owner"),
        "sources_revision": "1",
        "claims": [
            {
                "id": "claim-1",
                "statement": "Paris is the capital of France.",
                "source_ids": ["src-1"],
                "support": "The atlas extract names Paris as the capital.",
                "challenge": "No contrary extract was found after search.",
            }
        ],
    }


def report() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": author("owner"),
        "verification_revision": "1",
        "conclusion": "Paris is the capital of France.",
        "citations": [{"claim_id": "claim-1", "source_id": "src-1"}],
    }


def evidence(
    gate: str,
    axis: str,
    subject: str,
    author_name: str,
    config_version: str,
) -> dict[str, Any]:
    return {
        "gate": gate,
        "policy_id": axis,
        "result": "pass",
        "findings": "",
        "author": {"name": author_name, "kind": "script"},
        "subject": subject,
        "subject_revision": "1",
        "config_version": config_version,
    }


def append_evidence(
    engine: Path,
    database: Path,
    run_id: str,
    gate: str,
    axes: list[str],
    subject: str,
    prefix: str,
    config_version: str,
) -> None:
    for index, axis in enumerate(axes):
        response = call(
            engine,
            database,
            [
                "append",
                "--record-id",
                f"{prefix}-{index}",
                run_id,
                "review-evidence",
                json.dumps(
                    evidence(gate, axis, subject, prefix, config_version),
                    separators=(",", ":"),
                ),
            ],
        )
        if response.get("status") != "completed":
            raise JourneyFailure(f"append failed: {response}")
        if response["result"]["context"]["id"] != f"{prefix}-{index}":
            raise JourneyFailure(f"append identity mismatch: {response}")


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("source", "packaged"), required=True)
    parser.add_argument("--engine", required=True, help="loop-engine executable")
    parser.add_argument("--provider", required=True, help="research executable")
    parser.add_argument(
        "--profile",
        required=True,
        help="source profile path, or standard.json in packaged mode",
    )
    parser.add_argument(
        "--data-root",
        help="empty packaged dump root; required for --mode packaged",
    )
    return parser.parse_args(argv)


class Journey:
    def __init__(self, args: argparse.Namespace) -> None:
        self.mode = args.mode
        self.engine = Path(args.engine).expanduser().resolve()
        self.provider = Path(args.provider).expanduser().resolve()
        self.profile_arg = args.profile
        self.data_root = (
            Path(args.data_root).expanduser().resolve() if args.data_root else None
        )
        self.profile_source: Optional[Path] = None
        self.profile: dict[str, Any] = {}
        self.work: Optional[Path] = None
        self.database: Optional[Path] = None
        self.artifacts: Optional[Path] = None
        self.profile_path: Optional[Path] = None
        self.work_slot_bindings: dict[str, Any] = {}
        self.run_id = "research-journey"

    def preflight(self) -> None:
        """Reject bad inputs before creating or mutating any run state."""
        if self.mode == "packaged":
            if self.data_root is None:
                raise JourneyFailure("packaged mode requires --data-root")
            if Path(self.profile_arg).name != PACKAGED_PROFILE_NAME:
                raise JourneyFailure(
                    "packaged profile must be the dumped standard.json profile"
                )
            if self.data_root.exists():
                if not self.data_root.is_dir():
                    raise JourneyFailure(
                        f"packaged data-root is not a directory: {self.data_root}"
                    )
                if any(self.data_root.iterdir()):
                    raise JourneyFailure(
                        f"packaged data-root must be empty before data-dump: {self.data_root}"
                    )
        else:
            profile_source = Path(self.profile_arg).expanduser().resolve()
            if not profile_source.is_file():
                raise JourneyFailure(f"source profile does not exist: {profile_source}")
            self.profile_source = profile_source
            self.profile = json.loads(profile_source.read_text(encoding="utf-8"))

        for label, path in (
            ("engine binary", self.engine),
            ("provider binary", self.provider),
        ):
            if not path.is_file():
                raise JourneyFailure(f"{label} does not exist: {path}")
            if not os.access(path, os.X_OK):
                raise JourneyFailure(f"{label} is not executable: {path}")

    def _dump_packaged_data(self) -> None:
        assert self.data_root is not None
        self.data_root.parent.mkdir(parents=True, exist_ok=True)
        completed = subprocess.run(
            [str(self.provider), "data-dump", str(self.data_root)],
            text=True,
            capture_output=True,
            check=False,
        )
        if completed.returncode != 0:
            raise JourneyFailure(
                "packaged provider data-dump failed: "
                + (completed.stderr.strip() or f"exit {completed.returncode}")
            )
        dumped = self.data_root / DUMPED_PROFILE
        if not dumped.is_file():
            raise JourneyFailure(
                f"data-dump did not materialize the standard profile: {dumped}"
            )
        self.profile_source = dumped
        self.profile = json.loads(dumped.read_text(encoding="utf-8"))

    def _prepare_workspace(self, work: Path) -> None:
        self.work = work
        self.artifacts = work / "artifacts"
        self.artifacts.mkdir()
        self.database = work / "run.sqlite"
        self.profile_path = work / "standard.json"
        assert self.profile_source is not None
        shutil.copy2(self.profile_source, self.profile_path)
        profile = json.loads(self.profile_path.read_text(encoding="utf-8"))
        try:
            work_slot_journey.assert_no_review_bindings(
                profile.get("work_slot_bindings"),
                source=str(self.profile_source),
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error)) from error
        profile["artifact_root"] = str(self.artifacts)
        self.work_slot_bindings = work_slot_journey.bindings_for([BOUND_SLOT_ID])
        profile["work_slot_bindings"] = self.work_slot_bindings
        write_json(self.profile_path, profile)
        self.profile = profile

    def _write_provider_config(self, work: Path) -> Path:
        providers = work / "providers.toml"
        providers.write_text(
            f"[providers.research]\ncommand = {json.dumps(str(self.provider))}\nargs = []\n",
            encoding="utf-8",
        )
        return providers

    def _start(self, providers: Path) -> None:
        assert self.engine is not None and self.database is not None
        assert self.profile_path is not None
        started = call(
            self.engine,
            self.database,
            [
                "--config",
                str(providers),
                "start",
                "--id",
                self.run_id,
                "research",
                "@" + str(self.profile_path),
                "research journey",
            ],
        )
        if started.get("status") != "completed":
            raise JourneyFailure(f"start failed: {started}")

    def _engine_call(self) -> work_slot_journey.EngineCall:
        assert self.engine is not None and self.database is not None
        engine = self.engine
        database = self.database

        def call_engine(arguments: Sequence[str]) -> dict[str, Any]:
            return call(engine, database, list(arguments))

        return call_engine

    def _prove_work_slots_at_start(self) -> None:
        assert self.artifacts is not None
        try:
            work_slot_journey.prove_bound_visit(
                self._engine_call(),
                run_id=self.run_id,
                catalog=RESEARCH_SLOT_IDS,
                bindings=self.work_slot_bindings,
                bound_slot_id=BOUND_SLOT_ID,
                unbound_slot_id=UNBOUND_INVOKE_SLOT_ID,
                gated_event="scoped",
                artifact_root=self.artifacts,
                expected_state="scope",
            )
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error)) from error

    def _run_checked_prefix(self) -> list[str]:
        assert self.engine is not None and self.database is not None
        assert self.artifacts is not None
        self._prove_work_slots_at_start()
        shown = show_state(self.engine, self.database, self.run_id, "scope")
        if "review_policies" not in shown.get("initial_input", {}):
            raise JourneyFailure("show did not project frozen review_policies")
        if "artifact_schemas" not in shown.get("initial_input", {}):
            raise JourneyFailure("show did not project frozen artifact_schemas")
        schema = expect_denial(
            call(self.engine, self.database, ["event", self.run_id, "scoped"]),
            "research-schema-invalid",
            "schema",
        )
        if not schema.get("violations"):
            raise JourneyFailure(f"schema denial missing violations: {schema}")
        show_state(self.engine, self.database, self.run_id, "scope")
        write_json(self.artifacts / "brief.json", brief())
        scoped = call(self.engine, self.database, ["event", self.run_id, "scoped"])
        if scoped.get("status") != "completed":
            raise JourneyFailure(f"scoped failed: {scoped}")
        gather_shown = show_state(self.engine, self.database, self.run_id, "gather")
        try:
            work_slot_journey.assert_unbound_instructions(gather_shown, "sources.md")
        except work_slot_journey.WorkSlotJourneyFailure as error:
            raise JourneyFailure(str(error)) from error
        return [
            "frozen show policies and schemas before checked event",
            "schema denial before brief",
            "checked scoped progression",
            *WORK_SLOT_PROOF,
        ]

    def _run_full_source(self) -> list[str]:
        assert self.engine is not None and self.database is not None
        assert self.artifacts is not None
        proof = self._run_checked_prefix()
        config_version = self.profile["config_version"]
        verify_axes = [item["id"] for item in self.profile["review_policies"]["verify"]]
        synthesize_axes = [
            item["id"] for item in self.profile["review_policies"]["synthesize"]
        ]

        write_json(self.artifacts / "sources.json", sources())
        gathered = call(self.engine, self.database, ["event", self.run_id, "gathered"])
        if gathered.get("status") != "completed":
            raise JourneyFailure(f"gathered failed: {gathered}")
        show_state(self.engine, self.database, self.run_id, "verify")

        revised = call(self.engine, self.database, ["event", self.run_id, "revise"])
        if revised.get("status") != "completed":
            raise JourneyFailure(f"owning-phase revise failed: {revised}")
        show_state(self.engine, self.database, self.run_id, "gather")
        gathered_again = call(
            self.engine, self.database, ["event", self.run_id, "gathered"]
        )
        if gathered_again.get("status") != "completed":
            raise JourneyFailure(f"second gathered failed: {gathered_again}")
        show_state(self.engine, self.database, self.run_id, "verify")

        write_json(self.artifacts / "verification.json", verification())
        missing_verify = expect_denial(
            call(self.engine, self.database, ["event", self.run_id, "verified"]),
            "research-review-incomplete",
            "evidence",
        )
        if not missing_verify.get("diagnostics"):
            raise JourneyFailure(
                f"verify evidence denial missing diagnostics: {missing_verify}"
            )
        append_evidence(
            self.engine,
            self.database,
            self.run_id,
            "verify",
            verify_axes,
            "verification.json",
            "verify-review",
            config_version,
        )
        verified = call(self.engine, self.database, ["event", self.run_id, "verified"])
        if verified.get("status") != "completed":
            raise JourneyFailure(f"verified failed: {verified}")
        show_state(self.engine, self.database, self.run_id, "synthesize")

        write_json(self.artifacts / "report.json", report())
        missing_report = expect_denial(
            call(self.engine, self.database, ["event", self.run_id, "completed"]),
            "research-review-incomplete",
            "evidence",
        )
        if not missing_report.get("diagnostics"):
            raise JourneyFailure(
                f"synthesize evidence denial missing diagnostics: {missing_report}"
            )
        append_evidence(
            self.engine,
            self.database,
            self.run_id,
            "synthesize",
            synthesize_axes,
            "report.json",
            "synthesize-review",
            config_version,
        )
        completed = call(
            self.engine, self.database, ["event", self.run_id, "completed"]
        )
        if completed.get("status") != "completed":
            raise JourneyFailure(f"completed failed: {completed}")
        terminal = show_state(self.engine, self.database, self.run_id, "end")
        if terminal.get("lifecycle") != "final":
            raise JourneyFailure(f"terminal lifecycle was not final: {terminal}")
        proof.extend(
            [
                "checked gathered progression",
                "owning-phase revise verify->gather then gathered again",
                "missing-evidence denial at verify",
                "independent verify evidence success",
                "missing-evidence denial at synthesize",
                "independent synthesize evidence success",
                "fresh-process show persistence",
                "terminal completion",
            ]
        )
        return proof

    def run(self) -> dict[str, Any]:
        self.preflight()
        with tempfile.TemporaryDirectory(prefix="research-journey-") as temporary:
            work = Path(temporary)
            if self.mode == "packaged":
                self._dump_packaged_data()
            self._prepare_workspace(work)
            providers = self._write_provider_config(work)
            self._start(providers)
            if self.mode == "source":
                proof = self._run_full_source()
                profile_note = "copied shipped standard.json"
            else:
                proof = self._run_checked_prefix()
                profile_note = "dumped crates/research-provider/data/configs/standard.json"
            result = {
                "journey": "research",
                "mode": self.mode,
                "result": "passed",
                "profile": profile_note,
                "proof": proof,
                "synthetic_evidence_scope": (
                    "Deterministic mechanics only; synthetic records are not semantic verdict quality."
                ),
            }
            print(json.dumps(result, sort_keys=True))
            return result


def self_test() -> int:
    """Prove invalid packaged usage fails before mutating work roots."""
    with tempfile.TemporaryDirectory(prefix="research-journey-self-test-") as temp:
        root = Path(temp)
        executable = Path(sys.executable).resolve()

        missing = argparse.Namespace(
            mode="packaged",
            engine=str(executable),
            provider=str(executable),
            profile=PACKAGED_PROFILE_NAME,
            data_root=None,
        )
        try:
            Journey(missing).preflight()
        except JourneyFailure as error:
            if "requires --data-root" not in str(error):
                raise JourneyFailure(
                    f"missing data-root self-test got wrong error: {error}"
                ) from error
        else:
            raise JourneyFailure("packaged mode accepted a missing data-root")

        occupied = root / "occupied-data"
        occupied.mkdir()
        sentinel = occupied / "sentinel"
        sentinel.write_text("keep", encoding="utf-8")
        work_root = root / "must-not-exist"
        occupied_args = argparse.Namespace(
            mode="packaged",
            engine=str(executable),
            provider=str(executable),
            profile=PACKAGED_PROFILE_NAME,
            data_root=str(occupied),
        )
        try:
            Journey(occupied_args).preflight()
        except JourneyFailure as error:
            if "must be empty before data-dump" not in str(error):
                raise JourneyFailure(
                    f"non-empty data-root self-test got wrong error: {error}"
                ) from error
        else:
            raise JourneyFailure("packaged mode accepted a non-empty data-root")
        if sentinel.read_text(encoding="utf-8") != "keep":
            raise JourneyFailure("non-empty data-root self-test mutated the sentinel")
        if any(path.name != "sentinel" for path in occupied.iterdir()):
            raise JourneyFailure("non-empty data-root self-test wrote dump files")
        if work_root.exists():
            raise JourneyFailure("invalid packaged usage mutated a work root")

    print(
        "research journey interface self-test passed: "
        "invalid packaged data-root usage rejected pre-mutation"
    )
    return 0


def main(argv: Optional[Sequence[str]] = None) -> int:
    raw_argv = list(sys.argv[1:] if argv is None else argv)
    try:
        if raw_argv == ["--self-test"]:
            return self_test()
        Journey(parse_args(raw_argv)).run()
        return 0
    except JourneyFailure as error:
        print(f"research journey failed: {error}", file=sys.stderr)
        return 1
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"research journey failed before assertion: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
