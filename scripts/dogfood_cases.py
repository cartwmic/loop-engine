"""Fail-closed dispatch for isolated scripted software-change dogfood cases.

Each owner adds a scenario only with its public CLI fixture, retained streams,
and terminal assertions. Merely registering a name is not a passing scenario.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

SCENARIOS = (
    "sol-observation",
    "sol-persistence",
    "sol-terminal",
    "sol-review-context",
    "sol-recovery-core",
    "sol-reuse",
    "sol-evidence",
    "sol-recovery",
    "sol-profiles",
    "sol-advice-generic",
    "sol-advice-transition",
    "sol-advice-provider",
    "sol-proof-index",
)

# Owners add their callable here only after it drives the required public path
# and keeps the assertions/negative outcomes in its isolated fixture output.
import dogfood_advice_generic
import dogfood_advice_provider
import dogfood_advice_transition
import dogfood_evidence
import dogfood_observation
import dogfood_profiles
import dogfood_proof_index
import dogfood_reuse
import dogfood_review_context
import dogfood_recovery
import dogfood_terminal

IMPLEMENTED = {
    "sol-observation": dogfood_observation.observation_case,
    "sol-persistence": dogfood_recovery.persistence_case,
    "sol-reuse": dogfood_reuse.reuse_case,
    "sol-review-context": dogfood_review_context.review_context_case,
    "sol-terminal": dogfood_terminal.terminal_case,
    "sol-recovery-core": dogfood_recovery.recovery_case,
    "sol-evidence": dogfood_evidence.evidence_case,
    "sol-recovery": dogfood_evidence.recovery_case,
    "sol-advice-generic": dogfood_advice_generic.advice_case,
    "sol-advice-transition": dogfood_advice_transition.advice_transition_case,
    "sol-advice-provider": dogfood_advice_provider.advice_provider_case,
    "sol-profiles": dogfood_profiles.profiles_case,
    "sol-proof-index": dogfood_proof_index.proof_index_case,
}

CAPTURE_FILES = {
    "sol-observation": "outcome.json",
    "sol-persistence": "outcome.json",
    "sol-terminal": "outcome.json",
    "sol-review-context": "outcome.json",
    "sol-recovery-core": "sol-recovery-core-proof.json",
    "sol-reuse": "outcome.json",
    "sol-evidence": "sol-evidence-proof.json",
    "sol-recovery": "sol-recovery-proof.json",
    "sol-profiles": "sol-profiles-result.json",
    "sol-advice-generic": "sol-advice-generic-proof.json",
    "sol-advice-transition": "sol-advice-transition-proof.json",
    "sol-advice-provider": "sol-advice-provider-proof.json",
    "sol-proof-index": "sol-proof-index-proof.json",
}


def _case_root(journey):
    for attribute in (
        "_dogfood_case_root",
        "_dogfood_terminal_root",
        "_dogfood_advice_transition_root",
    ):
        root = getattr(journey, attribute, None)
        if root is not None:
            return Path(root)
    return Path(journey.work_root)


def dispatch(name, journey):
    if name not in SCENARIOS:
        raise ValueError(f"unknown dogfood scenario: {name}")
    proof = IMPLEMENTED.get(name)
    if proof is None:
        raise ValueError(f"dogfood scenario not implemented: {name}")
    if journey is None or getattr(journey, "mode", None) != "source":
        raise ValueError("dogfood scenarios require source mode")
    proof(journey)
    root = _case_root(journey)
    capture = root / CAPTURE_FILES[name]
    if not capture.is_file():
        raise ValueError(f"dogfood scenario {name} omitted retained outcome/assertions: {capture}")
    try:
        outcome = json.loads(capture.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(
            f"dogfood scenario {name} retained invalid outcome JSON: {capture}: {error}"
        ) from error
    if not isinstance(outcome, dict) or outcome.get("case", name) != name:
        raise ValueError(f"dogfood scenario {name} outcome identity is invalid: {outcome!r}")
    if outcome.get("status") in {"failed", "error"}:
        raise ValueError(f"dogfood scenario {name} retained a failed outcome: {outcome!r}")
    raw = capture.read_bytes()
    result = {
        "scenario": name,
        "status": "passed-scripted-assertions",
        "assertion_capture": str(capture),
        "assertion_capture_bytes": len(raw),
        "assertion_capture_sha256": "sha256:" + hashlib.sha256(raw).hexdigest(),
        "semantic_approval": "not-established-by-scripted-fixture",
    }
    (root / "dogfood-case-result.json").write_text(
        json.dumps(result, indent=2) + "\n", encoding="utf-8"
    )
    print(f"dogfood public scenario passed: {name}; assertions retained: {capture}")


def self_test():
    """Prove that unknown/unimplemented cases refuse and assertions propagate."""
    if len(SCENARIOS) != len(set(SCENARIOS)):
        raise AssertionError("dogfood scenario inventory contains duplicate names")
    if set(IMPLEMENTED) != set(SCENARIOS):
        raise AssertionError("dogfood scenario inventory has an unimplemented or unplanned case")
    if set(CAPTURE_FILES) != set(SCENARIOS):
        raise AssertionError("dogfood scenario inventory has a missing/extra outcome capture contract")
    for name in ("sol-unknown", *[case for case in SCENARIOS if case not in IMPLEMENTED]):
        try:
            dispatch(name, None)
        except ValueError as error:
            expected = "unknown dogfood scenario" if name not in SCENARIOS else "not implemented"
            if expected not in str(error):
                raise AssertionError(f"wrong fail-closed diagnostic for {name}: {error}") from error
        else:
            raise AssertionError(f"unknown/unimplemented scenario was accepted: {name}")

    if SCENARIOS:
        name = SCENARIOS[0]
        original = IMPLEMENTED.get(name)
        import tempfile
        from types import SimpleNamespace

        with tempfile.TemporaryDirectory(prefix="dogfood-case-dispatch-") as temporary:
            root = Path(temporary)

            def retain_fixture(_journey):
                (root / CAPTURE_FILES[name]).write_text('{"case":"' + name + '"}\n', encoding="utf-8")

            IMPLEMENTED[name] = retain_fixture
            try:
                dispatch(name, SimpleNamespace(mode="source", work_root=root))
                result = json.loads((root / "dogfood-case-result.json").read_text(encoding="utf-8"))
                if result.get("scenario") != name or not result.get("assertion_capture_sha256"):
                    raise AssertionError("successful dogfood assertion capture was not indexed")
            finally:
                if original is None:
                    del IMPLEMENTED[name]
                else:
                    IMPLEMENTED[name] = original

            (root / CAPTURE_FILES[name]).unlink()
            IMPLEMENTED[name] = lambda _journey: None
            try:
                try:
                    dispatch(name, SimpleNamespace(mode="source", work_root=root))
                except ValueError as error:
                    if "omitted retained outcome/assertions" not in str(error):
                        raise AssertionError(f"missing assertion capture was not refused: {error}") from error
                else:
                    raise AssertionError("dogfood scenario passed without retained assertion evidence")
            finally:
                if original is None:
                    del IMPLEMENTED[name]
                else:
                    IMPLEMENTED[name] = original

            def fail_at_public_assertion(_journey):
                raise AssertionError("deliberate missing public outcome assertion")

            IMPLEMENTED[name] = fail_at_public_assertion
            try:
                try:
                    dispatch(name, SimpleNamespace(mode="source", work_root=root))
                except AssertionError as error:
                    if "missing public outcome assertion" not in str(error):
                        raise
                else:
                    raise AssertionError("dispatcher swallowed a public outcome assertion failure")
                try:
                    dispatch(name, SimpleNamespace(mode="packaged", work_root=root))
                except ValueError as error:
                    if "source mode" not in str(error):
                        raise AssertionError(f"wrong mode was not refused: {error}") from error
                else:
                    raise AssertionError("dogfood scenario accepted packaged mode")
            finally:
                if original is None:
                    del IMPLEMENTED[name]
                else:
                    IMPLEMENTED[name] = original
