# Changelog

All notable changes to `lfsx-server` will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

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
