# lfsx

Command line companion for a self-hosted [LFSX](https://github.com/FerrLabs/LFSX) Git LFS server.

```bash
npm install -g @ferrlabs/lfsx        # or: cargo install lfsx

lfsx --url https://lfs.example.com doctor --repo my-org/my-project
lfsx --url https://lfs.example.com gc --repo my-org/my-project --dry-run
```

`doctor` checks that the server is up, that its storage is writable, that your token is accepted,
and — the one that matters — that the URL it advertises for transfers is the URL you reached it
on. A mismatch there lets negotiation succeed while every transfer fails.

`gc` reads the objects the repository still references and asks the server to sweep the rest. It
refuses to run from a shallow clone, which would retain a fraction of what it should.

The server itself is [`lfsx-server`](https://crates.io/crates/lfsx-server).
