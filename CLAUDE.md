# LFSX

Self-hosted Git LFS server, Rust + axum. Cargo workspace with a single crate, `server/`
(binary `lfsx-server`).

## Layout

```
server/src/
  model.rs     # batch protocol types (request, response, actions, errors)
  storage.rs   # LocalStore: paths, streaming write + verification, reads
  routes.rs    # axum handlers and router wiring
  config.rs    # environment variables and public URL construction
  error.rs     # domain errors and their HTTP mapping
  lib.rs       # app(config) -> Router
  main.rs      # bootstrap
server/tests/api.rs
```

## Invariants that must not break

- **Never emit `authenticated` in the batch response.** Advertising `"authenticated": true`
  without supplying a header makes the client send transfers with no credentials, and it loops
  on 401. This is the flaw that makes rudolfs unusable behind an authenticating reverse proxy.
  A test pins this down.
- **SHA-256 is recomputed while streaming** on every upload and compared against the declared
  oid. Content that does not match is rejected and nothing is left on disk.
- **Atomic writes**: staging file then `rename`, never a direct write to the final location.
- **Nothing is loaded into memory**: uploads and downloads stream, objects routinely run to
  several gigabytes.

## Conventions

See the parent workspace CLAUDE.md. In short: no explanatory comments, idiomatic Rust, one
responsibility per file, YAGNI. Single-line Conventional Commits.

## Tests

`cargo test`. Integration tests mount the router on a `tempdir` and drive it through
`tower::ServiceExt::oneshot`. Test behaviour and error paths, not the presence of routes.
