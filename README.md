<div align="center">

<img src=".github/logo.svg" width="96" alt="" />

# LFSX

**Your assets. Your disk.**

A fast, lightweight, secure Git LFS server.

[![CI](https://github.com/FerrLabs/LFSX/actions/workflows/ci.yml/badge.svg)](https://github.com/FerrLabs/LFSX/actions/workflows/ci.yml)
[![Coverage](https://sonar.ferrlabs.com/api/project_badges/measure?project=lfsx&metric=coverage&token=sqb_623f2242cd5fcf0124a37f3be11f1bae955d2607)](https://sonar.ferrlabs.com/dashboard?id=lfsx)
[![Quality Gate](https://sonar.ferrlabs.com/api/project_badges/measure?project=lfsx&metric=alert_status&token=sqb_623f2242cd5fcf0124a37f3be11f1bae955d2607)](https://sonar.ferrlabs.com/dashboard?id=lfsx)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)
[![Artifact Hub](https://img.shields.io/endpoint?url=https://artifacthub.io/badge/repository/lfsx)](https://artifacthub.io/packages/search?repo=lfsx)
[![Socket Badge](https://badge.socket.dev/cargo/package/lfsx/latest)](https://socket.dev/cargo/package/lfsx)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/FerrLabs/LFSX/badge)](https://scorecard.dev/viewer/?uri=github.com/FerrLabs/LFSX)

[Quick start](#quick-start) | [Documentation](https://lfsx.dev) | [Helm chart](chart/) | [Releases](https://github.com/FerrLabs/LFSX/releases)

</div>

LFSX stores the large binaries of a Git repository (game assets, textures, models, video) so
they never touch your host's LFS quota. Your repository stays on GitHub, GitLab or anywhere else;
only the LFS transfer is redirected to your own server.

Access is decided by the upstream repository: a client presents the token it would use to clone
over HTTPS, and LFSX asks the forge what that token is allowed to do. There are no accounts to
manage. See [Authentication](docs/authentication.md).

## Why

GitHub bills LFS storage and bandwidth separately from your plan, and a Unity or Unreal project
burns through the free tier in a single push. A 3 GB asset pack cloned by a CI job ten times a
month is 30 GB of metered traffic. Self-hosting removes the meter entirely: the cost becomes a
disk you already own.

LFSX is built around three properties:

**Fast.** Uploads and downloads stream end to end. Nothing is buffered in memory, so a
multi-gigabyte asset costs the same resident memory as a one-kilobyte icon. The SHA-256 is computed
on the bytes as they pass, not in a second read of the file. [Measured](docs/performance.md), not asserted.

**Lightweight.** One statically linked binary, a distroless image, no database. Objects live on
the filesystem addressed by digest, or in an S3-compatible bucket.

**Secure.** Access mirrors the permissions of the upstream Git repository, so revoking someone
there revokes them here. Every uploaded object is verified against its declared digest before being
accepted. Writes are atomic, so an interrupted transfer can never leave a corrupt object behind.
Object identifiers and repository names are validated before they reach the filesystem, so a
crafted request cannot escape the storage root.

## Quick start

### Run it

```bash
docker run -d --name lfsx \
  -p 8080:8080 \
  -v lfsx-data:/var/lib/lfsx \
  -e LFSX_PUBLIC_URL=https://lfs.example.com \
  ghcr.io/ferrlabs/lfsx:latest
```

Or without a container runtime, from the binaries attached to each
[release](https://github.com/FerrLabs/LFSX/releases), statically linked, so they need nothing
installed:

```bash
curl -fsSL https://github.com/FerrLabs/LFSX/releases/latest/download/lfsx-server-x86_64-unknown-linux-musl.tar.gz \
  | tar xz
LFSX_PUBLIC_URL=https://lfs.example.com ./lfsx-server
```

Replace `x86_64` with `aarch64` on a Raspberry Pi or an ARM server; `gnu` builds are there too if
you prefer dynamic linking. Every archive ships a `.sha256` next to it.

From crates.io, if you already have a Rust toolchain and would rather compile:

```bash
cargo install lfsx-server
```

Or from source, without the permission check, for a local trusted network:

```bash
LFSX_STORAGE_ROOT=./data LFSX_PUBLIC_URL=http://localhost:8080 LFSX_AUTH=disabled cargo run --release
```

### Point a repository at it

Commit a `.lfsconfig` at the root of the repository so every clone picks it up:

```ini
[lfs]
	url = https://lfs.example.com/my-org/my-project
```

The last two path segments are the organisation and the project; together they scope the storage.
Two repositories sharing a URL share their objects.

Then use Git LFS as usual:

```bash
git lfs install
git lfs track "*.psd"
git add .gitattributes assets/hero.psd
git commit -m "add hero artwork"
git push
```

> [!IMPORTANT]
> `git lfs install` must be run **before cloning**. Without it, files arrive as 130-byte pointer
> stubs and tools that read them (Unity, Unreal, image editors) fail in confusing ways.

### Verify it works

```bash
lfsx --url https://lfs.example.com doctor --repo my-org/my-project
```

```bash
npm install -g @ferrlabs/lfsx        # or: cargo install lfsx
```

That checks the server is up, its storage is writable, your token is accepted, and that the URL it
advertises for transfers is the one you reached it on: the mismatch that lets negotiation succeed
while every transfer fails. Install it alongside the server, or use the probes directly:

```bash
curl -sf https://lfs.example.com/health && echo " up"
curl -sf https://lfs.example.com/ready  && echo " serving"
```

`/health` says the process is alive, which is what a restart would fix. `/ready` writes and removes
a probe file under the storage root, so a volume that is missing, full or mounted read-only takes
the instance out of rotation instead of accepting traffic it cannot serve. Point `livenessProbe` at
the first and `readinessProbe` at the second. Neither needs credentials.

## Documentation

Full documentation lives at **[lfsx.dev](https://lfsx.dev)**. The same pages are in
[`docs/`](docs/) if you would rather read them here.

| | |
|---|---|
| [Configuration](docs/configuration.md) | Every environment variable, with defaults |
| [Authentication](docs/authentication.md) | How access mirrors the upstream repository |
| [Anonymous read](docs/anonymous-read.md) | Public repositories, and turning it off |
| [Storage layout](docs/storage-layout.md) | Where the bytes live, and why once |
| [Objects in a bucket](docs/buckets.md) | S3-compatible storage |
| [Compression](docs/compression.md), [Encryption at rest](docs/encryption.md) | What they cost and what they protect |
| [Locking](docs/locking.md) | File locks for assets that cannot be merged |
| [Reclaiming space](docs/reclaiming-space.md) | Collection, deduplication, verification |
| [Size limits](docs/size-limits.md) | Per-object ceilings and repository quotas |
| [Observability](docs/observability.md) | Metrics, traces, the audit trail, health and readiness |
| [Operations](docs/operations.md) | Backups, restores, and what to check when it breaks |
| [Kubernetes](docs/kubernetes.md), [Reverse proxy](docs/reverse-proxy.md) | Deploying it |
| [API](docs/api.md), [Protocol coverage](docs/protocol.md) | What it answers, and what it does not |
| [Clients](docs/clients.md) | Which Git LFS clients are exercised |
| [Performance](docs/performance.md) | Measured, not asserted |
| [Inspecting a repository](docs/inspecting.md), [Releases](docs/releases.md) | |

## Development

```bash
cargo test                              # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Integration tests mount the router on a temporary directory and drive it through
`tower::ServiceExt::oneshot`, so they exercise real routing, real streaming and the real
filesystem without binding a port.

```bash
bash ci/e2e.sh                          # push and clone through a real git lfs client
```

That one starts the binary and a stub forge, pushes a large asset with the actual client, clones it
back and compares the bytes, then takes a lock, fails to steal it and releases it. It runs on an
isolated `GIT_CONFIG_GLOBAL`, so it cannot touch your own git configuration.

It runs on every push against **Linux, macOS and Windows**, plus **git-lfs 3.0.2**, the oldest
version supported, since that is where the locking API settled. The clients a studio actually runs
are rarely the newest: Git for Windows, GitHub Desktop, Sourcetree, Rider and Unity each bundle
their own copy. [`docs/clients.md`](docs/clients.md) records what is covered and carries a short
manual checklist for the graphical clients, which cannot be automated and are what the artists
will be using.

```bash
cargo llvm-cov --workspace --lcov --output-path lcov.info
```

The same command CI runs before handing the report to
[sonar.ferrlabs.com](https://sonar.ferrlabs.com), which tracks coverage, duplication and smells
over time. A pull request is analysed into its own project and the workflow comments what that
change introduced, since the Community edition has no pull-request analysis of its own.

## License

[MPL-2.0](LICENSE).
