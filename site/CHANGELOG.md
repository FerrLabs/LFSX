# Changelog

All notable changes to `site` will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [2026.9.11] - 2026-09-04

### Features

- feat(server): keep a local copy of what the bucket serves (#324)

## [2026.9.10] - 2026-09-03

### Features

- feat(server): source encryption keys from a command as well as a file (#312)

## [2026.9.9] - 2026-09-02

### Features

- feat(server): lend the anonymous GitHub lookup the server's own App identity (#303)

## [2026.9.8] - 2026-09-02

### Features

- feat(server): export request traces over OTLP behind an opt-in endpoint (#301)

## [2026.9.7] - 2026-09-02

### Features

- feat(server): put the privileged operations on an audit trail naming the actor (#299)

## [2026.9.5] - 2026-09-02

### Bug Fixes

- fix(site): drop the fast-lightweight-secure triptych from the landing (#292)

## [2026.9.4] - 2026-09-01

### Features

- feat(server): cap concurrent transfers with a 503 backstop (#288)

## [2026.9.3] - 2026-09-01

### Features

- feat(release): attest the archives and ship an SBOM per crate (#283)

## [2026.9.2] - 2026-09-01

### Features

- feat(fuzz): regenerate the codec seeds through a seed binary (#282)

## [2026.9.1] - 2026-09-01

### Features

- feat(fuzz): fuzz the codec, the key parsers and the range header (#281)

## [2026.8.32] - 2026-08-31

### Features

- feat(site): speak French (#269)
- feat(site): dress the site in the LFSX design (#265)
- feat(chart): claim the Artifact Hub listing and badge the README (#263)

### Bug Fixes

- fix(release): version the site with a sequence so same-day releases publish (#268)

### Refactoring

- refactor(server): make the object id a type instead of a check (#262)

## [2026.8.31] - 2026-08-31

### Bug Fixes

- fix(locks): bound the lock path, the lock count and the dashboard table (#257)
- fix(test): start the wrapped lines of an intentional newline at column zero (#251)
- fix(cli): print the multi-line messages without their indentation (#250)

## [2026.8.29] - 2026-08-29

### Features

- feat(site): give the landing the Poster reference sections (#220)
- feat(site): give the landing the Poster argument, comparison and FAQ (#219)
- feat(chart): run more than one replica over a bucket (#217)

### Bug Fixes

- fix(api): collecting a repository for real is the administrator's call (#249)
- fix(api): stop 5xx bodies quoting the operating system (#248)
- fix(deps): pin esbuild to the version that verifies its binary (#246)
- fix(test): stop the refusal capture losing the race for tracing's global level (#243)
- fix(site): give the configuration descriptions room to read (#227)
- fix(site): stop the landing overflowing a phone screen (#224)
- fix(site): give the landing sections anchors and name every admin level (#223)

### Refactoring

- refactor(site): declare the landing band once (#221)
- refactor(auth): give github the send helper its siblings have (#215)

## [2026.8.25] - 2026-08-25

### Bug Fixes

- fix(chart): limit the ephemeral storage a pod may take (#214)
- perf(storage): read what a repository holds from a size index (#211)
- fix(config): refuse a Host header that is not a host (#209)
- perf(storage): measure what a repository holds a few objects at a time (#210)

### Refactoring

- refactor(storage): split collection and usage out of s3.rs (#212)

## [2026.8.24] - 2026-08-24

### Features

- feat(auth): cap how often this server will ask the forge anything (#208)
- feat(storage): send an object over 5 GiB to a bucket in parts (#183)
- feat(auth): authenticate against Gitea and Forgejo (#180)

### Bug Fixes

- fix(storage): draw a fresh key for every startup probe (#189)
- fix(ci): drop socket.yml triggerPaths (#188)
- fix(storage): ask the claim index once more before a sweep frees an object (#184)
- fix(locks): disable locking unless the bucket refuses a conditional write (#182)
- fix(storage): give up pre-signing unless the bucket proves it verifies checksums (#181)
- fix(storage): stop redirecting a download to bytes the codec has framed (#179)

## [2026.8.20] - 2026-08-20

### Features

- feat(storage): collect objects in a bucket (#160)

### Bug Fixes

- perf(storage): answer the elsewhere-claim question from a claim index (#169)
- fix(auth): stop reading an installation token's empty permissions as a refusal (#167)
- fix(auth): tell apart the two ways an anonymous GitLab lookup is refused (#164)
- fix(cli): refuse to publish without a usable npm token (#159)
- fix(auth): log every forge decision at the default level (#157)

## [2026.8.19] - 2026-08-19

### Breaking Changes

- feat(auth)!: require a credential unless anonymous read is asked for (#156)

### Features

- feat(site): scale the landing to the Poster composition (#153)
- feat(site): give the landing the Poster hero (#151)
- feat(site): restyle the documentation site to the Modernist direction (#149)

### Bug Fixes

- fix(auth): make a refused forge lookup visible at the default log level (#155)
- fix(site): serve the code font and give Archivo its italics (#152)
- fix(auth): accept a forge token that carries no permissions block (#147)
- fix(ci): keep the server publish workflows off site releases (#145)

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
