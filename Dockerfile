FROM --platform=$BUILDPLATFORM docker.io/library/rust:1.88-bookworm AS builder

ARG TARGETARCH

WORKDIR /src

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY server ./server
COPY cli ./cli

# A release bumps the version line in server/Cargo.toml, which is an input of
# this layer, so the registry layer cache is invalidated on exactly the builds
# that matter and every dependency recompiles. sccache keys on the compiler
# invocation instead and survives that. The backend is the Actions cache of the
# run, since the garage one the self-hosted pool uses does not resolve here.
ARG SCCACHE_VERSION=v0.16.0
ENV CARGO_INCREMENTAL=0
RUN set -eux; \
    url="https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/sccache-${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz"; \
    curl -fsSL "$url" -o /tmp/sccache.tar.gz; \
    tar -xzf /tmp/sccache.tar.gz -C /tmp; \
    mv /tmp/sccache-*/sccache /usr/local/bin/sccache; \
    chmod +x /usr/local/bin/sccache; \
    rm -rf /tmp/sccache*

# `set -eu` because this chain runs on `;`: without it a failed build exported a
# layer with no binary in it, and the error surfaced three steps later as a
# `COPY --from=builder ... not found` that named neither the cause nor the step.
RUN --mount=type=secret,id=gha-cache-url \
    --mount=type=secret,id=gha-runtime-token \
    set -eu ; \
    case "${TARGETARCH}" in \
        amd64) target=x86_64-unknown-linux-gnu ;; \
        arm64) target=aarch64-unknown-linux-gnu ; \
               apt-get update ; \
               apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu libc6-dev-arm64-cross ; \
               rm -rf /var/lib/apt/lists/* ; \
               export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc ;; \
        *) echo "unsupported architecture: ${TARGETARCH}" >&2 ; exit 1 ;; \
    esac ; \
    ACTIONS_CACHE_URL="$(cat /run/secrets/gha-cache-url 2>/dev/null || true)" ; \
    ACTIONS_RUNTIME_TOKEN="$(cat /run/secrets/gha-runtime-token 2>/dev/null || true)" ; \
    export ACTIONS_CACHE_URL ACTIONS_RUNTIME_TOKEN ; \
    if [ -n "${ACTIONS_RUNTIME_TOKEN}" ] ; then \
        export SCCACHE_GHA_ENABLED=true ; \
        sccache --start-server >/dev/null 2>&1 && export RUSTC_WRAPPER=sccache ; \
    fi ; \
    echo "sccache: ${RUSTC_WRAPPER:-not in use}" ; \
    rustup target add "${target}" ; \
    cargo build --release --locked --bin lfsx-server --target "${target}" ; \
    sccache --show-stats 2>/dev/null || true ; \
    install -D "target/${target}/release/lfsx-server" /out/lfsx-server ; \
    mkdir -p /out/storage

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /out/lfsx-server /usr/local/bin/lfsx-server
COPY --from=builder --chown=65532:65532 /out/storage /var/lib/lfsx

ENV LFSX_STORAGE_ROOT=/var/lib/lfsx

EXPOSE 8080
USER nonroot
ENTRYPOINT ["/usr/local/bin/lfsx-server"]
