<div align="center">

# LFSX

**A fast, lightweight, secure Git LFS server.**

[![CI](https://github.com/FerrLabs/LFSX/actions/workflows/ci.yml/badge.svg)](https://github.com/FerrLabs/LFSX/actions/workflows/ci.yml)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)

</div>

LFSX stores the large binaries of a Git repository — game assets, textures, models, video — so
they never touch your host's LFS quota. Your repository stays on GitHub, GitLab or anywhere else;
only the LFS transfer is redirected to your own server.

Access is decided by the upstream repository: a client presents the token it would use to clone
over HTTPS, and LFSX asks the forge what that token is allowed to do. There are no accounts to
manage. See [Authentication](#authentication).

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
> stubs and tools that read them — Unity, Unreal, image editors — fail in confusing ways.

### Verify it works

```bash
curl -sf https://lfs.example.com/health && echo " up"
curl -sf https://lfs.example.com/ready  && echo " serving"
```

`/health` says the process is alive, which is what a restart would fix. `/ready` writes and removes
a probe file under the storage root, so a volume that is missing, full or mounted read-only takes
the instance out of rotation instead of accepting traffic it cannot serve. Point `livenessProbe` at
the first and `readinessProbe` at the second. Neither needs credentials.

## Configuration

All configuration is by environment variable.

| Variable | Default | Purpose |
|---|---|---|
| `LFSX_BIND` | `0.0.0.0:8080` | listen address |
| `LFSX_STORAGE_ROOT` | `/var/lib/lfsx` | root of the object store |
| `LFSX_PUBLIC_URL` | `http://<bind>` | public URL used to build transfer links |
| `LFSX_AUTH` | `github` | permission source, or `disabled` to accept every request |
| `LFSX_GITHUB_API_URL` | `https://api.github.com` | API root, point it at your GitHub Enterprise host |
| `LFSX_AUTH_CACHE_TTL` | `60` | seconds a granted permission is reused before being checked again |
| `LFSX_AUTH_REJECTION_TTL` | `10` | seconds a refusal is remembered, so a bad token cannot hammer the forge |
| `LFSX_GC_GRACE` | `1209600` | seconds an object must have been untouched before collection can take it |
| `RUST_LOG` | `info` | log filter (`tracing_subscriber` syntax) |

`LFSX_PUBLIC_URL` must match the URL the client actually reaches. It is echoed in the batch
response, and the client reconnects to it for every object — if it is wrong, negotiation succeeds
and every transfer then fails.

`LFSX_AUTH=disabled` turns the server into an open one. It exists for local development and closed
networks, it is logged loudly at startup, and it is never the right setting for anything reachable
from the internet.

## Observability

`/metrics` serves the Prometheus text format, unauthenticated like the probes — an orchestrator
scraping it has no forge token, and refusing it would only mean the one moment you need numbers is
the one moment you cannot get them.

| Metric | Kind | Answers |
|---|---|---|
| `lfsx_requests_total{route,status}` | counter | what is being served, and what is failing |
| `lfsx_request_duration_seconds{route}` | histogram | how long transfers take |
| `lfsx_uploaded_bytes_total`, `lfsx_downloaded_bytes_total` | counter | throughput in and out |
| `lfsx_object_size_bytes` | histogram | what people are actually storing |
| `lfsx_rejections_total{cause}` | counter | why requests are refused, by cause rather than by status |
| `lfsx_objects_stored`, `lfsx_store_bytes` | gauge | how full the disk is getting |

Routes are labelled by their template, never by the path, so the object id can never turn into a
label and the series count stays bounded whatever you store.

The two gauges are measured by walking the store, so they are computed at most once a minute and
reused in between. Scraping every fifteen seconds costs nothing extra.

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
| `POST` | `/{org}/{repo}/objects/retain` | reclaim space, see [Reclaiming space](#reclaiming-space) |
| `POST` | `/{org}/{repo}/locks` | take a lock on a path |
| `GET` | `/{org}/{repo}/locks` | list locks, filterable by `path` or `id` |
| `POST` | `/{org}/{repo}/locks/verify` | the client's own locks, and everyone else's |
| `POST` | `/{org}/{repo}/locks/{id}/unlock` | release a lock |
| `GET` | `/metrics` | Prometheus exposition |
| `GET` | `/health` | liveness: the process is up |
| `GET` | `/ready` | readiness: the storage root is writable |

Objects already present are returned by `batch` with no actions, so the client skips re-uploading
them. Missing objects on a download are reported per object with a `404` error rather than failing
the whole batch.

## Locking

Binary assets cannot be merged. Two artists editing the same `.psd` or the same Unity scene means
one of them loses work, and locking is the only mechanism Git offers to stop that happening. It is
the difference between LFS being usable for a game project and being a hazard.

```bash
git lfs lock Assets/Scenes/Arena.unity
git lfs locks
git lfs unlock Assets/Scenes/Arena.unity
```

A lock belongs to the identity behind the token, resolved from the forge, so `git lfs locks` names
the person to go and talk to. Taking a lock someone else holds is refused with their name attached,
rather than silently overwritten.

Only the owner can release a lock. Anyone else needs `--force`, and force needs **admin** rights on
the repository — the same person who could rewrite the branch anyway. A colleague on holiday with a
scene locked is a real situation, and this is the escape hatch for it.

Locks live next to the objects, in `$LFSX_STORAGE_ROOT/.locks/`, so they are covered by the same
backup and disappear with the repository.

## Reclaiming space

Objects are written and never removed on their own. A repository that rewrites history, drops a
branch or replaces a large asset leaves the old blobs behind, and the disk only grows.

The server cannot decide what is still needed — it never sees your Git history. So you tell it.
`retain` takes the set of object ids the repository still references and sweeps everything else:

```bash
git lfs ls-files --all --long \
  | cut -d' ' -f1 \
  | jq -Rs '{ oids: (split("\n") - [""]), dry_run: true }' \
  | curl -sS -u "git:$GITHUB_TOKEN" -H 'content-type: application/json' \
      --data @- https://lfs.example.com/my-org/my-project/objects/retain
```

`--all` walks every ref, so the set covers every commit still reachable — which is the point: run
this from a full clone, not a shallow one, or you will retain a fraction of what you should.

It answers with what it would free, and frees nothing until you drop `dry_run`:

```json
{ "swept": 42, "bytes": 3221225472, "within_grace": 3, "dry_run": true }
```

Two safeguards, because this deletes data. An object is uploaded *before* the commit referencing
it is pushed, so anything touched within `LFSX_GC_GRACE` (two weeks by default, matching git's own
`gc.pruneExpire`) is never taken — that is the `within_grace` count. And a transfer still in
flight is skipped, since staging files are not objects yet.

Both are worth understanding before you shorten the grace period: an empty `oids` set legitimately
means *nothing is referenced any more*, and outside the grace window that sweeps the repository
clean. Run it dry first. Collection needs push rights on the repository.

## Kubernetes

A chart lives in [`chart/`](chart/) and is published to the same registry as the image:

```bash
helm install lfsx oci://ghcr.io/ferrlabs/charts/lfsx   --set ingress.enabled=true   --set ingress.className=nginx   --set ingress.host=lfs.example.com
```

It encodes the things that are easy to get wrong: `LFSX_PUBLIC_URL` derived from the ingress host,
the nginx annotations that keep large uploads from being rejected, a single replica over a
`ReadWriteOnce` volume with the `Recreate` strategy, and probes on `/health` and `/ready`. See
[chart/README.md](chart/README.md).

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

## Authentication

LFSX does not manage accounts. It asks the forge that hosts the repository what the caller is
allowed to do, so the answer is always the same one the repository gives.

The client presents the token it would use to clone over HTTPS — a personal access token, or the
`GITHUB_TOKEN` a CI job already has — as the password of an HTTP Basic credential, or as a bearer
token. LFSX resolves it against `GET /repos/{org}/{repo}` and maps the result:

| Rights on the repository | Objects |
|---|---|
| admin | download, upload, and force a lock open |
| push | download, upload, and take locks |
| pull only | download |
| none, or an unusable token | rejected |

`/health` stays open. Everything under `/{org}/{repo}/objects/` requires a token, and each answer
is cached for `LFSX_AUTH_CACHE_TTL` seconds so a push of two hundred objects costs one API call
rather than two hundred. That cache is also the delay before a revocation takes effect — shorten
it if that matters more than the round trips.

Refusals are remembered too, for the shorter `LFSX_AUTH_REJECTION_TTL`. Without that, a CI job
retrying with a revoked token spends one API call per attempt, forever, against the same budget
the server needs for real lookups — and an unauthenticated caller could drive that load on
purpose. The window is short on purpose: it is how long you keep being refused after being granted
access. A forge that cannot be reached is never cached, so an outage stays an outage rather than
becoming a lasting denial.

Git already sends the token if it is in the credential store for that host:

```bash
git config --global credential.https://lfs.example.com.username git
printf 'protocol=https\nhost=lfs.example.com\nusername=git\npassword=%s\n' "$GITHUB_TOKEN" \
  | git credential approve
```

In CI, the token the workflow already holds is enough — no secret to provision:

```yaml
- run: |
    printf 'protocol=https\nhost=lfs.example.com\nusername=git\npassword=%s\n' "${{ secrets.GITHUB_TOKEN }}" \
      | git credential approve
    git lfs pull
```

Only GitHub is supported today. GitLab and Gitea are tracked in
[#2](https://github.com/FerrLabs/LFSX/issues/2), and the rest of the roadmap is in
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

Authentication therefore lives in the server, which is what the section above describes.

## Releases

Versions are SemVer, and the major stays at `0` while the surface is still settling — a `0.x` bump
is free to change environment variables, the storage layout or the API. Releases are cut by
[FerrFlow](https://ferrflow.com) from the Conventional Commit history of `main` — a merged `feat:`
or `fix:` produces the tag, the [`CHANGELOG.md`](CHANGELOG.md) entry and the GitHub release, and
the release builds and pushes the image.

Images live at `ghcr.io/ferrlabs/lfsx` under three tags:

| Tag | Moves | For |
|---|---|---|
| `0.4.0` | never | production, where an upgrade should be a deliberate change |
| `0.4` | on every fix to that line | picking up fixes without picking up behaviour changes |
| `latest` | on every release | trying it out |

Every tag is a manifest list covering `linux/amd64` and `linux/arm64`, so a NAS, a Raspberry Pi or
a Graviton instance pulls the right image without being told. Each one is scanned for known
vulnerabilities, has to answer `/health` before it is allowed out, and is then signed with cosign
and shipped with a CycloneDX SBOM.

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

That one starts the binary and a stub forge, pushes a 64 MiB asset with the actual client, clones
it back and compares the bytes. It runs on an isolated `GIT_CONFIG_GLOBAL`, so it cannot touch
your own git configuration.

## License

[MPL-2.0](LICENSE).
