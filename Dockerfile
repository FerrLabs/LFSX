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

# The binary is linked against musl so the runtime image can be the static one,
# which is the difference between ten megabytes of glibc and none. ring and
# zstd are C, so cross-compiling needs a C compiler for the target, and Debian
# packages no aarch64 musl toolchain: `zig cc` is one download that targets
# both architectures, where apt would only solve the amd64 half.
ARG ZIG_VERSION=0.14.1
ARG ZIGBUILD_VERSION=0.23.4
RUN set -eux; \
    url="https://ziglang.org/download/${ZIG_VERSION}/zig-x86_64-linux-${ZIG_VERSION}.tar.xz"; \
    curl -fsSL "$url" -o /tmp/zig.tar.xz; \
    mkdir -p /opt/zig; \
    tar -xJf /tmp/zig.tar.xz -C /opt/zig --strip-components=1; \
    ln -s /opt/zig/zig /usr/local/bin/zig; \
    rm /tmp/zig.tar.xz; \
    cargo install cargo-zigbuild --version "${ZIGBUILD_VERSION}" --locked

# zig keeps a compilation cache and will not work without a writable one. Under
# the rootless builder CI uses, the default under $HOME was not writable, and
# the link died on "sub-compilation of libunwind failed: CacheCheckFailed".
ENV ZIG_GLOBAL_CACHE_DIR=/tmp/zig-cache
ENV ZIG_LOCAL_CACHE_DIR=/tmp/zig-cache

RUN --mount=type=secret,id=gha-cache-url \
    --mount=type=secret,id=gha-runtime-token \
    set -eu ; \
    case "${TARGETARCH}" in \
        amd64) target=x86_64-unknown-linux-musl ;; \
        arm64) target=aarch64-unknown-linux-musl ;; \
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
    cargo zigbuild --release --locked --bin lfsx-server --target "${target}" ; \
    sccache --show-stats 2>/dev/null || true ; \
    install -D "target/${target}/release/lfsx-server" /out/lfsx-server ; \
    mkdir -p /out/storage

# Static rather than cc, which exists to carry glibc and libgcc for a binary
# that no longer needs either. The trust roots are compiled in through
# webpki-roots, so nothing here reads a system store.
FROM gcr.io/distroless/static-debian12:nonroot

COPY --from=builder /out/lfsx-server /usr/local/bin/lfsx-server
COPY --from=builder --chown=65532:65532 /out/storage /var/lib/lfsx

ENV LFSX_STORAGE_ROOT=/var/lib/lfsx

EXPOSE 8080
USER nonroot
ENTRYPOINT ["/usr/local/bin/lfsx-server"]
