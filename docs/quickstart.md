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
