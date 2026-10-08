"""F4 black-box public evalctl proofs: real Pi/engine, Docker, scripted only."""
import copy
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib
import uuid

import pytest

REPO = Path(__file__).resolve().parents[3]
ROOT = REPO / "evals/software-change"
PROOFS = ROOT / "captures/F4/tests"


def settings(**options):
    data = tomllib.loads((ROOT / "campaigns/scripted-example.toml").read_text())
    data["campaign"] = "f4-" + uuid.uuid4().hex
    data["cases_path"] = str(ROOT / "example-case")
    data["target_options"].update(options)
    PROOFS.mkdir(parents=True, exist_ok=True)
    path = PROOFS / (data["campaign"] + ".json")
    path.write_text(json.dumps(data))
    return data, path


def invoke(*args, code=0):
    result = subprocess.run(["uv", "run", "--project", str(REPO / "evals"), "evalctl", *map(str, args)],
                            cwd=REPO, env=os.environ.copy(), text=True, capture_output=True, timeout=650)
    (PROOFS / (uuid.uuid4().hex + ".log")).write_text(result.stdout + result.stderr)
    assert result.returncode == code, result.stdout + result.stderr
    if code:
        return result
    directory = Path(next(line.split("Report written: ", 1)[1] for line in result.stdout.splitlines() if line.startswith("Report written: ")))
    summary = json.loads((directory / "summary.json").read_text())
    markdown = (directory / "report.md").read_text()
    # Test receipts are private; never leave generated reports for publication.
    destination = PROOFS / (directory.parent.name + "-" + directory.name)
    directory.rename(destination)
    return summary, destination, markdown


def complete_capture(summary, sample="m0-c0-t1"):
    capture = Path(summary["capture_run"]) / sample
    assert (capture / "workspace.tar").is_file() and (capture / "meta.json").is_file()
    meta = json.loads((capture / "meta.json").read_text())
    assert meta["assignment"]["sample_id"] == sample
    assert meta["harness"]["checkout_commit"]
    with tarfile.open(capture / "workspace.tar") as tar:
        names = {m.name.removeprefix("./") for m in tar}
        assert {"trial/run.sqlite", "trial/artifacts/intent.json", "trial/show.json", "trial/history.json",
                "trial/providers.toml", "trial/local-push-log", "public/repo/.git/HEAD", "local-remote.git/HEAD"} <= names
        assert not any(m.isdir() and Path(m.name.rstrip("/")).name in ("target", ".cache") for m in tar.getmembers())
        assert not any("target" in Path(n).parts[:-1] or ".cache" in Path(n).parts[:-1] for n in names)
        assert not any(n.startswith("opt/scoring") for n in names)
        assert b"[providers.software-change]" in tar.extractfile("./trial/providers.toml").read()
    return capture


# Normalization is explicit and ONLY removes identities, timings and provenance.
# All outcomes, individual tests, recalls, discipline, honesty, health, judges,
# role token figures and aggregate rates/costs remain in the exact comparison.
TIMING_KEYS = {"elapsed_seconds", "terminal_elapsed_seconds", "wall_seconds", "campaign_wall_seconds"}
IDENTITY_KEYS = {"id", "image_id"}  # applies ONLY to the container evidence object


def scored_values(summary):
    trials = copy.deepcopy(summary["trials"])
    for trial in trials:
        trial.pop("wall_seconds", None)
        trial.pop("segments", None)  # segment wall times/provenance are checked separately
        trial.pop("carried_from", None)
        metrics = trial["target_metrics"]
        metrics.pop("terminal_elapsed_seconds", None)
        for row in metrics.get("copy_log", []) + metrics.get("copy_out_log", []):
            row.pop("elapsed_seconds", None)
        for key in IDENTITY_KEYS:
            metrics.get("container", {}).pop(key, None)
    comparisons = copy.deepcopy(summary["comparisons"])
    for combo in comparisons:
        combo["core"].pop("wall_seconds", None)
    return trials, comparisons


def test_capture_first_fault_rescore_and_same_capture_parity():
    # The source/config markers and provider-looking filename are legitimate
    # private captured work. target/.cache payloads must not inflate snapshots.
    command = ["sh", "-c", 'printf "\\133providers.x\\135\\n" > /workspace/trial/example-providers.toml; '
               'mkdir -p target .cache; printf "Signature: 8a477f597d28d172789f06886806bc55\\n" > target/CACHEDIR.TAG; '
               'printf excluded > target/probe; printf excluded > .cache/probe; '
               'printf retained-file > /workspace/trial/target; exec sc-driver "$@"', "sh"]
    _, clean_settings = settings(scripted_driver="asks-and-builds", driver_command=command)
    clean, _, _ = invoke("campaign", "--settings", clean_settings)
    clean_capture = complete_capture(clean)
    with tarfile.open(clean_capture / "workspace.tar") as tar:
        assert tar.extractfile("./trial/example-providers.toml").read() == b"[providers.x]\n"
        assert tar.extractfile("./trial/target").read() == b"retained-file"
    replay, _, _ = invoke("rescore", clean_capture.parent)
    repeat, _, _ = invoke("rescore", clean_capture.parent)
    assert scored_values(clean) == scored_values(replay) == scored_values(repeat)

    _, faulty_settings = settings(scripted_driver="asks-and-builds", driver_command=command, scripted_scoring_error_trial=1)
    faulty, _, _ = invoke("campaign", "--settings", faulty_settings)
    assert faulty["trials"][0]["reason"] == "unclassified_error"
    capture = complete_capture(faulty)
    recovered, _, _ = invoke("rescore", capture.parent)
    assert scored_values(clean) == scored_values(recovered)
    assert recovered["trials"][0]["outcome"] == "success"

    # Recover the exact same bare workspace snapshot with explicit identity flags.
    external, _, _ = invoke("rescore", capture / "workspace.tar", "--sample", "external-trial",
                            "--case", ROOT / "example-case", "--model", "sc-scripted/driver", "--settings", clean_settings)
    assert external["trials"][0]["outcome"] == "success"
    assert external["trials"][0]["target_metrics"]["hidden_test_outcome"] == recovered["trials"][0]["target_metrics"]["hidden_test_outcome"]
    assert not external["trials"][0]["usage_by_role"].get("role-driver")  # unknown, not fabricated
    external_metrics = external["trials"][0]["target_metrics"]
    assert external_metrics["elicitation_recall"] is None
    assert external_metrics["simulated_owner"]["answer_count"] is None
    assert external_metrics["discipline"]["owner_question_count"] is None
    assert external_metrics["run_health"]["unattended_finish"] is None
    assert external["comparisons"][0]["core"]["workflow_cost"]["total_tokens"] is None


def test_terminated_run_scores_hidden_delivery_without_success():
    _, config = settings(scripted_driver="asks-and-builds", scripted_terminated=True)
    summary, _, markdown = invoke("campaign", "--settings", config)
    trial = summary["trials"][0]
    complete_capture(summary)
    assert trial["terminal"] and trial["outcome"] == "failure"
    assert trial["target_metrics"]["lifecycle"] == "terminated"
    assert trial["target_metrics"]["hidden_test_outcome"]["total_tests"] == 6
    assert summary["comparisons"][0]["core"]["terminated_trials"] == 1
    assert "terminated" in markdown
    refused = invoke("resume-trial", Path(summary["capture_run"]) / trial["sample_id"], code=2)
    assert "Command failed" in refused.stderr


def test_campaign_resume_reruns_only_infrastructure_and_merges():
    data, config = settings(scripted_driver="asks-and-builds", scripted_infrastructure_trial=2)
    data["trials_per_case"] = 2
    config.write_text(json.dumps(data))
    original, original_report, _ = invoke("campaign", "--settings", config)
    assert [t["outcome"] for t in original["trials"]] == ["success", "infrastructure"]
    complete_capture(original, "m0-c0-t2")
    merged, _, markdown = invoke("campaign", "--settings", config, "--resume", original_report)
    assert [t["outcome"] for t in merged["trials"]] == ["success", "success"]
    assert merged["trials"][0]["carried_from"] == str(original_report)
    assert merged["trials"][0]["target_metrics"] == original["trials"][0]["target_metrics"]
    assert merged["comparisons"][0]["core"]["pass_k"]["rate"] == 1
    assert "Carried trials" in markdown
    captures = Path(merged["capture_run"])
    assert not (captures / "m0-c0-t1").exists() and (captures / "m0-c0-t2/workspace.tar").exists()
    # A capture run is also an accepted continuation source; carry both, no launch.
    again, _, _ = invoke("campaign", "--settings", config, "--resume", captures)
    assert len(again["carried_trials"]) == 2
    data["idle_limit_seconds"] = data.get("idle_limit_seconds", 1800) + 1
    config.write_text(json.dumps(data))
    invoke("campaign", "--settings", config, "--resume", captures, code=2)


@pytest.mark.parametrize("via_campaign", [False, True])
def test_resume_continues_restored_db_owner_answers_and_excludes_headline(via_campaign):
    _, config = settings(scripted_driver="asks-and-builds", scripted_infrastructure_trial=1)
    original, _, _ = invoke("campaign", "--settings", config)
    assert original["trials"][0]["outcome"] == "infrastructure"
    capture = complete_capture(original)
    meta = json.loads((capture / "meta.json").read_text())
    assert len(meta["simulated_owner"]["answers"]) == 3
    if via_campaign:
        resumed, _, markdown = invoke("campaign", "--settings", config, "--resume", capture.parent, "--resume-interrupted")
    else:
        resumed, _, markdown = invoke("resume-trial", capture)
    trial = resumed["trials"][0]
    assert trial["terminal"] and trial["outcome"] == "success" and trial["resumed"]
    assert len(trial["segments"]) == 2
    assert [s["outcome"] for s in trial["segments"]] == ["infrastructure", "success"]
    assert trial["target_metrics"]["simulated_owner"]["answer_count"] == 3
    assert trial["usage_by_role"]["simulator"]["total_tokens"] == 90  # no repeated disclosure
    core = resumed["comparisons"][0]["core"]
    assert core["success_rate"]["denominator"] == core["pass_k"]["denominator"] == 0
    assert core["resumed_success_rate"]["rate"] == 1
    assert "Resumed trials (excluded" in markdown
    restored = complete_capture(resumed)
    # Original DB's durable history prefix and repo baseline survive; no start.
    with tarfile.open(capture / "workspace.tar") as before, tarfile.open(restored / "workspace.tar") as after:
        old_history = json.load(before.extractfile("./trial/history.json"))["result"]
        new_history = json.load(after.extractfile("./trial/history.json"))["result"]
        assert new_history[:len(old_history)] == old_history
        assert before.extractfile("./public/repo/.git/HEAD").read() == after.extractfile("./public/repo/.git/HEAD").read()
    replay, _, _ = invoke("rescore", restored.parent)
    assert replay["trials"][0]["resumed"] and len(replay["trials"][0]["segments"]) == 2
    assert replay["comparisons"][0]["core"]["success_rate"]["denominator"] == 0


def test_merge_rescored_captures_and_resume_only_missing_trial():
    data, config = settings(scripted_driver="asks-and-builds")
    data["trials_per_case"] = 3
    config.write_text(json.dumps(data))
    original, _, _ = invoke("campaign", "--settings", config)
    root = Path(original["capture_run"])
    rescored = []
    for sample in ("m0-c0-t1", "m0-c0-t2"):
        subset = PROOFS / (uuid.uuid4().hex + "-subset")
        subset.mkdir(mode=0o700)
        shutil.copytree(root / sample, subset / sample)
        manifest = json.loads((root / "run.json").read_text())
        manifest["sources"] = {}
        (subset / "run.json").write_text(json.dumps(manifest))
        score, _, _ = invoke("rescore", subset, "--settings", config)
        rescored.append(score)
    output = PROOFS / (uuid.uuid4().hex + "-merged")

    def merge(destination, *sources, code=0):
        result = subprocess.run(["uv", "run", "--project", str(REPO / "evals"), "evalctl",
                                 "merge-captures", str(destination), *map(str, sources), "--settings", str(config)],
                                cwd=REPO, capture_output=True, text=True, timeout=30)
        (PROOFS / (uuid.uuid4().hex + "-merge.log")).write_text(result.stdout + result.stderr)
        assert result.returncode == code, result.stdout + result.stderr
        return result

    sources = [Path(s["capture_run"]) for s in rescored]
    assert "Merged 2 captures:" in merge(output, *sources).stdout
    for sample in ("m0-c0-t1", "m0-c0-t2"):
        assert (output / sample / "workspace.tar").is_file()
    # Duplicate/settings failures validate before creating output; never overwrite.
    duplicate = PROOFS / (uuid.uuid4().hex + "-duplicate")
    merge(duplicate, sources[0], sources[0], code=2)
    assert not duplicate.exists()
    mismatch = PROOFS / (uuid.uuid4().hex + "-mismatch")
    data["idle_limit_seconds"] = data.get("idle_limit_seconds", 1800) + 1
    config.write_text(json.dumps(data))
    merge(mismatch, *sources, code=2)
    assert not mismatch.exists()
    data["idle_limit_seconds"] -= 1
    config.write_text(json.dumps(data))
    merge(output, *sources, code=2)
    resumed, _, _ = invoke("campaign", "--settings", config, "--resume", output)
    assert len(resumed["trials"]) == 3 and len(resumed["carried_trials"]) == 2
    assert all(t["outcome"] == "success" for t in resumed["trials"])
    for index in (0, 1):
        assert resumed["trials"][index]["target_metrics"] == rescored[index]["trials"][0]["target_metrics"]
        assert resumed["trials"][index]["usage_by_role"] == rescored[index]["trials"][0]["usage_by_role"]
    new_captures = Path(resumed["capture_run"])
    assert not (new_captures / "m0-c0-t1").exists()
    assert not (new_captures / "m0-c0-t2").exists()
    assert (new_captures / "m0-c0-t3/workspace.tar").is_file()
    assert resumed["comparisons"][0]["core"]["pass_k"]["rate"] == 1


def test_refused_report_keeps_private_capture_and_recovery_command():
    _, config = settings(scripted_driver="asks-and-builds", harmless_case_content="[providers.x]")
    result = invoke("campaign", "--settings", config, code=2)
    assert "Private captures kept. Rescore with: evalctl rescore " in result.stdout
    capture_run = Path(next(line.split("Captures retained: ")[1] for line in result.stdout.splitlines() if line.startswith("Captures retained: ")))
    assert (capture_run / "m0-c0-t1/workspace.tar").is_file()
    assert not (ROOT / "results" / capture_run.parent.name).exists()
    # Remove only the report-tripping irrelevant setting; capture itself is never scanned.
    data = json.loads(config.read_text())
    data["target_options"].pop("harmless_case_content")
    config.write_text(json.dumps(data))
    recovered, _, _ = invoke("rescore", capture_run, "--settings", config)
    assert recovered["trials"][0]["outcome"] == "success"
