#!/usr/bin/env python3
"""Check fictional repository document authority and relative Markdown links."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ACCEPTANCE_REQUIREMENTS = (
    "A caller can inspect the frozen policies and artifact schemas before evaluating a transition.",
    "A malformed subject artifact is denied with each violated structural rule named.",
    "A checked transition refuses stale, self-authored, duplicate-author, or configured-obligation-incomplete evidence.",
    "The reference workflow reaches its terminal state only after its configured validation obligations are satisfied.",
)
MARKDOWN_LINK = re.compile(r"(?<!!)\[[^\]]+\]\(([^)\s]+)(?:\s+['\"][^)]*['\"])?\)")


def markdown_links_resolve(root: Path) -> None:
    root = root.resolve()
    for document in sorted(root.rglob("*.md")):
        if not document.is_file():
            continue
        for target in MARKDOWN_LINK.findall(document.read_text(encoding="utf-8")):
            if target.startswith(("#", "/", "//", "http:", "https:", "mailto:")):
                continue
            target = target.split("#", 1)[0]
            if not target:
                continue
            resolved = (document.parent / target).resolve()
            try:
                resolved.relative_to(root)
            except ValueError as error:
                raise ValueError(f"link escapes companion root: {document.relative_to(root)} -> {target}") from error
            if not resolved.is_file():
                raise ValueError(f"missing Markdown link target: {document.relative_to(root)} -> {target}")


def check_documents(root: Path) -> None:
    authoritative = root / "docs" / "PRD.md"
    supplemental = root / "loop-engine-software-change-provider-prd.md"
    matrix = root / "implementation-evidence" / "requirement-to-proof.md"
    repository_readme = root / "README.md"
    provider_readme = root / "provider" / "README.md"
    for path in (authoritative, supplemental, matrix, repository_readme, provider_readme):
        if not path.is_file():
            raise ValueError(f"required companion missing: {path.relative_to(root)}")

    authoritative_text = authoritative.read_text(encoding="utf-8")
    supplemental_text = supplemental.read_text(encoding="utf-8")
    if "authoritative" not in authoritative_text.lower():
        raise ValueError("docs/PRD.md does not declare authority")
    if "supplemental" not in supplemental_text.lower() or "non-authoritative" not in supplemental_text.lower():
        raise ValueError("supplemental change PRD does not declare supplemental/non-authoritative status")

    repository_text = repository_readme.read_text(encoding="utf-8")
    provider_text = provider_readme.read_text(encoding="utf-8")
    for text, links in (
        (repository_text, ("docs/PRD.md", "implementation-evidence/requirement-to-proof.md")),
        (provider_text, ("../docs/PRD.md", "../implementation-evidence/requirement-to-proof.md")),
    ):
        for link in links:
            if f"]({link})" not in text:
                raise ValueError(f"README missing required link {link}")

    matrix_text = matrix.read_text(encoding="utf-8")
    for requirement in ACCEPTANCE_REQUIREMENTS:
        if requirement not in matrix_text:
            raise ValueError(f"proof matrix missing acceptance requirement: {requirement}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path, help="fictional repository root")
    args = parser.parse_args()
    root = args.root if args.root is not None else Path(__file__).resolve().parents[1]
    try:
        root = root.resolve()
        if not root.is_dir():
            raise ValueError(f"companion root is not a directory: {root}")
        markdown_links_resolve(root)
        check_documents(root)
    except (OSError, UnicodeError, ValueError) as error:
        print(f"assert-doc-authority: {error}", file=sys.stderr)
        return 1
    print("assert-doc-authority: passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
