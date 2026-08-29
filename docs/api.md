# API

The Git LFS protocol is small: four routes, plus a health check:

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/{org}/{repo}/objects/batch` | negotiation: the client announces its objects, the server answers per object with an upload or download link |
| `PUT` | `/{org}/{repo}/objects/{oid}` | store an object |
| `GET` | `/{org}/{repo}/objects/{oid}` | retrieve an object, whole or by `Range` |
| `POST` | `/{org}/{repo}/objects/verify` | post-upload verification |
| `GET` | `/{org}/{repo}` | a page showing what the repository holds |
| `GET` | `/{org}/{repo}/objects/stats` | the same numbers as JSON |
| `POST` | `/{org}/{repo}/objects/retain` | reclaim space, see [Reclaiming space](reclaiming-space.md) |
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
where it stopped instead of starting over, which on a home upstream is the difference between an
annoyance and an afternoon. A range that cannot be satisfied is refused with `416` carrying the
object's real size; a range we cannot parse is ignored and the whole object is served, since
refusing a transfer over a malformed header would be worse than the header.
