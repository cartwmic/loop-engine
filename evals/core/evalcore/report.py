"""Render first, scan every result byte, then publish the report pair."""
import json
import os
import re
import shutil
import tempfile
from collections.abc import Iterable, Mapping
from datetime import datetime, timezone
from pathlib import Path

CREDENTIAL_NAME = re.compile(r"(?:API_?KEY|TOKEN|SECRET|PASSWORD|PASSWD|CREDENTIAL|AUTHORIZATION|PRIVATE_KEY|ACCESS_KEY|(?:^|_)(?:PAT|AUTH|BEARER)(?:$|_))", re.I)
CONFIG_NAME = re.compile(r"(?:PROVIDER.*CONFIG|CONFIG.*PROVIDER)", re.I)
CONFIG_MARKERS = re.compile(r"\[\[?providers?(?:[.\]])|[\"'](?:api_key|apiKey|base_url|provider_config|resolved_provider_config)[\"']\s*:|\b(?:api_key|apiKey|base_url|provider_config|resolved_provider_config)\s*=", re.I)


# Minimum length of a config-derived string treated as secret. Environment
# credential values are always protected regardless of length.
MIN_SECRET_LENGTH = 16


class UnsafeReportError(ValueError):
    """A result matched protected material. Never include the matched text."""


def protected_values(environment: Mapping[str, str] | None = None, provider_config_paths: Iterable[Path] = ()) -> set[str]:
    env = os.environ if environment is None else environment
    protected = {value for name, value in env.items() if value and CREDENTIAL_NAME.search(name)}
    configs = [Path(p).read_text() for p in provider_config_paths]
    for name, value in env.items():
        if value and CONFIG_NAME.search(name):
            # Inline configs need not be legal filesystem path strings.
            try:
                is_file = "\n" not in value and Path(value).is_file()
            except (OSError, ValueError):
                is_file = False
            configs.append(Path(value).read_text() if is_file else value)
    for config in configs:
        if config.strip():
            protected.add(config.strip())
        # Catch copied fragments as well as whole resolved files. Never export these values.
        # Short fragments ("profile", "oauth", "email") are ordinary words, not secrets,
        # and would refuse every report; key/value markers are caught by CONFIG_MARKERS.
        protected.update(line.strip() for line in config.splitlines()
                         if len(line.strip()) >= MIN_SECRET_LENGTH and any(c.isalnum() for c in line)
                         and not line.lstrip().startswith("#"))
        try:
            parsed = json.loads(config)
        except ValueError:
            try:
                import tomllib
                parsed = tomllib.loads(config)
            except ValueError:
                parsed = None
        if parsed is not None:
            def strings(value):
                if isinstance(value, str) and len(value) >= MIN_SECRET_LENGTH:
                    protected.add(value)
                elif isinstance(value, dict):
                    for item in value.values():
                        strings(item)
                elif isinstance(value, list):
                    for item in value:
                        strings(item)
            strings(parsed)
    return protected


def scan_results(contents: Iterable[str], protected: Iterable[str]) -> None:
    # Scan both original and JSON-escaped values, including multiline/unicode secrets.
    needles = set()
    for value in protected:
        if value:
            needles.update((value, json.dumps(value, ensure_ascii=False)[1:-1], json.dumps(value)[1:-1]))
    def check(text):
        if CONFIG_MARKERS.search(text) or any(value in text for value in needles):
            raise UnsafeReportError("result refused: credential or provider configuration detected")
        # A scored field may itself contain serialized JSON. Inspect decoded strings
        # as well, so extra escape layers cannot hide a key or provider config.
        try:
            value = json.loads(text)
        except ValueError:
            return
        def walk(item):
            if isinstance(item, str):
                check(item)
            elif isinstance(item, dict):
                for key, nested in item.items():
                    check(key)
                    walk(nested)
            elif isinstance(item, list):
                for nested in item:
                    walk(nested)
        walk(value)
    for text in contents:
        check(text)


def render_budget(summary: dict) -> list[str]:
    """Tier, caps, actual spend and time, stops and the explicit cost verdict. Empty when nothing is budgeted."""
    budget = summary.get("budget")
    caps = (budget or {}).get("caps") or {}
    if not budget or not (budget.get("tier") or any(caps.get(k) is not None for k in ("wall_seconds", "usd", "tokens"))):
        return []
    actual = budget["actual"]
    wall = summary.get("campaign_wall_seconds")
    wall_text = "{CAMPAIGN_WALL_SECONDS}" if wall is None else f"{wall:.1f}"
    usd = "unknown (unpriced or unavailable)" if actual["usd"] is None else f"${actual['usd']:.4f}"
    lines = ["## Budget", ""]
    if budget.get("tier"):
        lines += [f"Tier: {budget['tier']}", ""]
    if budget.get("note"):
        lines += [budget["note"], ""]
    lines += [f"- Cost verdict: **{budget['cost_verdict']}**",
              f"- Estimated spend: {usd}; cap: {caps.get('usd')}; stop line: "
              + ("n/a" if caps.get("usd") is None else f"{caps['usd'] - caps['dollar_margin_usd']:.4f}"),
              f"- Tokens: {actual['tokens']}; cap: {caps.get('tokens')}",
              f"- Campaign wall seconds (command start to report written): {wall_text}; cap: {caps.get('wall_seconds')}",
              "- Dollar cap basis: " + budget["dollar_cap_basis"],
              "- Stops that fired: " + (", ".join(f"{e['reason']} at {e['elapsed_seconds']}s" for e in budget["stop_events"]) or "none"),
              "- Steps cut off: " + (", ".join(f"{c['step']} ({c['reason']}{', ' + c['sample_id'] if c.get('sample_id') else ''})"
                                               for c in budget["cut_off_steps"]) or "none"), ""]
    return lines


def render_markdown(summary: dict) -> str:
    lines = [f"# Campaign {summary['campaign']}", "", f"Target: {summary['target']}", "", *render_budget(summary),
             *([f"Projection cohort: {summary['projection_cohort']} (not pooled with other cohorts)", ""]
               if "projection_cohort" in summary else []),
             "Dollar figures are estimated from the campaign price table; null means unpriced.", "",
             "## Classification", summary["classification_rules"], ""]
    for key in ("settings", "pinned_models", "versions", "matched_assignment"):
        lines += [f"## {key.replace('_', ' ').title()}", "```json", json.dumps(summary[key], indent=2, ensure_ascii=False, allow_nan=False), "```", ""]
    for combo in summary["comparisons"]:
        lines += [f"## {combo['id']}", "", "### Core metrics", "```json",
                  json.dumps(combo["core"], indent=2, allow_nan=False), "```", "", "### Target metrics", "```json",
                  json.dumps(combo["target_metrics"], indent=2, ensure_ascii=False, allow_nan=False), "```", ""]
    if summary.get("execution_continuation"):
        change = summary["execution_continuation"]
        lines += ["## Authorized execution-limit continuation", "",
                  change["comparison_note"], "",
                  "Carried trials were not rescored or restarted. New assignments: "
                  + ", ".join(change["new_assignment_ids"]), "", "```json",
                  json.dumps(change, indent=2, ensure_ascii=False, allow_nan=False), "```", ""]
    if summary.get("resumed_trials"):
        lines += ["## Resumed trials (excluded from headline success rate and pass^k)", "```json",
                  json.dumps(summary["resumed_trials"], indent=2, ensure_ascii=False, allow_nan=False), "```", ""]
    if summary.get("carried_trials"):
        lines += ["## Carried trials", "```json", json.dumps(summary["carried_trials"], indent=2), "```", ""]
    lines += ["## Trials (including infrastructure and timeouts)", "```json",
              json.dumps(summary["trials"], indent=2, ensure_ascii=False, allow_nan=False), "```", ""]
    return "\n".join(lines)


def utc_timestamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")


def write_report(results_dir: Path, summary: dict, *, provider_config_paths: Iterable[Path] = (), environment: Mapping[str, str] | None = None, report_directory: Path | None = None) -> Path:
    """Publish a fresh run; report_directory groups multi-campaign demonstrations."""
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", summary["campaign"]):
        raise ValueError("invalid campaign directory name")
    json_text = json.dumps(summary, indent=2, ensure_ascii=False, allow_nan=False) + "\n"
    markdown = render_markdown(summary)
    scan_results((json_text, markdown), protected_values(environment, provider_config_paths))
    # No output directory, temporary file or report is created before the scan succeeds.
    parent = report_directory.parent if report_directory is not None else results_dir / summary["campaign"]
    parent.mkdir(parents=True, exist_ok=True)
    # Reserve exclusively, including when multiple processes finish together.
    while True:
        destination = report_directory if report_directory is not None else parent / utc_timestamp()
        try:
            destination.mkdir()
            break
        except FileExistsError:
            if report_directory is not None:
                raise
    staging = Path(tempfile.mkdtemp(prefix=".report-", dir=parent))
    try:
        (staging / "summary.json").write_text(json_text)
        (staging / "report.md").write_text(markdown)
        for file in staging.iterdir():
            file.rename(destination / file.name)
    except BaseException:
        shutil.rmtree(destination)
        raise
    finally:
        shutil.rmtree(staging)
    return destination
