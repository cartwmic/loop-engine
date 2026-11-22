#!/usr/bin/env bash
# Runs scripts/prepare-release.sh VERSION on kgate, for a host with no cargo (the agent-host pod).
# scripts/release-after-merge.sh uses this as its prepare step when cargo is not on PATH and kgate is
# (heavy jobs run on kgate, not on a host). The job bumps Cargo.toml and Cargo.lock in its copy of
# this worktree and returns them through /out; this script copies them back.
#
#   usage: scripts/prepare-release-kgate.sh VERSION     (run from the worktree root)
set -euo pipefail

usage() {
  echo "usage: scripts/prepare-release-kgate.sh VERSION" >&2
  echo "example: scripts/prepare-release-kgate.sh 0.66.0" >&2
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

if ! command -v kgate >/dev/null 2>&1; then
  echo "kgate is not on PATH: cannot prepare the release on the cluster" >&2
  exit 69
fi

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"   # kgate ships this worktree, so the job runs from its root

# The kgate image has no mise, so the job fetches the cargo-dist that mise.toml pins.
dist_version=$(python3 -c 'import tomllib; print(tomllib.load(open("mise.toml", "rb"))["tools"]["aqua:axodotdev/cargo-dist"])' 2>/dev/null) \
  || { echo "mise.toml has no aqua:axodotdev/cargo-dist pin" >&2; exit 1; }
if [[ ! $dist_version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "the mise.toml cargo-dist pin is not a stable version: $dist_version" >&2
  exit 1
fi

# The pod script takes the two versions as arguments ($1 cargo-dist, $2 release), so nothing is spliced into it.
pod_script='
set -eu
d=$(mktemp -d)
curl -fsSL -o "$d/dist.tar.xz" "https://github.com/axodotdev/cargo-dist/releases/download/v$1/cargo-dist-x86_64-unknown-linux-gnu.tar.xz"
tar -xJf "$d/dist.tar.xz" -C "$d" --strip-components=1
export PATH="$d:$PATH"
dist --version
scripts/prepare-release.sh "$2"
cp Cargo.toml Cargo.lock /out/
'

log=$(mktemp)
trap 'rm -f "$log"' EXIT
rc=0
kgate --wait 3600 -- sh -c "$pod_script" prepare-release "$dist_version" "$version" 2>&1 | tee "$log" || rc=$?

case $rc in
  0) ;;
  69)
    echo "kgate exit 69: the prepare job did not start or finish on the cluster, so nothing was prepared (kgate's log record has the reason)" >&2
    exit 69 ;;
  75)
    echo "kgate exit 75: the cluster had no room for the prepare pod in time, so the job never ran; rerun later" >&2
    exit 75 ;;
  *)
    echo "prepare on kgate failed with exit $rc; see the kgate output above" >&2
    exit "$rc" ;;
esac

# kgate reports where it copied the job's /out: "kgate: /out -> PATH (N files)".
out=$(sed -n 's|^kgate: /out -> \(.*\) ([0-9][0-9]* files)$|\1|p' "$log" | tail -n 1)
if [[ -z $out || ! -f $out/Cargo.toml || ! -f $out/Cargo.lock ]]; then
  echo "kgate returned no Cargo.toml and Cargo.lock in /out (its /out note: ${out:-none})" >&2
  exit 1
fi
cp "$out/Cargo.toml" "$out/Cargo.lock" .
echo "release v$version is prepared on kgate; Cargo.toml and Cargo.lock copied back from $out"
