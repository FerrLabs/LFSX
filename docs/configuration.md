# Configuration

All configuration is by environment variable.

| Variable | Default | Purpose |
|---|---|---|
| `LFSX_BIND` | `0.0.0.0:8080` | listen address |
| `LFSX_STORAGE_ROOT` | `/var/lib/lfsx` | root of the object store |
| `LFSX_PUBLIC_URL` | the requested host | public URL used to build transfer links |
| `LFSX_AUTH` | `github` | permission source: `github`, `gitlab`, `gitea` (or `forgejo`, the same provider), or `disabled` to accept every request |
| `LFSX_GITHUB_API_URL` | `https://api.github.com` | API root, point it at your GitHub Enterprise host |
| `LFSX_GITLAB_API_URL` | `https://gitlab.com/api/v4` | API root, point it at your self-managed GitLab |
| `LFSX_GITEA_API_URL` | none, required | API root of your Gitea or Forgejo instance, `https://git.example.com/api/v1` |
| `LFSX_ANONYMOUS_READ` | `false` | `true` to let a request with no credentials read a repository the forge serves publicly |
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
| `LFSX_S3_PRESIGN` | `false` | `true` to hand transfers to the bucket instead of streaming them through the server, ignored for downloads when compression or encryption is configured, and ignored entirely if the bucket does not prove it verifies upload checksums |
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
