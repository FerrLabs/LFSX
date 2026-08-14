# LFSX Helm chart

```bash
helm install lfsx oci://ghcr.io/ferrlabs/charts/lfsx \
  --set ingress.enabled=true \
  --set ingress.className=nginx \
  --set ingress.host=lfs.example.com
```

## What the chart decides for you

**One replica, `Recreate` strategy, `ReadWriteOnce` volume.** Two pods sharing one object store is
not a supported topology today: uploads stage a file and rename it, which is atomic on one
filesystem and undefined across two. The chart does not expose a replica count rather than let you
discover that in production.

**`LFSX_PUBLIC_URL` comes from the ingress host** unless you set `config.publicUrl` yourself. It is
echoed in the batch response and clients reconnect to it for every object, so a wrong value makes
negotiation succeed and every transfer fail.

With no ingress and no `config.publicUrl`, nothing is pinned and the server answers on whichever
host each request arrived at. That is the right setting when the same service is reached under more
than one name — a public host and an internal one — where any single fixed value would be wrong for
half the clients.

**With `ingress.className: nginx`**, the annotations that keep large uploads working are added for
you: no body size cap, no request buffering, and timeouts raised to an hour. Without them nginx
rejects anything over one megabyte, which for an LFS server means everything. Other controllers get
nothing extra — on Traefik the failure mode is attaching a buffering middleware, which the chart
simply does not do.

**The container runs as uid 65532 with a read-only root filesystem**, and the volume is mounted
with `fsGroup: 65532` so the non-root user can write to it. Nothing else is writable.

## Values

| Key | Default | Purpose |
|---|---|---|
| `image.repository` | `ghcr.io/ferrlabs/lfsx` | image to pull |
| `image.tag` | chart `appVersion` | override to pin a different build |
| `config.publicUrl` | derived from `ingress.host` | URL clients reach; unset means each request is answered on the host it used |
| `config.logLevel` | `info` | `RUST_LOG` filter |
| `auth.mode` | `github` | `disabled` accepts every request, for trusted networks only |
| `auth.githubApiUrl` | `https://api.github.com` | point at your GitHub Enterprise host |
| `auth.cacheTtl` | `60` | seconds a granted permission is reused |
| `auth.rejectionTtl` | `10` | seconds a refusal is remembered |
| `gc.grace` | `1209600` | seconds an object must be untouched before collection can take it |
| `persistence.enabled` | `true` | `false` uses an `emptyDir`, which loses every object on restart |
| `persistence.existingClaim` | `""` | bring your own PVC |
| `persistence.storageClass` | `""` | cluster default when empty |
| `persistence.size` | `100Gi` | claim size |
| `service.type` / `service.port` | `ClusterIP` / `80` | service shape |
| `ingress.enabled` | `false` | create an Ingress |
| `ingress.className` | `""` | `nginx` also brings the large-upload annotations |
| `ingress.host` | `""` | required when the ingress is enabled |
| `ingress.annotations` | `{}` | merged over the defaults for your class |
| `ingress.tls.enabled` | `true` | TLS block on the ingress |
| `ingress.tls.secretName` | `<release>-lfsx-tls` | certificate secret |
| `resources` | 50m / 64Mi requested | streaming means memory stays flat whatever the object size |
| `serviceAccount.create` | `true` | token automount is off either way |
| `podAnnotations` | Prometheus scrape hints | set to `{}` if your cluster discovers targets some other way |

## Storage

The claim holds every object, so size it for the repositories you expect and grow it there — LFSX
keeps nothing anywhere else. Backing up the release is backing up that volume.
