"""The proof runner reads pytest's summary: a skip on a listed reason is on purpose; any other skip fails the unit."""
import run_proof_groups as runner

PI = "software-change/tests/test_arms_adapters.py::test_four_arms_launch_through_real_pi_and_score_through_the_common_rule"
CORE = "core/tests/test_pi.py::test_actual_sdk_loads_provider_extension_and_makes_one_completion[None]"
OWNER = "software-change/tests/test_owner_context.py::test_real_pi_in_the_image_sends_the_owner_context[a]"
OTHER = "software-change/tests/test_cases.py::test_loads"


def summary(*lines):
    return "\n".join(["", *lines, "2 passed, 1 skipped in 0.1s"])


def test_skips_on_listed_reasons_are_on_purpose_and_keep_their_reason():
    text = summary(f"SKIPPED evals/{PI} - Skipped: arms image not available (image x is not built)",
                   f"SKIPPED evals/{CORE} - Skipped: Pi SDK not installed (pi provider requires the installed Pi SDK)",
                   f"SKIPPED evals/{OWNER} - Skipped: SC_OWNER_CONTEXT_IMAGE not set",
                   f"PASSED evals/{OTHER}")
    outcomes, reasons, unexpected = runner.parse_outcomes(text, [PI, CORE, OWNER, OTHER])
    on_purpose, refused = runner.judge_skips(outcomes, reasons)
    assert unexpected == [] and refused == []
    assert [e["id"] for e in on_purpose] == sorted([PI, CORE, OWNER])
    assert dict((e["id"], e["reason"]) for e in on_purpose)[OWNER] == "SC_OWNER_CONTEXT_IMAGE not set"
    assert outcomes[OTHER] == "PASSED"


def test_a_skip_for_another_reason_or_in_another_file_is_refused():
    text = summary(f"SKIPPED evals/{PI} - Skipped: flaky, skipped by hand",
                   f"SKIPPED evals/{OTHER} - Skipped: arms image not available (x)")
    outcomes, reasons, _ = runner.parse_outcomes(text, [PI, OTHER])
    on_purpose, refused = runner.judge_skips(outcomes, reasons)
    assert on_purpose == []
    assert sorted(e["id"] for e in refused) == sorted([PI, OTHER])


def test_a_skipped_test_still_counts_as_executed_and_a_test_with_no_line_is_missing():
    text = summary(f"SKIPPED evals/{OWNER} - Skipped: SC_OWNER_CONTEXT_IMAGE not set")
    outcomes, _, _ = runner.parse_outcomes(text, [OWNER, OTHER])
    assert set(outcomes) == {OWNER}
