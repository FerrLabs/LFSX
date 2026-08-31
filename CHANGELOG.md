# Changelog

All notable changes to `lfsx-server` will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [1.7.0] - 2026-08-31

### Features

- feat(chart): claim the Artifact Hub listing and badge the README (#263)

## [1.6.5] - 2026-08-31

### Refactoring

- refactor(server): make the object id a type instead of a check (#262)

## [1.6.4] - 2026-08-31

### Bug Fixes

- fix(locks): bound the lock path, the lock count and the dashboard table (#257)

## [1.6.3] - 2026-08-29

### Bug Fixes

- fix(test): start the wrapped lines of an intentional newline at column zero (#251)

## [1.6.2] - 2026-08-29

### Bug Fixes

- fix(cli): print the multi-line messages without their indentation (#250)

## [1.6.1] - 2026-08-29

### Bug Fixes

- fix(api): collecting a repository for real is the administrator's call (#249)

## [1.6.0] - 2026-08-29

### Features

- feat(site): give the landing the Poster reference sections (#220)
- feat(site): give the landing the Poster argument, comparison and FAQ (#219)

### Bug Fixes

- fix(api): stop 5xx bodies quoting the operating system (#248)
- fix(deps): pin esbuild to the version that verifies its binary (#246)
- fix(test): stop the refusal capture losing the race for tracing's global level (#243)
- fix(site): give the configuration descriptions room to read (#227)
- fix(site): stop the landing overflowing a phone screen (#224)
- fix(site): give the landing sections anchors and name every admin level (#223)

### Refactoring

- refactor(site): declare the landing band once (#221)

## [1.5.0] - 2026-08-25

### Features

- feat(chart): run more than one replica over a bucket (#217)

## [1.4.6] - 2026-08-25

### Refactoring

- refactor(auth): give github the send helper its siblings have (#215)

## [1.4.5] - 2026-08-25

### Bug Fixes

- fix(chart): limit the ephemeral storage a pod may take (#214)

## [1.4.4] - 2026-08-25

### Refactoring

- refactor(storage): split collection and usage out of s3.rs (#212)

## [1.4.3] - 2026-08-24

### Bug Fixes

- perf(storage): read what a repository holds from a size index (#211)

## [1.4.2] - 2026-08-24

### Bug Fixes

- fix(config): refuse a Host header that is not a host (#209)

## [1.4.1] - 2026-08-24

### Bug Fixes

- perf(storage): measure what a repository holds a few objects at a time (#210)

## [1.4.0] - 2026-08-24

### Features

- feat(auth): cap how often this server will ask the forge anything (#208)

## [1.3.2] - 2026-08-20

### Bug Fixes

- fix(storage): draw a fresh key for every startup probe (#189)
- fix(ci): drop socket.yml triggerPaths (#188)

## [1.3.1] - 2026-08-20

### Bug Fixes

- fix(storage): ask the claim index once more before a sweep frees an object (#184)

## [1.3.0] - 2026-08-20

### Features

- feat(storage): send an object over 5 GiB to a bucket in parts (#183)

## [1.2.2] - 2026-08-20

### Bug Fixes

- fix(locks): disable locking unless the bucket refuses a conditional write (#182)

## [1.2.1] - 2026-08-20

### Bug Fixes

- fix(storage): give up pre-signing unless the bucket proves it verifies checksums (#181)

## [1.2.0] - 2026-08-20

### Features

- feat(auth): authenticate against Gitea and Forgejo (#180)

## [1.1.4] - 2026-08-20

### Bug Fixes

- fix(storage): stop redirecting a download to bytes the codec has framed (#179)

## [1.1.3] - 2026-08-20

### Bug Fixes

- perf(storage): answer the elsewhere-claim question from a claim index (#169)

## [1.1.2] - 2026-08-19

### Bug Fixes

- fix(auth): stop reading an installation token's empty permissions as a refusal (#167)

## [1.1.1] - 2026-08-19

### Bug Fixes

- fix(auth): tell apart the two ways an anonymous GitLab lookup is refused (#164)

## [1.1.0] - 2026-08-19

### Features

- feat(storage): collect objects in a bucket (#160)

## [1.0.2] - 2026-08-19

### Bug Fixes

- fix(cli): refuse to publish without a usable npm token (#159)

## [1.0.1] - 2026-08-19

### Bug Fixes

- fix(auth): log every forge decision at the default level (#157)

## [1.0.0] - 2026-08-19

### Breaking Changes

- feat(auth)!: require a credential unless anonymous read is asked for (#156)

## [0.41.0] - 2026-08-19

### Features

- feat(site): scale the landing to the Poster composition (#153)
- feat(site): give the landing the Poster hero (#151)
- feat(site): restyle the documentation site to the Modernist direction (#149)

### Bug Fixes

- fix(auth): make a refused forge lookup visible at the default log level (#155)
- fix(site): serve the code font and give Archivo its italics (#152)

## [0.40.0] - 2026-08-18

### Features

- feat(site): declare the site as English-only (#144)

### Bug Fixes

- fix(auth): accept a forge token that carries no permissions block (#147)
- fix(ci): keep the server publish workflows off site releases (#145)
- fix(ci): build the site image from the site's own release tag (#143)

## [0.39.0] - 2026-08-18

### Features

- feat(site): add the documentation site for lfsx.dev (#135)

## [0.38.4] - 2026-08-18

### Bug Fixes

- fix(api): cap a batch and resolve its objects a few at a time (#134)

## [0.38.3] - 2026-08-18

### Bug Fixes

- perf(storage): remember what a repository holds on the seam and ask once per batch (#133)

## [0.38.2] - 2026-08-18

### Bug Fixes

- fix(storage): ask the bucket whether this instance can serve before answering ready (#132)

## [0.38.1] - 2026-08-17

### Refactoring

- refactor(storage): split the bucket backend and the route handlers by subject (#130)

## [0.38.0] - 2026-08-17

### Features

- feat(auth): let anyone read a repository the forge serves publicly (#124)

## [0.37.0] - 2026-08-17

### Features

- feat(gc): reclaim uploads a client never came back for (#123)

## [0.36.0] - 2026-08-17

### Features

- feat(storage): let a client upload straight to the bucket, and prove it did (#121)

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
