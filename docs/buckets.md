# Objects in a bucket

`LFSX_STORAGE=s3` puts the objects in an S3-compatible bucket (MinIO, Garage, Backblaze, AWS)
instead of on the volume, which is what unties capacity from one machine.

```bash
LFSX_STORAGE=s3
LFSX_S3_ENDPOINT=https://s3.example.com
LFSX_S3_BUCKET=assets
LFSX_S3_ACCESS_KEY=…
LFSX_S3_SECRET_KEY=…
```

**Locks move with the objects.** They are keys under `.locks/` in the same bucket, taken with a
conditional write, `If-None-Match: *`, so the store itself decides who arrived first and answers
`412` to everyone after. That is the mutual exclusion `create_new` gives on a filesystem, and it is
what makes a second replica possible: two servers sharing one bucket and nothing else agree on who
holds a scene, and a release on either frees it on both. Tested against MinIO rather than assumed,
including the case where both replicas ask at once.

That leans entirely on the store performing the condition, and not every implementation does, so it
is asked at startup: one key written twice, with the second required to be refused. A store that
writes both, or that cannot be asked, loses locking rather than pretending, and taking a lock there
answers `501`. [Locking](locking.md#what-makes-a-lock-unique) has the reasoning.

The volume is still needed as a write buffer for uploads, but it no longer carries state a second
replica would have to see.

**The layout is the one on disk.** The bytes live once under a key derived from their digest, and a
repository that holds them owns an empty marker beside it. That marker is the bucket's answer to a
hard link: it is why two projects sharing an asset pack cost the bucket once, and it is the only
thing consulted when a repository asks for an object, so a digest cannot be guessed into someone
else's assets.

**Transfers go through the server by default.** It streams to and from the bucket rather than
handing out a redirect, which is what keeps the byte counters, the ranges, the size ceiling and the
quota working. A download asks the bucket for exactly the range it wants, so resuming stays cheap.
The local volume is still used, as a write buffer: an upload lands there to be hashed and checked
before anything is sent, and is deleted once it has been.

**An object over 5 GiB goes up in parts.** S3 caps a single `PutObject` there, which for packaged
assets and captured footage is a ceiling reached rather than a theoretical one. Above it the object
is sent in parts of 64 MiB, grown when ten thousand of them would not cover it, and the store
assembles them: the key appears whole or not at all. A part that fails takes the upload with it and
the upload is abandoned explicitly, because parts of an incomplete upload are charged for and appear
in no listing, so nothing else would ever find them.

Each part is streamed out of the staging file rather than read into memory. The whole storage layer
is built on holding a few megabytes of an object at a time, and a part size does not get to change
that.

**`LFSX_S3_PRESIGN=true` redirects transfers instead.** The batch response hands the client a
pre-signed URL and the bytes never cross this server, which is what you want when the bucket is
closer to the clients than the server is, or when the server's egress is the thing you are paying
for.

Uploads go the same way, and the two things that made that unsafe are both closed rather than
accepted:

**The digest is bound into the signature.** The URL comes with an `x-amz-checksum-sha256` header the
client must send, and it is part of what was signed, so a conforming store refuses any body that does
not hash to the object the URL was cut for. A client holding an upload URL cannot put arbitrary bytes
behind it.

Conforming is doing the work in that sentence, so it is checked rather than assumed. At startup, with
`LFSX_S3_PRESIGN=true`, the server signs one upload, sends a body that deliberately does not match,
and looks at whether the bytes landed. What the store answered is not the question, since stores
disagree about which status says no; whether the key exists afterwards is exactly the property a
pre-signed upload rests on.

A store that kept them loses pre-signing, with an error in the log saying so, and uploads go back to
coming through this server, which hashes what it is sent. So does a store that could not be asked: an
unanswered question is not a yes. Losing it costs throughput and nothing else.

That is a heavy reaction to a header, and the reason is that the damage would not be one bad object.
Bytes live once at `.content/{oid}`, and a repository pushing a digest that is already there writes a
marker and uploads nothing. So whoever put the wrong bytes there put them in every repository that
will ever hold that object. `x-amz-checksum-*` is a late addition to the S3 API, and accepting a
header while ignoring it is the ordinary shape of an incomplete implementation.

**The upload goes to a key only that repository was signed for**, under `.incoming/`, not to the
shared content key. That is what keeps possession meaning something: bytes arriving there prove the
repository had the object, where a write to the shared key would prove only that somebody with push
rights knew a digest, which is all a leaked pointer file is. On `verify` the server checks the size
that actually arrived against the declared one, the object ceiling and the repository's budget, then
moves the object into the shared keyspace with a copy inside the bucket and writes the marker.

**An object over 5 GiB is not given an upload URL**, and comes through this server instead. A
pre-signed upload is one PUT to one href, because that is all the `basic` transfer adapter can do,
so the ceiling multipart removes for the streamed path is permanent for this one. Withholding the
URL is not a refusal: the push goes through, over the path that can send it in parts. It also keeps
`adopt` honest, since `CopyObject` stops at the same 5 GiB and nothing can now reach `.incoming/`
above it.

An upload nobody reports leaves its bytes under that key, and they are swept on the same schedule
and by the same setting as an interrupted transfer's staging file: once at boot, then hourly, for
anything older than `LFSX_STAGING_MAX_AGE`. A key written a moment ago is left alone, because a slow
client on a bad connection is not an abandoned one.

Two consequences worth knowing. `lfsx_uploaded_bytes` does not move for these, because nothing here
measured them and counting the client's own figure would make that counter mean two different things.
And **a server with `LFSX_ENCRYPTION_KEY_FILE` set keeps carrying uploads itself**: an object a client
writes directly arrives as it is, so handing out an upload URL would put plaintext in the bucket while
the operator believed otherwise. Encryption is a promise about what the storage provider can read, and
a faster upload is not worth breaking it quietly. The server says so at startup. With compression the
trade is milder and allowed: objects uploaded directly are simply not compressed.

**A download is redirected only when the bucket holds the object itself.** A pre-signed URL hands the
client whatever sits under that key, so with `LFSX_COMPRESSION` or `LFSX_ENCRYPTION_KEY_FILE` set it
would hand over a frame written under the plaintext digest. The client hashes what arrives, gets a
digest that is not the one it asked for, and rejects the object. With either of them set the redirect
is given up and downloads keep streaming, which is the only path that can decode. Startup says which
of the two is happening.

Compression is enough on its own to stop it, even though it still permits a direct upload. That
asymmetry is deliberate. An object that was never framed is a perfectly good entry, so uploading
straight to the bucket stays safe, while a single framed object anywhere in the store makes every
redirect a guess about which kind this one is.

The permission check does not move. A signature is cut only after the marker says this repository
holds the object, exactly as a plain download is refused without it, and the URL it signs is scoped
to one object and expires with the action. What changes is what the server can still see:
`lfsx_downloaded_bytes` stops counting bytes it no longer carries, and the bucket serves the ranges.

This is the one case where the batch response says `"authenticated": true`, and the one case where
it is true: see [below](#why-authentication-cannot-live-in-the-proxy) for why that field is a trap
everywhere else.

**What one repository holds is read from a listing.** A listing reports each key's own length, and a
marker is empty, so the size of the object a marker claims is not in it: that lives on the content
key, under a name the repository's prefix never reaches. Measuring a repository therefore used to
cost one `HEAD` per object, which for fifty thousand of them was fifty thousand requests every time
the cached figure expired.

So the number goes into a key name, where a listing can read it: one empty object per marker at
`{org}/{repo}/.sizes/{oid}.{size}`, and the total is the sum of what the names say. The same move
`.refs/` made for the reverse lookup, and for the same reason. The bucket is the only database here,
and an index is what a database would have given for nothing.

A bucket written before the index has markers and no sizes. The first reading measures it the old
way and writes down what it learned, so there is nothing to run and no flag to set: it converges by
being used. An entry whose marker has gone is counted by nobody, since only a marker says a
repository holds anything, so a sweep that fails to tidy one costs an empty key and nothing else.

The figure is still remembered for a minute, and that cache does not cross replicas. Each measures
its own and credits its own writes, so with a quota set and several replicas serving, a repository
can overshoot by roughly what they write between them inside one minute. Each replica's figure is
internally consistent and none of them is the whole truth, which is worth knowing before a quota is
treated as a hard wall.

**Capacity is not reported.** `lfsx_objects_stored` and `lfsx_store_bytes` are not measured against
a bucket: there is no cheap answer for what one holds, and building it from a full listing would cost
a request per object on every scrape. The gauges are left alone rather than pinned to a zero a
dashboard would average as an empty store, and the server says so at startup. Read capacity from the
bucket itself. Per-repository figures still work, since a repository is a prefix.

**Collection works here.** A repository's markers are its claims on the bytes, so a sweep drops the
markers the client no longer retains and removes a content object once no marker anywhere still
references it.

The hard part is that last question. A marker is `{org}/{repo}/.../{oid}`, so the digest is the
*suffix* of the key, and the org and repo that would make a prefix are exactly what the sweep does
not know. There is no query for "any key ending in this digest", so the bucket keeps a claim index:
one empty object at `.refs/{oid}/{org}/{repo}` for each repository holding that object. Asking
whether anyone else still holds it is then a listing of one short prefix, and a sweep costs what the
repository holds rather than what the bucket does. That matters on a bucket shared by many
repositories, where reading every key to collect a small repository is most of the work.

A bucket that predates the index is read whole once instead, and that pass builds the index as it
goes. Until something has walked the whole bucket and recorded that it did, the index is not
consulted at all. A marker written before the index existed has no ref beside it and would read as
an object nobody claims, which is exactly how a sweep deletes bytes another repository is still
serving.

That asymmetry is what the design turns on. The index may name a holder that has gone; it may never
miss one that is still there. So a ref is written before the marker it stands for and deleted after
it, and any failure to read the index counts as somebody still holding the object. A ref too many
leaves an object nobody reads. A ref too few loses data.

A listing that could not be finished is reported as `incomplete`, and when that happens no content
object is removed at all. The reference that would have saved it may sit in the pages that never
arrived, and freeing bytes another repository is still serving is the one mistake collection must
never make.

**A claim can arrive while a sweep is deciding**, and the index gets the last word because of it. A
repository pushing a digest that is already in the bucket writes a marker and uploads nothing, so a
sweep that worked out an object was unclaimed and then deleted it would leave that repository
holding a marker pointing at nothing, which its client meets as a missing object on the next pull.
So the question is asked once more, immediately before the bytes go, and a push writes its ref
before it so much as looks at whether the content is there. A claim that landed at any point before
that last question is one the sweep sees.

What is left is the width of a single request, between reading that answer and the delete already on
its way. Closing it completely needs a lease the deleting side takes and every push waits on, which
spends a round trip on the hot path to buy back a window this narrow. If it does happen, the object
is missing rather than wrong: `lfsx doctor` finds it and pushing again puts it back.

**Three things do not apply, and say so** rather than reporting an empty success. Compression
rewriting and verification are not implemented against a bucket yet and answer `501`. Deduplication
answers `501` too, for a different reason: content addressing already stores each object once, so
there is nothing left to fold in.

**Compression and encryption work here too.** They were refused against a bucket at first, because a
framed object was only readable through the file the codec opened. The codec reads from a bucket as
well now: the header and the index are two ranged `GET`s before the first frame, which is what the
format was shaped for. So `LFSX_COMPRESSION` and `LFSX_ENCRYPTION_KEY_FILE` mean the same thing on a
volume and in a bucket, and encrypting into storage somebody else operates is the case that most
deserves it.

What that costs, measured against a bucket on localhost, twenty downloads each:

```
raw                       12.3 ms   1.56 MB
compressed                 6.0 ms   1.68 MB plaintext
compressed, but noise     12.3 ms   1.20 MB plaintext
```

A compressible asset comes out ahead: the frames save far more on the wire than two extra round trips
cost. An already-compressed one saves nothing and still pays them, about 30% more per byte. And the
bucket here is on localhost, so those round trips are close to free in a way they will not be across
a WAN: on a remote bucket, expect two extra latencies per download before the first byte. If your
store is mostly PNG and MP3, leave compression off and the objects stay raw.

## A local copy of what the bucket serves

A bucket decouples capacity from one machine, and charges a round trip for every download to do
it. A CI fleet pulling the same asset pack all day pays that egress on bytes that have not
changed. Point `LFSX_S3_CACHE_DIR` at a directory, give it a ceiling, and the second reader gets
the object from local disk:

```bash
LFSX_S3_CACHE_DIR=/var/lib/lfsx/cache
LFSX_S3_CACHE_MAX_BYTES=53687091200
```

Both together or neither: a directory with no ceiling would fill the same volume the server
stages uploads on, which is a worse outage than the round trips it was meant to save, so it is
refused with a warning rather than guessed at. Size the ceiling against the working set, the
objects actually pulled in a day, not against the store: a cache holding everything is a second
copy of the bucket, and that is what the bucket was for.

What is cached is the stored form, byte for byte what the bucket holds, so compression and
encryption are unaffected and a cache directory is no more sensitive than the bucket it mirrors.
Filling happens behind the request: a cold download is served from the bucket at the speed it
always was, and the copy lands afterwards, so nobody waits for it. Two clients racing for the
same cold object produce one fetch.

Entries carry a digest beside them, and one that fails it is discarded rather than served, so a
truncated or rotted file costs a bucket read and not a corrupt download. Eviction drops the least
recently used first, which is why an asset pack pulled every day outlives one fetched once.

`lfsx_cache_hits_total`, `lfsx_cache_misses_total` and `lfsx_cache_bytes` say whether it is
earning its disk. Two things worth knowing: the cache is per replica, like the staging directory,
so two pods warm independently and the ceiling is per pod. And it does nothing under
`LFSX_S3_PRESIGN=true`, where the client fetches from the bucket directly and the server never
sees the bytes: the server says so at boot rather than leaving you to wonder.
