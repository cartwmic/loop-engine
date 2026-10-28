#!/usr/bin/env bash
# Keep this check list identical to .github/workflows/ci.yml.
# CI uses pushed-range Bookends; this local baseline uses ordinary working-tree mode.
set -euo pipefail

finish() {
  status=$?
  if [[ $status -eq 0 ]]; then
    printf '%s\n' PASS
  else
    printf '%s\n' FAIL
  fi
  exit "$status"
}
trap finish EXIT

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace
python3 scripts/external-work-journey.py
python3 scripts/policy-document-journey.py
python3 scripts/research-journey.py
python3 scripts/lean-change-journey.py
python3 scripts/lean-change-mutations.py
python3 scripts/generate-prd-journey.py

# Capture metadata separately so a Cargo failure retains its own exit status.
metadata="$(cargo metadata --format-version 1 --no-deps)"
BIN_DIR="$(printf '%s' "$metadata" | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug")')"
# Never fall back to an installed checker when the current build is missing.
test -x "$BIN_DIR/bookends-check"
export PATH="$BIN_DIR:$PATH"
unset BOOKENDS_BYPASS BOOKENDS_UPDATE_STREAM BOOKENDS_REMOTE
scripts/bookends-check-gate.sh
