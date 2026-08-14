<div align="center">

# LFSX

**A fast, lightweight, secure Git LFS server.**

[![CI](https://github.com/FerrLabs/LFSX/actions/workflows/ci.yml/badge.svg)](https://github.com/FerrLabs/LFSX/actions/workflows/ci.yml)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)

</div>

LFSX stores the large binaries of a Git repository — game assets, textures, models, video — so
they never touch your host's LFS quota. Your repository stays on GitHub, GitLab or anywhere else;
only the LFS transfer is redirected to your own server.

> [!WARNING]
> **Not production-ready.** Authentication is not implemented yet, so the server currently accepts
> every request. Run it on a trusted network only. See [Roadmap](#roadmap).

## Why

GitHub bills LFS storage and bandwidth separately from your plan, and a Unity or Unreal project
burns through the free tier in a single push. A 3 GB asset pack cloned by a CI job ten times a
month is 30 GB of metered traffic. Self-hosting removes the meter entirely — the cost becomes a
disk you already own.

LFSX is built around three properties:

**Fast.** Uploads and downloads stream end to end. Nothing is buffered in memory, so a
multi-gigabyte asset costs the same resident memory as a one-kilobyte icon. The SHA-256 is computed
on the bytes as they pass, not in a second read of the file.

**Lightweight.** One statically linked binary, one crate, a distroless image, no database. Objects
live on the filesystem, addressed by digest.

**Secure.** Every uploaded object is verified against its declared digest before being accepted.
Writes are atomic, so an interrupted transfer can never leave a corrupt object behind. Object
identifiers are validated before they reach the filesystem, so a crafted oid cannot escape the
storage root.

## Quick start

### Run it

```bash
docker run -d --name lfsx \
  -p 8080:8080 \
  -v lfsx-data:/var/lib/lfsx \
  -e LFSX_PUBLIC_URL=https://lfs.example.com \
  ghcr.io/ferrlabs/lfsx:latest
```

Or from source:

```bash
LFSX_STORAGE_ROOT=./data LFSX_PUBLIC_URL=http://localhost:8080 cargo run --release
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
> stubs and tools that read them — Unity, Unreal, image editors — fail in confusing ways.

### Verify it works

```bash
curl -sf https://lfs.example.com/health && echo " up"
```

## Configuration

All configuration is by environment variable.

| Variable | Default | Purpose |
|---|---|---|
| `LFSX_BIND` | `0.0.0.0:8080` | listen address |
| `LFSX_STORAGE_ROOT` | `/var/lib/lfsx` | root of the object store |
| `LFSX_PUBLIC_URL` | `http://<bind>` | public URL used to build transfer links |
| `RUST_LOG` | `info` | log filter (`tracing_subscriber` syntax) |

`LFSX_PUBLIC_URL` must match the URL the client actually reaches. It is echoed in the batch
response, and the client reconnects to it for every object — if it is wrong, negotiation succeeds
and every transfer then fails.

## Storage layout

Objects are content-addressed and fanned out two levels to keep directories small:

```
$LFSX_STORAGE_ROOT/<org>/<repo>/<oid[0:2]>/<oid[2:4]>/<oid>
```

Backing up the server is backing up that directory. Objects are immutable, so an incremental
file-level backup never rewrites what it already copied.

## API

The Git LFS protocol is small — four routes, plus a health check:

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/{org}/{repo}/objects/batch` | negotiation: the client announces its objects, the server answers per object with an upload or download link |
| `PUT` | `/{org}/{repo}/objects/{oid}` | store an object |
| `GET` | `/{org}/{repo}/objects/{oid}` | retrieve an object |
| `POST` | `/{org}/{repo}/objects/verify` | post-upload verification |
| `GET` | `/health` | liveness |

Objects already present are returned by `batch` with no actions, so the client skips re-uploading
them. Missing objects on a download are reported per object with a `404` error rather than failing
the whole batch.

The locking API is not implemented: git-lfs probes it, finds it missing, and falls back to
`lfs.locksverify false` on its own.

## Reverse proxy

Terminate TLS in front of LFSX. A minimal Traefik ingress:

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: lfsx
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
spec:
  ingressClassName: traefik
  tls:
    - hosts: [lfs.example.com]
      secretName: lfsx-tls
  rules:
    - host: lfs.example.com
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: lfsx
                port:
                  number: 80
```

Do not add a request body size limit or a buffering middleware — LFS transfers are large and must
stream. Traefik's `buffering` middleware in particular will break uploads.

> [!CAUTION]
> **Do not put HTTP authentication on the proxy.** It cannot work, and the failure is confusing:
> the batch call authenticates fine, then every object transfer loops on `401`. See
> [Why authentication cannot live in the proxy](#why-authentication-cannot-live-in-the-proxy).

## Roadmap

Authentication is the one thing standing between LFSX and a public deployment. Tracked in
[the issues](https://github.com/FerrLabs/LFSX/issues).

### Why authentication cannot live in the proxy

This is not obvious, and it rules out the approach most people reach for first.

The batch response carries the URLs the client will use for each object transfer. If the server
advertises them as already authenticated — `"authenticated": true` — without supplying an
`Authorization` header alongside, git-lfs treats those URLs as pre-signed storage links and calls
them with **no credentials at all**. Behind an authenticating proxy, every one of those calls
returns `401`, and the client retries in a loop.

This is exactly what makes [rudolfs](https://github.com/jasonwhite/rudolfs) unusable behind
Traefik BasicAuth: it answers `"authenticated": true` with `"header": null` and offers no way to
change that.

LFSX never emits `authenticated`, so the client authenticates each transfer itself. A test pins
the behaviour down — `batch_never_claims_the_transfer_is_pre_authenticated` — and it will fail if
anyone reintroduces the field.

Authentication therefore has to live in the server. The intended design mirrors the permissions of
the upstream Git repository: the client presents the same token it would use to clone over HTTPS,
the server checks that token's rights on the repository, and derives read or write access from
them. No separate accounts, no shared password, and access is revoked by removing someone from the
repository.

## Development

```bash
cargo test                              # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Integration tests mount the router on a temporary directory and drive it through
`tower::ServiceExt::oneshot`, so they exercise real routing, real streaming and the real
filesystem without binding a port.

## License

[MPL-2.0](LICENSE).
