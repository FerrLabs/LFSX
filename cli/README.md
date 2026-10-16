# lfsx

Command line companion for a self-hosted [LFSX](https://github.com/FerrLabs/LFSX) Git LFS server.

```bash
npm install -g @ferrlabs/lfsx        # or: cargo install lfsx

lfsx --url https://lfs.example.com doctor --repo my-org/my-project
lfsx --url https://lfs.example.com gc --repo my-org/my-project --dry-run
```

The token comes from `--token` or `LFSX_TOKEN`, and only those: `GITHUB_TOKEN` is not read. A token
is only sent over `https`, or `http` to `localhost`, `127.0.0.1` and `[::1]`; any other URL is
refused. In GitHub Actions, pass the job token explicitly with
`LFSX_TOKEN: ${{ github.token }}`.

`doctor` checks that the server is up, that its storage is writable, that your token is accepted,
and — the one that matters — that the URL it advertises for transfers is the URL you reached it
on. A mismatch there lets negotiation succeed while every transfer fails.

`gc` reads the objects the repository still references and asks the server to sweep the rest. It
fetches every branch, tag and pull or merge request ref first (`--no-review-refs` skips the last),
reads `--url` and `--repo` from `git config lfs.url` when not given, and refuses a shallow or
`--single-branch` clone, or a `--url` or `--repo` that does not match `lfs.url`.

The server itself is [`lfsx-server`](https://crates.io/crates/lfsx-server).
