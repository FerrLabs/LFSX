# Quick start

### Run it

```bash
docker run -d --name lfsx \
  -p 8080:8080 \
  -v lfsx-data:/var/lib/lfsx \
  -e LFSX_PUBLIC_URL=https://lfs.example.com \
  ghcr.io/ferrlabs/lfsx:latest
```

Or without a container runtime, from the binaries attached to each
[release](https://github.com/FerrLabs/LFSX/releases), statically linked, so they need nothing
installed:

```bash
curl -fsSL https://github.com/FerrLabs/LFSX/releases/latest/download/lfsx-server-x86_64-unknown-linux-musl.tar.gz \
  | tar xz
LFSX_PUBLIC_URL=https://lfs.example.com ./lfsx-server
```

Replace `x86_64` with `aarch64` on a Raspberry Pi or an ARM server; `gnu` builds are there too if
you prefer dynamic linking. Every archive ships a `.sha256` next to it.

From crates.io, if you already have a Rust toolchain and would rather compile:

```bash
cargo install lfsx-server
```

Or from source, without the permission check, for a local trusted network:

```bash
LFSX_STORAGE_ROOT=./data LFSX_PUBLIC_URL=http://localhost:8080 LFSX_AUTH=disabled cargo run --release
```

### Point a repository at it

Commit a `.lfsconfig` at the root of the repository so every clone picks it up:

```ini
[lfs]
	url = https://lfs.example.com/my-org/my-project
```

The last two path segments are the organisation and the project; together they scope the storage.
Two repositories sharing a URL share their objects.

Then use Git LFS as usual:

```bash
git lfs install
git lfs track "*.psd"
git add .gitattributes assets/hero.psd
git commit -m "add hero artwork"
git push
```

> [!IMPORTANT]
> `git lfs install` must be run **before cloning**. Without it, files arrive as 130-byte pointer
> stubs and tools that read them (Unity, Unreal, image editors) fail in confusing ways.

### Move an existing repository off GitHub or GitLab LFS

Do this from a clone while the old endpoint still answers, and before you delete anything there.

1. Fetch every object the old endpoint holds, not only the ones the current checkout needs:

   ```bash
   git lfs fetch --all origin
   ```

2. Commit the `.lfsconfig` from the previous section, then push every object to the new server:

   ```bash
   git config lfs.url https://lfs.example.com/my-org/my-project
   git lfs push --all origin $(git for-each-ref --format='%(refname)' refs/heads refs/tags refs/remotes/origin)
   ```

   The refs are listed because `git lfs push --all` alone walks only local branches and tags, while
   the fetch above also covered the remote-tracking ones.

3. Check a fresh clone with `git lfs install` already run: the files should arrive as real content,
   not pointer stubs.

Two things stay behind. Tags and branches created before the `.lfsconfig` commit do not contain it,
so checking one out still resolves to the old endpoint. That matters when CI builds an old tag:
keep the old objects reachable until nothing builds from them, or pass the new URL to CI with
`git config lfs.url`. And contributors working from a fork usually have pull rights on the upstream
repository only, so they cannot push LFS objects to it. Their objects need to be pushed by someone
who can write there.

### Verify it works

```bash
lfsx --url https://lfs.example.com doctor --repo my-org/my-project
```

```bash
npm install -g @ferrlabs/lfsx        # or: cargo install lfsx
```

That checks the server is up, its storage is writable, your token is accepted, and that the URL it
advertises for transfers is the one you reached it on: the mismatch that lets negotiation succeed
while every transfer fails. Install it alongside the server, or use the probes directly:

```bash
curl -sf https://lfs.example.com/health && echo " up"
curl -sf https://lfs.example.com/ready  && echo " serving"
```

`/health` says the process is alive, which is what a restart would fix. `/ready` writes and removes
a probe file under the storage root, so a volume that is missing, full or mounted read-only takes
the instance out of rotation instead of accepting traffic it cannot serve. Point `livenessProbe` at
the first and `readinessProbe` at the second. Neither needs credentials.
