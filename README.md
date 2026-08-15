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
on the bytes as they pass, not in a second read of the file. [Measured](#performance), not asserted.

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

Or without a container runtime, from the binaries attached to each
[release](https://github.com/FerrLabs/LFSX/releases) — statically linked, so they need nothing
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
> stubs and tools that read them — Unity, Unreal, image editors — fail in confusing ways.

### Verify it works

```bash
lfsx --url https://lfs.example.com doctor --repo my-org/my-project
```

```bash
npm install -g @ferrlabs/lfsx        # or: cargo install lfsx
```

That checks the server is up, its storage is writable, your token is accepted, and that the URL it
advertises for transfers is the one you reached it on — the mismatch that lets negotiation succeed
while every transfer fails. Install it alongside the server, or use the probes directly:

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
| `LFSX_PUBLIC_URL` | the requested host | public URL used to build transfer links |
| `LFSX_AUTH` | `github` | permission source: `github`, `gitlab`, or `disabled` to accept every request |
| `LFSX_GITHUB_API_URL` | `https://api.github.com` | API root, point it at your GitHub Enterprise host |
| `LFSX_GITLAB_API_URL` | `https://gitlab.com/api/v4` | API root, point it at your self-managed GitLab |
| `LFSX_AUTH_CACHE_TTL` | `60` | seconds a granted permission is reused before being checked again |
| `LFSX_AUTH_REJECTION_TTL` | `10` | seconds a refusal is remembered, so a bad token cannot hammer the forge |
| `LFSX_GC_GRACE` | `1209600` | seconds an object must have been untouched before collection can take it |
| `LFSX_STAGING_MAX_AGE` | `86400` | seconds before an abandoned upload's staging file is reclaimed |
| `LFSX_MAX_OBJECT_SIZE` | unlimited | bytes an object may reach before the server refuses it |
| `RUST_LOG` | `info` | log filter (`tracing_subscriber` syntax) |

`LFSX_PUBLIC_URL` is echoed in the batch response, and the client reconnects to it for every
object — if it is wrong, negotiation succeeds and every transfer then fails.

Left unset, the server answers on whatever host the request arrived at, honouring
`X-Forwarded-Proto` from the proxy in front. That is what you want when the same server is reached
under more than one name — a public host and an internal one, say — since a single fixed value
would be wrong for half the clients. Set it when you want to pin one name regardless of how the
request arrived; an explicit value always wins over the request.

`LFSX_AUTH=disabled` turns the server into an open one. It exists for local development and closed
networks, it is logged loudly at startup, and it is never the right setting for anything reachable
from the internet.

## Looking at a repository

Open `https://lfs.example.com/my-org/my-project` in a browser. It shows how many objects the
repository holds, how much disk they take, and what is locked and by whom — the questions that
otherwise need a shell.

There is no login screen and no session. The page sits behind the same permission check as every
transfer, so the browser asks for credentials itself and you give it the same token git uses. Read
access is enough to see it; nothing on the page changes anything, deletion stays an explicit API
call. `/{org}/{repo}/objects/stats` serves the same numbers as JSON.

## Performance

Numbers from `bench/throughput.sh`, run on a GitHub-hosted `ubuntu-latest` runner — Linux 6.17,
4 cores, 16 GiB, loopback, local disk. Rerun it yourself with `bash bench/throughput.sh`, or read
the [Benchmark workflow](.github/workflows/bench.yml) which publishes a table on every change to
the storage path.

| Measure | Result |
|---|---|
| Upload, 1 GiB single object | 117 MiB/s |
| Download, 1 GiB single object | 141 MiB/s |
| 1000 objects of 64 KiB, sequential | 1.4 ms per object, 45 MiB/s |
| Resident memory, idle → peak | 5 MiB → 6 MiB |

The memory row is the one worth looking at. A gigabyte moves through the process and its resident
set grows by one megabyte, which is what "nothing is buffered" means in practice rather than as a
claim. Upload is the slower direction because every byte is hashed and the object is flushed to
disk before it is acknowledged — that cost buys the guarantee that an accepted object is on disk
and matches its digest.

The small-object row is per-request overhead rather than bandwidth: at 64 KiB the transfer itself
is a fraction of a millisecond, so 1.4 ms is essentially what it costs to accept, verify, fsync and
rename one object. A Unity project pushing ten thousand small assets spends about fourteen seconds
of it.

No comparison against another implementation yet. Doing it honestly means driving both servers with
the same client rather than curl, since their object endpoints differ, and that harness does not
exist here — an unfair benchmark against a competitor is worse than none.

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
| `lfsx_objects_stored`, `lfsx_store_bytes` | gauge | how full the disk is getting, counting shared objects once |
| `lfsx_store_scans` | gauge | how often the expensive walk behind those two actually ran |

Routes are labelled by their template, never by the path, so the object id can never turn into a
label and the series count stays bounded whatever you store.

Those two count what the disk holds, not what the repositories logically hold: an object shared by
three projects is one set of bytes and is counted once. The per-repository page reports logical
size instead, since "this project uses 3 GiB of assets" is the useful answer there even when some
of it is shared.

The two disk gauges are measured by walking the store, so they are computed at most once a minute
and reused in between — and concurrent scrapes queue behind a single walk rather than each starting
their own, which is what keeps an unauthenticated endpoint from being a lever on a large disk.
`lfsx_store_scans` is how you check that: it should climb about once a minute under load, not once
per request.

`lfsx_downloaded_bytes_total` counts bytes as they are streamed, so a client that disconnects
halfway is not recorded as a full download.

## Storage layout

Objects are content-addressed and fanned out two levels to keep directories small:

```
$LFSX_STORAGE_ROOT/.content/<oid[0:2]>/<oid[2:4]>/<oid>   the bytes, once
$LFSX_STORAGE_ROOT/<org>/<repo>/<oid[0:2]>/<oid[2:4]>/<oid>   a hard link per repository
```

**The bytes are stored once.** Two projects sharing the same Synty or Quixel pack cost the disk
once, however many repositories push it — and for a studio that is most of the disk. Each
repository holds a hard link, so the filesystem keeps the reference count and the content survives
until the last repository lets go of it.

Sharing the bytes does not share them over the API. Every route resolves through the repository's
own path, so a repository cannot read, list or even learn the existence of an object it never
pushed — including by guessing a digest. `retain` frees space only when the object it drops was the
last reference; the report says zero bytes otherwise, rather than promising space another
repository is still using.

Objects already stored per repository, from before this, keep working untouched: they are ordinary
files with a single link, and nothing needs migrating.

Backing up the server is backing up that directory. Objects are immutable, so an incremental
file-level backup never rewrites what it already copied — but use a tool that preserves hard links
(`rsync -H`, `tar`), or the copy will expand every shared object back into a separate file.

[`docs/operations.md`](docs/operations.md) covers the rest of it: restoring, verifying the store
against its own digests afterwards, what a client sees when an object is missing and how to get it
back from a developer's cache, and migrating between servers.

## API

The Git LFS protocol is small — four routes, plus a health check:

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/{org}/{repo}/objects/batch` | negotiation: the client announces its objects, the server answers per object with an upload or download link |
| `PUT` | `/{org}/{repo}/objects/{oid}` | store an object |
| `GET` | `/{org}/{repo}/objects/{oid}` | retrieve an object, whole or by `Range` |
| `POST` | `/{org}/{repo}/objects/verify` | post-upload verification |
| `GET` | `/{org}/{repo}` | a page showing what the repository holds |
| `GET` | `/{org}/{repo}/objects/stats` | the same numbers as JSON |
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

Downloads honour `Range`, so a transfer that drops at 90% of a three-gigabyte asset resumes from
where it stopped instead of starting over — which on a home upstream is the difference between an
annoyance and an afternoon. A range that cannot be satisfied is refused with `416` carrying the
object's real size; a range we cannot parse is ignored and the whole object is served, since
refusing a transfer over a malformed header would be worse than the header.

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
The [`lfsx`](cli/) command does it from a clone:

```bash
lfsx --url https://lfs.example.com gc --repo my-org/my-project --dry-run
```

It refuses to run from a shallow clone, which would retain a fraction of what it should and sweep
the rest. Same command without `--dry-run` to actually free the space.

Under it is one endpoint, if you would rather call it yourself. `retain` takes the set of object
ids the repository still references and sweeps everything else:

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

Objects go, the fanout directories they lived in stay — an inode and a block per prefix, reused by
the next object that hashes into it. Removing them raced every upload: a push creates its fanout
directory, and until the staging file lands that directory is empty, so a collection running
alongside could take it and fail a push on a directory made moments earlier for that push.

An upload streams into a `.part` file next to its destination and is renamed on success. A process
kill or a host crash mid-transfer leaves one behind, and nothing used to reclaim it. The server now
sweeps them at boot — a crash is exactly what strands them — and hourly after that, logging the
count and the bytes it recovered.

`LFSX_STAGING_MAX_AGE` has to stay above the longest transfer you expect to serve: a `.part` file
younger than that is not litter, it is an upload in flight, and removing it would break a client
doing nothing wrong. A day is generous for a multi-gigabyte push on a home upstream.

Both are worth understanding before you shorten the grace period: an empty `oids` set legitimately
means *nothing is referenced any more*, and outside the grace window that sweeps the repository
clean. Run it dry first. Collection needs push rights on the repository.

## Size limits

`LFSX_MAX_OBJECT_SIZE` caps a single object, in bytes. Unset, there is no ceiling, which is fine
when the server has its volume to itself. Set it when it does not: an upload with no limit can fill
the disk, and a full disk fails every other repository on the server, so one careless push becomes
everyone's outage.

```bash
LFSX_MAX_OBJECT_SIZE=5368709120   # 5 GiB
```

The size is declared during batch negotiation, so an object over the ceiling is refused there —
before a byte moves — with a per-object error the client prints by name. The rest of the push goes
through; the limit refuses an object, not the commit it arrived with.

The transfer is capped as well, because the declared size is a claim by the client and the ceiling
has to hold against a body that ignores it. A stream that outgrows the limit is cut off at the
chunk that crosses it and the staging file is dropped, rather than read to the end to find out how
big it was.

Lowering the limit later does not strand what is already stored: it governs what may arrive, not
what a repository can still check out.

`LFSX_REPO_QUOTA` is the same idea one level up: a budget, in bytes, that any single `{org}/{repo}`
may hold. Unset, there is none.

```bash
LFSX_REPO_QUOTA=53687091200   # 50 GiB per repository
```

A per-object ceiling does not stop a project committing its renders directory a gigabyte at a time,
and on a server hosting a team the first symptom is unrelated repositories failing to push. The
budget turns that into one repository being told, in its own client, that it is out of room.

Negotiation refuses each object that would not fit, with a `507` the client prints, and the direct
`PUT` is guarded too for clients that skip negotiation. Downloads never are: a repository over
budget still serves every object it holds, because refusing a checkout punishes the wrong person and
fixes nothing.

The figure is what the repository holds, the same one `stats` and the dashboard report — not what it
costs the disk after deduplication. Two projects sharing a pack each count it against their own
budget, which is the number an operator is actually handing out. Collection is the way back under:
`retain` frees the room and the next push sees it immediately, without waiting for a cache to
expire.

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

| GitHub | GitLab | Objects |
|---|---|---|
| admin | Maintainer, Owner | download, upload, and force a lock open |
| push | Developer | download, upload, and take locks |
| pull only | Reporter | download |
| none, or an unusable token | Guest, or none | rejected |

GitLab grants inherited from a group count the same as ones set on the project, which is how most
organisations there are arranged. Developer is the level that may push, matching what GitLab itself
requires to write to the repository.

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

### Adding a forge

Two providers exist, so the shape is settled rather than guessed. A provider is one module under
`server/src/auth/` exposing two functions — `permission(client, api_url, token, namespace)` and
`login(client, api_url, token)` — plus a variant on `config::Provider` carrying its default API
root and environment variable, and two arms in `auth.rs`. Nothing else: the caching, the challenge
handling and the rejection accounting are shared and provider-blind.

The part worth care is the error mapping, because it is where the two existing providers already
disagree. GitHub answers `403` when rate-limited, GitLab answers `429`; both mean "ask again
later" and must map to `Error::Forge`, never to `Forbidden`. Getting that wrong tells a user with
full rights that they have none.

Gitea is the obvious third, tracked in [the issues](https://github.com/FerrLabs/LFSX/issues).

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

```bash
cargo llvm-cov --workspace --lcov --output-path lcov.info
```

The same command CI runs before handing the report to
[sonar.ferrlabs.com](https://sonar.ferrlabs.com), which tracks coverage, duplication and smells
over time. A pull request is analysed into its own project and the workflow comments what that
change introduced, since the Community edition has no pull-request analysis of its own.

## License

[MPL-2.0](LICENSE).
