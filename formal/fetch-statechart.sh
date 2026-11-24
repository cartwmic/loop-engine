#!/usr/bin/env bash
# Fetches statechart at one pinned commit into a new folder. formal/check.sh calls it
# when the folder is not in the cache.
#
#   formal/fetch-statechart.sh <repo-url> <commit> <dest>
#
# GitHub answers 429 under load, so a failed fetch is retried: four attempts in all,
# waiting 2, 4 and 8 seconds between them (LOOP_FORMAL_FETCH_DELAY sets the first
# wait; the tests set it to 0). Every failure is retried, not only 429, so a bad
# commit or a dead network also takes all four attempts before it fails.
# Exit 0: <dest> holds the pinned commit. Exit 2: the fetch failed for good (the
# message names git's last error) or the fetched commit is not the pinned one.
set -euo pipefail

die() { printf 'formal/fetch-statechart.sh: %s\n' "$*" >&2; exit 2; }

repo="$1"
commit="$2"
dest="$3"
attempts=4
delay="${LOOP_FORMAL_FETCH_DELAY:-2}"

command -v git >/dev/null || die "git is needed to fetch statechart"
tmp="$(mktemp -d "$(dirname "$dest")/statechart.XXXXXX")"
fail() { rm -rf "$tmp"; die "$@"; }
git -C "$tmp" init -q

attempt=1
until err="$(git -C "$tmp" fetch -q --depth 1 "$repo" "$commit" 2>&1)"; do
  last="$(printf '%s\n' "$err" | tail -n 1)"
  if [ "$attempt" -ge "$attempts" ]; then
    fail "could not fetch statechart $commit from $repo after $attempts attempts: $last"
  fi
  printf 'formal/fetch-statechart.sh: fetch attempt %s of %s failed, retrying in %ss: %s\n' \
    "$attempt" "$attempts" "$delay" "$last" >&2
  sleep "$delay"
  delay=$((delay * 2))
  attempt=$((attempt + 1))
done

git -C "$tmp" checkout -q FETCH_HEAD
[ "$(git -C "$tmp" rev-parse HEAD)" = "$commit" ] || fail "fetched the wrong statechart commit"
rm -rf "$dest"
mv "$tmp" "$dest"
