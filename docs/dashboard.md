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
| `LFSX_DASHBOARD_REPO` | none | `org/repo` whose admins may open it; required unless `LFSX_AUTH=disabled` |
| `LFSX_DASHBOARD_DIR` | `/usr/share/lfsx/dashboard` | where the built pages are; the image ships them there |

## Who can open it

The admins of `LFSX_DASHBOARD_REPO` on the forge, and nobody else. Sign in with the same token you
give git-lfs. The token is kept in the browser tab until it is closed or you sign out.

Pick a repository whose admins should run the server, an infrastructure repository rather than one
every contributor administers. It does not have to be in [`LFSX_ALLOWED`](allowed-namespaces.md):
the dashboard asks the forge about it directly, so an allow-list cannot lock its admins out.

With `LFSX_AUTH=disabled` there is no forge to ask, and the dashboard is open to anyone who can
reach the server, like everything else on it.

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
