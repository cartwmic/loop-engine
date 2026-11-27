#!/usr/bin/env bash
# Dry-run the downstream pin bump on the commit about to be tagged.
#
# The dotfiles repository installs each Loop Engine release: its skill importer
# (import_release_skills.py) copies the release's skill bundles and moves the
# tool pins. A release the importer cannot take turns dotfiles red once the tag
# is out (v0.71.3 removed a file the importer still listed). This runs the
# importer's own `check` mode on the tree of this commit, from the dotfiles
# default branch, and stops with its message before any tag is cut.
#
# release.yml runs it as a gate step of every build job (.github/release-gate-steps.yml),
# so both the dry-run and the publishing dispatch run it before `host` can create the tag.
# It reads the public dotfiles repository, needs no secret and writes nothing outside
# a temporary directory. A dotfiles repository that cannot be read stops the release.
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: scripts/check-downstream-bump.sh --plan FILE [--rev REV]

  --plan FILE  the release's cargo-dist plan (JSON): each downstream pin must match a planned archive
  --rev REV    commit of this checkout to check (default: HEAD)

environment:
  DOWNSTREAM_REPO  git URL of the downstream repository (default: https://github.com/cartwmic/dotfiles.git)
  DOWNSTREAM_REF   its branch (default: main)
USAGE
}

plan=
rev=HEAD
while [[ $# -gt 0 ]]; do
  case $1 in
    --plan) plan=${2:?--plan needs a file}; shift 2 ;;
    --rev) rev=${2:?--rev needs a commit}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
done
[[ -n $plan ]] || { usage; exit 2; }

repo=${DOWNSTREAM_REPO:-https://github.com/cartwmic/dotfiles.git}
ref=${DOWNSTREAM_REF:-main}
importer=dot_local/user_scripts/executable_import_release_skills.py

refuse() {
  # A workflow annotation on GitHub Actions, so the message is on the run page; plain text elsewhere.
  if [[ ${GITHUB_ACTIONS:-} == true ]]; then
    local message=${1//$'\n'/%0A}  # an annotation is one line; %0A is its newline
    echo "::error title=Release refused: the downstream bump would fail::$message"
  else
    echo "Release refused: the downstream bump would fail: $1" >&2
  fi
  exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

git clone --quiet --depth 1 --branch "$ref" "$repo" "$work/downstream" 2> "$work/clone.log" \
  || refuse "cannot read $repo ($ref) to run the check, so no tag is cut: $(tail -n 1 "$work/clone.log")"
[[ -f $work/downstream/$importer ]] \
  || refuse "$repo ($ref) has no $importer, so it cannot run the check; the importer moved or lacks the check mode"

echo "checking $(git rev-parse --short "$rev") against $repo ($ref at $(git -C "$work/downstream" rev-parse --short HEAD))"
if ! output=$(python3 "$work/downstream/$importer" check --repo-dir "$(git rev-parse --show-toplevel)" --rev "$rev" --plan "$plan" --source "$work/downstream" 2>&1); then
  refuse "$output"
fi
echo "$output"
