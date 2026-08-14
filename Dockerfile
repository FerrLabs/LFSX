FROM docker.io/library/rust:1.88-bookworm AS builder

WORKDIR /src

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY server ./server

RUN cargo build --release --locked --bin lfsx-server && mkdir -p /var/lib/lfsx

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /src/target/release/lfsx-server /usr/local/bin/lfsx-server
COPY --from=builder --chown=65532:65532 /var/lib/lfsx /var/lib/lfsx

ENV LFSX_STORAGE_ROOT=/var/lib/lfsx

EXPOSE 8080
USER nonroot
ENTRYPOINT ["/usr/local/bin/lfsx-server"]
