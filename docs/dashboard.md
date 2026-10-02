# Dashboard

A web page for whoever runs the server: what the store holds, the traffic since the last start, and
the access settings, which it can also change without a restart. It is off by default.

```bash
LFSX_DASHBOARD=true
LFSX_DASHBOARD_REPO=acme/infra
```

It is served at `/-/dashboard/`, so `https://lfs.example.com/-/dashboard/`. No organisation on a
forge can start with `-`, so the prefix never shadows a repository.

| Variable | Default | Purpose |
|---|---|---|
| `LFSX_DASHBOARD` | `false` | `true` to serve the dashboard and its API |
| `LFSX_DASHBOARD_REPO` | none | `org/repo` whose admins may sign in with their forge token; required unless `LFSX_AUTH=disabled` |
| `LFSX_DASHBOARD_DIR` | `/usr/share/lfsx/dashboard` | where the built pages are; the image ships them there |

## Who can open it

Anyone holding a dashboard token, and the admins of `LFSX_DASHBOARD_REPO` on the forge.

A dashboard token is issued by the server itself, from where it runs:

```bash
lfsx-server dashboard token create alice
lfsx-server dashboard token list
lfsx-server dashboard token revoke alice
```

`create` prints the token once. The store keeps only its SHA-256 hash, in `.lfsx/dashboard-tokens.json`
on the volume or in the bucket, so every replica accepts it. The command reads the same environment
as the server, so run it inside the container: `kubectl exec deploy/lfsx -- lfsx-server dashboard
token create alice`. Revoking a token ends the sessions it opened.

A forge token works too, for an admin of `LFSX_DASHBOARD_REPO`. Pick a repository whose admins should
run the server, an infrastructure repository rather than one every contributor administers. It does
not have to be in [`LFSX_ALLOWED`](allowed-namespaces.md): the dashboard asks the forge about it
directly, so an allow-list cannot lock its admins out.

With `LFSX_AUTH=disabled` there is no forge to ask, and only dashboard tokens get in.

Signing in trades the token for a session cookie valid 12 hours, `HttpOnly` and `SameSite=Strict`,
and `Secure` when `LFSX_PUBLIC_URL` is `https://`. The browser keeps the cookie, never the token.
Sessions are signed with a key the server keeps at `.lfsx/session.key`; deleting it ends every
session. A session opened with a forge token lasts its 12 hours even if its owner stops being an
admin, so revoke the forge token as well when someone leaves.

Scripts can skip the cookie and send a forge admin's token as `Authorization: Bearer` on every call.

## What it shows

The overview reads the store's size and object count (on a volume, not a bucket), the requests and
refusals counted since the server started, the bytes uploaded and downloaded, the transfers in
flight, the uploads by object size and the read cache's hit rate. The figures are this replica's;
behind a load balancer each replica counts its own. [Prometheus](observability.md) is the place for
history and for adding replicas up.

## Changing access

The Settings page edits the three access settings:

- the repositories this server serves, [`LFSX_ALLOWED`](allowed-namespaces.md)
- the repositories whose objects take write access to read, [`LFSX_RESTRICTED`](restricted-namespaces.md)
- [anonymous reads](anonymous-read.md), `LFSX_ANONYMOUS_READ`

A change applies at once on the replica that saved it. It is written to the store as
`.lfsx/access.json`, on the volume or in the bucket, and every replica reads it back within 30
seconds. A saved value takes precedence over the environment until **Reset to the environment**
deletes it.

Everything else on the page is read from the environment and takes a restart to change.
