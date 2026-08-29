# Compression

`LFSX_COMPRESSION=zstd` stores objects compressed. Unset, nothing is compressed, which is the
default because it is the one that cannot surprise anyone.

```bash
LFSX_COMPRESSION=zstd        # level 3
LFSX_COMPRESSION=zstd:9      # slower, smaller
```

The received wisdom is that an LFS store is already compressed, and for PNG, MP3 and OGG that is
true. It is badly wrong for meshes: measured on two real Unity projects, `.fbx` compresses **2.9×**
and **6.7×**, `.tga` **10.4×**, while `.png` gives up 1%. On a store where meshes are the bulk,
which is what a game project looks like, that came out at **71% smaller** overall.

**The object keeps its name.** A file is still stored under the digest of its plaintext, because
that name is what collection, deduplication and the shared store address it by. What changes is
that the file says what it is in its first bytes, which means two things worth knowing: turning
compression on rewrites nothing that is already stored, and turning it off again keeps serving
everything written while it was on. A mixed store is the normal state of an upgraded one.

**Ranges still work.** Objects are compressed in four-megabyte frames with an index, so serving the
tail of a three-gigabyte asset decompresses the frames it touches rather than everything before
them. A resumed transfer stays a resumed transfer.

**Memory stays flat.** One frame is decompressed at a time, whatever the object weighs.

**Objects that will not compress are stored as they arrived.** A frame that gives up less than five
percent of itself is written raw and flagged, and an object whose framed form would be no smaller is
left exactly as it was. Half a game store is PNG and OGG; none of it pays for the attempt twice.

Turning compression on only changes what arrives next. Folding in what is already stored is one call
per repository, with the same shape as the deduplication migration:

```bash
lfsx compress --repo FerrLabs/Blastlands --dry-run
lfsx compress --repo FerrLabs/Blastlands
```

The dry run compresses everything and writes nothing, so the figure it reports is the real saving
rather than an estimate. Each object is verified against its own digest before being rewritten: the
last moment that check is a simple one, since afterwards the file is no longer the bytes it is named
after. Anything that fails is left alone and counted as `refused`.

Objects shared between repositories stay shared: the copy under `.content` is what gets replaced and
this repository is relinked to it, so run it everywhere. Until a repository has had its turn, it
keeps the older bytes alive through its own link, and the store holds both forms.

What it costs: CPU on both ends of every transfer, and an integrity check that can no longer be a
`sha256sum`: `lfsx verify --repo <org/repo>` is what replaces it, reading every object back the way
a download does. See [Operations](operations.md). `stats`, the dashboard and
`LFSX_REPO_QUOTA` all count bytes on disk, so a repository's figure drops when compression is on;
that is the number an operator budgets a volume against.
