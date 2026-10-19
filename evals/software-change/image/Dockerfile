# Source is exported by scevals.image from this checkout at LOOP_ENGINE_REF.
FROM rust:1.98.0-bookworm AS build
ARG TARGETARCH
ARG LOOP_ENGINE_REF=v0.26.0
ARG LOOP_ENGINE_COMMIT
# Optional eval-only memory bound (empty = cargo default parallelism, unchanged behaviour).
ARG SC_CARGO_JOBS=
WORKDIR /src
COPY source.tar /tmp/source.tar
RUN test "$TARGETARCH" = arm64 && tar -xf /tmp/source.tar -C /src \
    && cargo build --locked --release ${SC_CARGO_JOBS:+-j $SC_CARGO_JOBS} -p loop-cli -p software-change-provider

FROM node:24.18.1-bookworm
ARG TARGETARCH
ARG LOOP_ENGINE_REF=v0.26.0
ARG LOOP_ENGINE_COMMIT
LABEL org.opencontainers.image.version=$LOOP_ENGINE_REF \
      org.opencontainers.image.revision=$LOOP_ENGINE_COMMIT
RUN test "$TARGETARCH" = arm64 \
    && apt-get update \
    && apt-get install -y --no-install-recommends git python3 ca-certificates curl tini \
    && rm -rf /var/lib/apt/lists/*
# Official arm64 release, checked against the release's published checksum.
RUN curl -fsSL https://github.com/dagu-org/dagu/releases/download/v2.14.0/dagu_2.14.0_linux_arm64.tar.gz -o /tmp/dagu.tar.gz \
    && echo 'c1061503cfe729593ac5433b29e6959830cc2dad50205f1003f4287998ee0cd6  /tmp/dagu.tar.gz' | sha256sum -c - \
    && tar -xzf /tmp/dagu.tar.gz -C /usr/local/bin dagu \
    && rm /tmp/dagu.tar.gz
RUN npm install --global @earendil-works/pi-coding-agent@1.0.0 \
    && npm cache clean --force
# Rust 1.98.0 (cargo, rustc, clippy, rustfmt) for cases in Rust repositories.
# The node bookworm base already carries gcc and the C linker.
COPY --from=build /usr/local/rustup /usr/local/rustup
COPY --from=build /usr/local/cargo/bin /usr/local/cargo/bin
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN cargo --version && rustc --version && cargo clippy --version && cargo fmt --version
COPY --from=build /src/target/release/loop-engine /src/target/release/software-change /usr/local/bin/
COPY --from=build /src/skills/using-loop-engine /opt/sc-evals/skills/using-loop-engine
COPY --from=build /src/crates/software-change-provider/skills/using-software-change-provider /opt/sc-evals/skills/using-software-change-provider
COPY pi-role.py ask-owner.py /opt/sc-evals/
RUN chmod 755 /opt/sc-evals/pi-role.py /opt/sc-evals/ask-owner.py \
    && ln -s /opt/sc-evals/pi-role.py /usr/local/bin/sc-driver \
    && ln -s /opt/sc-evals/pi-role.py /usr/local/bin/sc-implement \
    && ln -s /opt/sc-evals/pi-role.py /usr/local/bin/sc-review \
    && ln -s /opt/sc-evals/ask-owner.py /usr/local/bin/ask-owner \
    && mkdir -p /workspace /opt/sc-evals/pi \
    && /opt/sc-evals/pi-role.py --write-templates
WORKDIR /workspace
# No VOLUME declarations, host paths, credentials or model calls.
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["sleep", "infinity"]
