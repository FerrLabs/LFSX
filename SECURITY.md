# Security policy

## Reporting a vulnerability

Report privately through [GitHub Security Advisories](https://github.com/FerrLabs/LFSX/security/advisories/new),
which reaches the maintainers without the report becoming public first. If that page is unavailable
to you, email <contact@ferrlabs.com> with `SECURITY` in the subject, which is the address the
[organisation policy](https://github.com/FerrLabs/.github/blob/main/SECURITY.md) names.

Please do not open a public issue for a vulnerability. An issue is visible to everyone the moment it
exists, including to whoever would use it.

**What to expect:** an acknowledgement within 3 working days, an assessment of whether it is a
vulnerability and how severe within 10 working days, and a fixed release before any public
disclosure. If a report turns out not to be a vulnerability we will say so and why, rather than
leaving it unanswered.

Include what you have: the version, whether the deployment uses a volume or a bucket, which auth
provider, and the smallest sequence of requests that shows the problem. A proof of concept against
a server you control is welcome and is not required.

## What is in scope

The server and the CLI in this repository, the published container image, and the Helm chart:

- Serving an object to a caller the forge would refuse, or refusing one it would admit.
- Reading or writing outside the storage root through a crafted object identifier or repository
  name.
- Accepting an object whose bytes do not hash to its declared digest.
- Handing out a pre-signed URL that grants more than the batch response was about to grant.
- Two callers being told they hold the same lock.
- Leaking a token, an encryption key or a bucket credential into a log, an error or the pod spec.

## What is not in scope

- Anything reachable only with `LFSX_AUTH=disabled`, which accepts every request by design and says
  so at boot. That mode is for a trusted network.
- The permissions the forge itself grants. LFSX mirrors them; if a token can push to a repository
  upstream, it can push objects for that repository here, and that is the model rather than a bug.
- Denial of service by volume from an authenticated client. There is a transfer cap and a lookup
  budget, and the reverse proxy owns request-rate limiting, which
  [the documentation says outright](docs/reverse-proxy.md).
- Reports produced by a scanner with no analysis of whether the finding is reachable here.

## Supported versions

The latest minor release. This project is young enough that backporting to an older line would
promise more than one maintainer can keep; upgrading is the fix.

## Verifying what you run

Every release archive ships a checksum, a build provenance attestation and a cosign signature
bundle, and the container image is signed at push. [`docs/releases.md`](docs/releases.md) has the
commands to check each of them, which is worth doing before you trust a binary that guards your
assets.
