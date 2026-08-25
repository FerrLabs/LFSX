# Kubernetes

A chart lives in [`chart/`](../chart/) and is published to the same registry as the image:

```bash
helm install lfsx oci://ghcr.io/ferrlabs/charts/lfsx   --set ingress.enabled=true   --set ingress.className=nginx   --set ingress.host=lfs.example.com
```

It encodes the things that are easy to get wrong: `LFSX_PUBLIC_URL` derived from the ingress host,
the nginx annotations that keep large uploads from being rejected, a single replica over a
`ReadWriteOnce` volume with the `Recreate` strategy, and probes on `/health` and `/ready`. See
[chart/README.md](../chart/README.md).

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
