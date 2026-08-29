#!/usr/bin/env python3
"""Check the repository's pinned cargo-nextest configuration and executable."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from nextest_policy import CONFIG_PATH, PolicyError, check_installed_version, self_test, validate_config


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="exercise policy rejection cases")
    parser.add_argument("--config", type=Path, default=CONFIG_PATH)
    parser.add_argument(
        "--config-only",
        action="store_true",
        help="validate the checked-in config without invoking cargo-nextest",
    )
    args = parser.parse_args()

    try:
        if args.self_test:
            self_test()
            print("nextest policy self-test passed")
            return 0
        config = validate_config(args.config)
        report = {"config": config}
        if not args.config_only:
            report["version"] = check_installed_version()
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    except PolicyError as error:
        print(f"nextest policy failed closed: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
