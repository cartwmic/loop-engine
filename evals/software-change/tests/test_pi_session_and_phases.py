"""Session hook golden argv and Pi session phase attribution. Real scripts run; only the external `pi` binary is a stub.

The fixtures are real Pi 1.2.0 session records from the arms image, produced by a scripted no-network backend:
the driver read a Superpowers SKILL.md and ran one bash call; the implementer ran one bash call.
"""
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from scevals import phases

IMAGE_DIR = Path(__file__).resolve().parents[1] / "image"
FIXTURES = Path(__file__).resolve().parent / "fixtures" / "pi-session"
DRIVER = FIXTURES / "driver-superpowers-skill.jsonl"
WORKER = FIXTURES / "worker-implementer.jsonl"

DRIVER_ARGV = ["--print", "--no-session", "--no-extensions", "--no-skills", "--no-prompt-templates", "--no-themes",
               "--provider", "inspect", "--model", "role-driver", "--models", "role-driver",
               "--skill", "/opt/sc-evals/skills/using-loop-engine",
               "--skill", "/opt/sc-evals/skills/using-software-change-provider", "--", "Run X"]
WORKER_ARGV = ["--print", "--no-session", "--no-extensions", "--no-skills", "--no-prompt-templates", "--no-themes",
               "--provider", "inspect", "--model", "role-implementer", "--models", "role-implementer",
               "--skill", "/opt/sc-evals/skills/using-loop-engine",
               "--skill", "/opt/sc-evals/skills/using-software-change-provider",
               "--append-system-prompt",
               "You are a delegated worker. Return evidence to the driver; do not mutate Loop Engine workflow state. "
               "Implement the assigned scoped change.", "--", "Impl Y"]

def row(phase, role, seconds, turns=0, input_tokens=0, output_tokens=0):
    return {"phase": phase, "role": role, "turns": turns, "input_tokens": input_tokens,
            "output_tokens": output_tokens, "cache_read_tokens": 0, "cache_write_tokens": 0,
            "total_tokens": input_tokens + output_tokens, "seconds": seconds}


# Derived by hand from the fixture timestamps and usage: the driver's skill phase is the read, then every later
# gap; the first gap (user prompt to first reply) is unmarked. Totals are per (phase, role).
DRIVER_SKILL = row("skill:brainstorming", "role-driver", 0.045, turns=3, input_tokens=33, output_tokens=15)
DRIVER_UNMARKED = row("unmarked", "role-driver", 0.015)
WORKER_UNMARKED = row("unmarked", "role-implementer", 0.045, turns=2, input_tokens=22, output_tokens=10)
EXPECTED_ROWS = [DRIVER_SKILL, DRIVER_UNMARKED, WORKER_UNMARKED]


def run_role(tmp_path, name, prompt, session_dir=None):
    """Run the real role script under the name it is installed as, with a stub `pi` that records its argv."""
    bin_dir = tmp_path / "bin"
    stub_dir = tmp_path / "stub"
    bin_dir.mkdir(exist_ok=True)
    stub_dir.mkdir(exist_ok=True)
    (bin_dir / name).symlink_to(IMAGE_DIR / "pi-role.py")
    stub = stub_dir / "pi"
    stub.write_text(f"#!{sys.executable}\nimport json, os, sys\n"
                    "open(os.environ['SC_TEST_RECORD'], 'w').write(json.dumps(sys.argv[1:]))\n")
    stub.chmod(0o755)
    record = tmp_path / "argv.json"
    env = {"PATH": f"{stub_dir}{os.pathsep}{os.environ['PATH']}", "SC_TEST_RECORD": str(record)}
    if session_dir is not None:
        env["SC_PI_SESSION_DIR"] = session_dir
    # The image chmods the script at build time. Running it through the interpreter keeps the tracked mode
    # and makes argv[0] the role name.
    result = subprocess.run([sys.executable, str(bin_dir / name), prompt], env=env, capture_output=True, text=True,
                            timeout=60)
    assert result.returncode == 0, result.stderr
    return json.loads(record.read_text())


def test_unset_session_dir_keeps_todays_driver_argv(tmp_path):
    assert run_role(tmp_path, "sc-driver", "Run X") == DRIVER_ARGV


def test_unset_session_dir_keeps_todays_worker_argv(tmp_path):
    assert run_role(tmp_path, "sc-implement", "Impl Y") == WORKER_ARGV


def test_empty_session_dir_is_inert(tmp_path):
    assert run_role(tmp_path, "sc-driver", "Run X", session_dir="") == DRIVER_ARGV


def test_session_dir_keeps_the_record_in_place_of_no_session(tmp_path):
    argv = run_role(tmp_path, "sc-driver", "Run X", session_dir="/workspace/trial/pi-sessions")
    assert argv == ["--print", "--session-dir", "/workspace/trial/pi-sessions", *DRIVER_ARGV[2:]]
    assert "--no-session" not in argv


def assert_rows_equal(rows, expected):
    assert [{**row, "seconds": round(row["seconds"], 3)} for row in rows] == expected


def test_fixture_splits_tokens_and_time_by_skill_and_role():
    record = phases.attribute(DRIVER).to_record()
    assert record["available"] is True and record["reason"] is None
    assert_rows_equal(record["phases"], [DRIVER_SKILL, DRIVER_UNMARKED])


def test_directory_sums_every_session_record(tmp_path):
    (tmp_path / "driver.jsonl").write_text(DRIVER.read_text())
    (tmp_path / "worker.jsonl").write_text(WORKER.read_text())
    record = phases.attribute(tmp_path).to_record()
    assert record["available"] is True
    assert_rows_equal(record["phases"], EXPECTED_ROWS)


def test_cli_prints_the_attribution_of_the_fixture(capsys):
    assert phases.main([str(DRIVER)]) == 0
    record = json.loads(capsys.readouterr().out)
    assert record["available"] is True
    assert_rows_equal(record["phases"], [DRIVER_SKILL, DRIVER_UNMARKED])


@pytest.mark.parametrize("prompt, expected", [
    ("/opsx-propose add a thing", [row("command:opsx-propose", "role-driver", 0.015), DRIVER_SKILL]),
    ("/skill:brainstorming add a thing", [row("skill:brainstorming", "role-driver", 0.060, 3, 33, 15)]),
    ("/workspace/public/repo is the repo", [DRIVER_SKILL, DRIVER_UNMARKED]),
])
def test_user_message_marker_starts_its_phase(tmp_path, prompt, expected):
    def prompt_text(lines):
        set_message(lines, "user", lambda message: message.__setitem__("content", [{"type": "text", "text": prompt}]))
    record = phases.attribute(edited(tmp_path, "driver.jsonl", prompt_text)).to_record()
    assert_rows_equal(record["phases"], expected)


def test_read_of_a_file_that_is_not_a_skill_starts_no_phase(tmp_path):
    def read_readme(lines):
        def point_at_readme(message):
            for block in message["content"]:
                if block.get("type") == "toolCall" and block.get("name") == "read":
                    block["arguments"]["path"] = "/opt/sc-evals/arms/superpowers/skills/brainstorming/README.md"
        set_message(lines, "assistant", point_at_readme)
    record = phases.attribute(edited(tmp_path, "driver.jsonl", read_readme)).to_record()
    assert_rows_equal(record["phases"], [row("unmarked", "role-driver", 0.060, 3, 33, 15)])


def assert_unavailable(attribution, reason_prefix):
    record = attribution.to_record()
    assert record["available"] is False and record["phases"] is None, record
    assert record["reason"].startswith(reason_prefix), record["reason"]


def test_missing_record_is_unavailable_not_zero(tmp_path):
    assert_unavailable(phases.attribute(tmp_path / "absent.jsonl"), "session record missing")


def test_directory_without_records_is_unavailable_not_zero(tmp_path):
    assert_unavailable(phases.attribute(tmp_path), "session record missing")


def lines_of(path):
    return path.read_text().splitlines()


def edited(tmp_path, name, mutate):
    lines = lines_of(DRIVER)
    mutate(lines)
    path = tmp_path / name
    path.write_text("\n".join(lines) + "\n")
    return path


def set_message(lines, role, change):
    index = next(i for i, line in enumerate(lines) if json.loads(line).get("message", {}).get("role") == role)
    entry = json.loads(lines[index])
    change(entry["message"])
    lines[index] = json.dumps(entry)


def drop_usage(message):
    del message["usage"]


def drop_timestamp(message):
    del message["timestamp"]


def rewind_timestamp(message):
    message["timestamp"] = 1


UNPARSEABLE_CASES = {
    "truncated line": lambda lines: lines.__setitem__(1, '{"type":"model_change"'),
    "no session header": lambda lines: lines.pop(0),
    "assistant without usage": lambda lines: set_message(lines, "assistant", drop_usage),
    "message without timestamp": lambda lines: set_message(lines, "user", drop_timestamp),
    "timestamp going back": lambda lines: set_message(lines, "toolResult", rewind_timestamp),
}


@pytest.mark.parametrize("case", sorted(UNPARSEABLE_CASES))
def test_unparseable_record_is_unavailable_not_zero(tmp_path, case):
    path = edited(tmp_path, "driver.jsonl", UNPARSEABLE_CASES[case])
    assert_unavailable(phases.attribute(path), "unparseable session record driver.jsonl")


def test_one_unparseable_file_makes_the_directory_unavailable(tmp_path):
    (tmp_path / "worker.jsonl").write_text(WORKER.read_text())
    (tmp_path / "driver.jsonl").write_text("not a session\n")
    assert_unavailable(phases.attribute(tmp_path), "unparseable session record driver.jsonl")
