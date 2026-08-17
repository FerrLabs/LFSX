<div align="center">

# LFSX

**A fast, lightweight, secure Git LFS server.**

[![CI](https://github.com/FerrLabs/LFSX/actions/workflows/ci.yml/badge.svg)](https://github.com/FerrLabs/LFSX/actions/workflows/ci.yml)
[![Coverage](https://sonar.ferrlabs.com/api/project_badges/measure?project=lfsx&metric=coverage&token=sqb_623f2242cd5fcf0124a37f3be11f1bae955d2607)](https://sonar.ferrlabs.com/dashboard?id=lfsx)
[![Quality Gate](https://sonar.ferrlabs.com/api/project_badges/measure?project=lfsx&metric=alert_status&token=sqb_623f2242cd5fcf0124a37f3be11f1bae955d2607)](https://sonar.ferrlabs.com/dashboard?id=lfsx)
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
| `LFSX_STAGING_MAX_AGE` | `86400` | seconds before an interrupted upload's leftovers are reclaimed, on the volume and in the bucket |
| `LFSX_LOCK_MAX_AGE` | never | seconds a lock may go untouched before anyone can take it |
| `LFSX_MAX_OBJECT_SIZE` | unlimited | bytes an object may reach before the server refuses it |
| `LFSX_REPO_QUOTA` | unlimited | bytes a single repository may hold |
| `LFSX_STORAGE` | `local` | `s3` to keep objects in a bucket instead of on the volume |
| `LFSX_S3_ENDPOINT` / `LFSX_S3_BUCKET` / `LFSX_S3_REGION` | — | where the bucket is; endpoint and bucket are required with `LFSX_STORAGE=s3` |
| `LFSX_S3_ACCESS_KEY` / `LFSX_S3_SECRET_KEY` | — | credentials for it, required with `LFSX_STORAGE=s3` |
| `LFSX_S3_PATH_STYLE` | `true` | `false` for virtual-host addressing; MinIO and Garage want path style |
| `LFSX_S3_PRESIGN` | `false` | `true` to redirect downloads to the bucket instead of streaming them through the server |
| `LFSX_COMPRESSION` | `none` | `zstd`, or `zstd:1`…`zstd:19` to pick the level, to compress objects at rest |
| `LFSX_ENCRYPTION_KEY_FILE` | — | path to a file holding one or more 32-byte keys as hex, to encrypt objects at rest |
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
files with a single link. Nothing needs migrating for them to serve — but they never collapse
either, so a server that has been running since before 0.20.0 keeps paying full price for every
pack two projects share. Folding them in is one call per repository:

```bash
lfsx dedupe --repo FerrLabs/Blastlands --dry-run
lfsx dedupe --repo FerrLabs/Blastlands
```

It moves each object into the shared store and links it back, or links to what is already there and
frees the copy. Bytes are only freed by the *second* repository to hold them, so run it everywhere
before judging the result. Running it again does nothing: an object already sharing its inode is
recognised and skipped.

Two things it refuses rather than risk. An object whose bytes do not hash to its own name never
enters the shared store, because everything that links there later would inherit them. And a shared
entry that does not match its name is never adopted, so a repository with a good copy keeps it. Both
are counted as `refused` and named in the log.

The repository's file is never removed before its replacement exists: the link is made under a
temporary name and renamed over the original, so an interrupted run leaves either the old file or
the new link. It needs admin rights on the repository.

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
| `POST` | `/{org}/{repo}/objects/dedupe` | fold objects stored before the shared store into it |
| `POST` | `/{org}/{repo}/objects/compress` | fold objects stored before compression into it |
| `POST` | `/{org}/{repo}/objects/audit` | read every object back and check it against its own digest |
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

## What the protocol asks for and what this answers

**Transfers.** The batch response answers `basic`, which is the only adapter this server implements
and the one every client supports. It is chosen from what the client advertised rather than
assumed, so adding another is a change in one place.

**Locks are paged.** `GET /locks` and `POST /locks/verify` honour `limit` and `cursor`, defaulting to
100 and capped at 1000. A response carries `next_cursor` only when there is another page, so an
absent cursor ends the walk. The cursor is the id of the last lock returned and the list is ordered
by id, which means a lock released mid-walk is skipped rather than shifting everything after it out
of view. `verify` pages over the whole list before splitting it into yours and theirs, so both sides
agree on where the page ends.

Without this a studio that has locked an art directory received every lock in one body, and a client
that sent `limit` believed it had seen the list.

**`ref` is accepted and not acted on.** Clients send the branch they are working on, and this server
cannot make it change any answer: permissions come from the forge at repository granularity — pull,
push, admin — so there is nothing branch-shaped to consult. Refusing the field would break a client
sending exactly what the specification tells it to send, so it is parsed and ignored deliberately
rather than by omission. The day it matters is
[#46](https://github.com/FerrLabs/LFSX/issues/46): once a public repository can be read without a
token, write access has to say which refs, and that is where `ref` earns its keep.

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
the repository — the same person who could rewrite the branch anyway.

`LFSX_LOCK_MAX_AGE` is the answer to the same situation without an administrator in it. Unset, a
lock lasts until someone releases it, which is what happened before this existed. Set, a lock nobody
has touched for that long can be taken by anyone who could have taken it in the first place:

```bash
LFSX_LOCK_MAX_AGE=1209600   # two weeks
```

**A stale lock is not deleted, it is taken.** Until somebody claims it, it is still listed and still
names its holder, because the useful answer is not "this is free" but "marie had this and has not
touched it in three weeks". The takeover is recorded in the log with the previous owner, the new one,
and how long it had been.

The clock runs from when the lock was taken. Last-touched is closer to what people mean by stale,
and it would mean guessing which object a path maps to; the claim is the thing this server can
answer for.

One honest limitation: **`git lfs locks` cannot show you any of this.** The protocol's lock is an id,
a path, a timestamp and an owner, with nowhere to put a "stale" flag, so no phrasing of the JSON
would make the client display it. The repository page shows it, which is where somebody goes to ask
why they cannot take a scene.

Locks live next to the objects, under `.locks/`, so they are covered by the same backup and
disappear with the repository. That means `$LFSX_STORAGE_ROOT/.locks/` on a volume and the same
prefix in the bucket when objects are in one: whatever holds the objects holds the locks, because a
second replica has to agree with the first about who is holding what.

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
`PUT` is guarded too for clients that skip negotiation — including when they skip declaring a size,
since the budget travels with the transfer and cuts it off at the byte that crosses the line. An
object the repository already holds is never refused at either gate: re-sending it asks for no new
room. Downloads never are either: a repository over budget still serves every object it holds,
because refusing a checkout punishes the wrong person and fixes nothing.

The figure is what the repository holds on disk, the same one `stats` and the dashboard report — not
what it costs after deduplication, and not what it weighed before compression. Two projects sharing a pack each count it against their own
budget, which is the number an operator is actually handing out. Collection is the way back under:
`retain` frees the room and the next push sees it immediately, without waiting for a cache to
expire.

## Compression

`LFSX_COMPRESSION=zstd` stores objects compressed. Unset, nothing is compressed, which is the
default because it is the one that cannot surprise anyone.

```bash
LFSX_COMPRESSION=zstd        # level 3
LFSX_COMPRESSION=zstd:9      # slower, smaller
```

The received wisdom is that an LFS store is already compressed, and for PNG, MP3 and OGG that is
true. It is badly wrong for meshes: measured on two real Unity projects, `.fbx` compresses **2.9×**
and **6.7×**, `.tga` **10.4×**, while `.png` gives up 1%. On a store where meshes are the bulk —
which is what a game project looks like — that came out at **71% smaller** overall.

**The object keeps its name.** A file is still stored under the digest of its plaintext, because
that name is what collection, deduplication and the shared store address it by. What changes is
that the file says what it is in its first bytes, which means two things worth knowing: turning
compression on rewrites nothing that is already stored, and turning it off again keeps serving
everything written while it was on. A mixed store is the normal state of an upgraded one.

**Ranges still work.** Objects are compressed in four-megabyte frames with an index, so serving the
tail of a three-gigabyte asset decompresses the frames it touches rather than everything before
them. A resumed transfer stays a resumed transfer.

**Memory stays flat.** One frame is decompressed at a time, whatever the object weighs.

**Objects that will not compress are stored as they arrived.** A frame that gives up less than five
percent of itself is written raw and flagged, and an object whose framed form would be no smaller is
left exactly as it was. Half a game store is PNG and OGG; none of it pays for the attempt twice.

Turning compression on only changes what arrives next. Folding in what is already stored is one call
per repository, with the same shape as the deduplication migration:

```bash
lfsx compress --repo FerrLabs/Blastlands --dry-run
lfsx compress --repo FerrLabs/Blastlands
```

The dry run compresses everything and writes nothing, so the figure it reports is the real saving
rather than an estimate. Each object is verified against its own digest before being rewritten — the
last moment that check is a simple one, since afterwards the file is no longer the bytes it is named
after. Anything that fails is left alone and counted as `refused`.

Objects shared between repositories stay shared: the copy under `.content` is what gets replaced and
this repository is relinked to it, so run it everywhere. Until a repository has had its turn, it
keeps the older bytes alive through its own link, and the store holds both forms.

What it costs: CPU on both ends of every transfer, and an integrity check that can no longer be a
`sha256sum` — `lfsx verify --repo <org/repo>` is what replaces it, reading every object back the way
a download does. See [`docs/operations.md`](docs/operations.md). `stats`, the dashboard and
`LFSX_REPO_QUOTA` all count bytes on disk, so a repository's figure drops when compression is on;
that is the number an operator budgets a volume against.

## Encryption at rest

**Read this part first: for most self-hosted deployments the better answer is the volume.** LUKS, an
encrypted EBS volume, or a storage class that does it transparently costs nothing in this server, has
no key handling in our code to get wrong, and covers the same disks. Reach for what follows when the
storage itself is the thing you do not trust: a shared NAS, a bucket somebody else operates, a
laptop, a disk you will one day return under warranty.

```bash
head -c 32 /dev/urandom | xxd -p -c 64 > /etc/lfsx/key
LFSX_ENCRYPTION_KEY_FILE=/etc/lfsx/key
```

A path rather than the key itself, deliberately. A key in an environment variable is in the pod
spec, in `docker inspect`, and in every log that dumps the environment. A file comes from a
Kubernetes Secret mount or from FerrVault without any of that. The server refuses to start if the
file is missing or unreadable, because the alternative is a server that quietly writes plaintext
while an operator believes otherwise.

### What it protects, in the words that are true

It protects the bytes at rest: the disk, the snapshot, the backup, the volume you decommission, the
bucket whose provider is not you. **It does not protect against anyone who has the running server**,
because that process holds the key by construction. Anyone who can read its memory, its environment,
or its mounted Secret can read every object. "Encrypted" is exactly the word that gives false
comfort, so it is worth being blunt about where the line is.

### What it does to the store

**The object keeps its name.** The oid is the digest of the plaintext, the client declares it, and
that is the protocol. So the upload hashes the bytes as they stream past and encrypts them after: the
filename stays the plaintext digest, the contents become ciphertext. Reverse those two and every
object fails its own verification. It also means deduplication is untouched, since two repositories
pushing identical bytes still agree on the name.

**Nothing is buffered.** Objects are encrypted in the same four-megabyte frames compression uses,
each sealed on its own with ChaCha20-Poly1305, so serving the tail of a three-gigabyte asset costs
the frames it touches. A single pass over a whole object would put the authentication tag at the end
and force the server to hold the entire thing before serving a byte.

**`Content-Length` is the plaintext length**, read from the object header rather than from the file,
which is now longer than the object by a header and a tag per frame.

**Compression runs first** when both are on. Sealed bytes are indistinguishable from random and give
up no ground at any level, so the other order would spend the CPU for nothing. The usual caveat
applies: compressing before encrypting means the ciphertext length leaks how compressible the
plaintext was.

**A frame is bound to where it was written.** Its index, whether it is the last one, and the object
id are all authenticated, so frames cannot be reordered, an object cannot be truncated to a shorter
one that still opens, and a file cannot be moved on top of another in the shared content store.

### Turning it on, and rotating

Turning it on is not a flag day. Objects already written stay plaintext and keep being served; new
ones are encrypted. Both kinds live in one store because each object says in its first bytes which it
is. Objects written before the key existed are not rewritten, and re-pushing them is what converts
them today.

On Kubernetes the chart mounts an existing Secret rather than taking the key as a value, because a
Helm value ends up in the release secret and in whatever printed the command:

```bash
kubectl create secret generic lfsx-key --from-file=key=/dev/stdin <<< "$(head -c 32 /dev/urandom | xxd -p -c 64)"
helm upgrade lfsx oci://ghcr.io/ferrlabs/charts/lfsx --set encryption.existingSecret=lfsx-key
```

Rotation is a new line at the top of the file:

```
# the key new objects are written under
9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
# still here for everything already on disk
5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03
```

Every key in the file is accepted for reads and the first is used for writes, so rotating does not
mean re-encrypting the store. Each object records which key it was written under, by a hash of that
key rather than by a number, so an id can never come to name a different key than the one it was
written with. A key that is deleted while objects still reference it makes those objects unreadable,
and the server says so by name instead of reporting corruption.

## Objects in a bucket

`LFSX_STORAGE=s3` puts the objects in an S3-compatible bucket — MinIO, Garage, Backblaze, AWS —
instead of on the volume, which is what unties capacity from one machine.

```bash
LFSX_STORAGE=s3
LFSX_S3_ENDPOINT=https://s3.example.com
LFSX_S3_BUCKET=assets
LFSX_S3_ACCESS_KEY=…
LFSX_S3_SECRET_KEY=…
```

**Locks move with the objects.** They are keys under `.locks/` in the same bucket, taken with a
conditional write — `If-None-Match: *`, so the store itself decides who arrived first and answers
`412` to everyone after. That is the mutual exclusion `create_new` gives on a filesystem, and it is
what makes a second replica possible: two servers sharing one bucket and nothing else agree on who
holds a scene, and a release on either frees it on both. Tested against MinIO rather than assumed,
including the case where both replicas ask at once.

The volume is still needed as a write buffer for uploads, but it no longer carries state a second
replica would have to see.

**The layout is the one on disk.** The bytes live once under a key derived from their digest, and a
repository that holds them owns an empty marker beside it. That marker is the bucket's answer to a
hard link: it is why two projects sharing an asset pack cost the bucket once, and it is the only
thing consulted when a repository asks for an object, so a digest cannot be guessed into someone
else's assets.

**Transfers go through the server by default.** It streams to and from the bucket rather than
handing out a redirect, which is what keeps the byte counters, the ranges, the size ceiling and the
quota working. A download asks the bucket for exactly the range it wants, so resuming stays cheap.
The local volume is still used, as a write buffer: an upload lands there to be hashed and checked
before anything is sent, and is deleted once it has been.

**`LFSX_S3_PRESIGN=true` redirects transfers instead.** The batch response hands the client a
pre-signed URL and the bytes never cross this server, which is what you want when the bucket is
closer to the clients than the server is, or when the server's egress is the thing you are paying
for.

Uploads go the same way, and the two things that made that unsafe are both closed rather than
accepted:

**The digest is bound into the signature.** The URL comes with an `x-amz-checksum-sha256` header the
client must send, and it is part of what was signed, so the store refuses any body that does not hash
to the object the URL was cut for. A client holding an upload URL cannot put arbitrary bytes behind
it.

**The upload goes to a key only that repository was signed for**, under `.incoming/`, not to the
shared content key. That is what keeps possession meaning something: bytes arriving there prove the
repository had the object, where a write to the shared key would prove only that somebody with push
rights knew a digest, which is all a leaked pointer file is. On `verify` the server checks the size
that actually arrived against the declared one, the object ceiling and the repository's budget, then
moves the object into the shared keyspace with a copy inside the bucket and writes the marker.

An upload nobody reports leaves its bytes under that key, and they are swept on the same schedule
and by the same setting as an interrupted transfer's staging file: once at boot, then hourly, for
anything older than `LFSX_STAGING_MAX_AGE`. A key written a moment ago is left alone, because a slow
client on a bad connection is not an abandoned one.

Two consequences worth knowing. `lfsx_uploaded_bytes` does not move for these, because nothing here
measured them and counting the client's own figure would make that counter mean two different things.
And **a server with `LFSX_ENCRYPTION_KEY_FILE` set keeps carrying uploads itself**: an object a client
writes directly arrives as it is, so handing out an upload URL would put plaintext in the bucket while
the operator believed otherwise. Encryption is a promise about what the storage provider can read, and
a faster upload is not worth breaking it quietly. The server says so at startup. With compression the
trade is milder and allowed: objects uploaded directly are simply not compressed.

The permission check does not move. A signature is cut only after the marker says this repository
holds the object, exactly as a plain download is refused without it, and the URL it signs is scoped
to one object and expires with the action. What changes is what the server can still see:
`lfsx_downloaded_bytes` stops counting bytes it no longer carries, and the bucket serves the ranges.

This is the one case where the batch response says `"authenticated": true`, and the one case where
it is true: see [below](#why-authentication-cannot-live-in-the-proxy) for why that field is a trap
everywhere else.

**Capacity is not reported.** `lfsx_objects_stored` and `lfsx_store_bytes` are not measured against
a bucket: there is no cheap answer for what one holds, and building it from a full listing would cost
a request per object on every scrape. The gauges are left alone rather than pinned to a zero a
dashboard would average as an empty store, and the server says so at startup. Read capacity from the
bucket itself. Per-repository figures still work, since a repository is a prefix.

**Four things do not apply, and say so** rather than reporting an empty success. Collection,
compression and verification are not implemented against a bucket yet and answer `501`.
Deduplication answers `501` too, for a different reason: content addressing already stores each
object once, so there is nothing left to fold in.

**Compression and encryption work here too.** They were refused against a bucket at first, because a
framed object was only readable through the file the codec opened. The codec reads from a bucket as
well now: the header and the index are two ranged `GET`s before the first frame, which is what the
format was shaped for. So `LFSX_COMPRESSION` and `LFSX_ENCRYPTION_KEY_FILE` mean the same thing on a
volume and in a bucket, and encrypting into storage somebody else operates is the case that most
deserves it.

What that costs, measured against a bucket on localhost, twenty downloads each:

```
raw                       12.3 ms   1.56 MB
compressed                 6.0 ms   1.68 MB plaintext
compressed, but noise     12.3 ms   1.20 MB plaintext
```

A compressible asset comes out ahead: the frames save far more on the wire than two extra round trips
cost. An already-compressed one saves nothing and still pays them, about 30% more per byte. And the
bucket here is on localhost, so those round trips are close to free in a way they will not be across
a WAN: on a remote bucket, expect two extra latencies per download before the first byte. If your
store is mostly PNG and MP3, leave compression off and the objects stay raw.

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
later" and must map to `Error::RateLimited`, never to `Forbidden`. Getting that wrong tells a user
with full rights that they have none.

`Error::RateLimited` and `Error::Forge` are separate on purpose, and a new provider should keep them
apart. A throttled forge is working: it has said when to come back, and the answer is a `503`
carrying `Retry-After` so the client waits. `502` reads as a transient upstream failure and git-lfs
comes straight back, spending another request on the same exhausted quota, which turns one rate
limit into a CI run's worth of them. The duration comes from `Retry-After` when the forge sends one,
from `x-ratelimit-reset` when it sends an absolute reset instead, and from a one-minute default when
it says neither: never from zero. `lfsx_rejections_total{cause="forge_rate_limited"}` counts these
separately from `forge_unreachable`, because "the forge is throttling us" and "the forge is broken"
are different afternoons.

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

LFSX never emits `authenticated` for a URL that points back at itself, so the client authenticates
each transfer itself. A test pins the behaviour down —
`batch_never_claims_a_transfer_through_this_server_is_pre_authenticated` — and it will fail if
anyone sets the field on the ordinary path.

The exception proves the rule rather than bending it. With `LFSX_S3_PRESIGN=true` the href is a
pre-signed bucket URL, which genuinely carries its own credentials and genuinely must be called
without an `Authorization` header — the proxy in front of this server is not even in that path. The
field is not a claim about the server's own authentication; it is a claim about the URL, and it is
set only when the URL was signed.

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

That one starts the binary and a stub forge, pushes a large asset with the actual client, clones it
back and compares the bytes, then takes a lock, fails to steal it and releases it. It runs on an
isolated `GIT_CONFIG_GLOBAL`, so it cannot touch your own git configuration.

It runs on every push against **Linux, macOS and Windows**, plus **git-lfs 3.0.2** — the oldest
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
