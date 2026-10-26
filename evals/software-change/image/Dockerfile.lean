# The lean-change provider layered on the Stage 2 arms image. Engine, software-change provider, Pi and the owner context
# stay the base image's bytes (the engine crates are identical between 8aeedb5 and the provider's base).
# Build from this directory with source.tar holding the tracked tree: docker build -f Dockerfile.lean -t TAG .
ARG BASE_IMAGE=loop-engine-sc-evals:arms-s2-8aeedb5ee
FROM rust:1.98.0-bookworm AS build
ARG TARGETARCH
WORKDIR /src
COPY source.tar /tmp/source.tar
RUN test "$TARGETARCH" = arm64 && tar -xf /tmp/source.tar -C /src \
    && cargo build --locked --release -j 1 -p lean-change-provider

ARG LEAN_SOURCE_SHA
FROM ${BASE_IMAGE}
ARG LEAN_SOURCE_SHA
LABEL com.loop-engine.lean-change.source-sha256=$LEAN_SOURCE_SHA
COPY --from=build /src/target/release/lean-change /usr/local/bin/lean-change
COPY --from=build /src/crates/lean-change-provider/skills/using-lean-change-provider /opt/sc-evals/skills/using-lean-change-provider
# pi-role.py loads the provider skill named by SC_PROVIDER_SKILL (default: software-change).
COPY pi-role.py /opt/sc-evals/pi-role.py
RUN chmod 755 /opt/sc-evals/pi-role.py \
    && lean-change --version \
    && test -s /opt/sc-evals/skills/using-lean-change-provider/SKILL.md \
    && printf '%s\n' "lean-change $(lean-change --version)" >> /opt/sc-evals/arms/versions.txt
