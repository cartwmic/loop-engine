"""Projection-cohort labels: which commission projection produced a run, and refusal to pool across cohorts.

A label lives in settings.target_options["projection_label"]. Settings without it are the explicit
"baseline" cohort (records produced before any commission projection label existed). Labels are
computed from the candidate bytes; existing results, summaries and labels are never rewritten.
"""
import hashlib
import json
import re
from pathlib import Path

LABEL_KEY = "projection_label"
BASELINE = "baseline"
LABEL_FIELDS = ("commission_projection", "product_tree_sha256", "patch_sha256", "skill_md_sha256", "binary_identity")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
PROJECTION = re.compile(r"^[a-z][a-z0-9_]*/v[0-9]+$")


def validate_label(label) -> dict:
    """Strict: exactly the five fields, each a non-empty string; hashes are lowercase hex SHA-256."""
    if not isinstance(label, dict) or set(label) != set(LABEL_FIELDS):
        raise ValueError(f"projection_label must have exactly the fields {', '.join(LABEL_FIELDS)}")
    if any(not isinstance(label[k], str) or not label[k].strip() for k in LABEL_FIELDS):
        raise ValueError("projection_label fields must be non-empty strings")
    if not PROJECTION.fullmatch(label["commission_projection"]):
        raise ValueError("projection_label.commission_projection is not a projection version (name/vN)")
    for key in ("product_tree_sha256", "patch_sha256", "skill_md_sha256"):
        if not SHA256.fullmatch(label[key]):
            raise ValueError(f"projection_label.{key} is not a lowercase hex SHA-256")
    return label


def tree_sha256(src) -> str:
    """Same algorithm as the candidate launcher: sorted relative paths and file digests, .git and target excluded."""
    src = Path(src)
    digest = hashlib.sha256()
    for path in sorted(src.rglob("*")):
        if path.is_file() and "target" not in path.parts and ".git" not in path.parts:
            digest.update(str(path.relative_to(src)).encode())
            digest.update(hashlib.sha256(path.read_bytes()).digest())
    return digest.hexdigest()


def file_sha256(path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def label_from_candidate(src, patch, *, commission_projection: str, binary_identity: str,
                         skill="crates/software-change-provider/skills/using-software-change-provider/SKILL.md") -> dict:
    """Compute a label from the actual candidate tree, patch file and SKILL.md bytes; nothing is taken on trust."""
    src = Path(src)
    return validate_label({
        "commission_projection": commission_projection,
        "product_tree_sha256": tree_sha256(src),
        "patch_sha256": file_sha256(patch),
        "skill_md_sha256": file_sha256(src / skill),
        "binary_identity": binary_identity,
    })


def label_of(settings) -> dict | None:
    """The validated label of recorded settings (dict) or CampaignSettings, or None for the baseline cohort."""
    options = settings.get("target_options", {}) if isinstance(settings, dict) else settings.target_options
    if LABEL_KEY not in (options or {}):
        return None
    return validate_label(options[LABEL_KEY])


def cohort(settings) -> str:
    label = label_of(settings)
    if label is None:
        return BASELINE
    canonical = json.dumps(label, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    return f"{label['commission_projection']}@{hashlib.sha256(canonical).hexdigest()[:16]}"


def check_same_cohort(original, current, *, action: str) -> str:
    """Refuse to combine records from different projection cohorts (baseline is a cohort of its own)."""
    before, after = cohort(original), cohort(current)
    if before != after:
        raise ValueError(f"{action} refused: projection cohort {before} differs from {after}; cohorts are not pooled")
    return after
