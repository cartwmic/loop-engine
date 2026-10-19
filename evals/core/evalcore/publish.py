"""Bounded report publication.

The command must end by the cap D with something readable. So a small limit-only record (no private text)
is written first; the full report is then rendered, scanned and staged by a child process that the parent
kills at D. Only a finished staged report replaces the record. The child imports no model code.
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from .report import UnsafeReportError, protected_values, render_markdown, scan_results, utc_timestamp

WALL_TOKEN = "{CAMPAIGN_WALL_SECONDS}"
UNSAFE_EXIT = 3
KILL_MARGIN_SECONDS = 1.0
RECORD_NOTE = ("Limit-only record: the full report was not finished before the campaign cap. "
               "It holds outcomes, reasons and budget figures only, no private text.")


def minimal_record(summary: dict, elapsed: float) -> dict:
    """Fixed-shape, private-text-free. Case IDs and campaign names are already exported by the full report."""
    budget = summary.get("budget") or {}
    return {
        "schema_version": 1, "record": "limit-only", "complete": False, "note": RECORD_NOTE,
        "campaign": summary["campaign"], "target": summary["target"], "tier": budget.get("tier"),
        "caps": budget.get("caps"), "cost_verdict": budget.get("cost_verdict"),
        "actual": budget.get("actual"), "campaign_wall_seconds": round(elapsed, 3),
        "stop_events": budget.get("stop_events", []), "cut_off_steps": budget.get("cut_off_steps", []),
        "setup_limit": summary.get("setup_limit"),
        "trials": [{key: t.get(key) for key in ("sample_id", "combination_id", "case_id", "trial", "outcome", "reason", "limit_step")}
                   for t in summary["trials"]],
    }


def minimal_markdown(record: dict) -> str:
    lines = [f"# Campaign {record['campaign']} (limit-only record)", "", record["note"], "",
             f"Elapsed seconds: {record['campaign_wall_seconds']}", f"Cost verdict: {record['cost_verdict']}", ""]
    lines += [f"- {t['sample_id']}: {t['outcome']} ({t['reason']}{', ' + t['limit_step'] if t['limit_step'] else ''})"
              for t in record["trials"]]
    return "\n".join(lines) + "\n"


def finish_fallback(destination: Path, record: dict, command_start: float, publishing_started: float) -> float:
    """The fallback record stands as the result, so its duration must be the real command-to-written time, not the time at
    which it was first written, and the publication receipt must exist like on the normal path."""
    elapsed = time.monotonic() - command_start
    cap = (record.get("caps") or {}).get("wall_seconds")
    record["campaign_wall_seconds"] = round(elapsed, 3)
    if isinstance(record.get("actual"), dict):
        record["actual"] = {**record["actual"], "wall_seconds": round(elapsed, 3)}
    record["wall_within_cap"] = None if cap is None else elapsed <= cap
    (destination / "publication.json").write_text(json.dumps({
        "command_start_to_report_written_seconds": elapsed, "publication_started_seconds": publishing_started - command_start,
        "cap_seconds": cap, "within_cap": record["wall_within_cap"], "full_report": False}, indent=2) + "\n")
    return elapsed


def record_publication_cut_off(destination: Path, record: dict, command_start: float, publishing_started: float) -> None:
    """Name the cut-off in the limit-only record the owner will read: campaign wall time ran out at publication."""
    finish_fallback(destination, record, command_start, publishing_started)
    cut = {"step": "publication", "reason": "campaign_wall_time", "sample_id": None}
    record["cut_off_steps"] = [*record.get("cut_off_steps", []), cut]
    record["stop_events"] = [*record.get("stop_events", []), {"reason": "campaign_wall_time", "step": "publication"}]
    record["publication_cut_off"] = True
    (destination / "summary.json").write_text(json.dumps(record, indent=2) + "\n")
    (destination / "report.md").write_text(minimal_markdown(record) + "\nCut off: publication (campaign_wall_time).\n")


def reserve_destination(results_dir: Path, campaign: str, report_directory: Path | None) -> Path:
    parent = report_directory.parent if report_directory is not None else results_dir / campaign
    parent.mkdir(parents=True, exist_ok=True)
    while True:
        destination = report_directory if report_directory is not None else parent / utc_timestamp()
        try:
            destination.mkdir()
            return destination
        except FileExistsError:
            if report_directory is not None:
                raise


def remove_destination(destination: Path, results_dir: Path) -> None:
    shutil.rmtree(destination, ignore_errors=True)
    for directory in (destination.parent, results_dir):
        try:
            directory.rmdir()
        except OSError:
            pass


def publish(results_dir: Path, summary: dict, *, provider_config_paths=(), report_directory: Path | None = None,
            deadline: float | None = None, command_start: float | None = None, minimal_only: bool = False) -> Path:
    """Publish by the deadline (monotonic time) or leave the limit-only record."""
    import re
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", summary["campaign"]):
        raise ValueError("invalid campaign directory name")
    command_start = time.monotonic() if command_start is None else command_start
    destination = reserve_destination(results_dir, summary["campaign"], report_directory)
    record = minimal_record(summary, time.monotonic() - command_start)
    (destination / "summary.json").write_text(json.dumps(record, indent=2) + "\n")
    (destination / "report.md").write_text(minimal_markdown(record))
    if minimal_only:
        return destination
    publishing_started = time.monotonic()
    # The kill and the fallback write take a moment, so the renderer is stopped slightly before D to keep the written time inside it.
    remaining = None if deadline is None else deadline - publishing_started - KILL_MARGIN_SECONDS
    if remaining is not None and remaining <= 0:
        record_publication_cut_off(destination, record, command_start, publishing_started)
        print("Cap reached before the full report could start; limit-only record kept.", flush=True)
        return destination
    work = Path(tempfile.mkdtemp(prefix="evalctl-publish-"))  # private: holds the unscanned summary
    try:
        staging = work / "staged"
        staging.mkdir()
        request = work / "request.json"
        request.write_text(json.dumps({"summary": summary, "provider_config_paths": [str(p) for p in provider_config_paths],
                                        "staging": str(staging)}, ensure_ascii=False, allow_nan=False))
        child = subprocess.Popen([sys.executable, "-m", "evalcore.publish", str(request)], start_new_session=True,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            code = child.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
            record_publication_cut_off(destination, record, command_start, publishing_started)
            print("Full report not finished by the cap; child killed, limit-only record kept.", flush=True)
            return destination
        if code == UNSAFE_EXIT:
            remove_destination(destination, results_dir)
            raise UnsafeReportError("result refused: credential or provider configuration detected")
        if code != 0:
            finish_fallback(destination, record, command_start, publishing_started)
            (destination / "summary.json").write_text(json.dumps(record, indent=2) + "\n")
            (destination / "report.md").write_text(minimal_markdown(record))
            print("Full report failed to render; limit-only record kept.", flush=True)
            return destination
        # Finalize the one field that depends on when the report is written, then move into place.
        elapsed = time.monotonic() - command_start
        staged = json.loads((staging / "summary.json").read_text())
        staged["campaign_wall_seconds"] = elapsed
        limit = (staged.get("budget") or {}).get("caps", {}).get("wall_seconds")
        if staged.get("budget") is not None:
            staged["budget"]["actual"]["wall_seconds"] = elapsed
            staged["budget"]["wall_within_cap"] = None if limit is None else elapsed <= limit
        (staging / "summary.json").write_text(json.dumps(staged, indent=2, ensure_ascii=False, allow_nan=False) + "\n")
        markdown = (staging / "report.md").read_text().replace(WALL_TOKEN, f"{elapsed:.1f}")
        (staging / "report.md").write_text(markdown)
        for file in sorted(staging.iterdir()):
            # The private work directory may be on another filesystem: copy beside the target, then replace atomically.
            partial = destination / ("." + file.name + ".partial")
            shutil.copyfile(file, partial)
            os.replace(partial, destination / file.name)
        (destination / "publication.json").write_text(json.dumps({
            "command_start_to_report_written_seconds": time.monotonic() - command_start,
            "publication_started_seconds": publishing_started - command_start,
            "cap_seconds": limit, "within_cap": None if limit is None else time.monotonic() - command_start <= limit}, indent=2) + "\n")
        return destination
    finally:
        shutil.rmtree(work, ignore_errors=True)


def child_main(request_path: str) -> int:
    """Render, scan and stage. Runs in its own process so a blocked renderer can be killed."""
    request = json.loads(Path(request_path).read_text())
    summary = request["summary"]
    delay = os.environ.get("EVALCORE_PROOF_RENDER_DELAY_SECONDS")  # proof hook: make rendering block
    if delay:
        time.sleep(float(delay))
    summary["campaign_wall_seconds"] = None
    json_text = json.dumps(summary, indent=2, ensure_ascii=False, allow_nan=False) + "\n"
    markdown = render_markdown(summary)
    try:
        scan_results((json_text, markdown), protected_values(None, [Path(p) for p in request["provider_config_paths"]]))
    except UnsafeReportError:
        return UNSAFE_EXIT
    staging = Path(request["staging"])
    (staging / "summary.json").write_text(json_text)
    (staging / "report.md").write_text(markdown)
    return 0


if __name__ == "__main__":
    raise SystemExit(child_main(sys.argv[1]))
