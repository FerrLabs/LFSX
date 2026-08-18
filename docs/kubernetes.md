# Kubernetes

A chart lives in [`chart/`](../chart/) and is published to the same registry as the image:

```bash
helm install lfsx oci://ghcr.io/ferrlabs/charts/lfsx   --set ingress.enabled=true   --set ingress.className=nginx   --set ingress.host=lfs.example.com
```

It encodes the things that are easy to get wrong: `LFSX_PUBLIC_URL` derived from the ingress host,
the nginx annotations that keep large uploads from being rejected, a single replica over a
`ReadWriteOnce` volume with the `Recreate` strategy, and probes on `/health` and `/ready`. See
[chart/README.md](../chart/README.md).
