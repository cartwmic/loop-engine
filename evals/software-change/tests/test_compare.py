"""T7: paired comparison over single-trial summaries: blocks and attempts, unavailable values, visibility of
limit, infrastructure and resumed trials, driver time beside trial wall time, and read-only inputs."""
import copy
import hashlib
import json
import shutil
from pathlib import Path

import pytest

from scevals.compare import compare

RESULTS = Path(__file__).resolve().parents[1] / "results"
SHIPPED = {
    "tier-0-full-sonnet": RESULTS / "tier-0-full-sonnet" / "20261018T132346.405605Z" / "summary.json",
    "tier-0-full-sol": RESULTS / "tier-0-full-sol" / "20261018T135956.603628Z" / "summary.json",
}
BASE = json.loads(SHIPPED["tier-0-full-sonnet"].read_text())
WORKFLOW_USD = {"role-driver": 2.6649131, "role-implementer": 0.0135152, "role-reviewer": 0.9627927}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def make_trial(*, metrics: dict | None = None, **changes) -> dict:
    """A copy of the shipped Tier 0 trial (success, priced, unlabelled) with top-level changes and extra target metrics."""
    trial = copy.deepcopy(BASE["trials"][0]) | changes
    if metrics is not None:
        trial["target_metrics"] = trial["target_metrics"] | metrics
    return trial


def arm_metrics(*, driver_seconds=300.0, question_words=40, answer_words=12, grade=None) -> dict:
    """The metrics the target records when an arm is set (see the module docstring of compare.py)."""
    return {"driver_elapsed_seconds": driver_seconds,
            "owner_interaction": {"question_words": question_words, "answer_words": answer_words},
            "outcome_grade": {"intent_faithful": True, "unstable": False} if grade is None else grade}


def label(*, task="T1", repetition=1, arm="native", order=1, attempt=1, reason=None) -> dict:
    block = {"task": task, "repetition": repetition, "arm": arm, "order": order, "attempt": attempt}
    if reason is not None:
        block["attempt_reason"] = reason
    return block


def campaign(root: Path, name: str, trial: dict, *, block: dict | None = None, trials: list | None = None) -> Path:
    summary = copy.deepcopy(BASE)
    summary["campaign"] = name
    summary["settings"]["target_options"] = {} if block is None else {"block": block}
    summary["trials"] = trials if trials is not None else [trial]
    directory = root / "campaigns" / name / "20261019T000000.000000Z"
    directory.mkdir(parents=True)
    path = directory / "summary.json"
    path.write_text(json.dumps(summary))
    return path


def run(tmp_path: Path, paths: list[Path]) -> tuple[Path, dict, str]:
    destination = compare(paths, tmp_path / "out")
    return destination, json.loads((destination / "summary.json").read_text()), (destination / "report.md").read_text()


def row_by_arm(report: dict, arm: str, attempt: int = 1) -> dict:
    (block,) = report["blocks"]
    return next(r for r in block["attempts"] if r["arm"] == arm and r["attempt"] == attempt)


def test_rerun_sits_beside_the_original_and_never_replaces_it(tmp_path):
    paths = [
        campaign(tmp_path, "native-a1", make_trial(wall_seconds=1000.0, metrics=arm_metrics()),
                 block=label(arm="native", order=1, attempt=1)),
        campaign(tmp_path, "native-a2", make_trial(wall_seconds=1100.0, metrics=arm_metrics()),
                 block=label(arm="native", order=1, attempt=2,
                             reason="harness restarted the trial after a host reboot")),
        campaign(tmp_path, "super-a1", make_trial(wall_seconds=900.0, metrics=arm_metrics()),
                 block=label(arm="superpowers", order=2, attempt=1)),
    ]
    _, report, markdown = run(tmp_path, paths)
    (block,) = report["blocks"]
    assert [(r["arm"], r["attempt"]) for r in block["attempts"]] == [("native", 1), ("native", 2), ("superpowers", 1)]
    assert block["realised_order"] == [{"order": 1, "arm": "native"}, {"order": 2, "arm": "superpowers"}]
    assert block["attempts"][0]["trial_wall_seconds"] == 1000.0
    assert block["attempts"][1]["trial_wall_seconds"] == 1100.0
    assert "attempt 2: harness restarted the trial after a host reboot" in markdown
    assert "Realised order: 1: native, 2: superpowers" in markdown


@pytest.mark.parametrize("attempt", [2, 3])
def test_a_rerun_without_a_recorded_reason_is_refused(tmp_path, attempt):
    path = campaign(tmp_path, "rerun", make_trial(), block=label(attempt=attempt))
    with pytest.raises(ValueError, match="attempt_reason"):
        compare([path], tmp_path / "out")
    assert not (tmp_path / "out").exists()


def test_limit_infrastructure_and_resumed_trials_stay_visible(tmp_path):
    paths = [
        campaign(tmp_path, "limit", make_trial(outcome="limit", reason="hard_time", limit_step="scoring",
                                               terminal=False, metrics=arm_metrics()),
                 block=label(arm="native", order=1)),
        campaign(tmp_path, "infra", make_trial(outcome="infrastructure", reason="provider_service_error",
                                               terminal=False), block=label(arm="superpowers", order=2)),
        campaign(tmp_path, "resumed", make_trial(resumed=True, segments=[{"wall_seconds": 600.0}, {"wall_seconds": 589.5}]),
                 block=label(arm="openspec", order=3)),
    ]
    _, report, markdown = run(tmp_path, paths)
    assert row_by_arm(report, "native")["outcome"] == "limit"
    assert row_by_arm(report, "superpowers")["outcome"] == "infrastructure"
    assert row_by_arm(report, "openspec")["resumed"] is True
    assert row_by_arm(report, "openspec")["segments"] == 2
    assert "limit (hard_time)" in markdown and "limit at scoring" in markdown
    assert "infrastructure, excluded from success rate" in markdown
    assert "resumed, 2 segments" in markdown


@pytest.mark.parametrize(("damage", "tokens_known"), [
    pytest.param(lambda t: t.update(priced_usd_by_role=None), True, id="unpriced-model"),
    pytest.param(lambda t: t["priced_usd_by_role"].pop("role-reviewer"), True, id="price-missing-for-one-role"),
    pytest.param(lambda t: t.update(usage_unknown_roles=["role-reviewer"]), False, id="usage-unknown-for-one-role"),
])
def test_missing_price_or_usage_renders_unavailable_not_zero(tmp_path, damage, tokens_known):
    trial = make_trial()
    damage(trial)
    _, report, markdown = run(tmp_path, [campaign(tmp_path, "priced", trial, block=label())])
    row = row_by_arm(report, "native")
    assert row["workflow_usd"] is None
    assert row["workflow_tokens"] == (BASE["comparisons"][0]["core"]["workflow_cost"]["total_tokens"] if tokens_known else None)
    assert "unavailable" in markdown
    assert "$0.0000" not in markdown


def test_empty_usage_renders_unavailable_for_tokens_and_dollars(tmp_path):
    _, report, _ = run(tmp_path, [campaign(tmp_path, "no-usage", make_trial(usage_by_role={}), block=label())])
    row = row_by_arm(report, "native")
    assert row["workflow_usd"] is None and row["workflow_tokens"] is None
    assert row["overhead_usd"] is None and row["overhead_tokens"] is None


def test_driver_time_sits_beside_trial_wall_time_and_slow_scoring_does_not_inflate_driver_time(tmp_path):
    trial = make_trial(wall_seconds=2400.0, metrics=arm_metrics(driver_seconds=310.5))
    _, report, markdown = run(tmp_path, [campaign(tmp_path, "slow-scoring", trial, block=label())])
    row = row_by_arm(report, "native")
    assert row["driver_seconds"] == 310.5
    assert row["trial_wall_seconds"] == 2400.0
    assert row["workflow_usd"] == pytest.approx(sum(WORKFLOW_USD.values()))
    assert "| 310.5 | 2400.0 |" in markdown


def test_summaries_without_the_new_metrics_render_them_unavailable(tmp_path):
    _, report, markdown = run(tmp_path, [campaign(tmp_path, "predates", make_trial(), block=label())])
    row = row_by_arm(report, "native")
    assert row["driver_seconds"] is None
    assert row["owner_question_words"] is None and row["owner_answer_words"] is None
    assert row["intent"] is None
    assert row["delivery"] == {"numerator": 6, "denominator": 6}
    assert row["owner_questions"] == 3
    assert "| unavailable | 1189.5 | 3 | unavailable / unavailable |" in markdown


@pytest.mark.parametrize(("grade", "expected"), [
    ({"intent_faithful": True, "unstable": False}, "faithful"),
    ({"intent_faithful": False, "unstable": False}, "not faithful"),
    ({"intent_faithful": True, "unstable": True}, "unstable"),
    ({"intent_faithful": None, "unstable": False}, None),
])
def test_intent_verdict_shows_unstable_apart_and_missing_as_unavailable(tmp_path, grade, expected):
    _, report, _ = run(tmp_path, [campaign(tmp_path, "grade", make_trial(metrics=arm_metrics(grade=grade)), block=label())])
    assert row_by_arm(report, "native")["intent"] == expected


def test_inputs_are_never_modified_and_each_run_writes_a_new_report_pair(tmp_path):
    paths = [campaign(tmp_path, "native", make_trial(), block=label(arm="native")),
             campaign(tmp_path, "super", make_trial(), block=label(arm="superpowers", order=2))]
    before = {path: sha256(path) for path in paths}
    first, _, _ = run(tmp_path, paths)
    second = compare(paths, tmp_path / "out")
    assert first != second
    assert {path: sha256(path) for path in paths} == before
    for destination in (first, second):
        assert sorted(p.name for p in destination.iterdir()) == ["report.md", "summary.json"]
        assert json.loads((destination / "summary.json").read_text())["schema_version"] == 1


def test_output_inside_an_input_campaign_directory_is_refused_before_anything_is_written(tmp_path):
    path = campaign(tmp_path, "native", make_trial(), block=label())
    before = sorted(p.relative_to(path.parent) for p in path.parent.rglob("*"))
    with pytest.raises(ValueError, match="inside an input campaign directory"):
        compare([path], path.parent / "reports")
    assert sorted(p.relative_to(path.parent) for p in path.parent.rglob("*")) == before


def test_duplicate_attempts_are_refused(tmp_path):
    paths = [campaign(tmp_path, "one", make_trial(), block=label(arm="native")),
             campaign(tmp_path, "two", make_trial(), block=label(arm="native"))]
    with pytest.raises(ValueError, match="duplicate attempt"):
        compare(paths, tmp_path / "out")


@pytest.mark.parametrize(("block", "message"), [
    pytest.param({"task": "T1", "repetition": 1, "arm": "native", "order": 1}, "needs", id="missing-attempt"),
    pytest.param({**label(), "extra": 1}, "may add attempt_reason", id="unknown-key"),
    pytest.param(label(repetition=0), "repetition", id="repetition-zero"),
    pytest.param({**label(), "arm": ""}, "non-empty", id="empty-arm"),
    pytest.param(label(order=True), "order", id="boolean-order"),
])
def test_malformed_block_labels_are_refused(tmp_path, block, message):
    with pytest.raises(ValueError, match=message):
        compare([campaign(tmp_path, "bad", make_trial(), block=block)], tmp_path / "out")


def test_a_labelled_campaign_must_hold_exactly_one_trial(tmp_path):
    path = campaign(tmp_path, "two-trials", make_trial(), block=label(), trials=[make_trial(), make_trial()])
    with pytest.raises(ValueError, match="exactly one trial"):
        compare([path], tmp_path / "out")


def test_tier0_full_copies_render_read_only_and_match_core_aggregates(tmp_path):
    copies = []
    for name, shipped in SHIPPED.items():
        target = tmp_path / "copies" / name / shipped.parent.name / "summary.json"
        target.parent.mkdir(parents=True)
        shutil.copyfile(shipped, target)
        copies.append(target)
    assert [sha256(c) for c in copies] == [sha256(SHIPPED[c.parents[1].name]) for c in copies]
    before = {path: sha256(path) for path in copies}

    destination, report, markdown = run(tmp_path, copies)

    assert {path: sha256(path) for path in copies} == before
    (block,) = report["blocks"]
    assert (block["task"], block["repetition"]) == ("label-keys", 1)
    assert all(row["labelled"] is False and row["order"] is None for row in block["attempts"])
    for shipped in SHIPPED.values():
        core = json.loads(shipped.read_text())["comparisons"][0]["core"]
        arm = core["workflow_cost"]["by_role"]["role-driver"]["model"]
        row = next(r for r in block["attempts"] if r["arm"] == arm)
        assert row["workflow_usd"] == pytest.approx(core["workflow_cost"]["estimated_usd"], abs=1e-9)
        assert row["workflow_tokens"] == core["workflow_cost"]["total_tokens"]
        assert row["overhead_usd"] == pytest.approx(core["evaluation_overhead"]["estimated_usd"], abs=1e-9)
        assert row["overhead_tokens"] == core["evaluation_overhead"]["total_tokens"]
        assert row["driver_seconds"] is None and row["intent"] is None
        assert row["trial_wall_seconds"] == pytest.approx(core["wall_seconds"])
    sol = next(r for r in block["attempts"] if r["arm"] == "codex/gpt-6.1-sol")
    assert (sol["outcome"], sol["reason"]) == ("failure", "target_failure")
    assert sol["hidden_tests"] == {"passed": True, "passed_tests": 6, "total_tests": 6}
    assert "failure (target_failure)" in markdown
    assert destination.parent == tmp_path / "out"
