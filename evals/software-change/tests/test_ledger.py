"""Stage ledger on real JSON files: admission, refusal, the handed cap, overshoot and reruns. No command is launched."""
from datetime import datetime, timezone
import json

import pytest

from scevals.ledger import CLASS_CAPS, LedgerError, admit, init_ledger, record, status

NOW = datetime(2026, 10, 19, 12, 0, tzinfo=timezone.utc)


def stage(tmp_path, dollars, hours):
    path = tmp_path / "stage.json"
    init_ledger(path, stage_dollars=dollars, stage_hours=hours)
    return path


def test_init_starts_with_the_full_allowance_and_no_entries(tmp_path):
    path = stage(tmp_path, dollars=400, hours=40)
    summary = status(path)
    assert summary["remaining"] == {"dollars": 400.0, "hours": 40.0}
    assert summary["spent"] == {"dollars": 0.0, "hours": 0.0}
    assert summary["open_entries"] == []
    assert summary["overshoot"] == {"dollars": 0.0, "hours": 0.0}
    assert json.loads(path.read_text())["entries"] == []


def test_init_refuses_an_existing_ledger_and_non_positive_caps(tmp_path):
    path = stage(tmp_path, dollars=400, hours=40)
    with pytest.raises(LedgerError, match="already exists"):
        init_ledger(path, stage_dollars=400, stage_hours=40)
    with pytest.raises(LedgerError, match="greater than 0"):
        init_ledger(tmp_path / "zero.json", stage_dollars=0, stage_hours=40)


def test_admit_hands_the_class_cap_not_the_whole_allowance(tmp_path):
    path = stage(tmp_path, dollars=400, hours=40)
    entry = admit(path, "T1 native block-1", "T1", now=NOW)
    assert entry["id"] == 1 and entry["attempt"] == 1 and entry["class"] == "T1"
    assert entry["handed_cap"] == {"dollars": 12.0, "hours": 1.25} == CLASS_CAPS["T1"]
    assert entry["start"] == "2026-10-19T12:00:00+00:00" and entry["end"] is None
    assert status(path)["reserved"] == {"dollars": 12.0, "hours": 1.25}


def test_admit_accepts_remaining_exactly_equal_to_the_class_cap(tmp_path):
    path = stage(tmp_path, dollars=12, hours=1.25)
    entry = admit(path, "T1 smoke", "T1", now=NOW)
    assert entry["handed_cap"] == {"dollars": 12.0, "hours": 1.25}


def test_admit_refuses_when_remaining_dollars_are_below_the_class_cap_and_writes_nothing(tmp_path):
    path = stage(tmp_path, dollars=20, hours=40)
    first = admit(path, "T1 trial", "T1", now=NOW)
    record(path, first["id"], outcome="success", hours=0.5, dollars=10.0, now=NOW)
    before = path.read_bytes()
    with pytest.raises(LedgerError, match="refused: remaining \\$10.00"):
        admit(path, "T1 next", "T1", now=NOW)
    assert path.read_bytes() == before


def test_admit_refuses_when_remaining_hours_are_below_the_class_cap(tmp_path):
    path = stage(tmp_path, dollars=400, hours=4)
    first = admit(path, "T1 trial", "T1", now=NOW)
    record(path, first["id"], outcome="failure", hours=3.0, dollars=1.0, now=NOW)
    with pytest.raises(LedgerError, match="refused: remaining \\$399.00 and 1.00 h"):
        admit(path, "T1 next", "T1", now=NOW)


def test_open_entry_reserves_its_cap_until_it_is_recorded(tmp_path):
    path = stage(tmp_path, dollars=20, hours=40)
    first = admit(path, "T1 a", "T1", now=NOW)
    with pytest.raises(LedgerError, match="refused"):
        admit(path, "T1 b", "T1", now=NOW)
    record(path, first["id"], outcome="success", hours=0.4, dollars=3.0, now=NOW)
    second = admit(path, "T1 b", "T1", now=NOW)
    assert second["id"] == 2


def test_overshoot_is_reported_by_record_and_status_not_prevented(tmp_path):
    path = stage(tmp_path, dollars=12, hours=10)
    entry = admit(path, "T1 trial", "T1", now=NOW)
    result = record(path, entry["id"], outcome="limit", hours=1.0, dollars=13.5, now=NOW)
    assert result["entry"]["dollars"] == 13.5 and result["entry"]["end"] == "2026-10-19T12:00:00+00:00"
    assert result["status"]["overshoot"] == {"dollars": 1.5, "hours": 0.0}
    assert status(path)["overshoot"] == {"dollars": 1.5, "hours": 0.0}
    with pytest.raises(LedgerError, match="refused"):
        admit(path, "T1 next", "T1", now=NOW)


def test_unpriced_command_stays_null_and_blocks_admission_until_resolved(tmp_path):
    path = stage(tmp_path, dollars=400, hours=40)
    entry = admit(path, "T1 trial", "T1", now=NOW)
    result = record(path, entry["id"], outcome="success", hours=0.5, dollars=None, now=NOW)
    assert result["entry"]["dollars"] is None
    assert result["status"]["unpriced_commands"] == 1
    assert result["status"]["remaining"]["dollars"] is None
    with pytest.raises(LedgerError, match="no reported dollars"):
        admit(path, "T1 next", "T1", now=NOW)


def test_rerun_is_attempt_two_only_with_a_reason_and_keeps_attempt_one(tmp_path):
    path = stage(tmp_path, dollars=400, hours=40)
    first = admit(path, "T1 native block-1", "T1", now=NOW)
    record(path, first["id"], outcome="failure", hours=0.2, dollars=1.0, now=NOW)
    with pytest.raises(LedgerError, match="owner-approved"):
        admit(path, "T1 native block-1", "T1", now=NOW)
    rerun = admit(path, "T1 native block-1", "T1", reason="owner approved rerun", now=NOW)
    assert rerun["attempt"] == 2 and rerun["reason"] == "owner approved rerun"
    entries = json.loads(path.read_text())["entries"]
    assert [(item["attempt"], item["outcome"]) for item in entries] == [(1, "failure"), (2, None)]


def test_command_with_an_open_entry_cannot_be_admitted_again(tmp_path):
    path = stage(tmp_path, dollars=400, hours=40)
    admit(path, "grade T1 native", "T1", now=NOW)
    with pytest.raises(LedgerError, match="open entry"):
        admit(path, "grade T1 native", "T1", reason="again", now=NOW)


def test_unknown_class_and_missing_ledger_are_refused(tmp_path):
    path = stage(tmp_path, dollars=400, hours=40)
    with pytest.raises(LedgerError, match="unknown command class 'T4'"):
        admit(path, "T4 trial", "T4", now=NOW)
    with pytest.raises(LedgerError, match="does not exist"):
        admit(tmp_path / "missing.json", "T1 trial", "T1", now=NOW)


@pytest.mark.parametrize("hours, dollars, outcome, match", [
    (0.5, 1.0, "", "outcome"),
    (-0.5, 1.0, "success", "hours"),
    (True, 1.0, "success", "hours"),
    (0.5, -1.0, "success", "dollars"),
    (float("nan"), 1.0, "success", "hours"),
])
def test_record_refuses_invalid_values(tmp_path, hours, dollars, outcome, match):
    path = stage(tmp_path, dollars=400, hours=40)
    entry = admit(path, "T1 trial", "T1", now=NOW)
    with pytest.raises(LedgerError, match=match):
        record(path, entry["id"], outcome=outcome, hours=hours, dollars=dollars, now=NOW)
    assert status(path)["open_entries"] == [entry["id"]]


def test_record_refuses_unknown_and_already_recorded_entries(tmp_path):
    path = stage(tmp_path, dollars=400, hours=40)
    with pytest.raises(LedgerError, match="no ledger entry 7"):
        record(path, 7, outcome="success", hours=0.1, dollars=1.0, now=NOW)
    entry = admit(path, "T1 trial", "T1", now=NOW)
    record(path, entry["id"], outcome="success", hours=0.1, dollars=1.0, now=NOW)
    with pytest.raises(LedgerError, match="already recorded"):
        record(path, entry["id"], outcome="success", hours=0.1, dollars=1.0, now=NOW)
