# LFSX

Self-hosted Git LFS server, Rust + axum. Cargo workspace with a single crate, `server/`
(binary `lfsx-server`).

## Layout

```
server/src/
  model.rs            # batch protocol types (request, response, actions, errors)
  namespace.rs        # validated {org}/{repo} pair, the only way to address storage
  storage.rs          # LocalStore: paths, streaming write + verification, reads
  routes.rs           # axum handlers and router wiring
  auth.rs             # Permission, Authorizer, the middleware over the object routes
  auth/credentials.rs # Basic and Bearer parsing
  auth/github.rs      # token -> permission on {org}/{repo}
  auth/cache.rs       # short-lived permission cache, keyed by token digest
  state.rs            # AppState shared by the handlers and the middleware
  config.rs           # environment variables and public URL construction
  error.rs            # domain errors and their HTTP mapping
  lib.rs              # app(config) -> Router
  main.rs             # bootstrap
server/tests/api.rs    # protocol and storage
server/tests/auth.rs   # middleware against a stub forge
```

## Invariants that must not break

- **Never emit `authenticated` in the batch response.** Advertising `"authenticated": true`
  without supplying a header makes the client send transfers with no credentials, and it loops
  on 401. This is the flaw that makes rudolfs unusable behind an authenticating reverse proxy.
  A test pins this down.
- **SHA-256 is recomputed while streaming** on every upload and compared against the declared
  oid. Content that does not match is rejected and nothing is left on disk.
- **Atomic writes**: staging file then `rename`, never a direct write to the final location.
- **Permissions come from the forge, never from LFSX.** The token is resolved against the upstream
  repository and mapped to read or write. Do not add accounts, shared secrets or a local user
  table. `LFSX_AUTH=disabled` is for local development only and must stay opt-in.
- **Path segments are validated before they reach the filesystem**: `Namespace::new` is the only
  way to build one, and `LocalStore::validate_oid` guards the object id. Both keep a crafted
  request inside the storage root.
- **Nothing is loaded into memory**: uploads and downloads stream, objects routinely run to
  several gigabytes.

## Conventions

See the parent workspace CLAUDE.md. In short: no explanatory comments, idiomatic Rust, one
responsibility per file, YAGNI. Single-line Conventional Commits.

## Releases

FerrFlow owns the version. It reads the Conventional Commits on `main`, bumps
`server/Cargo.toml`, refreshes `Cargo.lock`, writes `CHANGELOG.md`, tags and creates the GitHub
release; publishing the release builds and pushes `ghcr.io/ferrlabs/lfsx`. Never bump the version
or edit the changelog by hand, and keep PR titles conventional — the squash message is what drives
the bump.

Versioning is `zerover`: the major stays at `0`, a feature or a breaking change bumps the minor,
a fix bumps the patch. Going to `1.0.0` is a deliberate decision, and it belongs with
authentication (#1), not with whatever commit happens to carry a `!`.

## Tests

`cargo test`. Integration tests mount the router on a `tempdir` and drive it through
`tower::ServiceExt::oneshot`. Test behaviour and error paths, not the presence of routes.

`ci/e2e.sh` is the other half: it starts the real binary and `server/examples/stub-forge.rs`,
then pushes and clones through the actual `git lfs` client. It runs in CI and locally, on an
isolated `GIT_CONFIG_GLOBAL` so it cannot touch your own git configuration. Anything that depends
on how the client behaves — challenge handling, smudge, the locking probe — belongs there rather
than in an `oneshot` test, because a handcrafted request will not reproduce it.
