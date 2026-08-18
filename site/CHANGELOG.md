# Changelog

All notable changes to `site` will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [2026.8.18] - 2026-08-18

### Features

- feat(site): declare the site as English-only (#144)
- feat(site): add the documentation site for lfsx.dev (#135)
- feat(auth): let anyone read a repository the forge serves publicly (#124)
- feat(gc): reclaim uploads a client never came back for (#123)
- feat(storage): let a client upload straight to the bucket, and prove it did (#121)
- feat(storage): read the framed format out of a bucket (#120)
- feat(locks): let anyone take a lock nobody has touched (#116)
- feat(locks): keep the locks wherever the objects are (#115)
- feat(storage): encrypt objects at rest when an operator supplies a key (#109)
- feat(storage): redirect downloads to the bucket, and send a length S3 will accept (#107)
- feat(locks): page the lock list, and negotiate the transfer instead of assuming it (#96)
- feat(storage): add an S3-compatible object store (#94)
- feat(storage): read a repository back and check it against its own digests (#92)
- feat(storage): fold an existing store into compressed frames (#91)
- feat(storage): compress objects at rest with zstd (#90)
- feat(storage): fold objects stored before the shared store into it (#88)
- feat(storage): budget how much a single repository may hold (#83)
- feat(storage): reject uploads over a configurable size limit (#79)
- feat(storage): reclaim staging files left by interrupted uploads (#78)
- feat(ci): cache compiled artifacts during the image build (#76)
- feat(storage): store identical objects once across repositories (#75)
- feat(auth): support GitLab as a forge provider (#71)
- feat(server): honour Range on download (#73)
- feat(chart): allow the public URL to be left unpinned (#66)
- feat(server): answer on the host the client asked for (#65)
- feat(server): serve a read-only page for each repository (#64)
- feat(cli): publish lfsx to npm (#62)
- feat(cli): add lfsx with doctor and gc (#59)
- feat(ci): attach prebuilt binaries to each release (#55)
- feat(ci): publish lfsx-server to crates.io on release (#56)
- feat(server): expose Prometheus metrics (#50)
- feat(server): implement the file locking API (#45)
- feat(chart): add a Helm chart published to ghcr.io (#43)
- feat(auth): remember forge refusals so a bad token cannot hammer the API (#42)
- feat(server): add a readiness probe that checks the storage root (#41)
- feat(ci): publish the image for linux/arm64 as well (#25)
- feat(server): collect objects no longer referenced by any commit (#23)
- feat(server): authenticate against the upstream Git repository (#20)
- feat(server): implement the Git LFS batch API with streaming storage

### Bug Fixes

- fix(ci): build the site image from the site's own release tag (#143)
- fix(api): cap a batch and resolve its objects a few at a time (#134)
- perf(storage): remember what a repository holds on the seam and ask once per batch (#133)
- fix(storage): ask the bucket whether this instance can serve before answering ready (#132)
- fix(auth): tell a client how long to wait when the forge is throttling us (#118)
- perf(gc): ask the filesystem how many names an object has (#119)
- fix(storage): store the object in a bucket, not the frames a compressing server made (#105)
- fix(ci): pin the docker reusable past the syntax fix (#85)
- fix(ci): unbreak the image build, and prove the Dockerfile before a release needs it (#84)
- fix(storage): stop collection removing a fanout directory out from under an upload (#82)
- fix(ci): build the tagged source when an image is rebuilt by hand (#70)
- fix(docker): copy every workspace member into the image build (#68)
- fix(cli): publish npm under the @ferrlabs scope (#67)
- fix(ci): stop the server after the lock checks, not before (#49)
- fix(auth): treat a rate-limited forge as an outage, not a denial (#34)
- fix(server): shut down gracefully on SIGTERM as well as Ctrl-C (#33)

### Refactoring

- refactor(storage): split the bucket backend and the route handlers by subject (#130)
- refactor(storage): walk a repository once, and report when the walk was partial (#98)
- refactor(server): validate the namespace once and split the sweep out of the store (#35)
