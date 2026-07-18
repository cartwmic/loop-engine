#!/usr/bin/env python3
"""Build semantic-judge v1 requests from exact git revisions or staged index trees."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
REPO_ROOT = ROOT.parents[2]

FOUNDATION_PARENT_REVISION = "7552af5968b4a2c10aefd01fbfa6c351817e1b8b"
FOUNDATION_SEED_ID = "foundation-seed"
FOUNDATION_SEED_SHA256 = "3f1bd3489401ca6114ac1ef756ad4e87798a2d1ed3973c16625fd87167c1b3cd"
# Non-authoritative bundled reference only; authority comes from foundation Git blobs.
FOUNDATION_SEED_FROZEN_PATH = ROOT / "frozen" / "foundation-seed.v1.md"
MANIFEST_PATH = "quality/rubrics/manifest.json"
RUBRICS_DIR = "quality/rubrics"

# Frozen source path list: (repo path, section header, rubric header, blob sha256).
FOUNDATION_SEED_SOURCES: tuple[tuple[str, str, str, str], ...] = (
    (
        "docs/invariants.md",
        "### I47. Every commit is documentation-coherent",
        "## docs/invariants.md — I47",
        "8034714761107b669b5e5c9ab2941d257b5a69e562221d7e4dbb58db06b82b28",
    ),
    (
        "docs/testing.md",
        "## Git enforcement direction",
        "## docs/testing.md — Git enforcement direction",
        "204ccaab4a5f44f578f256b4b5dc4ba851febf0155ce8bc87c8c267a0d3a4037",
    ),
    (
        "docs/tenets.md",
        "## 27. Documentation evolves with every commit",
        "## docs/tenets.md — 27. Documentation evolves with every commit",
        "f2cb60c8cd68087b94ca284b901a36909f74367f8378d01885275ad341503fe4",
    ),
    (
        "docs/architecture.md",
        "## Composition and enforcement",
        "## docs/architecture.md — Composition and enforcement",
        "6bea0ef07491ceaa68158f90ce1162bf9778ae40560fcc18cc32baa187420633",
    ),
)


class RequestBuilderError(Exception):
    """Raised when request material cannot be built deterministically."""


def git(repo_root: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=repo_root, text=True)


def git_run(repo_root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=repo_root, capture_output=True, text=True, check=False)


def git_show_revision(repo_root: Path, revision: str, path: str) -> str | None:
    completed = git_run(repo_root, "show", f"{revision}:{path}")
    if completed.returncode != 0:
        return None
    return completed.stdout


def git_show_index(repo_root: Path, path: str) -> str | None:
    completed = git_run(repo_root, "show", f":{path}")
    if completed.returncode != 0:
        return None
    return completed.stdout


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def extract_markdown_section(text: str, header: str) -> str:
    lines = text.splitlines(keepends=True)
    start: int | None = None
    header_level = len(header) - len(header.lstrip("#"))
    for index, line in enumerate(lines):
        if line.rstrip("\n") == header:
            start = index + 1
            break
    if start is None:
        raise RequestBuilderError(f"foundation source section not found: {header!r}")

    body: list[str] = []
    for line in lines[start:]:
        stripped = line.rstrip("\n")
        if stripped.startswith("#"):
            level = len(stripped) - len(stripped.lstrip("#"))
            if level <= header_level:
                break
        body.append(line)
    return "".join(body).strip("\n")


def load_foundation_seed_source_blob(
    git_repo_root: Path, path: str, expected_digest: str
) -> str:
    content = git_show_revision(git_repo_root, FOUNDATION_PARENT_REVISION, path)
    if content is None:
        raise RequestBuilderError(
            f"foundation revision {FOUNDATION_PARENT_REVISION} missing source at {path}"
        )
    actual_digest = sha256_text(content)
    if actual_digest != expected_digest:
        raise RequestBuilderError(
            f"foundation source digest mismatch at {path}: "
            f"expected {expected_digest}, got {actual_digest}"
        )
    return content


def compose_foundation_seed_rubric(git_repo_root: Path) -> dict[str, str]:
    parts = [
        "# Foundation seed rubric v1",
        "",
        f"Parent revision: `{FOUNDATION_PARENT_REVISION}`",
        "",
        "This rubric is frozen by T012. Focused rubric files under `quality/rubrics/*.md` apply only to commits after T025.",
        "",
    ]
    for path, section_header, rubric_header, expected_digest in FOUNDATION_SEED_SOURCES:
        source_blob = load_foundation_seed_source_blob(git_repo_root, path, expected_digest)
        section_body = extract_markdown_section(source_blob, section_header)
        parts.extend([rubric_header, "", section_body, ""])

    content = "\n".join(parts).rstrip("\n") + "\n"
    digest = sha256_text(content)
    if digest != FOUNDATION_SEED_SHA256:
        raise RequestBuilderError(
            "composed foundation-seed digest mismatch: "
            f"expected {FOUNDATION_SEED_SHA256}, got {digest}"
        )
    return {"id": FOUNDATION_SEED_ID, "content": content}


def foundation_seed_provenance_evidence(git_repo_root: Path) -> list[dict[str, Any]]:
    evidence: list[dict[str, Any]] = []
    for path, section_header, rubric_header, expected_digest in FOUNDATION_SEED_SOURCES:
        _content = load_foundation_seed_source_blob(git_repo_root, path, expected_digest)
        evidence.append(
            {
                "command": f"git show {FOUNDATION_PARENT_REVISION}:{path}",
                "exit_code": 0,
                "stdout": json.dumps(
                    {
                        "path": path,
                        "revision": FOUNDATION_PARENT_REVISION,
                        "content_sha256": expected_digest,
                        "section_header": section_header,
                        "rubric_header": rubric_header,
                    },
                    sort_keys=True,
                ),
                "stderr": "",
            }
        )

    evidence.append(
        {
            "command": (
                f"compose foundation-seed rubric from {FOUNDATION_PARENT_REVISION} source blobs"
            ),
            "exit_code": 0,
            "stdout": json.dumps(
                {
                    "rubric_id": FOUNDATION_SEED_ID,
                    "content_sha256": FOUNDATION_SEED_SHA256,
                    "source_revision": FOUNDATION_PARENT_REVISION,
                    "source_paths": [source[0] for source in FOUNDATION_SEED_SOURCES],
                },
                sort_keys=True,
            ),
            "stderr": "",
        }
    )
    return evidence


def load_foundation_seed_rubric_from_git(git_repo_root: Path = REPO_ROOT) -> dict[str, str]:
    return compose_foundation_seed_rubric(git_repo_root)


def load_rubrics_from_parent_manifest(
    repo_root: Path, parent_revision: str, manifest: dict[str, Any]
) -> list[dict[str, str]]:
    rubrics: list[dict[str, str]] = []
    entries = manifest.get("rubrics")
    if not isinstance(entries, list) or not entries:
        raise RequestBuilderError("parent manifest rubrics must be a non-empty array")

    for entry in entries:
        if not isinstance(entry, dict):
            raise RequestBuilderError("parent manifest rubric entry must be an object")
        rubric_id = entry.get("id")
        content_path = entry.get("content_path")
        expected_digest = entry.get("content_sha256")
        if not isinstance(rubric_id, str) or not rubric_id:
            raise RequestBuilderError("parent manifest rubric entry missing id")
        if not isinstance(content_path, str) or not content_path:
            raise RequestBuilderError(f"parent manifest rubric {rubric_id} missing content_path")

        repo_relative_path = f"{RUBRICS_DIR}/{content_path}"
        content = git_show_revision(repo_root, parent_revision, repo_relative_path)
        if content is None:
            raise RequestBuilderError(
                f"parent revision {parent_revision} missing rubric content at {repo_relative_path}"
            )
        if isinstance(expected_digest, str) and expected_digest:
            actual_digest = sha256_text(content)
            if actual_digest != expected_digest:
                raise RequestBuilderError(
                    f"parent rubric {rubric_id} digest mismatch at {repo_relative_path}: "
                    f"expected {expected_digest}, got {actual_digest}"
                )
        rubrics.append({"id": rubric_id, "content": content})
    return rubrics


def load_parent_rubrics(
    repo_root: Path, parent_revision: str, *, git_repo_root: Path = REPO_ROOT
) -> tuple[list[dict[str, str]], list[dict[str, Any]]]:
    manifest_text = git_show_revision(repo_root, parent_revision, MANIFEST_PATH)
    if manifest_text is None:
        return (
            [load_foundation_seed_rubric_from_git(git_repo_root)],
            foundation_seed_provenance_evidence(git_repo_root),
        )

    try:
        manifest = json.loads(manifest_text)
    except json.JSONDecodeError as exc:
        raise RequestBuilderError(f"invalid parent manifest json at {parent_revision}") from exc

    manifest_parent = manifest.get("parent_revision")
    if manifest_parent != FOUNDATION_PARENT_REVISION:
        raise RequestBuilderError(
            "parent manifest parent_revision must be foundation parent "
            f"{FOUNDATION_PARENT_REVISION}, got {manifest_parent!r}"
        )
    return load_rubrics_from_parent_manifest(repo_root, parent_revision, manifest), []


def parse_name_status_line(line: str) -> tuple[str, str]:
    parts = line.split("\t")
    if len(parts) < 2:
        raise RequestBuilderError(f"invalid name-status line: {line!r}")
    status = parts[0]
    if status.startswith("R") or status.startswith("C"):
        if len(parts) < 3:
            raise RequestBuilderError(f"invalid rename/copy status line: {line!r}")
        return status, parts[-1]
    return status, parts[1]


def staged_resulting_doc_content(repo_root: Path, status: str, path: str) -> str:
    if status.startswith("D"):
        return "(deleted in staged candidate)"
    if status.startswith("R") or status.startswith("C"):
        content = git_show_index(repo_root, path)
        if content is None:
            return "(see exact diff)"
        return content
    if status.startswith("A") or status.startswith("M") or status.startswith("T"):
        content = git_show_index(repo_root, path)
        if content is None:
            return "(see exact diff)"
        return content
    return "(see exact diff)"


def build_relevant_docs_from_status(
    repo_root: Path, status_lines: list[str]
) -> list[dict[str, str]]:
    relevant_docs: list[dict[str, str]] = []
    for raw_line in status_lines:
        line = raw_line.strip()
        if not line:
            continue
        status, path = parse_name_status_line(line)
        if not path.endswith(".md"):
            continue
        relevant_docs.append(
            {
                "path": path,
                "content": staged_resulting_doc_content(repo_root, status, path),
            }
        )
    return relevant_docs


def build_relevant_docs_from_revision_diff(
    repo_root: Path, candidate_revision: str, status_lines: list[str]
) -> list[dict[str, str]]:
    relevant_docs: list[dict[str, str]] = []
    for raw_line in status_lines:
        line = raw_line.strip()
        if not line:
            continue
        status, path = parse_name_status_line(line)
        if not path.endswith(".md"):
            continue
        if status.startswith("M"):
            content = git_show_revision(repo_root, candidate_revision, path)
            relevant_docs.append(
                {
                    "path": path,
                    "content": content if content is not None else "(see exact diff)",
                }
            )
        else:
            relevant_docs.append({"path": path, "content": "(see exact diff)"})
    return relevant_docs


def build_exact_staged_request(
    repo_root: Path,
    *,
    mode: str = "local",
    timeout_seconds: int = 900,
) -> dict[str, Any]:
    parent_revision = git(repo_root, "rev-parse", "HEAD").strip()
    candidate_revision = git(repo_root, "write-tree").strip()
    diff = git(repo_root, "diff", "--cached", parent_revision)
    status_lines = [
        line for line in git(repo_root, "diff", "--cached", "--name-status", parent_revision).splitlines() if line.strip()
    ]
    check = git_run(repo_root, "diff", "--cached", "--check", parent_revision)
    rubrics, rubric_evidence = load_parent_rubrics(repo_root, parent_revision)

    return {
        "schema_version": 1,
        "mode": mode,
        "parent_revision": parent_revision,
        "candidate_revision": candidate_revision,
        "diff": diff,
        "relevant_docs": build_relevant_docs_from_status(repo_root, status_lines),
        "rubrics": rubrics,
        "deterministic_evidence": [
            {
                "command": f"git diff --cached --check {parent_revision}",
                "exit_code": check.returncode,
                "stdout": check.stdout,
                "stderr": check.stderr,
            },
            *rubric_evidence,
        ],
        "timeout_seconds": timeout_seconds,
    }


def build_revision_pair_request(
    repo_root: Path,
    *,
    parent_revision: str,
    candidate_revision: str,
    mode: str = "publication",
    timeout_seconds: int = 900,
) -> dict[str, Any]:
    diff = git(repo_root, "diff", parent_revision, candidate_revision)
    status_lines = [
        line
        for line in git(repo_root, "diff", "--name-status", parent_revision, candidate_revision).splitlines()
        if line.strip()
    ]
    check = git_run(repo_root, "diff", "--check", parent_revision, candidate_revision)
    rubrics, rubric_evidence = load_parent_rubrics(repo_root, parent_revision)

    return {
        "schema_version": 1,
        "mode": mode,
        "parent_revision": parent_revision,
        "candidate_revision": candidate_revision,
        "diff": diff,
        "relevant_docs": build_relevant_docs_from_revision_diff(
            repo_root, candidate_revision, status_lines
        ),
        "rubrics": rubrics,
        "deterministic_evidence": [
            {
                "command": f"git diff --check {parent_revision} {candidate_revision}",
                "exit_code": check.returncode,
                "stdout": check.stdout,
                "stderr": check.stderr,
            },
            *rubric_evidence,
        ],
        "timeout_seconds": timeout_seconds,
    }


def verify_protocol() -> int:
    failures: list[str] = []

    def fail(message: str) -> None:
        failures.append(message)

    try:
        load_foundation_seed_rubric_from_git(REPO_ROOT)
    except RequestBuilderError as exc:
        fail(f"foundation-seed git compose failed: {exc}")

    original_frozen = ""
    if FOUNDATION_SEED_FROZEN_PATH.is_file():
        original_frozen = FOUNDATION_SEED_FROZEN_PATH.read_text(encoding="utf-8")
    try:
        FOUNDATION_SEED_FROZEN_PATH.write_text(
            "candidate-controlled tampered foundation-seed\n", encoding="utf-8"
        )
        tampered = load_foundation_seed_rubric_from_git(REPO_ROOT)
        if sha256_text(tampered["content"]) != FOUNDATION_SEED_SHA256:
            fail("tampered candidate frozen file must not affect foundation git rubric")
    except RequestBuilderError as exc:
        fail(f"foundation-seed git compose must ignore tampered frozen file: {exc}")
    finally:
        if original_frozen:
            FOUNDATION_SEED_FROZEN_PATH.write_text(original_frozen, encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="semantic-judge-protocol-") as tmpdir:
        repo = Path(tmpdir)
        git(repo, "init")
        git(repo, "config", "user.email", "protocol@test")
        git(repo, "config", "user.name", "protocol")
        base = repo / "docs"
        base.mkdir(parents=True)
        (base / "note.md").write_text("parent\n", encoding="utf-8")
        git(repo, "add", "docs/note.md")
        git(repo, "commit", "-m", "parent")
        parent_revision = git(repo, "rev-parse", "HEAD").strip()

        (base / "note.md").write_text("staged\n", encoding="utf-8")
        git(repo, "add", "docs/note.md")
        (base / "note.md").write_text("unstaged\n", encoding="utf-8")

        request = build_exact_staged_request(repo, mode="local", timeout_seconds=30)
        if request["parent_revision"] != parent_revision:
            fail("staged request parent_revision must be HEAD")
        if request["mode"] != "local":
            fail("staged request mode must be local")
        if "unstaged" in request["diff"]:
            fail("staged request diff must not include unstaged working-tree content")
        if "staged" not in request["diff"]:
            fail("staged request diff must include staged content")
        doc = next((item for item in request["relevant_docs"] if item["path"] == "docs/note.md"), None)
        if doc is None:
            fail("staged request must include staged markdown relevant_docs")
        elif "staged" not in doc["content"]:
            fail("relevant_docs must use staged index content, not working tree")
        elif "unstaged" in doc["content"]:
            fail("relevant_docs must not use unstaged working-tree content")
        rubric = request["rubrics"][0]
        if rubric["id"] != FOUNDATION_SEED_ID:
            fail("parent without manifest must use foundation-seed rubric from Git blobs")
        if sha256_text(rubric["content"]) != FOUNDATION_SEED_SHA256:
            fail("staged request rubric must match pinned foundation-seed digest")
        provenance_commands = [
            item["command"]
            for item in request["deterministic_evidence"]
            if item["command"].startswith(f"git show {FOUNDATION_PARENT_REVISION}:")
        ]
        if len(provenance_commands) != len(FOUNDATION_SEED_SOURCES):
            fail("fallback rubric request must expose foundation source provenance")

        seed_content = load_foundation_seed_rubric_from_git(REPO_ROOT)["content"]
        manifest = {
            "schema_version": 1,
            "parent_revision": FOUNDATION_PARENT_REVISION,
            "rubrics": [
                {
                    "id": FOUNDATION_SEED_ID,
                    "content_path": "foundation-seed.v1.md",
                    "content_sha256": FOUNDATION_SEED_SHA256,
                }
            ],
        }
        rubrics_dir = repo / "quality" / "rubrics"
        rubrics_dir.mkdir(parents=True)
        (rubrics_dir / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        (rubrics_dir / "foundation-seed.v1.md").write_text(seed_content, encoding="utf-8")
        git(repo, "add", "quality/rubrics/manifest.json", "quality/rubrics/foundation-seed.v1.md")
        git(repo, "commit", "-m", "add rubrics")
        parent_with_manifest = git(repo, "rev-parse", "HEAD").strip()

        (rubrics_dir / "foundation-seed.v1.md").write_text("candidate-tree rubric\n", encoding="utf-8")

        loaded, manifest_provenance = load_parent_rubrics(repo, parent_with_manifest)
        if manifest_provenance:
            fail("parent manifest path must not emit foundation fallback provenance")
        if len(loaded) != 1:
            fail("parent manifest must load exactly one rubric")
        elif loaded[0]["content"] != seed_content:
            fail("parent manifest rubric must load from parent revision blob")
        elif loaded[0]["content"] == "candidate-tree rubric\n":
            fail("parent manifest rubric must not read candidate working-tree rubric")

    if failures:
        for message in failures:
            print(f"FAIL {message}", file=sys.stderr)
        return 1

    print("OK request builder protocol")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--verify-protocol",
        action="store_true",
        help="Run hermetic protocol checks for staged-tree and parent-rubric loading.",
    )
    parser.add_argument(
        "--staged",
        action="store_true",
        help="Emit a local-mode request for the exact staged index tree against HEAD.",
    )
    parser.add_argument(
        "--publication-smoke",
        action="store_true",
        help="Emit T012 publication-mode smoke for fixed parent/candidate revisions.",
    )
    parser.add_argument("--parent", default=FOUNDATION_PARENT_REVISION)
    parser.add_argument("--candidate", default="581c23bb085718d37d994d77d59d3d70b7ea309f")
    parser.add_argument("--timeout-seconds", type=int, default=900)
    args = parser.parse_args()

    if args.verify_protocol:
        return verify_protocol()

    if args.staged:
        request = build_exact_staged_request(
            REPO_ROOT, mode="local", timeout_seconds=args.timeout_seconds
        )
    elif args.publication_smoke:
        request = build_revision_pair_request(
            REPO_ROOT,
            parent_revision=args.parent,
            candidate_revision=args.candidate,
            mode="publication",
            timeout_seconds=args.timeout_seconds,
        )
    else:
        parser.error("one of --verify-protocol, --staged, or --publication-smoke is required")

    json.dump(request, sys.stdout, ensure_ascii=False)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
