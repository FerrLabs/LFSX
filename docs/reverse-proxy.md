# Reverse proxy

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

Do not add a request body size limit or a buffering middleware: LFS transfers are large and must
stream. Traefik's `buffering` middleware in particular will break uploads.

## Rate limiting belongs here

The proxy is where request-rate limits live: it sees the client address before any
load balancer rewrites it, and it sheds abusive traffic without spending a connection
on the server behind it. LFSX itself only carries one backstop, a cap on transfers
served at once (`LFSX_MAX_CONCURRENT_TRANSFERS`, default 128), for the bare deployment
with nothing in front. Past the cap it answers `503` with a `Retry-After`, which
git-lfs honours by retrying.

Size limits around how git-lfs actually behaves: a single push or fetch opens up to
8 parallel transfers by default (`lfs.concurrenttransfers`), so a per-address limit
below that breaks a single legitimate user. With nginx:

```nginx
limit_req_zone $binary_remote_addr zone=lfs_req:10m rate=30r/s;
limit_conn_zone $binary_remote_addr zone=lfs_conn:10m;

server {
    location / {
        limit_req zone=lfs_req burst=60 nodelay;
        limit_conn lfs_conn 16;
        limit_req_status 429;
        limit_conn_status 429;
        proxy_pass http://lfsx;
    }
}
```

With Traefik, the equivalent pair is a `rateLimit` middleware for request rate and
`inFlightReq` for concurrent connections:

```yaml
apiVersion: traefik.io/v1alpha1
kind: Middleware
metadata:
  name: lfsx-ratelimit
spec:
  rateLimit:
    average: 30
    burst: 60
---
apiVersion: traefik.io/v1alpha1
kind: Middleware
metadata:
  name: lfsx-inflight
spec:
  inFlightReq:
    amount: 16
```

The batch endpoint is one cheap request per push or fetch; the object transfers are
the expensive part. If you want to limit only one of them, the transfer URLs live
under `/{org}/{repo}/objects/`.

> [!CAUTION]
> **Do not put HTTP authentication on the proxy.** It cannot work, and the failure is confusing:
> the batch call authenticates fine, then every object transfer loops on `401`. See
> [Why authentication cannot live in the proxy](#why-authentication-cannot-live-in-the-proxy).
