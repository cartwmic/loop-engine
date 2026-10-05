"""Public CLI secrecy checks; synthetic credentials only, no paid models."""
import json
import os
from pathlib import Path
import shutil
import subprocess

import pytest

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parents[1]
CASE = ROOT / "example-case"


def invoke(directory, cases=CASE, extra_env=None, extra_args=()):
    return subprocess.run(["uv", "run", "--project", "evals", "evalctl", "check-results",
                           str(directory), "--cases", str(cases), *extra_args],
                          cwd=REPO, env={**os.environ, **(extra_env or {})},
                          capture_output=True, text=True, timeout=60)


def results(tmp_path, text='{"safe": true}'):
    directory = tmp_path / "results"
    directory.mkdir()
    (directory / "report.md").write_text(text)
    return directory


def test_safe_result_and_all_cases(tmp_path):
    cases = tmp_path / "cases"
    shutil.copytree(CASE, cases / "first")
    second = cases / "second"
    shutil.copytree(CASE, second)
    (second / "hidden/brief.md").write_text((second / "hidden/brief.md").read_text() + "\nDistinct private sentence only in case two.\n")
    directory = results(tmp_path)
    result = invoke(directory, cases)
    assert result.returncode == 0 and "against 2 cases" in result.stdout
    (directory / "other.bin").write_bytes(b"Distinct private sentence only in case two.")
    result = invoke(directory, cases)
    assert result.returncode == 1 and "FAIL" in result.stdout
    assert "Distinct private sentence" not in result.stdout + result.stderr


@pytest.mark.parametrize("source", ["hidden/brief.md", "hidden/tests/test_labels.py", "hidden/reference/labels.py"])
def test_hidden_whole_files_and_lines_are_rejected(tmp_path, source):
    text = (CASE / source).read_text()
    directory = results(tmp_path, text)
    assert invoke(directory).returncode == 1
    line = next(line.strip() for line in text.splitlines() if "self.assertEqual" in line or "return " in line or "Keys must" in line)
    # Nested JSON encoding exercises escape layers, not merely a raw substring.
    (directory / "report.md").write_text(json.dumps({"field": json.dumps({"nested": line})}))
    result = invoke(directory)
    assert result.returncode == 1
    assert line not in result.stdout + result.stderr


@pytest.mark.parametrize("text,env", [
    ('synthetic-result-key', {"EVAL_TEST_API_KEY": "synthetic-result-key"}),
    ('api_key = "synthetic-unlisted-key"', {}),
    ('{"api_key": "synthetic-unlisted-key"}', {}),
    ('[providers.synthetic]\ncommand = "synthetic-provider"', {}),
    ('https://synthetic-resolved.invalid', {"TEST_PROVIDER_CONFIG": '{"base_url":"https://synthetic-resolved.invalid"}'}),
])
def test_credentials_and_provider_config_without_echo(tmp_path, text, env):
    result = invoke(results(tmp_path, text), extra_env=env)
    assert result.returncode == 1
    assert text not in result.stdout + result.stderr


def test_explicit_provider_config_and_binary_hidden_content(tmp_path):
    directory = results(tmp_path, "synthetic-provider-command")
    config = tmp_path / "provider.toml"
    config.write_text('[providers.test]\ncommand = "synthetic-provider-command"\n')
    assert invoke(directory, extra_args=("--provider-config", str(config))).returncode == 1
    case = tmp_path / "case"
    shutil.copytree(CASE, case)
    (case / "hidden/reference/blob.bin").write_bytes(b"\xff\xfeprivate-binary-payload")
    (case / "case.toml").write_text((case / "case.toml").read_text().replace('["labels.py"]', '["labels.py", "blob.bin"]'))
    (directory / "report.md").write_bytes(b"\xff\xfeprivate-binary-payload")
    assert invoke(directory, case).returncode == 1


def test_empty_missing_and_symlink_inputs_fail_closed(tmp_path):
    directory = tmp_path / "results"
    directory.mkdir()
    assert invoke(directory).returncode == 2
    (directory / "report.md").write_text("safe")
    assert invoke(directory, tmp_path / "missing").returncode == 2
    (directory / "link").symlink_to(directory / "report.md")
    assert invoke(directory).returncode == 2
