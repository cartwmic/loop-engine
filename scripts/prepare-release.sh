#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: scripts/prepare-release.sh VERSION" >&2
  echo "example: scripts/prepare-release.sh 0.18.0" >&2
}

if [[ $# -ne 1 ]]; then
  usage
  exit 2
fi

version=${1#v}
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "VERSION must be a stable semantic version such as 0.18.0" >&2
  exit 2
fi

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

if [[ -n $(git status --porcelain=v1 --untracked-files=all) ]]; then
  echo "release preparation requires a clean working tree" >&2
  exit 1
fi

current_version=$(python3 - <<'PY'
from pathlib import Path
import re

text = Path("Cargo.toml").read_text()
match = re.search(r'(?ms)^\[workspace\.package\]\n(?:(?!^\[).)*?^version = "([^"]+)"$', text)
if match is None:
    raise SystemExit("Cargo.toml has no [workspace.package] version")
print(match.group(1))
PY
)

if [[ $version == "$current_version" ]]; then
  echo "workspace version is already $version" >&2
  exit 1
fi

python3 - "$current_version" "$version" <<'PY'
from pathlib import Path
import re
import sys

old, new = sys.argv[1:]
path = Path("Cargo.toml")
text = path.read_text()
pattern = re.compile(r'(?ms)(^\[workspace\.package\]\n(?:(?!^\[).)*?^version = ")' + re.escape(old) + r'("$)')
updated, count = pattern.subn(rf"\g<1>{new}\g<2>", text)
if count != 1:
    raise SystemExit(f"expected one [workspace.package] version {old}, found {count}")
path.write_text(updated)
PY

# A workspace version bump changes the workspace package records in Cargo.lock.
# Refresh those records without updating third-party dependencies, then prove the
# documented release packages still build from the resulting locked graph.
cargo update --workspace --offline
cargo build --locked \
  -p loop-cli \
  -p lean-change-provider \
  -p policy-document-provider \
  -p research-provider \
  -p bookends-check

metadata="$(cargo metadata --format-version 1 --no-deps)"
BIN_DIR="$(printf '%s' "$metadata" | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug")')"
test -x "$BIN_DIR/bookends-check"
unset BOOKENDS_BYPASS BOOKENDS_UPDATE_STREAM BOOKENDS_REMOTE
PATH="$BIN_DIR:$PATH" scripts/bookends-check-gate.sh
cargo fmt --all -- --check
python3 scripts/generate-release.py --check
dist plan --output-format=json > /tmp/loop-engine-dist-plan.json
python3 scripts/assert-dist-plan.py /tmp/loop-engine-dist-plan.json
python3 scripts/assert-release-gates.py

echo "release v$version is prepared"
echo "review Cargo.toml and Cargo.lock; obtain separate owner authorization to commit/push"
echo "require successful push-event ci.yml for the exact dispatched SHA, then authorize a dry-run dispatch"
echo "only after successful dry-run proof and separate publication authorization, dispatch release.yml with tag=v$version"
