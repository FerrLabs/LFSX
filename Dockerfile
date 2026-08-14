FROM --platform=$BUILDPLATFORM docker.io/library/rust:1.88-bookworm AS builder

ARG TARGETARCH

WORKDIR /src

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY server ./server
COPY cli ./cli

RUN case "${TARGETARCH}" in \
        amd64) target=x86_64-unknown-linux-gnu ;; \
        arm64) target=aarch64-unknown-linux-gnu ; \
               apt-get update ; \
               apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu libc6-dev-arm64-cross ; \
               rm -rf /var/lib/apt/lists/* ; \
               export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc ;; \
        *) echo "unsupported architecture: ${TARGETARCH}" >&2 ; exit 1 ;; \
    esac ; \
    rustup target add "${target}" ; \
    cargo build --release --locked --bin lfsx-server --target "${target}" ; \
    install -D "target/${target}/release/lfsx-server" /out/lfsx-server ; \
    mkdir -p /out/storage

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /out/lfsx-server /usr/local/bin/lfsx-server
COPY --from=builder --chown=65532:65532 /out/storage /var/lib/lfsx

ENV LFSX_STORAGE_ROOT=/var/lib/lfsx

EXPOSE 8080
USER nonroot
ENTRYPOINT ["/usr/local/bin/lfsx-server"]
