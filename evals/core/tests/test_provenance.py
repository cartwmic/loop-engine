"""Projection-cohort labels: strict validation, truthful computation, explicit non-pooling refusals."""
import hashlib
import json
from pathlib import Path

import pytest

from evalcore.provenance import (BASELINE, check_same_cohort, cohort, label_from_candidate, tree_sha256,
                                 validate_label)
from evalcore.report import render_markdown
from test_fake_campaign import invoke, run, settings

H = "a" * 64


def label(**overrides):
    return {"commission_projection": "external_review_ref/v1", "product_tree_sha256": H, "patch_sha256": "b" * 64,
            "skill_md_sha256": "c" * 64, "binary_identity": "software-change built from tree " + H, **overrides}


@pytest.mark.parametrize("bad", [
    {k: v for k, v in label().items() if k != "patch_sha256"},
    {**label(), "extra": "x"},
    label(product_tree_sha256="A" * 64),
    label(skill_md_sha256="short"),
    label(commission_projection="v1"),
    label(binary_identity=" "),
    "not-a-dict",
])
def test_label_validation_is_strict(bad):
    with pytest.raises(ValueError):
        validate_label(bad)


def test_label_is_computed_from_actual_candidate_bytes(tmp_path):
    skill = tmp_path / "crates/software-change-provider/skills/using-software-change-provider/SKILL.md"
    skill.parent.mkdir(parents=True)
    skill.write_text("skill v2.2\n")
    (tmp_path / "src.rs").write_text("fn main() {}\n")
    (tmp_path / "target").mkdir()
    (tmp_path / "target/ignored").write_text("build output")
    (tmp_path / ".git").mkdir()
    (tmp_path / ".git/HEAD").write_text("ref")
    patch = tmp_path.parent / (tmp_path.name + ".patch")
    patch.write_text("--- a\n+++ b\n")
    computed = label_from_candidate(tmp_path, patch, commission_projection="external_review_ref/v1",
                                    binary_identity="test binary")
    expected_tree = hashlib.sha256()
    for path in sorted(tmp_path.rglob("*")):
        if path.is_file() and "target" not in path.parts and ".git" not in path.parts:
            expected_tree.update(str(path.relative_to(tmp_path)).encode())
            expected_tree.update(hashlib.sha256(path.read_bytes()).digest())
    assert computed["product_tree_sha256"] == expected_tree.hexdigest() == tree_sha256(tmp_path)
    assert computed["patch_sha256"] == hashlib.sha256(patch.read_bytes()).hexdigest()
    assert computed["skill_md_sha256"] == hashlib.sha256(skill.read_bytes()).hexdigest()
    before = computed["product_tree_sha256"]
    (tmp_path / "target/ignored").write_text("changed build output")
    assert tree_sha256(tmp_path) == before
    (tmp_path / "src.rs").write_text("fn main() { changed }\n")
    assert tree_sha256(tmp_path) != before


def test_cohorts_baseline_labelled_and_distinct():
    assert cohort({"target_options": {}}) == BASELINE
    assert cohort({}) == BASELINE
    one = cohort({"target_options": {"projection_label": label()}})
    two = cohort({"target_options": {"projection_label": label(patch_sha256="d" * 64)}})
    assert one.startswith("external_review_ref/v1@") and one != two != BASELINE
    with pytest.raises(ValueError, match="cohorts are not pooled"):
        check_same_cohort({"target_options": {}}, {"target_options": {"projection_label": label()}}, action="merge")
    assert check_same_cohort({"target_options": {"projection_label": label()}},
                             {"target_options": {"projection_label": label()}}, action="merge") == one


def test_old_summaries_render_without_a_cohort_line():
    summary = {"campaign": "old", "target": "fake", "classification_rules": "rules", "settings": {},
               "pinned_models": {}, "versions": {}, "matched_assignment": [], "comparisons": [], "trials": []}
    assert "Projection cohort" not in render_markdown(summary)
    assert "Projection cohort: baseline" in render_markdown({**summary, "projection_cohort": BASELINE})


def report_of(root):
    return next((root / "results/test").glob("*/report.md")).read_text()


def test_labelled_and_baseline_campaigns_record_their_cohort(campaign_root):
    labelled = run(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1,
                   target_options={"inject_errors": False, "projection_label": label()})
    assert labelled["projection_cohort"] == cohort({"target_options": {"projection_label": label()}})
    assert labelled["settings"]["target_options"]["projection_label"] == label()
    assert f"Projection cohort: {labelled['projection_cohort']} (not pooled" in report_of(campaign_root)


def test_unlabelled_campaign_is_the_explicit_baseline_cohort(campaign_root):
    summary = run(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1,
                  target_options={"inject_errors": False})
    assert summary["projection_cohort"] == BASELINE
    assert "Projection cohort: baseline" in report_of(campaign_root)


def test_invalid_label_is_refused_before_any_trial(campaign_root):
    path = settings(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1,
                    target_options={"inject_errors": False, "projection_label": label(patch_sha256="x")})
    result = invoke(campaign_root, "campaign", "--settings", str(path))
    # the CLI exports only the error class, never its text
    assert result.returncode == 2 and "Command failed (ValueError); no sensitive error text exported." in result.stderr
    assert not (campaign_root / "results").exists()
    assert not list((campaign_root / "logs").glob("*.eval")) if (campaign_root / "logs").exists() else True


def test_resume_across_cohorts_is_refused_explicitly(campaign_root, monkeypatch):
    run(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1, target_options={"inject_errors": False})
    result_dir = next((campaign_root / "results/test").glob("*"))
    before = sorted(p.name for p in (campaign_root / "results/test").iterdir())
    path = settings(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1,
                    target_options={"inject_errors": False, "projection_label": label()})
    result = invoke(campaign_root, "campaign", "--settings", str(path), "--resume", str(result_dir))
    assert result.returncode == 2 and "Command failed (ValueError); no sensitive error text exported." in result.stderr
    assert sorted(p.name for p in (campaign_root / "results/test").iterdir()) == before
    # the refusal is the explicit cohort one (the CLI hides error text, so check the same call in process)
    from evalcore.continuation import resume_campaign
    from evalcore.settings import CampaignSettings
    from fake_target import create_target
    monkeypatch.setenv("EVAL_FAKE_ROOT", str(campaign_root))
    current = CampaignSettings.model_validate(json.loads(path.read_text()))
    with pytest.raises(ValueError, match="resume refused: projection cohort baseline differs"):
        resume_campaign(create_target(), current, result_dir)
    assert sorted(p.name for p in (campaign_root / "results/test").iterdir()) == before
