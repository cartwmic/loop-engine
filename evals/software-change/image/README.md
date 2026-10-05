# Software-change trial image (draft)

This is software-change-specific infrastructure, not the target-neutral core.
From the checkout root, run `uv run --project evals evalctl image-check`.
It exports the committed tree at local Git ref `v0.24.0`, builds both Rust binaries
inside native arm64 Linux, prints actual tool versions and a JSON `IMAGE_CHECK`
receipt, and inspects a started trial-style container's mount configuration.
It requires native arm64 Docker and build-network access, but no models or keys.
No live/paid model call is made. Capture output locally under ignored `captures/`.
Use `--ref REF --tag TAG` for another pinned local ref/image. Version assertions
compare the executables with the archived ref's Cargo workspace version; the image
label records the resolved commit. Uncommitted product changes are never copied.

Default image: `loop-engine-sc-evals:v0.24.0`. The Dockerfile expects `source.tar`
in its build context, created with `git archive` from the resolved ref. Build args
`LOOP_ENGINE_REF` (default `v0.24.0`) and `LOOP_ENGINE_COMMIT` record that identity;
use `scevals.image.build_image` rather than packaging a working checkout.
Pins: Rust 1.98.0, Node 24.18.1, official Dagu 2.14.0 Linux arm64 (SHA-256 checked),
and the owner's npm package `@earendil-works/pi-coding-agent@1.0.0`.
Base image tags pin versions, not immutable distribution package snapshots.

## T3/T4 contracts

- `scevals.image.DEFAULT_IMAGE`, `build_image(ref, tag)`, and
  `trial_container(image)` are importable. The last yields `(container_id, config)`
  and guarantees cleanup. `trial_container_command(image)` supplies its Docker
  creation argv. Containers start in `/workspace` with `sleep infinity` under tini;
  copy public files in, use `docker exec`, and copy evidence out. Nothing is mounted.
- An Inspect adapter must supply equivalent compose configuration: arm64 image,
  `/workspace`, no volumes/binds/mounts, no image `VOLUME`. Inspect may inject tools
  by copy, not mounts. Record the **actual** config on every trial, not this smoke
  check's receipt. The hidden brief stays on the host; tests/reference are copied
  only after the terminal state. This task does not implement that lifecycle.
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
  `http://localhost:13131/v1`. T4 opens Inspect's public `sandbox_agent_bridge`
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
  **`question: str`**, and prints the returned **string** answer. T4 starts the
  public `sandbox_service("sc_owner", {"ask_owner": handler}, until=done, sandbox=env, ...)`
  and waits for its started event before launching the driver. The async host
  handler receives `question: str` and returns `str`; it retains requirement IDs
  and simulator usage on the host, not in the answer or container.
- For service instances T4 supplies `SC_OWNER_INSTANCE` (the same safe token passed
  to Inspect's `instance` argument); the client is
  `/var/tmp/sandbox-services/sc_owner/<instance>/sc_owner.py`. Without an instance,
  it is `/var/tmp/sandbox-services/sc_owner/sc_owner.py`. No custom HTTP owner port
  is needed: Inspect services use copied clients and file RPC inside the container.
  `SC_OWNER_TIMEOUT_SECONDS` defaults to 120, must be positive, and bounds a call.
  Empty questions exit 2; service errors, malformed responses and timeouts exit 1
  with a generic error (private host exceptions are never printed).

## Focused proofs

`uv run --project evals pytest -q evals/software-change/tests/test_image.py`
checks image command and pinned-config mechanics.
After building the image, run
`uv run --project evals pytest -q evals/software-change/tests/test_image_bridge.py`.
That proof runs **real Pi** through Inspect's public sandbox agent bridge with
scripted, no-network ModelAPI backends: each role completes a bash tool call, the
implementer/reviewer mappings swap, and `ask-owner` reaches the real host-side
sandbox service. It requires Docker/the built image but no model credentials.
It is not proof of a complete software-change workflow (T4/T5 own that).
