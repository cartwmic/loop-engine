# Software-change trial image

## Overview

This is software-change-specific infrastructure, not the target-neutral core.
It is the mount-free native arm64 Linux image every trial runs in. It is not a
general sandbox: it pins one Loop Engine ref, and it does not resist a deliberate
model escape (the threat boundary is a single trusted owner).

## Setup

From the checkout root:

```sh
uv run --project evals evalctl image-check
```

It exports the committed tree at local Git ref `v0.26.0`, builds both Rust binaries
inside native arm64 Linux, prints actual tool versions and a JSON `IMAGE_CHECK`
receipt, and inspects a started trial-style container's mount configuration.
It requires native arm64 Docker and build-network access, but no models or keys.
No live/paid model call is made. Capture output locally under ignored `captures/`.
Use `--ref REF --tag TAG` for another pinned local ref/image. Version assertions
compare the executables with the archived ref's Cargo workspace version; the image
label records the resolved commit. Uncommitted product changes are never copied.

Default image: `loop-engine-sc-evals:v0.26.0`. The Dockerfile expects `source.tar`
in its build context, created with `git archive` from the resolved ref. Build args
`LOOP_ENGINE_REF` (default `v0.26.0`) and `LOOP_ENGINE_COMMIT` record that identity;
use `scevals.image.build_image` rather than packaging a working checkout.
Pins: Rust 1.98.0, Node 24.18.1, official Dagu 2.14.0 Linux arm64 (SHA-256 checked),
and the owner's npm package `@earendil-works/pi-coding-agent@1.0.0`.
Base image tags pin versions, not immutable distribution package snapshots.

## Usage

The trial lifecycle in `scevals.target` uses this image. The helpers and container
contract below are internal to this harness, with no standalone API-stability
promise; the internal target contract is in checkout-root `evals/core/README.md`.
Its container contract:

- `scevals.image.DEFAULT_IMAGE`, `build_image(ref, tag)`, and
  `trial_container(image)` are importable. The last yields `(container_id, config)`
  and guarantees cleanup. `trial_container_command(image)` supplies its Docker
  creation argv. Containers start in `/workspace` with `sleep infinity` under tini;
  copy public files in, use `docker exec`, and copy evidence out. Nothing is mounted.
- An Inspect adapter must supply equivalent compose configuration: arm64 image,
  `/workspace`, no volumes/binds/mounts, no image `VOLUME`. Inspect may inject tools
  by copy, not mounts. Record the **actual** config on every trial, not this smoke
  check's receipt. The hidden brief stays on the host; tests/reference are copied
  only after the terminal state; `scevals.target` implements that lifecycle.
- Run the default driver with `sc-driver PROMPT` (Pi print mode). Workers use
  `sc-implement PROMPT` and `sc-review PROMPT`. Prompts may be `@FILE` arguments
  or piped stdin. These commands accept prompt arguments only, not Pi flags.
  Leading `--` permits prompt text starting with a dash. Calls start fresh sessions
  and preserve their working directory; the trial adapter owns the process timeout.
- Each command generates its own fresh Pi agent directory with exactly one model
  in `models.json`: respectively `role-driver`, `role-implementer`, `role-reviewer`,
  under provider `inspect`, API `openai-completions`. Template configs live at
  `/opt/sc-evals/pi/<role>/models.json`. Both shipped skills from the pinned ref
  live at `/opt/sc-evals/skills/` and are explicitly loaded for all roles.
- `SC_BRIDGE_URL` is the OpenAI-compatible **base URL including `/v1`**, default
  `http://localhost:13131/v1`. The trial lifecycle opens Inspect's public `sandbox_agent_bridge`
  on port 13131 (or chooses a port and passes the full URL), with `model_aliases`
  mapping those three exact names to the campaign's role-bound Inspect Models.
  The only API key in the container is the literal dummy `inspect-no-credential`;
  no host model credentials or resolved provider configuration are copied in.
  Pi's built-in catalog is not removed, but no other authenticated provider is
  configured; ordinary role-command use cannot choose a model. Deliberate bypass
  is outside the frozen threat boundary. Inspect, not the dummy Pi prices, accounts
  for model usage.
- `ask-owner QUESTION` (or piped stdin) imports Inspect's generated sandbox client
  for service **`sc_owner`**, calls method **`ask_owner`** with keyword
  **`question: str`**, and prints the returned **string** answer. The trial lifecycle starts the
  public `sandbox_service("sc_owner", {"ask_owner": handler}, until=done, sandbox=env, ...)`
  and waits for its started event before launching the driver. The async host
  handler receives `question: str` and returns `str`; it retains requirement IDs
  and simulator usage on the host, not in the answer or container.
- For service instances the lifecycle supplies `SC_OWNER_INSTANCE` (the same safe token passed
  to Inspect's `instance` argument); the client is
  `/var/tmp/sandbox-services/sc_owner/<instance>/sc_owner.py`. Without an instance,
  it is `/var/tmp/sandbox-services/sc_owner/sc_owner.py`. No custom HTTP owner port
  is needed: Inspect services use copied clients and file RPC inside the container.
  `SC_OWNER_TIMEOUT_SECONDS` defaults to 120, must be positive, and bounds a call.
  Empty questions exit 2; service errors, malformed responses and timeouts exit 1
  with a generic error (private host exceptions are never printed).

## Validation

```sh
uv run --project evals pytest -q evals/software-change/tests/test_image.py
uv run --project evals pytest -q evals/software-change/tests/test_image_bridge.py
```

The first checks image command and pinned-config mechanics. Run the second after
building the image. That proof runs **real Pi** through Inspect's public sandbox agent bridge with
scripted, no-network ModelAPI backends: each role completes a bash tool call, the
implementer/reviewer mappings swap, and `ask-owner` reaches the real host-side
sandbox service. It requires Docker/the built image but no model credentials.
It is not proof of a complete software-change workflow; see the target README.

## Troubleshooting

- **The build fails fetching layers or Dagu.** Building needs network access; the
  Dagu archive is SHA-256 checked and a mismatch stops the build.
- **Version assertions fail.** The ref's Cargo workspace version differs from the
  built executables. Rebuild from a clean pinned ref; uncommitted changes are
  never included.
- **`ask-owner` exits 1.** The host service is not running, timed out
  (`SC_OWNER_TIMEOUT_SECONDS`, default 120) or returned a malformed reply. Exit 2
  means an empty question.
- **A role command cannot reach a model.** `SC_BRIDGE_URL` must include `/v1` and
  point at the Inspect sandbox agent bridge (default port 13131).
