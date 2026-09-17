#!/bin/sh
# Portable bookends-check gate for pre-push and required CI.
# Prefer a PATH `bookends-check` binary; otherwise
# `cargo run -p bookends-check --bin bookends-check`.
# Bypass channel: BOOKENDS_BYPASS=<class>:<reason> (omit --bypass when unset/empty).
# The pre-push hook and publication CI set BOOKENDS_UPDATE_STREAM=1 so the
# wrapper passes Git's update stream through to the checker. Other callers use
# the ordinary current-working-tree check without blocking on stdin. Missing
# history or a failed receipt is RED, never a skip.
set -eu

git_root=$(git rev-parse --show-toplevel 2>/dev/null) || git_root=
if [ -n "$git_root" ]; then
  cd "$git_root"
fi

remote=${BOOKENDS_REMOTE:-}
if [ -z "$remote" ] && [ "$#" -ge 1 ]; then
  remote=$1
fi
if [ "${BOOKENDS_UPDATE_STREAM:-}" = "1" ]; then
  set -- --updates-stdin
else
  # Direct callers such as the ordinary repository gate have no update
  # protocol and must not block waiting on their inherited stdin.
  exec </dev/null
  set --
fi
if [ -n "$remote" ]; then
  set -- "$@" --remote "$remote"
fi
# The pre-push hook supplies the remote name as its first argument.  The
# checker fetches from that actual source remote without interpreting the
# remote URL as a refspec.
if [ -n "${BOOKENDS_BYPASS:-}" ]; then
  set -- "$@" --bypass "$BOOKENDS_BYPASS"
fi
if [ -n "${BOOKENDS_RECEIPT_ROOT:-}" ]; then
  set -- "$@" --receipt-root "$BOOKENDS_RECEIPT_ROOT"
fi
if [ -n "${BOOKENDS_MAX_COMMITS:-}" ]; then
  set -- "$@" --max-commits "$BOOKENDS_MAX_COMMITS"
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
