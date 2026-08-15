# Operations: backup, restore and disaster recovery

An LFS server holds the artwork that is too big for git — the files a studio cannot regenerate and
git itself is not keeping a copy of. This is the page you need on the day that goes wrong, so it
covers the restore as carefully as the backup.

## What to back up

`$LFSX_STORAGE_ROOT`, and nothing else. There is no database, no cache to warm and no state
anywhere but that directory. Configuration lives in the environment — back it up with whatever
holds your Helm values or unit file, not from here.

```
$LFSX_STORAGE_ROOT/.content/<oid[0:2]>/<oid[2:4]>/<oid>       the bytes, once
$LFSX_STORAGE_ROOT/<org>/<repo>/<oid[0:2]>/<oid[2:4]>/<oid>   a hard link per repository
$LFSX_STORAGE_ROOT/.locks/<org>/<repo>/<id>.json              who holds what
```

Locks are the one thing worth losing. They say which artist is currently editing which scene, they
are re-taken in seconds, and a lock restored from last night's backup is a lie about the present.
Copy them if it costs nothing, but never delay a restore over them.

Files ending in `.part` are uploads in flight. Copy or skip them as you please: on the restored
server they are reclaimed within `LFSX_STAGING_MAX_AGE`.

## Backup

**Objects are immutable.** A file is named after the SHA-256 of its own contents, so it is written
once and never rewritten. An incremental file-level backup copies each object exactly once, for as
long as it lives — the daily delta is genuinely new work, not a re-upload of the archive.

**Preserve hard links, or the restore expands.** The bytes are stored once under `.content` and
every repository holding them keeps a hard link. A copy that does not understand links turns one
40 GB asset pack shared by four repositories into 160 GB, and that is discovered when the restore
target runs out of space:

```bash
rsync -aH --delete /var/lib/lfsx/ backup@vault:/backups/lfsx/
```

`-H` is the whole point. `tar` preserves links by default; `cp -a` does not (`cp -a --link` is not
the same thing — it links the copy to the *source*, which is not a backup at all).

**The server can stay up.** An upload streams into a `.part` file and is renamed on success, and
rename is atomic, so no half-written object ever appears under its final name. The two things a
snapshot can catch mid-flight are harmless: a `.part` file, reclaimed on the next sweep, and an
object under `.content` whose repository link had not been made yet, which is collected the next
time that repository runs `retain`.

Volume snapshots — LVM, ZFS, a CSI `VolumeSnapshot` on the chart's PVC — work for the same reason
and are usually faster to restore than a file-level copy. They are also the only sensible option
once the store passes a few terabytes, where walking it costs more than copying it.

## Restore

1. **Restore the volume, then start the server** — not the other way around. A server pointed at a
   half-restored directory will answer `batch` saying objects are missing, and clients will
   cheerfully re-upload gigabytes you already have.

2. **Check ownership.** The container runs as uid 65532, and a restore performed as root leaves a
   store the server cannot write to. That is what `/ready` is for: it probes the root by writing to
   it, so a broken restore takes the instance out of rotation instead of failing every push.

   ```bash
   chown -R 65532:65532 /var/lib/lfsx
   ```

3. **Ask the server.** `/ready` covers the volume; `lfsx doctor` covers the deployment around it,
   including the public URL, which is the setting that makes negotiation succeed and every transfer
   fail:

   ```bash
   lfsx --url https://lfs.example.com doctor --repo FerrLabs/Blastlands
   ```

4. **Verify the bytes.** Every object is named after its own digest, so the store checks itself
   without a manifest — rehash each file and compare it to its filename:

   ```bash
   find "$LFSX_STORAGE_ROOT/.content" -type f -print0 |
     xargs -0 -P 8 -I{} sh -c 'expected=$(basename "$1"); actual=$(sha256sum <"$1" | cut -d" " -f1);
       [ "$expected" = "$actual" ] || echo "corrupt: $1"' _ {}
   ```

   **This holds only for a store written without `LFSX_COMPRESSION`.** With compression on, an
   object is no longer the bytes it is named after, so every compressed file would be reported as
   corrupt by the command above. Verifying a compressed store means decompressing it, which is a
   server-side job rather than a shell one-liner — until it exists, the honest procedure for such a
   store is to restore it and read objects back through the API.

   Silence means the store is intact. This reads every byte you hold, so it is I/O bound and worth
   an evening rather than a maintenance window — but it is the only check that distinguishes a
   restore that worked from one that merely finished. Objects pushed before deduplication existed
   live at their repository path with a single link and no `.content` entry; point the same command
   at `$LFSX_STORAGE_ROOT` to cover those too, at the cost of hashing shared objects once per
   repository that holds them.

Delete whatever the check reports. A corrupt object is worse than a missing one — it fails the
client's own hash verification after a full download, and it will do that on every retry. Once it
is gone, recover it the way the next section describes.

## When an object is missing

This is what a clone against a store that lost an object actually looks like:

```
Cloning into 'clone'...
done.
Downloading Assets/Hero.psd (4.1 KB)
Error downloading object: Assets/Hero.psd (b0ccacc): Smudge error: Error downloading
Assets/Hero.psd (b0ccacc3…): [b0ccacc3…] object not found: [404] object not found

error: external filter 'git-lfs filter-process' failed
fatal: Assets/Hero.psd: smudge filter lfs failed
warning: Clone succeeded, but checkout failed.
```

Read that last line carefully: **the clone succeeded.** Git history is intact — it was never on
this server. What failed is the checkout, and the file left in the working tree is the pointer, a
hundred-odd bytes of text where the artwork should be. A developer who does not read the warning
can commit on top of that and push the pointer back, so say so in the channel before saying
anything else.

**Do not run `git lfs fsck` in the broken clone.** It repairs what it can, which here means moving
the pointer into `.git/lfs/bad` — no help, and one more confused developer.

The recovery is a developer's local cache. Anyone who cloned before the loss still has the bytes in
`.git/lfs/objects`, and pushing them back is one command from a healthy clone:

```bash
git lfs fsck                    # confirm this clone is the healthy one
git lfs push --all origin       # re-upload every object for every ref
```

`--all` walks all refs rather than what the last push touched, so one developer with a complete
clone repopulates the repository. Pick whoever cloned earliest and has not run `git lfs prune`;
several people running it is harmless, since the second upload is negotiated away as already
present. Then clone the repository somewhere fresh and check that a real file comes back — the
push reporting success only means the server accepted the bytes.

Objects nobody holds any more are gone. That is the case backups exist for, and the reason to test
the restore before needing it.

## Folding an old store into the shared one

A server that predates 0.20.0 wrote every object as a plain file under its repository. Those keep
serving, but they never deduplicate, so two projects holding the same pack pay for it twice. One
call per repository fixes that:

```bash
lfsx dedupe --repo FerrLabs/Blastlands --dry-run   # what it would move and free
lfsx dedupe --repo FerrLabs/Blastlands
```

Run it against every repository before judging the saving: the first one to fold in moves its bytes
into the shared store and frees nothing, and the second is the one that stops paying. Re-running is
safe and reports nothing left to do.

It is deliberately conservative. Objects are verified against their own digest before being admitted
or adopted, so neither a corrupt file nor a corrupt shared entry can spread; anything that fails is
counted as `refused`, named in the log and left exactly where it is. The repository's file is never
removed before its replacement link exists, so an interrupted run leaves either the old file or the
new link, never a gap.

The saving shows up in `lfsx_store_bytes`, which counts what the disk holds, rather than in
`stats`, which counts what each repository holds.

## Migrating to another server

Two routes, and the choice is usually made for you by whether you can read the old server's disk.

**Copy the store.** Fastest, keeps everything, needs filesystem access to both:

```bash
rsync -aH --delete /var/lib/lfsx/ newhost:/var/lib/lfsx/
```

Run it once while the old server is still serving, switch the clients, then run it again — the
second pass moves only what arrived in between, which for immutable objects is quick. Update
`LFSX_PUBLIC_URL` on the new server before pointing anyone at it, and `lfsx doctor` against it
before announcing anything.

**Re-push from clones.** Slower and only as complete as the clones you have, but it needs no access
to the old machine — which is the situation when the old server encrypted its store, as rudolfs
does, and the files on disk are unreadable by anything but the server that wrote them:

```bash
git config lfs.url https://lfs.example.com/FerrLabs/Blastlands
git lfs push --all origin
```

Committing the new URL in `.lfsconfig` is what moves everyone else, so do that in the same change.

Either way, keep the old store until a fresh clone from the new server round-trips a real file.
Deleting it earlier turns a migration into the previous section.
