"""Owner-approved (2026-10-17 05:28 PDT) label gate for eval-only F1 builds.

A trial image built from a revision listed in projection-label-required.txt is a projection binary: every measured
run with it must carry a valid target_options.projection_label whose binary_identity is exactly the trial image ID.
Unlabelled or mismatched runs are refused before any trial (invalid, never baseline). Other images are unaffected.
"""
from pathlib import Path

from evalcore.provenance import label_of

REQUIRED_FILE = Path(__file__).resolve().parents[1] / "projection-label-required.txt"


def required_revisions(path=REQUIRED_FILE) -> frozenset:
    if not path.is_file():
        return frozenset()
    return frozenset(line.split("#", 1)[0].strip() for line in path.read_text().splitlines() if line.split("#", 1)[0].strip())


def check_projection_label_gate(settings, identity, required=None) -> None:
    required = required_revisions() if required is None else required
    revision = ((identity.get("Config") or {}).get("Labels") or {}).get("org.opencontainers.image.revision")
    if revision not in required:
        return
    label = label_of(settings)
    if label is None:
        raise ValueError("projection binary image requires target_options.projection_label; unlabelled runs are invalid")
    if label["binary_identity"] != identity.get("Id"):
        raise ValueError("projection_label.binary_identity must equal the trial image ID")
