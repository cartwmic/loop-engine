"""evalctl and the target-owned subcommand registration contract."""
import time

COMMAND_START = time.monotonic()  # the campaign clock starts with the command, before the slow imports below

import argparse
import importlib
import json
import os
import sys
import tomllib
from datetime import datetime, timezone
from importlib.metadata import entry_points
from pathlib import Path

from pydantic import ValidationError

from .report import UnsafeReportError
from .runner import run_campaign
from .settings import CampaignSettings
from .target import OwnerApprovalRequired


def read_settings(path: str | None) -> dict:
    if path is None:
        return {}
    with Path(path).open("rb") as stream:
        return json.load(stream) if path.endswith(".json") else tomllib.load(stream)


def load_target(spec: str):
    module, separator, factory = spec.partition(":")
    if not separator:
        raise ValueError("target must name module:factory")
    return getattr(importlib.import_module(module), factory)()


def campaign(args):
    data = read_settings(args.settings)
    for key in ("target", "campaign", "preset", "case_count", "trials_per_case", "profile", "cases_path",
                "simulator_model", "judge_model", "idle_limit_seconds", "hard_limit_seconds"):
        value = getattr(args, key)
        if value is not None:
            data[key] = value
    if args.no_outer_deadlines:
        data["hard_limit_seconds"] = None
        data["idle_limit_seconds"] = None
    if args.model:
        data["models"] = args.model
        data.pop("combinations", None)
    if args.combination:
        data["combinations"] = [json.loads(value) for value in args.combination]
        data.pop("models", None)
    if "campaign" not in data:
        data["campaign"] = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    settings = CampaignSettings.model_validate(data)
    target = load_target(settings.target)
    config_paths = tuple(Path(p) for p in args.provider_config)
    if args.resume:
        from .continuation import resume_campaign
        destination = resume_campaign(target, settings, args.resume, resume_interrupted=args.resume_interrupted,
                                      provider_config_paths=config_paths, clock_start=COMMAND_START)
    else:
        if args.resume_interrupted:
            raise ValueError("--resume-interrupted requires --resume")
        destination = run_campaign(target, settings, provider_config_paths=config_paths, clock_start=COMMAND_START)
    print(f"Report written: {destination}")
    return 0


def build_parser(plugins: list[str] = ()) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="evalctl")
    parser.add_argument("--plugin", action="append", default=[], help="trusted module exporting register_subcommands(subparsers)")
    commands = parser.add_subparsers(dest="command", required=True)
    cmd = commands.add_parser("campaign", help="run a matched Inspect campaign")
    cmd.add_argument("--settings", help="campaign TOML or JSON (paths relative to invocation directory)")
    cmd.add_argument("--resume", type=Path, help="prior result or capture run directory; identical settings required")
    cmd.add_argument("--resume-interrupted", action="store_true", help="continue captured non-terminal infrastructure/harness stops instead of rerunning")
    cmd.add_argument("--target", help="trusted Python module:factory; default scevals.target:create_target")
    cmd.add_argument("--campaign")
    models = cmd.add_mutually_exclusive_group()
    models.add_argument("--model", action="append", help="Inspect model ID; repeat to compare models in all roles")
    models.add_argument("--combination", action="append", help='JSON object: id, driver, implementer, reviewer; repeat to compare combinations')
    cmd.add_argument("--preset", choices=("small", "medium"))
    cmd.add_argument("--case-count", type=int)
    cmd.add_argument("--trials-per-case", type=int)
    cmd.add_argument("--profile")
    cmd.add_argument("--cases-path")
    cmd.add_argument("--simulator-model")
    cmd.add_argument("--judge-model")
    cmd.add_argument("--idle-limit-seconds", type=int)
    cmd.add_argument("--hard-limit-seconds", type=int)
    cmd.add_argument("--no-outer-deadlines", action="store_true", help="disable both hard and idle trial deadlines (continuation changes require an explicit exception)")
    cmd.add_argument("--provider-config", action="append", default=[], help="host-only resolved provider config path to protect from export")
    cmd.set_defaults(handler=campaign)
    # Targets register case-check, image-check, ac5-demo, calibrate, check-results
    # here; core does not know their arguments or implementation.
    seen = set()
    for plugin in entry_points(group="loop_engine_evals.commands"):
        plugin.load()(commands)
        seen.add(plugin.value.split(":")[0])
    for module_name in plugins:
        if module_name not in seen:
            importlib.import_module(module_name).register_subcommands(commands)
            seen.add(module_name)
    return parser


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    bootstrap = argparse.ArgumentParser(add_help=False)
    bootstrap.add_argument("--plugin", action="append", default=[])
    bootstrap.add_argument("--target")
    bootstrap.add_argument("--settings")
    try:
        known, _ = bootstrap.parse_known_args(argv)
        plugins = [*filter(None, os.environ.get("EVALCTL_PLUGINS", "").split(",")), *known.plugin]
        target_spec = known.target or read_settings(known.settings).get("target")
        if target_spec:
            module = importlib.import_module(target_spec.split(":")[0])
            if hasattr(module, "register_subcommands"):
                plugins.append(module.__name__)
        args = build_parser(plugins).parse_args(argv)
        return args.handler(args) or 0
    except ValidationError:
        print("Settings invalid: supply models OR combinations, cases_path, simulator_model and judge_model; check positive counts and Inspect provider/model IDs. A dollar budget needs a price for every model in use.", file=sys.stderr)
        return 2
    except OwnerApprovalRequired:
        print("Owner-pending: replace required model IDs with owner-chosen Inspect models and obtain paid-call approval before launch; no trial started.", file=sys.stderr)
        return 2
    except UnsafeReportError:
        print("Result refused: credential or provider configuration detected; no report written.", file=sys.stderr)
        return 2
    except Exception as error:
        # Exception messages may contain resolved configs, keys or private case text.
        print(f"Command failed ({type(error).__name__}); no sensitive error text exported.", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
