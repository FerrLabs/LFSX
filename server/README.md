# lfsx-server

A fast, lightweight, secure Git LFS server: streaming transfers, digest verification on every
upload, permissions mirrored from the upstream Git repository, and file locking.

```bash
cargo install lfsx-server
LFSX_STORAGE_ROOT=./data LFSX_PUBLIC_URL=https://lfs.example.com lfsx-server
```

Most deployments are better served by the container image or the prebuilt binaries, since
`cargo install` compiles from source.

Configuration, the API, authentication, locking and reclaiming space are documented at
[github.com/FerrLabs/LFSX](https://github.com/FerrLabs/LFSX#readme).
