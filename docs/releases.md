# Releases

Versions are SemVer, and the major stays at `0` while the surface is still settling: a `0.x` bump
is free to change environment variables, the storage layout or the API. Releases are cut by
[FerrFlow](https://ferrflow.com) from the Conventional Commit history of `main`: a merged `feat:`
or `fix:` produces the tag, the [`CHANGELOG.md`](../CHANGELOG.md) entry and the GitHub release, and
the release builds and pushes the image.

Images live at `ghcr.io/ferrlabs/lfsx` under three tags:

| Tag | Moves | For |
|---|---|---|
| `0.4.0` | never | production, where an upgrade should be a deliberate change |
| `0.4` | on every fix to that line | picking up fixes without picking up behaviour changes |
| `latest` | on every release | trying it out |

Every tag is a manifest list covering `linux/amd64` and `linux/arm64`, so a NAS, a Raspberry Pi or
a Graviton instance pulls the right image without being told. Each one is scanned for known
vulnerabilities, has to answer `/health` before it is allowed out, and is then signed with cosign
and shipped with a CycloneDX SBOM.
