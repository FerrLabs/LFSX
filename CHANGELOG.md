# Changelog

All notable changes to `lfsx-server` will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [0.35.0] - 2026-08-17

### Features

- feat(storage): read the framed format out of a bucket (#120)

## [0.34.2] - 2026-08-17

### Bug Fixes

- fix(auth): tell a client how long to wait when the forge is throttling us (#118)

## [0.34.1] - 2026-08-17

### Bug Fixes

- perf(gc): ask the filesystem how many names an object has (#119)

## [0.34.0] - 2026-08-17

### Features

- feat(locks): let anyone take a lock nobody has touched (#116)

## [0.33.0] - 2026-08-17

### Features

- feat(locks): keep the locks wherever the objects are (#115)

## [0.32.0] - 2026-08-16

### Features

- feat(storage): encrypt objects at rest when an operator supplies a key (#109)

## [0.31.0] - 2026-08-16

### Features

- feat(storage): redirect downloads to the bucket, and send a length S3 will accept (#107)

## [0.30.2] - 2026-08-16

### Bug Fixes

- fix(storage): store the object in a bucket, not the frames a compressing server made (#105)

## [0.30.1] - 2026-08-16

### Refactoring

- refactor(storage): walk a repository once, and report when the walk was partial (#98)

## [0.30.0] - 2026-08-16

### Features

- feat(locks): page the lock list, and negotiate the transfer instead of assuming it (#96)

## [0.29.0] - 2026-08-16

### Features

- feat(storage): add an S3-compatible object store (#94)

## [0.28.0] - 2026-08-15

### Features

- feat(storage): read a repository back and check it against its own digests (#92)

## [0.27.0] - 2026-08-15

### Features

- feat(storage): fold an existing store into compressed frames (#91)

## [0.26.0] - 2026-08-15

### Features

- feat(storage): compress objects at rest with zstd (#90)

## [0.25.0] - 2026-08-15

### Features

- feat(storage): fold objects stored before the shared store into it (#88)

## [0.24.2] - 2026-08-15

### Bug Fixes

- fix(ci): pin the docker reusable past the syntax fix (#85)

## [0.24.1] - 2026-08-15

### Bug Fixes

- fix(ci): unbreak the image build, and prove the Dockerfile before a release needs it (#84)

## [0.24.0] - 2026-08-15

### Features

- feat(storage): budget how much a single repository may hold (#83)

## [0.23.1] - 2026-08-15

### Bug Fixes

- fix(storage): stop collection removing a fanout directory out from under an upload (#82)

## [0.23.0] - 2026-08-15

### Features

- feat(storage): reject uploads over a configurable size limit (#79)

## [0.22.0] - 2026-08-15

### Features

- feat(storage): reclaim staging files left by interrupted uploads (#78)

## [0.21.0] - 2026-08-14

### Features

- feat(ci): cache compiled artifacts during the image build (#76)

## [0.20.0] - 2026-08-14

### Features

- feat(storage): store identical objects once across repositories (#75)

## [0.19.0] - 2026-08-14

### Features

- feat(auth): support GitLab as a forge provider (#71)

## [0.18.0] - 2026-08-14

### Features

- feat(server): honour Range on download (#73)

## [0.17.3] - 2026-08-14

### Bug Fixes

- fix(ci): build the tagged source when an image is rebuilt by hand (#70)

## [0.17.2] - 2026-08-14

### Bug Fixes

- fix(docker): copy every workspace member into the image build (#68)

## [0.17.1] - 2026-08-14

### Bug Fixes

- fix(cli): publish npm under the @ferrlabs scope (#67)

## [0.17.0] - 2026-08-14

### Features

- feat(chart): allow the public URL to be left unpinned (#66)

## [0.16.0] - 2026-08-14

### Features

- feat(server): answer on the host the client asked for (#65)

## [0.15.0] - 2026-08-14

### Features

- feat(server): serve a read-only page for each repository (#64)

## [0.14.0] - 2026-08-14

### Features

- feat(cli): publish lfsx to npm (#62)

## [0.13.0] - 2026-08-14

### Features

- feat(cli): add lfsx with doctor and gc (#59)

## [0.12.0] - 2026-08-14

### Features

- feat(ci): attach prebuilt binaries to each release (#55)

## [0.11.0] - 2026-08-14

### Features

- feat(ci): publish lfsx-server to crates.io on release (#56)

## [0.10.0] - 2026-08-14

### Features

- feat(server): expose Prometheus metrics (#50)

## [0.9.1] - 2026-08-14

### Bug Fixes

- fix(ci): stop the server after the lock checks, not before (#49)

## [0.9.0] - 2026-08-14

### Features

- feat(server): implement the file locking API (#45)

## [0.8.0] - 2026-08-14

### Features

- feat(chart): add a Helm chart published to ghcr.io (#43)

## [0.7.0] - 2026-08-14

### Features

- feat(auth): remember forge refusals so a bad token cannot hammer the API (#42)

## [0.6.0] - 2026-08-14

### Features

- feat(server): add a readiness probe that checks the storage root (#41)

## [0.5.3] - 2026-08-14

### Refactoring

- refactor(server): validate the namespace once and split the sweep out of the store (#35)

## [0.5.2] - 2026-08-14

### Bug Fixes

- fix(auth): treat a rate-limited forge as an outage, not a denial (#34)

## [0.5.1] - 2026-08-14

### Bug Fixes

- fix(server): shut down gracefully on SIGTERM as well as Ctrl-C (#33)

## [0.5.0] - 2026-08-14

### Features

- feat(ci): publish the image for linux/arm64 as well (#25)

## [0.4.0] - 2026-08-14

### Features

- feat(server): collect objects no longer referenced by any commit (#23)

## [0.3.0] - 2026-08-14

### Features

- feat(server): authenticate against the upstream Git repository (#20)

## [0.2.0] - 2026-08-14

### Features

- feat(server): implement the Git LFS batch API with streaming storage
