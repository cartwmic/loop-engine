# The lean-change provider layered on the Stage 2 arms image. The base supplies Pi, the owner context, ask-owner and the
# role launchers. This layer builds loop-engine and lean-change from the checked-out source, so a lean trial runs the
# provider and the engine it needs from the same commit. It replaces the base's older loop-engine binary; the base's
# software-change provider stays in the image but no lean arm uses it.
# Build from this directory with source.tar holding the tracked tree (git archive HEAD), see ARMS.md section 10:
#   docker build -f Dockerfile.lean --build-arg LEAN_SOURCE_SHA=$(sha256sum source.tar | cut -d' ' -f1) -t TAG .
ARG BASE_IMAGE=loop-engine-sc-evals:arms-s2-8aeedb5ee
FROM rust:1.98.0-bookworm AS build
ARG TARGETARCH
# Optional memory bound for the cargo build (empty = cargo default parallelism).
ARG SC_CARGO_JOBS=
WORKDIR /src
COPY source.tar /tmp/source.tar
RUN test "$TARGETARCH" = arm64 && tar -xf /tmp/source.tar -C /src \
    && cargo build --locked --release ${SC_CARGO_JOBS:+-j $SC_CARGO_JOBS} -p loop-cli -p lean-change-provider

FROM ${BASE_IMAGE}
ARG LEAN_SOURCE_SHA
LABEL com.loop-engine.lean-change.source-sha256=$LEAN_SOURCE_SHA
COPY --from=build /src/target/release/loop-engine /src/target/release/lean-change /usr/local/bin/
COPY --from=build /src/skills/using-loop-engine /opt/sc-evals/skills/using-loop-engine
COPY --from=build /src/crates/lean-change-provider/skills/using-lean-change-provider /opt/sc-evals/skills/using-lean-change-provider
# pi-role.py loads the provider skill named by SC_PROVIDER_SKILL (default: software-change).
COPY pi-role.py /opt/sc-evals/pi-role.py
RUN chmod 755 /opt/sc-evals/pi-role.py \
    && loop-engine --version && lean-change --version \
    && test -s /opt/sc-evals/skills/using-lean-change-provider/SKILL.md \
    && printf '%s\n' "loop-engine $(loop-engine --version)" "lean-change $(lean-change --version)" >> /opt/sc-evals/arms/versions.txt
