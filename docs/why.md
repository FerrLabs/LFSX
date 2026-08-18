# Why

GitHub bills LFS storage and bandwidth separately from your plan, and a Unity or Unreal project
burns through the free tier in a single push. A 3 GB asset pack cloned by a CI job ten times a
month is 30 GB of metered traffic. Self-hosting removes the meter entirely — the cost becomes a
disk you already own.

LFSX is built around three properties:

**Fast.** Uploads and downloads stream end to end. Nothing is buffered in memory, so a
multi-gigabyte asset costs the same resident memory as a one-kilobyte icon. The SHA-256 is computed
on the bytes as they pass, not in a second read of the file. [Measured](performance.md), not asserted.

**Lightweight.** One statically linked binary, one crate, a distroless image, no database. Objects
live on the filesystem, addressed by digest.

**Secure.** Access mirrors the permissions of the upstream Git repository, so revoking someone
there revokes them here. Every uploaded object is verified against its declared digest before being
accepted. Writes are atomic, so an interrupted transfer can never leave a corrupt object behind.
Object identifiers and repository names are validated before they reach the filesystem, so a
crafted request cannot escape the storage root.
