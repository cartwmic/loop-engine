"""merge-captures refuses to pool projection cohorts before its settings-equality check (no Docker, no models)."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

import pytest

from evalcore.settings import CampaignSettings, matched_assignment
from scevals.lifecycle import merge_captures
from scevals.target import SoftwareChangeTarget

REPO = Path(__file__).resolve().parents[3]
CASES = REPO / "evals/software-change/example-case"
LABEL = {"commission_projection": "external_review_ref/v1", "product_tree_sha256": "a" * 64,
         "patch_sha256": "b" * 64, "skill_md_sha256": "c" * 64, "binary_identity": "test binary"}
HIDDEN = "Command failed (ValueError); no sensitive error text exported."


def settings_data(**options):
    return {"campaign": "merge-cohort", "models": ["fake/a"], "cases_path": str(CASES), "case_count": 1,
            "trials_per_case": 1, "simulator_model": "fake/a", "judge_model": "fake/a", "target_options": options}


def prepare(current, source_options):
    root = Path(tempfile.mkdtemp(prefix="merge-cohort-"))
    config = root / "settings.json"
    config.write_text(json.dumps(current))
    source = root / "source"
    source.mkdir()
    recorded = CampaignSettings.model_validate(settings_data(**source_options))
    _, assignments = matched_assignment(recorded, SoftwareChangeTarget().load_cases(recorded.cases_path))
    (source / "summary.json").write_text(json.dumps({
        "settings": recorded.recorded(), "matched_assignment": [a.model_dump() for a in assignments], "trials": []}))
    return config, source, root / "merged"


def cli(config, source, out):
    return subprocess.run(["uv", "run", "--project", str(REPO / "evals"), "evalctl", "merge-captures", str(out),
                           str(source), "--settings", str(config)], cwd=REPO, capture_output=True, text=True,
                          timeout=60)


def in_process(config, source, out, *, more=(), exception=None):
    merge_captures(argparse.Namespace(settings=str(config), out_dir=out, src_dirs=[source, *more],
                                      execution_exception=exception))


def extra_source(root, options):
    source = root / ("source-" + str(len(list(root.glob("source*")))))
    source.mkdir()
    recorded = CampaignSettings.model_validate(settings_data(**options))
    _, assignments = matched_assignment(recorded, SoftwareChangeTarget().load_cases(recorded.cases_path))
    (source / "summary.json").write_text(json.dumps({
        "settings": recorded.recorded(), "matched_assignment": [a.model_dump() for a in assignments], "trials": []}))
    return source


def test_merge_across_cohorts_is_refused_before_output_exists():
    config, source, out = prepare(settings_data(projection_label=LABEL), {})
    result = cli(config, source, out)
    assert result.returncode == 2 and HIDDEN in result.stderr
    assert not out.exists()
    # the CLI hides error text: the same call in process shows it is the explicit cohort refusal
    with pytest.raises(ValueError, match="merge refused: projection cohort baseline differs"):
        in_process(config, source, out)
    assert not out.exists()


def test_same_cohort_passes_the_cohort_check_and_reaches_the_existing_checks():
    config, source, out = prepare(settings_data(projection_label=LABEL), {"projection_label": LABEL})
    # no scored trials in the source: the existing refusal, not the cohort one
    with pytest.raises(ValueError, match="no scored captures to merge"):
        in_process(config, source, out)
    assert not out.exists()


def test_execution_exception_path_still_refuses_across_cohorts():
    # the exception path skips the scoring-settings equality check; the cohort check comes first regardless
    config, source, out = prepare(settings_data(projection_label=LABEL), {})
    exception = out.parent / "exception.json"
    exception.write_text(json.dumps({"kind": "disable-outer-deadlines", "owner": "o", "authority": "a", "reason": "r",
                                     "original_limits": {}, "new_limits": {}}))
    with pytest.raises(ValueError, match="merge refused: projection cohort baseline differs"):
        in_process(config, source, out, exception=exception)
    assert not out.exists()


def test_mixed_label_sources_in_one_merge_are_refused():
    config, source, out = prepare(settings_data(projection_label=LABEL), {"projection_label": LABEL})
    other = extra_source(out.parent, {"projection_label": {**LABEL, "patch_sha256": "d" * 64}})
    with pytest.raises(ValueError, match="merge refused: projection cohort external_review_ref/v1@"):
        in_process(config, source, out, more=[other])
    assert not out.exists()


def test_the_label_is_recorded_and_only_read_by_the_gate_never_forwarded():
    import inspect as pyinspect
    import scevals.target as target_module
    source = pyinspect.getsource(target_module)
    # the target never reads the label key itself (no quoted key), only hands settings to the gate
    assert "'projection_label'" not in source and '"projection_label"' not in source
    assert "check_projection_label_gate(context.settings, identity)" in source
    labelled = CampaignSettings.model_validate(settings_data(projection_label=LABEL))
    assert labelled.recorded()["target_options"]["projection_label"] == LABEL


def gate_identity(revision, image_id="sha256:" + "e" * 64):
    return {"Id": image_id, "Config": {"Labels": {"org.opencontainers.image.revision": revision}}}


def test_label_gate_refuses_unlabelled_or_mismatched_runs_on_projection_binaries_only():
    from scevals.label_gate import check_projection_label_gate, required_revisions
    required = frozenset({"f1commit"})
    unlabelled = CampaignSettings.model_validate(settings_data())
    with pytest.raises(ValueError, match="unlabelled runs are invalid"):
        check_projection_label_gate(unlabelled, gate_identity("f1commit"), required)
    wrong = CampaignSettings.model_validate(settings_data(projection_label=LABEL))
    with pytest.raises(ValueError, match="must equal the trial image ID"):
        check_projection_label_gate(wrong, gate_identity("f1commit"), required)
    right = CampaignSettings.model_validate(settings_data(projection_label={**LABEL, "binary_identity": "sha256:" + "e" * 64}))
    check_projection_label_gate(right, gate_identity("f1commit"), required)
    # baseline images are unaffected, labelled or not
    check_projection_label_gate(unlabelled, gate_identity("3e1f9ee9"), required)
    # the shipped list names the eval-only v2.2c commit
    assert "ac60b738858c92dd5c26b56cac9228525acf634e" in required_revisions()
