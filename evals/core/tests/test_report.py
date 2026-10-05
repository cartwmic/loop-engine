import json

import pytest

from evalcore.report import UnsafeReportError, protected_values, scan_results, write_report


def summary(value="safe"):
    return {"campaign": "safe", "target": "fake", "classification_rules": "fixed k",
            "settings": {}, "pinned_models": {}, "versions": {}, "matched_assignment": [],
            "comparisons": [{"id": "test", "core": {}, "target_metrics": {"score": value}}], "trials": []}


@pytest.mark.parametrize("secret", ["synthetic-only-test-value", 'quote"backslash\\\n雪', "x"])
def test_secret_scan_before_any_write(campaign_root, secret):
    results = campaign_root / "results"
    with pytest.raises(UnsafeReportError):
        write_report(results, summary(secret), environment={"ANY_API_KEY": secret})
    assert not results.exists()


@pytest.mark.parametrize("name", ["OPENROUTER_API_KEY", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN", "GITHUB_PAT", "AUTH", "MODEL_PASSWORD"])
def test_credential_environment_names(name):
    assert protected_values({name: "synthetic-protected", "PATH": "/safe/path"}) == {"synthetic-protected"}


def test_config_full_text_fragment_and_parsed_value(campaign_root):
    path = campaign_root / "providers.toml"
    content = '[providers.test]\nbase_url = "https://synthetic-provider.invalid/v1"\napi_key = "synthetic-config-key"\n'
    path.write_text(content)
    protected = protected_values({}, [path])
    for text in (content, 'base_url = "https://synthetic-provider.invalid/v1"', "synthetic-config-key"):
        with pytest.raises(UnsafeReportError):
            scan_results([json.dumps({"score": text})], protected)
    with pytest.raises(UnsafeReportError):
        write_report(campaign_root / "results", summary(content), provider_config_paths=[path], environment={})
    assert not (campaign_root / "results").exists()


def test_inline_config_and_structural_config_markers():
    config = '{"base_url": "https://synthetic-provider.invalid", "api_key": "synthetic-only-key"}'
    assert "synthetic-only-key" in protected_values({"MODEL_PROVIDER_CONFIG": config})
    for text in ('[providers.test]\nmodel = "test"', '{"api_key": "unlisted"}', '{"resolved_provider_config": {}}'):
        with pytest.raises(UnsafeReportError):
            scan_results([text], [])


def test_safe_pair_and_no_overwrite(campaign_root):
    output = write_report(campaign_root / "results", summary(), environment={})
    assert sorted(p.name for p in output.iterdir()) == ["report.md", "summary.json"]
    before = (output / "summary.json").read_text()
    second = write_report(campaign_root / "results", summary("changed"), environment={})
    assert second != output and second.parent == output.parent
    import re
    assert re.fullmatch(r"\d{8}T\d{6}\.\d{6}Z", output.name)
    assert json.loads((second / "summary.json").read_text())["comparisons"][0]["target_metrics"]["score"] == "changed"
    assert (output / "summary.json").read_text() == before
    with pytest.raises(FileExistsError):
        write_report(campaign_root / "results", summary("changed"), environment={}, report_directory=output)
    assert (output / "summary.json").read_text() == before


def test_nonfinite_target_metrics_refused(campaign_root):
    with pytest.raises(ValueError):
        write_report(campaign_root / "results", summary(float("nan")), environment={})
    assert not (campaign_root / "results").exists()


def test_escaped_serialized_field_cannot_hide_config_or_secret():
    blob = json.dumps({"api_key": "synthetic-unknown-secret"})
    with pytest.raises(UnsafeReportError):
        scan_results([json.dumps({"score": blob})], [])
    secret = 'synthetic-quote"slash\\newline\n雪'
    escaped = secret
    for _ in range(3):
        escaped = json.dumps(escaped)
    with pytest.raises(UnsafeReportError):
        scan_results([json.dumps({"score": escaped})], [secret])


def test_writer_rejects_directory_escape(campaign_root):
    data = summary()
    data["campaign"] = "../escape"
    with pytest.raises(ValueError):
        write_report(campaign_root / "results", data, environment={})
    assert not (campaign_root / "results").exists()


def test_short_config_words_do_not_refuse_reports(tmp_path):
    # Pi's auth file shape: ordinary words like "profile" and "oauth" must not
    # block a report whose settings mention "profile"; the token still does.
    auth = tmp_path / "auth.json"
    token = "synthetic-access-token-0123456789abcdef"
    auth.write_text(json.dumps({"openai-codex": {"type": "oauth", "access": token,
                                                  "scopes": ["openid", "profile", "email"]}}, indent=2))
    protected = protected_values({}, [auth])
    scan_results([json.dumps({"profile": "standard-14", "auth": "oauth email"})], protected)
    with pytest.raises(UnsafeReportError):
        scan_results([json.dumps({"leak": token})], protected)
