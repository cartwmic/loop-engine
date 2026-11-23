#!/usr/bin/env bash
# The one command for the formal model. It runs the sync check first (exit 1 if a
# file the model covers changed since the model was last confirmed), then
# model-checks formal/LoopRun.tla with TLC through statechart. Exit 0: every rule
# holds. Exit 1: a rule breaks (the report below names it and prints the shortest
# path), the sync check failed, or its test failed. Exit 2: the check could not run
# (a tool is missing, a download failed, or formal/COVERS is missing or malformed).
#
# Everything it fetches is pinned: statechart by git commit, the TLC jar by
# sha256. Both are cached under $LOOP_FORMAL_CACHE (default
# ~/.cache/loop-engine-formal), so a second run downloads nothing.
#
# Java 11 or later and Node 22.18 or later (or 23.6 or later) must be on PATH.
# When there is no java, mise installs the Temurin 25 that statechart pins.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

die() { printf 'formal/check.sh: %s\n' "$*" >&2; exit 2; }

# The sync check comes first, then its test. Both need only python3 and git, so they
# answer on any machine, with no Java and no downloads. The test runs in CI too,
# because the `checks` job runs this script.
command -v python3 >/dev/null || die "python3 is needed for the sync check"
python3 "$here/check-sync.py" || exit $?
python3 "$here/test_check_sync.py" || exit $?

STATECHART_REPO=https://github.com/cursor/plugins
STATECHART_COMMIT=7a583913eb1b8d1a0ee76753d1a2cf072e7b54f9   # pstack/skills/statechart
TLA2TOOLS_URL=https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar
TLA2TOOLS_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
JAVA_TOOL=java@temurin-25

cache="${LOOP_FORMAL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/loop-engine-formal}"
mkdir -p "$cache"

sha256_of() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# statechart, at the pinned commit.
statechart_dir="$cache/statechart-$STATECHART_COMMIT"
if [ ! -x "$statechart_dir/pstack/skills/statechart/scripts/statechart" ]; then
  command -v git >/dev/null || die "git is needed to fetch statechart"
  tmp="$(mktemp -d "$cache/statechart.XXXXXX")"
  git -C "$tmp" init -q
  git -C "$tmp" fetch -q --depth 1 "$STATECHART_REPO" "$STATECHART_COMMIT" \
    || die "could not fetch statechart $STATECHART_COMMIT from $STATECHART_REPO"
  git -C "$tmp" checkout -q FETCH_HEAD
  [ "$(git -C "$tmp" rev-parse HEAD)" = "$STATECHART_COMMIT" ] || die "fetched the wrong statechart commit"
  rm -rf "$statechart_dir"
  mv "$tmp" "$statechart_dir"
fi
statechart="$statechart_dir/pstack/skills/statechart/scripts/statechart"

# The TLC jar, checked against its sha256.
if [ -z "${TLA2TOOLS:-}" ]; then
  jar="$cache/tla2tools-1.7.4.jar"
  if [ ! -f "$jar" ] || [ "$(sha256_of "$jar")" != "$TLA2TOOLS_SHA256" ]; then
    command -v curl >/dev/null || die "curl is needed to fetch tla2tools.jar"
    curl -fsSL -o "$jar.part" "$TLA2TOOLS_URL" || die "could not download $TLA2TOOLS_URL"
    [ "$(sha256_of "$jar.part")" = "$TLA2TOOLS_SHA256" ] || die "tla2tools.jar does not match its pinned sha256"
    mv "$jar.part" "$jar"
  fi
  export TLA2TOOLS="$jar"
fi

# Java.
if [ -z "${JAVA:-}" ]; then
  # `java -version`, not `command -v`: a mise shim is on PATH with no version set.
  if java -version >/dev/null 2>&1; then
    JAVA="$(command -v java)"
  elif command -v mise >/dev/null; then
    # Run from the cache dir so mise reads none of the repo's config.
    (cd "$cache" && mise install "$JAVA_TOOL" >&2) || die "mise could not install $JAVA_TOOL"
    JAVA="$(cd "$cache" && mise which java --tool "$JAVA_TOOL")" || die "mise could not find $JAVA_TOOL"
  else
    die "java (11 or later) is not on PATH, and mise is not installed to provide it"
  fi
  export JAVA
fi

command -v node >/dev/null || die "node (22.18 or later) is not on PATH"

work="$(mktemp -d)"
out="$work/out"          # statechart writes only to a new folder
status=0
(cd "$here" && "$statechart" LoopRun.tla --config LoopRun.cfg --view view.mjs --out "$out") >"$work/stdout.txt" 2>&1 || status=$?
# The report holds the broken rule and its shortest path; CI would lose it with the temp dir.
# Without a report (exit 2) what statechart printed is the error.
if [ -f "$out/report.md" ]; then cat "$out/report.md"; else cat "$work/stdout.txt"; fi
if [ "$status" -eq 0 ]; then
  rm -rf "$work"
else
  printf 'formal/check.sh: exit %s; full TLC output kept in %s\n' "$status" "$work" >&2
fi
exit "$status"
