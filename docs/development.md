# Development

```bash
cargo test                              # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Integration tests mount the router on a temporary directory and drive it through
`tower::ServiceExt::oneshot`, so they exercise real routing, real streaming and the real
filesystem without binding a port.

```bash
bash ci/e2e.sh                          # push and clone through a real git lfs client
```

That one starts the binary and a stub forge, pushes a large asset with the actual client, clones it
back and compares the bytes, then takes a lock, fails to steal it and releases it. It runs on an
isolated `GIT_CONFIG_GLOBAL`, so it cannot touch your own git configuration.

It runs on every push against **Linux, macOS and Windows**, plus **git-lfs 3.0.2**, the oldest
version supported, since that is where the locking API settled. The clients a studio actually runs
are rarely the newest: Git for Windows, GitHub Desktop, Sourcetree, Rider and Unity each bundle
their own copy. [Clients](clients.md) records what is covered and carries a short
manual checklist for the graphical clients, which cannot be automated and are what the artists
will be using.

```bash
cargo llvm-cov --workspace --lcov --output-path lcov.info
```

The same command CI runs before handing the report to
[sonar.ferrlabs.com](https://sonar.ferrlabs.com), which tracks coverage, duplication and smells
over time. A pull request is analysed into its own project and the workflow comments what that
change introduced, since the Community edition has no pull-request analysis of its own.
