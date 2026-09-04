# Releases

Versions are SemVer. Releases are cut by [FerrFlow](https://ferrflow.com) from the Conventional
Commit history of `main`: a merged `feat:` or `fix:` produces the tag, the
[`CHANGELOG.md`](../CHANGELOG.md) entry and the GitHub release, and the release builds and pushes
the image.

Images live at `ghcr.io/ferrlabs/lfsx` under three tags:

| Tag | Moves | For |
|---|---|---|
| `1.7.0` | never | production, where an upgrade should be a deliberate change |
| `1.7` | on every fix to that line | picking up fixes without picking up behaviour changes |
| `latest` | on every release | trying it out |

Every tag is a manifest list covering `linux/amd64` and `linux/arm64`, so a NAS, a Raspberry Pi or
a Graviton instance pulls the right image without being told. Each one is scanned for known
vulnerabilities, has to answer `/health` before it is allowed out, and is then signed with cosign
and shipped with a CycloneDX SBOM.

## Verifying what you downloaded

Every release ships four kinds of proof: a `.sha256` beside each archive, a build provenance
attestation on each archive, a `.sigstore` signature bundle beside each archive, and a CycloneDX
SBOM per crate (`lfsx-server.cdx.json`, `lfsx.cdx.json`). They answer different questions.

The checksum proves the download survived the wire:

```bash
sha256sum -c lfsx-server-x86_64-unknown-linux-musl.tar.gz.sha256
```

The attestation proves the bytes were built by this repository's release workflow from a specific
commit, which is the claim a checksum next to the artifact it checks cannot make:

```bash
gh attestation verify lfsx-server-x86_64-unknown-linux-musl.tar.gz --repo FerrLabs/LFSX
```

The signature bundle says the same thing without asking GitHub. `gh attestation verify` reads
GitHub's attestation store, so it needs GitHub to answer; the bundle beside the archive is checked
against the public transparency log by cosign alone:

```bash
cosign verify-blob lfsx-server-x86_64-unknown-linux-musl.tar.gz \
  --bundle lfsx-server-x86_64-unknown-linux-musl.tar.gz.sigstore \
  --certificate-identity-regexp '^https://github.com/FerrLabs/LFSX/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

The two identity flags are the point of the check: without them cosign confirms that somebody
signed the bytes, not that this repository's workflow did.

The SBOM lists every crate in the build for scanners and licence tooling, and is attested the same
way. The container image is verified separately: it is signed with cosign at push and its
signature and SBOM live next to it in the registry.
