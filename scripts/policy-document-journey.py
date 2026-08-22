#!/usr/bin/env python3
"""Separate-process public-boundary journey for policy-document provider."""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Sequence

import work_slot_journey

BOUND_SLOT_ID = "semantic-review"
UNBOUND_INVOKE_SLOT_ID = "deterministic-review"
POLICY_DOCUMENT_SLOT_IDS = (
    "deterministic-review",
    "semantic-review",
)
WORK_SLOT_PROOF = [
    "frozen sparse work_slot_bindings in initial_input",
    "show work_slots catalog snapshot",
    "prepare ready remains check-free and ungated",
    "unbound deterministic-review keeps stored instructions",
    "bound instruction redaction",
    "unbound invoke rejection",
    "event gated before succeeded invoke",
    "dummy worker packet receipt",
    "overlay succeeded then checked event",
    "history invocation started and succeeded",
]
CONSTRUCTOR_PROOF = [
    "semantic-review constructor vs shipped readme-2 and agents-2",
    "one worker per semantic policy in profile order",
    "exact example_prompt, author/model, mode, and complete target object",
    "preamble/schema bytes and preview-bindings output",
    "unsupported slot, empty policies, missing prompt/target/mode, and invalid roster fail closed",
    "resulting-profile preview equality and pre-start hash guard",
    "shipped policy-document profiles omit frozen work_slot_bindings",
]


class ConstructorClosed(RuntimeError):
    """The skill constructor rejected invalid input."""


def repository_root() -> Path:
    return Path(__file__).resolve().parent.parent


def extract_skill_jq(skill: str) -> str:
    marker = "<<'JQ'\n"
    start = skill.index(marker) + len(marker)
    end = skill.index("\nJQ\n", start)
    return skill[start:end]


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_constructor_jq(
    filter_text: str,
    profile: Path,
    extra: Sequence[str],
) -> str:
    completed = subprocess.run(
        ["jq", *extra, filter_text, str(profile)],
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()
        raise ConstructorClosed(detail or f"jq exited {completed.returncode}")
    return completed.stdout


def fan_out_workers(binding: dict[str, Any], *, engine: str) -> list[dict[str, Any]]:
    if binding.get("command") != engine:
        raise AssertionError(
            f"constructor binding command {binding.get('command')!r} != {engine!r}"
        )
    args = binding.get("args")
    if not isinstance(args, list) or not args or args[0] != "fan-out":
        raise AssertionError(f"constructor binding was not fan-out: {binding}")
    workers: list[dict[str, Any]] = []
    index = 1
    while index < len(args):
        if args[index] != "--worker" or index + 1 >= len(args):
            raise AssertionError(f"constructor fan-out args were not worker pairs: {args}")
        worker = json.loads(args[index + 1])
        if not isinstance(worker, dict):
            raise AssertionError(f"constructor worker is not an object: {worker}")
        workers.append(worker)
        index += 2
    return workers


def expect_constructor_closed(run, *, needle: str, context: str) -> None:
    try:
        run()
    except ConstructorClosed as error:
        if needle not in str(error):
            raise AssertionError(f"{context} failed for the wrong reason: {error}") from error
    else:
        raise AssertionError(f"{context} unexpectedly succeeded")


def assert_semantic_review_constructor(engine: Path) -> None:
    """Execute the skill constructor against shipped profiles and fail-closed cases."""
    repository = repository_root()
    skill_path = (
        repository
        / "crates/policy-document-provider/skills/using-policy-document-provider/SKILL.md"
    )
    preamble_path = (
        repository / "crates/policy-document-provider/data/semantic-review-worker-preamble.md"
    )
    schema_path = (
        repository
        / "crates/policy-document-provider/data/semantic-review-worker-output-schema.json"
    )
    readme_profile = repository / "crates/policy-document-provider/data/readme.json"
    agents_profile = repository / "crates/policy-document-provider/data/agents.json"
    skill = skill_path.read_text(encoding="utf-8")
    jq_filter = extract_skill_jq(skill)
    preamble = preamble_path.read_text(encoding="utf-8")
    schema = load_json(schema_path)
    assert schema == {"required": ["axis", "author", "result", "findings"]}, schema
    dummy_engine = "/tmp/loop-engine-constructor-proof"
    dummy_pi = "/tmp/pi-constructor-proof"
    dummy_cursor = "/tmp/cursor-provider-extension"
    dummy_bridge = "/tmp/claude-bridge-extension"
    roster = [
        {"author": "reviewer-a", "model": "model-a"},
        {"author": "reviewer-b", "model": "model-b"},
    ]

    def jq_args(roster_path: Path, *, slot_id: str = "semantic-review") -> list[str]:
        return [
            "--arg",
            "slot",
            slot_id,
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
            str(preamble_path),
            "--slurpfile",
            "schema_documents",
            str(schema_path),
            "--slurpfile",
            "roster_documents",
            str(roster_path),
        ]

    def run_pd(profile: Path, roster_path: Path, *, slot_id: str = "semantic-review") -> dict[str, Any]:
        stdout = run_constructor_jq(jq_filter, profile, jq_args(roster_path, slot_id=slot_id))
        profile.write_text(stdout, encoding="utf-8")
        return load_json(profile)

    # bookends:LE-63 — README-like and AGENTS-like targets are exercised as policy inputs, not Bookends classes.
    for shipped in (readme_profile, agents_profile):
        raw = shipped.read_text(encoding="utf-8")
        parsed = load_json(shipped)
        work_slot_journey.assert_no_review_bindings(
            parsed.get("work_slot_bindings"),
            source=str(shipped),
        )
        assert "work_slot_bindings" not in raw, shipped

    with tempfile.TemporaryDirectory(prefix="policy-document-constructor-") as temporary:
        root = Path(temporary)
        roster_path = root / "roster.json"
        write_json(roster_path, roster)

        for source_profile, label in ((readme_profile, "readme-2"), (agents_profile, "agents-2")):
            target_file = root / f"{label}-target.md"
            dest = root / f"{label}.json"
            copied = load_json(source_profile)
            copied["target"]["path"] = str(target_file.resolve())
            target_file.write_text("# target\n", encoding="utf-8")
            write_json(dest, copied)
            source = load_json(dest)
            result = run_pd(dest, roster_path)
            bindings = result.get("work_slot_bindings")
            if not isinstance(bindings, dict) or "semantic-review" not in bindings:
                raise AssertionError(f"{label} constructor omitted semantic-review bindings")
            assert bindings == result["work_slot_bindings"]
            workers = fan_out_workers(bindings["semantic-review"], engine=dummy_engine)
            policies = source["semantic_policies"]
            assert all("required_authors" not in policy for policy in policies), label
            if len(workers) != len(policies):
                raise AssertionError(
                    f"{label} worker count {len(workers)} != {len(policies)}"
                )
            target_json = json.dumps(source["target"], separators=(",", ":"))
            for worker, policy in zip(workers, policies):
                assert worker.get("command") == dummy_pi, label
                assert worker.get("output_schema") == schema, label
                worker_preamble = worker.get("preamble")
                if not isinstance(worker_preamble, str) or not worker_preamble.startswith(
                    preamble
                ):
                    raise AssertionError(f"{label} preamble did not start with exact provider bytes")
                if policy["example_prompt"] not in worker_preamble:
                    raise AssertionError(f"{label} omitted exact example_prompt for {policy['id']}")
                if policy["id"] not in worker_preamble:
                    raise AssertionError(f"{label} omitted axis id {policy['id']!r}")
                if roster[0]["author"] not in worker_preamble:
                    raise AssertionError(f"{label} omitted author {roster[0]['author']!r}")
                args = worker.get("args")
                if not isinstance(args, list) or "--model" not in args:
                    raise AssertionError(f"{label} omitted --model: {worker}")
                model_index = args.index("--model")
                if args[model_index + 1] != roster[0]["model"]:
                    raise AssertionError(f"{label} froze the wrong model")
                for fragment in (
                    "policy-document",
                    "semantic-review",
                    source["mode"],
                    source["target"]["id"],
                    source["target"]["path"],
                    target_json,
                ):
                    if fragment not in worker_preamble:
                        raise AssertionError(
                            f"{label} worker omitted assignment field {fragment!r}"
                        )
            preview_path = root / f"{label}-bindings.json"
            write_json(preview_path, bindings)
            preview = subprocess.run(
                [str(engine), "preview-bindings", f"@{preview_path}"],
                capture_output=True,
                text=True,
            )
            if preview.returncode != 0:
                raise AssertionError(
                    f"preview-bindings rejected {label}: {preview.stderr or preview.stdout}"
                )
            preview_text = f"{preview.stdout}\n{preview.stderr}".lower()
            if "preamble" not in preview_text or "output_schema" not in preview_text.replace(
                "-", "_"
            ):
                if "required" not in preview_text:
                    raise AssertionError(
                        f"preview-bindings omitted preamble/schema visibility for {label}: {preview_text}"
                    )
            confirmed = sha256_file(dest)
            original = dest.read_bytes()
            dest.write_bytes(original + b"\n")
            if sha256_file(dest) == confirmed:
                raise AssertionError(
                    f"{label} pre-start hash guard would not detect a post-preview mutation"
                )
            dest.write_bytes(original)
            if sha256_file(dest) != confirmed:
                raise AssertionError(f"{label} hash-guard restore mutated the resulting profile")

        two_author = root / "readme-two-authors.json"
        two_doc = load_json(root / "readme-2.json")
        two_doc["semantic_policies"][0]["required_authors"] = 2
        write_json(two_author, two_doc)
        two_result = run_pd(two_author, roster_path)
        two_workers = fan_out_workers(
            two_result["work_slot_bindings"]["semantic-review"], engine=dummy_engine
        )
        expected_two = len(two_doc["semantic_policies"]) + 1
        if len(two_workers) != expected_two:
            raise AssertionError(
                f"required_authors=2 worker count {len(two_workers)} != {expected_two}"
            )
        first_axis = two_doc["semantic_policies"][0]["id"]
        assert roster[0]["author"] in two_workers[0]["preamble"]
        assert roster[1]["author"] in two_workers[1]["preamble"]
        assert first_axis in two_workers[0]["preamble"] and first_axis in two_workers[1]["preamble"]

        expect_constructor_closed(
            lambda: run_pd(root / "readme-2.json", roster_path, slot_id="design-review"),
            needle="unsupported slot",
            context="policy-document unsupported slot",
        )
        empty_pd = load_json(root / "readme-2.json")
        empty_pd["semantic_policies"] = []
        empty_path = root / "empty-policies.json"
        write_json(empty_path, empty_pd)
        expect_constructor_closed(
            lambda: run_pd(empty_path, roster_path),
            needle="semantic_policies must be non-empty",
            context="policy-document empty policies",
        )
        prompt_pd = load_json(root / "readme-2.json")
        prompt_pd["semantic_policies"][0]["example_prompt"] = ""
        prompt_path = root / "missing-prompt.json"
        write_json(prompt_path, prompt_pd)
        expect_constructor_closed(
            lambda: run_pd(prompt_path, roster_path),
            needle="example_prompt",
            context="policy-document missing prompt",
        )
        mode_pd = load_json(root / "readme-2.json")
        del mode_pd["mode"]
        mode_path = root / "missing-mode.json"
        write_json(mode_path, mode_pd)
        expect_constructor_closed(
            lambda: run_pd(mode_path, roster_path),
            needle="mode must be draft or audit",
            context="policy-document missing mode",
        )
        target_pd = load_json(root / "readme-2.json")
        target_pd["target"]["path"] = "relative/README.md"
        target_path = root / "relative-target.json"
        write_json(target_path, target_pd)
        expect_constructor_closed(
            lambda: run_pd(target_path, roster_path),
            needle="complete {id,path}",
            context="policy-document missing/relative target",
        )
        del_target = load_json(root / "readme-2.json")
        del del_target["target"]
        del_target_path = root / "missing-target.json"
        write_json(del_target_path, del_target)
        expect_constructor_closed(
            lambda: run_pd(del_target_path, roster_path),
            needle="complete {id,path}",
            context="policy-document missing target",
        )
        duplicate_roster = root / "duplicate-roster.json"
        write_json(
            duplicate_roster,
            [roster[0], {"author": roster[0]["author"], "model": "model-c"}],
        )
        expect_constructor_closed(
            lambda: run_pd(root / "readme-2.json", duplicate_roster),
            needle="pairwise distinct",
            context="policy-document duplicate author",
        )
        empty_author = root / "empty-author.json"
        write_json(empty_author, [{"author": "", "model": "model-a"}])
        expect_constructor_closed(
            lambda: run_pd(root / "readme-2.json", empty_author),
            needle="non-empty author and model",
            context="policy-document empty author",
        )
        empty_model = root / "empty-model.json"
        write_json(empty_model, [{"author": "reviewer-a", "model": ""}])
        expect_constructor_closed(
            lambda: run_pd(root / "readme-2.json", empty_model),
            needle="non-empty author and model",
            context="policy-document empty model",
        )
        short_roster = root / "short-roster.json"
        write_json(short_roster, [roster[0]])
        expect_constructor_closed(
            lambda: run_pd(two_author, short_roster),
            needle="enough authors",
            context="policy-document insufficient roster",
        )



def call(engine: Path, database: Path, arguments: list[str]) -> dict[str, Any]:
    """Invoke one fresh CLI process and parse its public JSON response."""
    completed = subprocess.run(
        [str(engine), "--database", str(database), "--json", *arguments],
        text=True,
        capture_output=True,
        check=False,
    )
    if not completed.stdout.strip():
        raise RuntimeError(f"CLI returned no JSON: {completed.stderr}")
    value = json.loads(completed.stdout)
    if value.get("status") not in (None, "completed", "rejected"):
        raise RuntimeError(value)
    return value


def engine_call_for(engine: Path, database: Path) -> work_slot_journey.EngineCall:
    def call_engine(arguments: Sequence[str]) -> dict[str, Any]:
        return call(engine, database, list(arguments))

    return call_engine


def expect_denial(response: dict[str, Any], code: str, phase: str) -> dict[str, Any]:
    assert response["status"] == "rejected", response
    assert response["code"] == code, response
    assert response["details"]["phase"] == phase, response
    return response["details"]


def show_state(engine: Path, database: Path, run_id: str, state: str) -> dict[str, Any]:
    shown = call(engine, database, ["show", run_id])
    assert shown["status"] == "completed", shown
    assert shown["result"]["current_state"] == state, shown
    return shown["result"]


def evidence(
    axis: str,
    digest: str,
    author: str,
    profile_version: str,
    target_id: str,
) -> dict[str, Any]:
    return {
        "gate": "semantic-review",
        "policy_id": axis,
        "result": "pass",
        "findings": "",
        "author": {"name": author, "kind": "script"},
        "target_id": target_id,
        "target_sha256": digest,
        "profile_version": profile_version,
    }


def append_evidence(
    engine: Path,
    database: Path,
    run_id: str,
    axes: list[str],
    digest: str,
    prefix: str,
    profile_version: str,
    target_id: str,
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
                    evidence(
                        axis,
                        digest,
                        prefix,
                        profile_version,
                        target_id,
                    ),
                    separators=(",", ":"),
                ),
            ],
        )
        assert response["status"] == "completed", response
        assert response["result"]["context"]["id"] == f"{prefix}-{index}", response


def self_test() -> int:
    """Run the constructor contract without creating a provider run."""
    engine = repository_root() / "target/debug/loop-engine"
    if not engine.is_file() or not engine.stat().st_mode & 0o111:
        raise AssertionError(
            "policy-document public self-test requires target/debug/loop-engine"
        )
    assert_semantic_review_constructor(engine)
    print("policy-document journey self-test passed: constructor and preview contract")
    return 0


def main() -> int:
    if len(sys.argv) == 1 or sys.argv[1:] == ["--self-test"]:
        try:
            return self_test()
        except (AssertionError, OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
            print(f"policy-document journey self-test failed: {error}", file=sys.stderr)
            return 1

    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True)
    parser.add_argument("--provider", required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--mode", choices=("draft", "audit"), default="draft")
    args = parser.parse_args()
    engine = Path(args.engine).resolve()
    provider = Path(args.provider).resolve()
    shipped_profile = Path(args.profile).resolve()
    if not shipped_profile.is_file():
        parser.error(f"profile is not a file: {shipped_profile}")
    if not engine.is_file():
        parser.error(f"engine is not a file: {engine}")

    assert_semantic_review_constructor(engine)

    with tempfile.TemporaryDirectory(prefix="policy-document-journey-") as temporary:
        work = Path(temporary)
        target = work / "README.md"
        target.write_text("", encoding="utf-8")
        database = work / "run.sqlite"
        providers = work / "providers.toml"
        providers.write_text(
            f'[providers.policy-document]\ncommand = {json.dumps(str(provider))}\nargs = []\n',
            encoding="utf-8",
        )

        profile_path = work / "readme.json"
        shutil.copy2(shipped_profile, profile_path)
        profile = json.loads(profile_path.read_text(encoding="utf-8"))
        work_slot_journey.assert_no_review_bindings(
            profile.get("work_slot_bindings"),
            source=str(shipped_profile),
        )
        profile["mode"] = args.mode
        profile["target"]["path"] = str(target)
        artifact_root = work / "artifacts"
        artifact_root.mkdir()
        profile["artifact_root"] = str(artifact_root)
        work_slot_bindings = work_slot_journey.bindings_for([BOUND_SLOT_ID])
        profile["work_slot_bindings"] = work_slot_bindings
        profile_path.write_text(json.dumps(profile, indent=2) + "\n", encoding="utf-8")
        axes = [item["id"] for item in profile["semantic_policies"]]
        profile_version = profile["profile_version"]
        target_id = profile["target"]["id"]
        run_id = f"policy-document-{args.mode}-journey"
        engine_call = engine_call_for(engine, database)

        started = call(
            engine,
            database,
            [
                "--config",
                str(providers),
                "start",
                "--id",
                run_id,
                "policy-document",
                "@" + str(profile_path),
                f"README {args.mode} journey",
            ],
        )
        assert started["status"] == "completed", started
        shown = show_state(engine, database, run_id, "prepare")
        assert shown["initial_input"]["mode"] == args.mode, shown
        work_slot_journey.assert_catalog(shown, POLICY_DOCUMENT_SLOT_IDS)
        work_slot_journey.assert_frozen_bindings(shown, work_slot_bindings)
        work_slot_journey.assert_unbound_instructions(
            shown, "Draft or revise target document"
        )

        ready = call(engine, database, ["event", run_id, "ready"])
        assert ready["status"] == "completed", ready
        deterministic_shown = show_state(engine, database, run_id, "deterministic-review")
        work_slot_journey.assert_unbound_instructions(
            deterministic_shown, "Run configured deterministic checks"
        )

        # bookends:LE-56 — the public deterministic gate returns all actionable policy findings.
        deterministic = expect_denial(
            call(engine, database, ["event", run_id, "passed"]),
            "policy-document-nonconforming",
            "deterministic",
        )
        assert [item["policy_id"] for item in deterministic["violations"]] == [
            "document-present",
            "project-title",
            "purpose",
            "onboarding",
            "usage",
            "validation",
            "onboarding-command",
            "validation-command",
        ]
        show_state(engine, database, run_id, "deterministic-review")

        conforming = (
            "# Product\n\n## Purpose\nUseful tool.\n\n## Installation\n"
            "```sh\ncargo build\n```\n\n## Usage\nRun it.\n\n## Validation\n"
            "```sh\ncargo test\n```\n"
        )
        target.write_text(conforming, encoding="utf-8")
        moved = call(engine, database, ["event", run_id, "passed"])
        assert moved["status"] == "completed", moved
        show_state(engine, database, run_id, "semantic-review")
        work_slot_journey.prove_bound_visit(
            engine_call,
            run_id=run_id,
            catalog=POLICY_DOCUMENT_SLOT_IDS,
            bindings=work_slot_bindings,
            bound_slot_id=BOUND_SLOT_ID,
            unbound_slot_id=UNBOUND_INVOKE_SLOT_ID,
            gated_event="passed",
            artifact_root=artifact_root,
            expected_state="semantic-review",
        )

        # bookends:LE-57 — the public semantic gate refuses until independent evidence exists.
        missing = expect_denial(
            call(engine, database, ["event", run_id, "passed"]),
            "policy-document-review-incomplete",
            "semantic",
        )
        diagnostics = missing["details"]["diagnostics"]
        assert [(item["policy_id"], item["kind"]) for item in diagnostics] == [
            (axis, "missing") for axis in axes
        ]
        initial_digest = hashlib.sha256(target.read_bytes()).hexdigest()
        assert missing["target_sha256"] == initial_digest
        append_evidence(
            engine,
            database,
            run_id,
            axes,
            initial_digest,
            "initial-review",
            profile_version,
            target_id,
        )

        # Finalization must rerun deterministic policy before consulting evidence.
        # bookends:LE-61 — finalization rechecks the externally changed document, not stale engine state.
        # bookends:LE-62 — the target file and Loop Engine state are deliberately separate commits.
        broken = conforming.replace("## Validation", "Validation")
        target.write_text(broken, encoding="utf-8")
        final_deterministic = expect_denial(
            call(engine, database, ["event", run_id, "passed"]),
            "policy-document-nonconforming",
            "deterministic",
        )
        assert [item["policy_id"] for item in final_deterministic["violations"]] == [
            "validation",
            "validation-command",
        ]

        repaired = conforming + "\nRepair revision.\n"
        target.write_text(repaired, encoding="utf-8")
        stale = expect_denial(
            call(engine, database, ["event", run_id, "passed"]),
            "policy-document-review-incomplete",
            "semantic",
        )
        stale_diagnostics = stale["details"]["diagnostics"]
        assert [item["kind"] for item in stale_diagnostics].count("stale") == len(axes)
        assert [item["kind"] for item in stale_diagnostics].count("missing") == len(axes)
        stale_record_ids = {
            item["record_id"]
            for item in stale_diagnostics
            if item.get("kind") == "stale"
        }
        assert stale_record_ids == {f"initial-review-{index}" for index in range(len(axes))}
        # bookends:LE-58 — the next semantic request inspects every prior review record before reporting stale/missing findings.
        # bookends:LE-59 — the external fresh-review cycle carries those prior record identities into provider diagnostics.
        fresh_digest = hashlib.sha256(target.read_bytes()).hexdigest()
        assert fresh_digest != initial_digest
        assert stale["target_sha256"] == fresh_digest
        append_evidence(
            engine,
            database,
            run_id,
            axes,
            fresh_digest,
            "fresh-review",
            profile_version,
            target_id,
        )

        # bookends:LE-55 — this same public run is executed in both draft and audit modes.
        # bookends:LE-60 — the actor can repeat repair and evaluation until completion.
        final = call(engine, database, ["event", run_id, "passed"])
        assert final["status"] == "completed", final
        terminal = show_state(engine, database, run_id, "end")
        assert terminal["lifecycle"] == "final", terminal
        assert len(terminal["context"]) == len(axes) * 2, terminal
        print(
            json.dumps(
                {
                    "journey": "policy-document",
                    "result": "passed",
                    "mode": args.mode,
                    "profile": "copied shipped readme.json",
                    "proof": [
                        "deterministic all-findings denial",
                        "missing-evidence denial",
                        "checked deterministic progression",
                        "final deterministic recheck denial",
                        "stale-evidence denial",
                        "fresh-evidence success",
                        "fresh-process show persistence",
                        "terminal completion",
                        "copied shipped profile omitted review work_slot_bindings",
                        *CONSTRUCTOR_PROOF,
                        *WORK_SLOT_PROOF,
                    ],
                },
                sort_keys=True,
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
