#!/bin/sh
# The scripted lean campaign for profiles b and c, in the built lean image, on a Docker daemon: build the base image at the
# recorded ref, the arms image, and the lean image from this checkout's HEAD, then run test_lean_campaign.py (both profiles).
# It runs as the one command of a Docker kgate job, see ../../kgate-docker.toml:
#   run-and-wake -- kgate --profile kgate-docker.toml --node cartwmic-g14-wsl -- sh evals/software-change/lean-campaign-kgate.sh
# The lean image is built from the committed tree (git archive HEAD), so commit before you run it. Output copies go to /out
# (kgate returns them to ~/.local/state/kgate/<id>.out/). The Rust builds are bounded to SC_CARGO_JOBS jobs (default 4).
set -eu

BASE_REF=8aeedb5ee4ef58d4a0fcbab8fef98b2a195e0d46
BASE_TAG=loop-engine-sc-evals:8aeedb5ee
ARMS_TAG=loop-engine-sc-evals:arms-s2-8aeedb5ee   # Dockerfile.lean's default BASE_IMAGE
LEAN_TAG=loop-engine-sc-evals:lean-$(git rev-parse --short=12 HEAD)
OUT=${OUT:-/out}
JOBS=${SC_CARGO_JOBS:-4}
VENV=${VENV:-/tmp/lean-venv}
export SC_IMAGE_CARGO_JOBS=$JOBS
# The harness's own environment, from evals/uv.lock (a plain pip install resolves a newer agent-client-protocol that inspect-ai cannot import).
export UV_PROJECT_ENVIRONMENT=/tmp/lean-env
mkdir -p "$OUT"

# The Docker sidecar's default bridge has MTU 1500 but the pod's own network has 1280, so a large download inside a build step
# (the Dagu tarball, 50 MB) stalls and is reset. Measured 2026-11-22 on the G14: from a container on the default bridge the download
# timed out after 150 s; with --network host it took 13 s. The harness calls the build with no network option, so every
# image build here goes through this wrapper, which builds on the pod's network.
REAL_DOCKER=$(command -v docker)
WRAP=$(mktemp -d)
cat > "$WRAP/docker" <<WRAPPER
#!/bin/sh
if [ "\${1:-}" = build ]; then shift; exec "$REAL_DOCKER" build --network host "\$@"; fi
exec "$REAL_DOCKER" "\$@"
WRAPPER
chmod 755 "$WRAP/docker"
export PATH="$WRAP:$PATH"

step() { printf '\n=== %s (%s)\n' "$1" "$(date -u +%H:%M:%S)"; }

step "daemon"
docker version --format 'server {{.Server.Version}} {{.Server.Os}}/{{.Server.Arch}}'
docker info --format 'storage driver: {{.Driver}}; cpus: {{.NCPU}}; memory: {{.MemTotal}}'

step "harness install"
python3 -m venv "$VENV"
"$VENV/bin/pip" install --quiet uv
UV="$VENV/bin/uv run --frozen --project evals"
$VENV/bin/uv sync --frozen --project evals

step "base image at $BASE_REF"
$UV evalctl image-check --ref "$BASE_REF" --tag "$BASE_TAG" > "$OUT/image-check.txt" 2>&1 || { cat "$OUT/image-check.txt"; exit 1; }
tail -n 15 "$OUT/image-check.txt"

step "arms image"
docker build -f evals/software-change/image/Dockerfile.arms --build-arg BASE_IMAGE="$BASE_TAG" -t "$ARMS_TAG" evals/software-change/image

step "lean image from $(git rev-parse HEAD)"
CTX=$(mktemp -d)
cp evals/software-change/image/Dockerfile.lean evals/software-change/image/pi-role.py "$CTX"
git archive --format=tar --output="$CTX/source.tar" HEAD
docker build -f "$CTX/Dockerfile.lean" --build-arg SC_CARGO_JOBS="$JOBS" --build-arg LEAN_SOURCE_SHA="$(sha256sum "$CTX/source.tar" | cut -d' ' -f1)" \
  -t "$LEAN_TAG" "$CTX"
docker image inspect --format '{{.Id}} {{.Architecture}} {{.Size}}' "$LEAN_TAG" > "$OUT/lean-image.txt"
docker run --rm "$LEAN_TAG" cat /opt/sc-evals/arms/versions.txt > "$OUT/lean-versions.txt"
docker system df > "$OUT/docker-df.txt"
cat "$OUT/lean-image.txt" "$OUT/lean-versions.txt" "$OUT/docker-df.txt"

step "scripted lean campaign, profiles b and c, in $LEAN_TAG"
rc=0
SC_LEAN_IMAGE="$LEAN_TAG" SC_EVALCTL="$UV evalctl" $UV pytest -vv -rA -p no:cacheprovider evals/software-change/tests/test_lean_campaign.py \
  > "$OUT/campaign.txt" 2>&1 || rc=$?
cat "$OUT/campaign.txt"
# The harness keeps the trial's captures (driver output, engine records) under captures/; send them back with the result.
[ -d evals/software-change/captures ] && tar -C evals/software-change -czf "$OUT/captures.tgz" captures
exit "$rc"
