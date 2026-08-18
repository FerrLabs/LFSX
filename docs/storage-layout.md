# Storage layout

Objects are content-addressed and fanned out two levels to keep directories small:

```
$LFSX_STORAGE_ROOT/.content/<oid[0:2]>/<oid[2:4]>/<oid>   the bytes, once
$LFSX_STORAGE_ROOT/<org>/<repo>/<oid[0:2]>/<oid[2:4]>/<oid>   a hard link per repository
```

**The bytes are stored once.** Two projects sharing the same Synty or Quixel pack cost the disk
once, however many repositories push it — and for a studio that is most of the disk. Each
repository holds a hard link, so the filesystem keeps the reference count and the content survives
until the last repository lets go of it.

Sharing the bytes does not share them over the API. Every route resolves through the repository's
own path, so a repository cannot read, list or even learn the existence of an object it never
pushed — including by guessing a digest. `retain` frees space only when the object it drops was the
last reference; the report says zero bytes otherwise, rather than promising space another
repository is still using.

Objects already stored per repository, from before this, keep working untouched: they are ordinary
files with a single link. Nothing needs migrating for them to serve — but they never collapse
either, so a server that has been running since before 0.20.0 keeps paying full price for every
pack two projects share. Folding them in is one call per repository:

```bash
lfsx dedupe --repo FerrLabs/Blastlands --dry-run
lfsx dedupe --repo FerrLabs/Blastlands
```

It moves each object into the shared store and links it back, or links to what is already there and
frees the copy. Bytes are only freed by the *second* repository to hold them, so run it everywhere
before judging the result. Running it again does nothing: an object already sharing its inode is
recognised and skipped.

Two things it refuses rather than risk. An object whose bytes do not hash to its own name never
enters the shared store, because everything that links there later would inherit them. And a shared
entry that does not match its name is never adopted, so a repository with a good copy keeps it. Both
are counted as `refused` and named in the log.

The repository's file is never removed before its replacement exists: the link is made under a
temporary name and renamed over the original, so an interrupted run leaves either the old file or
the new link. It needs admin rights on the repository.

Backing up the server is backing up that directory. Objects are immutable, so an incremental
file-level backup never rewrites what it already copied — but use a tool that preserves hard links
(`rsync -H`, `tar`), or the copy will expand every shared object back into a separate file.

[Operations](operations.md) covers the rest of it: restoring, verifying the store
against its own digests afterwards, what a client sees when an object is missing and how to get it
back from a developer's cache, and migrating between servers.
