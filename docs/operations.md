# Operations: backup, restore and disaster recovery

An LFS server holds the artwork that is too big for git: the files a studio cannot regenerate and
git itself is not keeping a copy of. This is the page you need on the day that goes wrong, so it
covers the restore as carefully as the backup.

## What to back up

`$LFSX_STORAGE_ROOT`, and the encryption key if you use one. There is no database and no cache to
warm. Configuration lives in the environment: back it up with whatever holds your Helm values or
unit file, not from here. This is the layout of a volume deployment; with `LFSX_STORAGE=s3`, `azure`
or `gcs` the state is in the bucket instead, see [Backing up a bucket](#backing-up-a-bucket).

```
$LFSX_STORAGE_ROOT/.content/<oid[0:2]>/<oid[2:4]>/<oid>       the bytes, once
$LFSX_STORAGE_ROOT/<org>/<repo>/<oid[0:2]>/<oid[2:4]>/<oid>   a hard link per repository
$LFSX_STORAGE_ROOT/.locks/<org>/<repo>/<id>.json              who holds what
$LFSX_STORAGE_ROOT/.lfsx/access.json                          allowed / restricted saved from the dashboard
$LFSX_STORAGE_ROOT/.lfsx/access-<forge>.json                  the same, one per named forge
$LFSX_STORAGE_ROOT/.lfsx/dashboard-tokens.json                dashboard tokens, stored as SHA-256 hashes
$LFSX_STORAGE_ROOT/.lfsx/session.key                          the key dashboard sessions are signed with
```

The `.lfsx/` files are small and easy to forget, and each one fails differently when it is missing
from a restore:

| Missing after a restore | What happens |
|---|---|
| `access.json`, `access-<forge>.json` | `LFSX_ALLOWED` and `LFSX_RESTRICTED` from the environment apply again, with no warning. If the dashboard had narrowed them, the restored server is more open than the one you lost. |
| `dashboard-tokens.json` | No dashboard token exists any more and the sessions they opened end. Admins of `LFSX_DASHBOARD_REPO` can still sign in with a forge token; reissue the others with `lfsx-server dashboard token create`. With `LFSX_AUTH=disabled` nobody can sign in until you do. |
| `session.key` | A new key is made on first use and every open dashboard session ends. Nothing else depends on it. |

The reverse holds too: an `access.json` restored from last month brings last month's lists back, and
what the dashboard saved takes precedence over the environment. The server rereads the file every 30
seconds, so check the Access page after a restore instead of assuming the environment is in force.

**The encryption key is not in the storage root, and it is not optional.** With
`LFSX_ENCRYPTION_KEY_FILE` or `LFSX_ENCRYPTION_KEY_COMMAND` set, every object written since is
ciphertext and the key is the only way back to it. Lose it and those objects cannot be read from the
store, a snapshot or a bucket. Keep every key the file lists, not just the first: an object records
which key it was written under, and one whose key is gone is reported unreadable by name. Keep the
key somewhere other than the backup of the data, or the encryption protects nothing the backup did
not already give away. On Kubernetes that is the Secret behind `encryption.existingSecret`. See
[Encryption at rest](encryption.md).

Locks are the one thing worth losing. They say which artist is currently editing which scene, they
are re-taken in seconds, and a lock restored from last night's backup is a lie about the present.
Copy them if it costs nothing, but never delay a restore over them.

Files ending in `.part` are uploads in flight. Copy or skip them as you please: on the restored
server they are reclaimed within `LFSX_STAGING_MAX_AGE`.

## Backup

**Objects are immutable.** A file is named after the SHA-256 of its own contents, so it is written
once and never rewritten. An incremental file-level backup copies each object exactly once, for as
long as it lives: the daily delta is genuinely new work, not a re-upload of the archive.

**Preserve hard links, or the restore expands.** The bytes are stored once under `.content` and
every repository holding them keeps a hard link. A copy that does not understand links turns one
40 GB asset pack shared by four repositories into 160 GB, and that is discovered when the restore
target runs out of space:

```bash
rsync -aH --delete /var/lib/lfsx/ backup@vault:/backups/lfsx/
```

`-H` is the whole point. `tar` preserves links by default; `cp -a` does not (`cp -a --link` is not
the same thing: it links the copy to the *source*, which is not a backup at all).

**The server can stay up.** An upload streams into a `.part` file and is renamed on success, and
rename is atomic, so no half-written object ever appears under its final name. The two things a
snapshot can catch mid-flight are harmless: a `.part` file, reclaimed on the next sweep, and an
object under `.content` whose repository link had not been made yet, which is collected the next
time that repository runs `retain`.

Volume snapshots (LVM, ZFS, a CSI `VolumeSnapshot` on the chart's PVC) work for the same reason
and are usually faster to restore than a file-level copy. They are also the only sensible option
once the store passes a few terabytes, where walking it costs more than copying it.

## Restore

1. **Restore the volume, then start the server**, not the other way around. A server pointed at a
   half-restored directory will answer `batch` saying objects are missing, and clients will
   cheerfully re-upload gigabytes you already have.

2. **Check ownership.** The container runs as uid 65532, and a restore performed as root leaves a
   store the server cannot write to. That is what `/ready` is for: it probes the root by writing to
   it, so a broken restore takes the instance out of rotation instead of failing every push. On a
   bucket deployment the volume is still staging, so that probe still matters and still runs.

   ```bash
   chown -R 65532:65532 /var/lib/lfsx
   ```

3. **Ask the server.** `/ready` covers the storage the instance actually serves from: the volume
   always, and with `LFSX_STORAGE=s3` the bucket as well, so a rotated key or a bucket that is gone
   takes the instance out instead of failing every transfer with a 500. The response says which of
   the two answered. `lfsx doctor` covers the deployment around it,
   including the public URL, which is the setting that makes negotiation succeed and every transfer
   fail:

   ```bash
   lfsx --url https://lfs.example.com doctor --repo FerrLabs/Blastlands
   ```

4. **Verify the bytes.** Every object is named after its own digest, so the store checks itself
   without a manifest: rehash each file and compare it to its filename:

   ```bash
   find "$LFSX_STORAGE_ROOT/.content" -type f -print0 |
     xargs -0 -P 8 -I{} sh -c 'expected=$(basename "$1"); actual=$(sha256sum <"$1" | cut -d" " -f1);
       [ "$expected" = "$actual" ] || echo "corrupt: $1"' _ {}
   ```

   **This holds only for a store written without `LFSX_COMPRESSION`.** With compression on, an
   object is no longer the bytes it is named after, so every compressed file would be reported as
   corrupt by the command above. Ask the server instead, which reads each object back the way a
   download does and checks it against its own name:

   ```bash
   lfsx verify --repo FerrLabs/Blastlands
   ```

   It names what fails and separates the two kinds of failure: an object that read fine and is not
   what it claims, and one that could not be read at all, the answer a failing disk gives first.
   It also says when it could not see the whole repository, because an audit that quietly skipped
   part of it would read as a clean bill. It exits non-zero on any of the three, so it belongs in a
   cron as much as in a restore.

   Silence means the store is intact. This reads every byte you hold, so it is I/O bound and worth
   an evening rather than a maintenance window, but it is the only check that distinguishes a
   restore that worked from one that merely finished. Objects pushed before deduplication existed
   live at their repository path with a single link and no `.content` entry; point the same command
   at `$LFSX_STORAGE_ROOT` to cover those too, at the cost of hashing shared objects once per
   repository that holds them.

Delete whatever the check reports. A corrupt object is worse than a missing one: it fails the
client's own hash verification after a full download, and it will do that on every retry. Once it
is gone, recover it the way the next section describes.

## Backing up a bucket

With `LFSX_STORAGE=s3`, `azure` or `gcs` the objects, the locks and the `.lfsx/` files above are all
keys in the bucket: `.content/`, `<org>/<repo>/` markers, `.locks/`, `.lfsx/`, plus the `.refs/`
and `.sizes/` indexes. The volume is a write buffer and holds nothing worth keeping, so there is
nothing to back up there and no `.part` file to care about. The encryption key is still yours to
keep, exactly as above.

`rsync -aH` does not apply, and the hard-link warning goes with it: a marker is an empty key, not a
link, so any tool that copies keys is a faithful backup. Copy the whole bucket, indexes included,
and keep `.lfsx/` in the copy, since the table above applies to it unchanged.

What protects a bucket is configured on the bucket, not in this server. Two settings matter:

- **Versioning** (S3 and GCS object versioning, Azure blob versioning or soft delete) is the answer
  to a deletion. Collection and `retain` remove content objects and markers for real, and a
  versioned bucket turns that into a delete marker you can undo. Objects are never rewritten, so
  versioning adds nothing for them beyond that. It does keep every superseded `access.json` and
  `dashboard-tokens.json`, which is useful, and every lock that was ever taken and released, which
  is not: add a lifecycle rule that expires noncurrent versions under `.locks/`.
- **Replication** to a second bucket or region survives losing the first, and replicates deletions
  and mistakes along with everything else, so it is a second copy and not a backup. Keep it in another account if a leaked credential is part
  of what you plan for. It is also
  asynchronous and does not order keys: a replica can hold a marker whose content object has not
  arrived. After promoting one, run `lfsx doctor` per repository and expect to re-push what it
  reports missing.

Restoring is copying the keys back to an empty bucket, or promoting a replica, and pointing
`LFSX_S3_BUCKET` (or the Azure or GCS equivalent) at it. Start the server afterwards: `/ready`
checks the bucket as well as the volume, and the locking probe runs at boot against the restored
bucket. `lfsx verify` answers `501` against a bucket, so the byte-level check in the restore
section is unavailable; `lfsx doctor` is the check that the restore is whole.

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

Read that last line carefully: **the clone succeeded.** Git history is intact: it was never on
this server. What failed is the checkout, and the file left in the working tree is the pointer, a
hundred-odd bytes of text where the artwork should be. A developer who does not read the warning
can commit on top of that and push the pointer back, so say so in the channel before saying
anything else.

**Do not run `git lfs fsck` in the broken clone.** It repairs what it can, which here means moving
the pointer into `.git/lfs/bad`: no help, and one more confused developer.

The recovery is a developer's local cache. Anyone who cloned before the loss still has the bytes in
`.git/lfs/objects`, and pushing them back is one command from a healthy clone:

```bash
git lfs fsck                    # confirm this clone is the healthy one
git lfs push --all origin       # re-upload every object for every ref
```

`--all` walks all refs rather than what the last push touched, so one developer with a complete
clone repopulates the repository. Pick whoever cloned earliest and has not run `git lfs prune`;
several people running it is harmless, since the second upload is negotiated away as already
present. Then clone the repository somewhere fresh and check that a real file comes back, the
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

`gc`, `dedupe`, `compress` and `verify` walk the whole repository in one request and wait for the
answer for as long as the server takes. Only the connection attempt has a limit, ten seconds;
`doctor` also gives up on a server that stops answering after thirty.

The saving shows up in `lfsx_store_bytes`, which counts what the disk holds, rather than in
`stats`, which counts what each repository holds.

## Folding an old store into compressed frames

`LFSX_COMPRESSION=zstd` changes what arrives next; what is already stored keeps its own shape until
told otherwise.

```bash
lfsx compress --repo FerrLabs/Blastlands --dry-run
lfsx compress --repo FerrLabs/Blastlands
```

The dry run does the whole job and throws the result away, so its number is measured rather than
estimated. Run it for every repository: objects shared between them live once under `.content`, and
a repository that has not had its turn keeps the older bytes alive through its own link.

The same conservatism as the deduplication migration, verified against its own digest before being
rewritten, never removed before its replacement exists, idempotent, refusals counted and named. An
object that would not get smaller is left alone rather than wrapped.

## Restarts and rollouts

On SIGTERM the server stops accepting new connections and lets transfers already running finish, so
a restart interrupts only what outlives the wait. Give the process as long as your largest transfer
needs: on Kubernetes that is `terminationGracePeriodSeconds`, an hour in the chart, with a short
`preStop` (`lfsx-server sleep <seconds>`) so the endpoint is removed before the listener closes.
Under systemd or Docker, raise `TimeoutStopSec` or `--stop-timeout` the same way. A transfer cut
anyway leaves a partial file in staging, which the next boot and the hourly sweep reclaim.

## Migrating to another server

Two routes, and the choice is usually made for you by whether you can read the old server's disk.

**Copy the store.** Fastest, keeps everything, needs filesystem access to both:

```bash
rsync -aH --delete /var/lib/lfsx/ newhost:/var/lib/lfsx/
```

Run it once while the old server is still serving, switch the clients, then run it again, the
second pass moves only what arrived in between, which for immutable objects is quick. Update
`LFSX_PUBLIC_URL` on the new server before pointing anyone at it, and `lfsx doctor` against it
before announcing anything.

**Re-push from clones.** Slower and only as complete as the clones you have, but it needs no access
to the old machine, which is the situation when the old server encrypted its store, as rudolfs
does, and the files on disk are unreadable by anything but the server that wrote them:

```bash
git config lfs.url https://lfs.example.com/FerrLabs/Blastlands
git lfs push --all origin
```

Committing the new URL in `.lfsconfig` is what moves everyone else, so do that in the same change.

Either way, keep the old store until a fresh clone from the new server round-trips a real file.
Deleting it earlier turns a migration into the previous section.
