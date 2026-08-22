#!/bin/sh
# Portable bookends-check gate for pre-push and required CI.
# Prefer a PATH `bookends-check` binary; otherwise
# `cargo run -p bookends-check --bin bookends-check`.
# Bypass channel: BOOKENDS_BYPASS=<class>:<reason> (omit --bypass when unset/empty).
# Pre-push and CI exec this wrapper with no --bypass flag; the env var is the
# portable bypass channel. Missing binary without a runnable cargo crate is
# RED and nonzero, never a silent skip or green.
set -eu

# Git pre-push feeds ref lines on stdin; the checker does not consume them.
exec </dev/null

git_root=$(git rev-parse --show-toplevel 2>/dev/null) || git_root=
if [ -n "$git_root" ]; then
  cd "$git_root"
fi

if [ -n "${BOOKENDS_BYPASS:-}" ]; then
  set -- --bypass "$BOOKENDS_BYPASS"
else
  set --
fi

if command -v bookends-check >/dev/null 2>&1; then
  exec bookends-check "$@"
fi

if command -v cargo >/dev/null 2>&1; then
  cargo_output=
  if cargo_output=$(cargo run -p bookends-check --bin bookends-check -- "$@"); then
    printf '%s\n' "$cargo_output"
    exit 0
  fi

  # A checker failure already includes its RED marker; a build/fallback
  # failure does not. Never print two status markers.
  case "$cargo_output" in
    RED*) printf '%s\n' "$cargo_output" ;;
    *)
      printf '%s\n' RED
      if [ -n "$cargo_output" ]; then
        printf '%s\n' "$cargo_output"
      fi
      ;;
  esac
  exit 1
fi

# A missing binary without cargo is a hard failure, never a skip.
printf '%s\n' RED
exit 1
