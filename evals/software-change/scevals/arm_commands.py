"""evalctl subcommands for the arm comparison: grader-calibrate (G4), compare (G7), stage-ledger and findings-check (G10).

Each handler parses its arguments and calls the module that owns the behaviour. None launches a trial, a grader call
or a review: grader-calibrate calls the grader only through its calibration function, and the ledger only records.
"""
import argparse
import asyncio
import json
import sys
from pathlib import Path

from .cases import load_case
from .compare import compare
from .findings import FindingError, check_findings_file
from .grader_cli import calibrate
from .ledger import CLASS_CAPS, LedgerError, admit, init_ledger, record, status

CASE_CHECK_PREFIX = "CASE_CHECK "


def no_op_failing_from_receipt(receipt_path: Path, case) -> list[str]:
    """Requirements whose tagged hidden tests do not all pass on the untouched repository (case-check no-op counts)."""
    lines = [line.removeprefix(CASE_CHECK_PREFIX) for line in receipt_path.read_text().splitlines()
             if line.startswith(CASE_CHECK_PREFIX)]
    if len(lines) != 1:
        raise ValueError("case-check receipt needs exactly one CASE_CHECK line")
    receipt = json.loads(lines[0])
    if receipt.get("id") != case.id or receipt.get("version") != case.version:
        raise ValueError("case-check receipt is for a different case or case version")
    if receipt.get("valid") is not True:
        raise ValueError("case-check receipt is not VALID; calibrate only a valid case")
    requirements = receipt["no_op"]["requirements"]
    if set(requirements) != set(case.requirement_ids):
        raise ValueError("case-check receipt requirements differ from the case requirements")
    return [rid for rid in case.requirement_ids if requirements[rid]["passed"] < requirements[rid]["total"]]


def _refused(error: Exception) -> int:
    print(error, file=sys.stderr)
    return 2


def grader_calibrate(args) -> int:
    try:
        case = load_case(args.case)
        receipt = asyncio.run(calibrate(
            case_path=args.case, grader_model=args.grader_model, driver_model=args.driver_model,
            near_miss_workspace=args.near_miss_workspace, near_miss_requirement=args.near_miss_requirement,
            no_op_failing=no_op_failing_from_receipt(args.case_check_receipt, case),
            owner_labels=args.owner_labels, output=args.output))
    except ValueError as error:
        return _refused(error)
    print(f"Calibration mode: {receipt['mode']}")
    for reason in receipt["reasons"]:
        print(f"  reason: {reason}")
    print(f"Receipt written: {args.output}")
    return 0


def compare_summaries(args) -> int:
    try:
        destination = compare(args.summaries, args.out)
    except ValueError as error:
        return _refused(error)
    print(f"Report written: {destination}")
    return 0


def _ledger_action(action) -> int:
    """Print the ledger's JSON result, or refuse with its message. A refused action has written nothing."""
    try:
        result = action()
    except LedgerError as error:
        return _refused(error)
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    return 0


def ledger_init(args) -> int:
    return _ledger_action(lambda: init_ledger(args.ledger, args.stage_dollars, args.stage_hours))


def ledger_admit(args) -> int:
    return _ledger_action(lambda: admit(args.ledger, args.command_text, args.command_class, args.reason))


def ledger_record(args) -> int:
    return _ledger_action(lambda: record(args.ledger, args.entry_id, args.outcome, args.hours, args.dollars))


def ledger_status(args) -> int:
    return _ledger_action(lambda: status(args.ledger))


def findings_check(args) -> int:
    try:
        records = check_findings_file(args.findings)
    except FindingError as error:
        print(f"INVALID: {error}")
        return 1
    print(f"VALID: {len(records)} finding record(s)")
    return 0


def register_subcommands(subparsers: argparse._SubParsersAction):
    parser = subparsers.add_parser(
        "grader-calibrate", help="run outcome-grade-v1 controls and owner-label acceptance; write a normal or diagnostic-only receipt")
    parser.add_argument("case", type=Path, help="software-change case directory, normally the public example case")
    parser.add_argument("--grader-model", required=True, help="Inspect model ID of the grader; refused when its provider family matches the driver's")
    parser.add_argument("--driver-model", required=True, help="Inspect model ID of the driver whose runs the grader judges")
    parser.add_argument("--case-check-receipt", type=Path, required=True,
                        help="captured case-check output holding one CASE_CHECK line for this valid case")
    parser.add_argument("--near-miss-workspace", type=Path, required=True, help="workspace that must not satisfy the near-miss requirement")
    parser.add_argument("--near-miss-requirement", required=True, help="requirement id the near-miss workspace must not satisfy")
    parser.add_argument("--owner-labels", type=Path, help="owner-labelled workspaces (schema_version 1); without them the receipt is diagnostic-only")
    parser.add_argument("--output", type=Path, required=True, help="new receipt file; an existing file is never overwritten")
    parser.set_defaults(handler=grader_calibrate)

    parser = subparsers.add_parser("compare", help="paired report over single-trial campaign summaries, written to a new timestamped directory")
    parser.add_argument("summaries", type=Path, nargs="+", help="summary.json files of single-trial campaigns; never modified")
    parser.add_argument("--out", type=Path, required=True, help="directory that receives the new report pair")
    parser.set_defaults(handler=compare_summaries)

    ledger_parser = subparsers.add_parser(
        "stage-ledger", help="record every command of a stage against its allowance; launches nothing")
    ledger_commands = ledger_parser.add_subparsers(dest="ledger_command", required=True)
    init_parser = ledger_commands.add_parser("init", help="create a ledger with the stage dollar and hour caps")
    init_parser.add_argument("ledger", type=Path)
    init_parser.add_argument("--stage-dollars", type=float, required=True)
    init_parser.add_argument("--stage-hours", type=float, required=True)
    init_parser.set_defaults(handler=ledger_init)
    admit_parser = ledger_commands.add_parser(
        "admit", help="admit a command whose class cap fits the remaining allowance; prints the handed cap")
    admit_parser.add_argument("ledger", type=Path)
    admit_parser.add_argument("--command", dest="command_text", metavar="COMMAND", required=True,
                              help="the command the stage will run")
    admit_parser.add_argument("--class", dest="command_class", required=True, choices=sorted(CLASS_CAPS),
                              help="class whose cap the command is admitted against")
    admit_parser.add_argument("--reason", help="owner-approved reason; required for attempt 2 or later")
    admit_parser.set_defaults(handler=ledger_admit)
    record_parser = ledger_commands.add_parser("record", help="record an admitted command's outcome, hours and reported dollars")
    record_parser.add_argument("ledger", type=Path)
    record_parser.add_argument("entry_id", type=int)
    record_parser.add_argument("--outcome", required=True)
    record_parser.add_argument("--hours", type=float, required=True)
    record_parser.add_argument("--dollars", type=float, help="reported dollars; omit when unavailable")
    record_parser.set_defaults(handler=ledger_record)
    status_parser = ledger_commands.add_parser("status", help="print spent, reserved, remaining and overshoot")
    status_parser.add_argument("ledger", type=Path)
    status_parser.set_defaults(handler=ledger_status)

    parser = subparsers.add_parser("findings-check", help="validate a finding ledger file (format only; runs no review)")
    parser.add_argument("findings", type=Path, help="JSON array of finding records")
    parser.set_defaults(handler=findings_check)
