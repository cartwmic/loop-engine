"""Finding-ledger validator on real records: a complete finding is accepted; missing or unknown fields are refused."""
import json

import pytest

from scevals.findings import (
    DISPOSITIONS, FINDING_FIELDS, VERIFICATION_FIELDS, FindingError, check_findings, check_findings_file, validate_finding,
)


def complete_finding(**changes):
    record = {
        "finding_id": "F-1",
        "source": "trial T1-native-1 review round 1",
        "summary": "empty input returns None instead of raising ValueError",
        "verification": {"status": "reproduced", "method": "test", "evidence": "hidden test test_empty_input fails"},
        "material": True,
        "discovered_at": "2026-10-19T12:00:00+00:00",
        "already_known": False,
        "disposition": "fixed",
        "false_positive": False,
        "duplicate_of": None,
    }
    record.update(changes)
    return record


def with_verification(**changes):
    verification = dict(complete_finding()["verification"])
    verification.update(changes)
    return complete_finding(verification=verification)


def test_complete_finding_is_accepted_unchanged():
    record = complete_finding()
    assert validate_finding(record) == record


@pytest.mark.parametrize("name", sorted(FINDING_FIELDS))
def test_missing_finding_field_is_refused(name):
    record = complete_finding()
    del record[name]
    with pytest.raises(FindingError, match=f"missing field.*{name}"):
        validate_finding(record)


def test_unknown_finding_field_is_refused():
    with pytest.raises(FindingError, match="unknown field.*severity"):
        validate_finding(complete_finding(severity="high"))


@pytest.mark.parametrize("name", sorted(VERIFICATION_FIELDS))
def test_missing_verification_field_is_refused(name):
    verification = dict(complete_finding()["verification"])
    del verification[name]
    with pytest.raises(FindingError, match=f"verification is missing field.*{name}"):
        validate_finding(complete_finding(verification=verification))


def test_unknown_verification_field_is_refused():
    with pytest.raises(FindingError, match="verification has unknown field.*confidence"):
        validate_finding(with_verification(confidence=0.9))


@pytest.mark.parametrize("method", ["test", "grader"])
def test_reproduced_finding_names_its_verifying_method(method):
    assert validate_finding(with_verification(method=method))["verification"]["method"] == method


def test_reproduced_finding_without_a_method_is_refused():
    with pytest.raises(FindingError, match="verification method must be one of grader, test"):
        validate_finding(with_verification(method=None))


def test_unverified_finding_must_have_a_null_method():
    unverified = with_verification(status="unverified", method=None, evidence="not attempted in this review")
    assert validate_finding(unverified)["verification"]["status"] == "unverified"
    with pytest.raises(FindingError, match="must have verification method null"):
        validate_finding(with_verification(status="unverified", method="test", evidence="not attempted"))


@pytest.mark.parametrize("name, value", [("material", "yes"), ("already_known", 1), ("false_positive", None)])
def test_flags_must_be_true_or_false(name, value):
    with pytest.raises(FindingError, match=f"{name} must be true or false"):
        validate_finding(complete_finding(**{name: value}))


@pytest.mark.parametrize("disposition", ["merged", "Fixed", None, ["fixed"]])
def test_disposition_outside_the_closed_set_is_refused(disposition):
    assert "fixed" in DISPOSITIONS
    with pytest.raises(FindingError, match="disposition must be one of"):
        validate_finding(complete_finding(disposition=disposition))


@pytest.mark.parametrize("discovered_at", ["2026-10-19T12:00:00", "yesterday", None, 1760875200])
def test_discovery_time_must_be_a_timezone_aware_timestamp(discovered_at):
    with pytest.raises(FindingError, match="discovered_at must be an ISO 8601 timestamp with a timezone"):
        validate_finding(complete_finding(discovered_at=discovered_at))


@pytest.mark.parametrize("name", ["finding_id", "source", "summary", "duplicate_of"])
def test_identifying_text_must_be_non_empty(name):
    with pytest.raises(FindingError, match=f"{name} must be a non-empty string|duplicate_of must be a non-empty string"):
        validate_finding(complete_finding(**{name: "   "}))


def test_finding_cannot_duplicate_itself():
    with pytest.raises(FindingError, match="cannot be a duplicate of itself"):
        validate_finding(complete_finding(duplicate_of="F-1"))


def test_ledger_accepts_a_duplicate_that_names_an_earlier_finding(tmp_path):
    original = complete_finding()
    duplicate = complete_finding(finding_id="F-2", duplicate_of="F-1", disposition="open", false_positive=False)
    path = tmp_path / "findings.json"
    path.write_text(json.dumps([original, duplicate]))
    assert check_findings_file(path) == [original, duplicate]


def test_ledger_accepts_an_empty_list(tmp_path):
    path = tmp_path / "findings.json"
    path.write_text("[]")
    assert check_findings_file(path) == []


def test_ledger_refuses_a_record_that_fails_and_names_its_position():
    with pytest.raises(FindingError, match="record 2: finding is missing field"):
        check_findings([complete_finding(), {"finding_id": "F-2"}])


def test_ledger_refuses_repeated_finding_ids():
    with pytest.raises(FindingError, match="duplicate finding_id: F-1"):
        check_findings([complete_finding(), complete_finding()])


def test_ledger_refuses_a_duplicate_of_an_unknown_finding():
    with pytest.raises(FindingError, match="F-2 is a duplicate of unknown finding F-9"):
        check_findings([complete_finding(), complete_finding(finding_id="F-2", duplicate_of="F-9")])


def test_ledger_refuses_a_non_array_and_invalid_json(tmp_path):
    with pytest.raises(FindingError, match="must be a JSON array"):
        check_findings(complete_finding())
    path = tmp_path / "findings.json"
    path.write_text("[{")
    with pytest.raises(FindingError, match="not valid JSON"):
        check_findings_file(path)
