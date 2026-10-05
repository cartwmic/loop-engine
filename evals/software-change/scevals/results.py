"""Host-only, read-only secrecy audit. Never export needles or matched bytes."""
from pathlib import Path

from evalcore.report import UnsafeReportError, protected_values, scan_results
from .cases import load_case


def hidden_values(cases_path: Path) -> tuple[set[str], set[bytes], int]:
    paths = ([cases_path] if (cases_path / "case.toml").is_file()
             else sorted(p.parent for p in cases_path.glob("*/case.toml")))
    if not paths:
        raise ValueError("no cases found")
    text, binary = set(), set()
    for path in paths:
        case = load_case(path)
        for file in sorted((case.path / "hidden").rglob("*")):
            if not file.is_file():
                continue
            content = file.read_bytes()
            if content:
                binary.add(content)
            try:
                value = content.decode("utf-8")
            except UnicodeError:
                continue
            if value.strip():
                text.add(value.strip())
            # Punctuation-only lines (e.g. a closing brace) are not content
            # evidence; all nonblank lines containing a letter/digit are searched.
            text.update(line.strip() for line in value.splitlines()
                        if any(c.isalnum() for c in line))
    return text, binary, len(paths)


def audit_results(directory: Path, cases_path: Path, *, provider_config_paths=(), environment=None):
    if not directory.is_dir() or directory.is_symlink():
        raise ValueError("results must be a directory")
    entries = sorted(directory.rglob("*"))
    if any(p.is_symlink() for p in entries):
        raise ValueError("result symlinks are not supported")
    files = [p for p in entries if p.is_file()]
    if not files:
        raise ValueError("no result files found")
    hidden, binary, count = hidden_values(cases_path)
    # Standalone audits lack campaign settings. Protect host Pi auth whenever
    # present, including old Codex reports audited from another invocation.
    from evalcore.codex import codex_auth_path
    auth = codex_auth_path()
    if auth.exists():
        provider_config_paths = (*provider_config_paths, auth)
    protected = protected_values(environment, provider_config_paths)
    byte_needles = binary | {value.encode("utf-8") for value in hidden | protected}
    for file in files:
        content = file.read_bytes()
        if any(value in content for value in byte_needles):
            raise UnsafeReportError("protected result content detected")
        # Decode even non-UTF8 files for marker/escape scanning; no file is skipped.
        scan_results((content.decode("utf-8", errors="replace"),), hidden | protected)
    return len(files), count


def check_results(args):
    try:
        files, cases = audit_results(args.directory, args.cases,
                                     provider_config_paths=args.provider_config)
    except UnsafeReportError:
        print("FAIL: protected case text, credential or provider configuration detected; matched content withheld.")
        return 1
    print(f"PASS: scanned {files} result files against {cases} cases and runtime credential/provider configuration protections.")
    return 0


def register_subcommands(subparsers):
    parser = subparsers.add_parser("check-results", help="audit every result file without printing protected content")
    parser.add_argument("directory", type=Path)
    parser.add_argument("--cases", type=Path, required=True, help="one case or repository of immediate case directories")
    parser.add_argument("--provider-config", type=Path, action="append", default=[],
                        help="host-only resolved provider config file; repeat to protect additional configs")
    parser.set_defaults(handler=check_results)
