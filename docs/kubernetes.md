# Kubernetes

A chart lives in [`chart/`](../chart/) and is published to the same registry as the image:

```bash
helm install lfsx oci://ghcr.io/ferrlabs/charts/lfsx   --set ingress.enabled=true   --set ingress.className=nginx   --set ingress.host=lfs.example.com
```

It encodes the things that are easy to get wrong: `LFSX_PUBLIC_URL` derived from the ingress host,
the nginx annotations that keep large uploads from being rejected, a single replica over a
`ReadWriteOnce` volume with the `Recreate` strategy, and probes on `/health` and `/ready`. See
[chart/README.md](../chart/README.md).

### More than one replica needs a bucket

A volume deployment is one pod, and the chart refuses to render more: an upload is staged and
renamed, which is atomic on one filesystem and undefined across two, and the locks the pods would
have to agree on live in that same directory.

`storage.type=s3` is what changes that. The objects and the locks move into the bucket, which is the
whole reason locks were put there, and two servers sharing one bucket and nothing else agree on who
is holding what. Set `persistence.enabled=false` with it, since the claim is `ReadWriteOnce` and each
replica stages its own uploads regardless, and `replicaCount` to what you want. Anything else fails
the render with the reason rather than deploying something that cannot work.

Read [Objects in a bucket](buckets.md) first: locking there depends on the store performing a
conditional write, which the server probes at startup and disables locking over if it will not.

### Ephemeral storage is tied to persistence

Every upload is written to a staging file under `LFSX_STORAGE_ROOT` before anything is sent on, so
where that path lives decides which budget a transfer spends.

Left at the default, `persistence.enabled` puts it on the claim and the pod's ephemeral storage
carries only logs. Turned off, which a bucket deployment might reasonably do since the volume is
then just a write buffer, that path becomes an `emptyDir`: ephemeral storage, charged against the
node.

So the chart sets `resources.limits.ephemeral-storage` rather than leaving it open. A pod with no
limit that fills the node's disk gets pods evicted, and not necessarily this one. The default `2Gi`
is generous for logs and a floor for the other case: **a bucket deployment on an `emptyDir` has to
raise it past the largest object it expects to carry at once**, or that push is the thing that gets
evicted.
