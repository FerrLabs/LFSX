# LFSX Helm chart

```bash
helm install lfsx oci://ghcr.io/ferrlabs/charts/lfsx \
  --set ingress.enabled=true \
  --set ingress.className=nginx \
  --set ingress.host=lfs.example.com
```

## What the chart decides for you

**One replica on a volume, and the chart refuses more.** Two pods sharing one claim is not a
topology it can serve: an upload is staged and renamed, which is atomic on one filesystem and
undefined across two, and the locks the pods must agree on live in that same directory. Setting
`replicaCount` above 1 with `storage.type=local` fails the render with that sentence rather than
letting you find it in production.

**More than one replica needs a bucket.** With `storage.type=s3` the objects and the locks move into
the bucket, which is what lets two servers agree on who holds what, and the strategy becomes
`RollingUpdate`. It also needs `persistence.enabled=false`: the claim is `ReadWriteOnce` so a second
pod could not mount it, and each replica stages its own uploads anyway.

```bash
helm install lfsx oci://ghcr.io/ferrlabs/charts/lfsx   --set storage.type=s3   --set storage.s3.endpoint=https://s3.example.com   --set storage.s3.bucket=assets   --set storage.s3.existingSecret=lfsx-s3   --set persistence.enabled=false   --set replicaCount=2
```

**The bucket keys come from a Secret you already hold**, never from a value. A Helm value ends up in
the release secret and in whatever CI printed the command, which is the same reason the chart will
not take an encryption key either. `storage.s3.existingSecret` names it; `accessKeyKey` and
`secretKeyKey` name the entries inside it.

Three things stay per replica and are worth knowing before you run several. The usage figure behind
`limits.repoQuota` is measured and cached per pod, so a quota can be overshot by what the replicas
write between them inside a minute. The permission cache is per pod, so the forge sees more lookups.
And `LFSX_AUTH_LOOKUP_BUDGET` is a ceiling per pod, so the ceiling across the deployment is that
number times the replica count.

**`LFSX_PUBLIC_URL` comes from the ingress host** unless you set `config.publicUrl` yourself. It is
echoed in the batch response and clients reconnect to it for every object, so a wrong value makes
negotiation succeed and every transfer fail.

With no ingress and no `config.publicUrl`, nothing is pinned and the server answers on whichever
host each request arrived at. That is the right setting when the same service is reached under more
than one name (a public host and an internal one) where any single fixed value would be wrong for
half the clients.

**With `ingress.className: nginx`**, the annotations that keep large uploads working are added for
you: no body size cap, no request buffering, and timeouts raised to an hour. Without them nginx
rejects anything over one megabyte, which for an LFS server means everything. Other controllers get
nothing extra: on Traefik the failure mode is attaching a buffering middleware, which the chart
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
| `config.otlpEndpoint` | `""` | HTTP traces URL of an OTLP collector; empty keeps traces off |
| `auth.mode` | `github` | `github`, `gitlab`, `gitea` (which also covers Forgejo), or `disabled` which accepts every request |
| `auth.githubApiUrl` | `https://api.github.com` | point at your GitHub Enterprise host |
| `auth.gitlabApiUrl` | `https://gitlab.com/api/v4` | point at your self-managed GitLab |
| `auth.giteaApiUrl` | `""` | API root of your Gitea or Forgejo instance, `https://git.example.com/api/v1`; required with `auth.mode: gitea`, there is no default host to fall back to |
| `auth.cacheTtl` | `60` | seconds a granted permission is reused |
| `auth.rejectionTtl` | `10` | seconds a refusal is remembered |
| `gc.grace` | `1209600` | seconds an object must be untouched before collection can take it |
| `limits.maxObjectSize` | `""` | bytes a single object may reach; empty means no ceiling |
| `limits.repoQuota` | `""` | bytes a single repository may hold; empty means no budget |
| `limits.maxConcurrentTransfers` | `""` | uploads and downloads served at once; empty keeps the server default of 128, `0` removes the cap |
| `compression` | `""` | `zstd` (or `zstd:1`…`zstd:19`) to store objects compressed; empty stores them as they arrive |
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

The claim holds every object, so size it for the repositories you expect and grow it there. LFSX
keeps nothing anywhere else. Backing up the release is backing up that volume: snapshot the PVC,
and read [`docs/operations.md`](../docs/operations.md) before you need the restore.

`limits.maxObjectSize` is worth setting here even though the server ships with no ceiling: a claim
has a fixed size, and a single upload that fills it takes down every repository on the release at
once.
