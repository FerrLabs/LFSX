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

> [!CAUTION]
> **Do not put HTTP authentication on the proxy.** It cannot work, and the failure is confusing:
> the batch call authenticates fine, then every object transfer loops on `401`. See
> [Why authentication cannot live in the proxy](#why-authentication-cannot-live-in-the-proxy).
