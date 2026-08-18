# Objects in a bucket

`LFSX_STORAGE=s3` puts the objects in an S3-compatible bucket — MinIO, Garage, Backblaze, AWS —
instead of on the volume, which is what unties capacity from one machine.

```bash
LFSX_STORAGE=s3
LFSX_S3_ENDPOINT=https://s3.example.com
LFSX_S3_BUCKET=assets
LFSX_S3_ACCESS_KEY=…
LFSX_S3_SECRET_KEY=…
```

**Locks move with the objects.** They are keys under `.locks/` in the same bucket, taken with a
conditional write — `If-None-Match: *`, so the store itself decides who arrived first and answers
`412` to everyone after. That is the mutual exclusion `create_new` gives on a filesystem, and it is
what makes a second replica possible: two servers sharing one bucket and nothing else agree on who
holds a scene, and a release on either frees it on both. Tested against MinIO rather than assumed,
including the case where both replicas ask at once.

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

**`LFSX_S3_PRESIGN=true` redirects transfers instead.** The batch response hands the client a
pre-signed URL and the bytes never cross this server, which is what you want when the bucket is
closer to the clients than the server is, or when the server's egress is the thing you are paying
for.

Uploads go the same way, and the two things that made that unsafe are both closed rather than
accepted:

**The digest is bound into the signature.** The URL comes with an `x-amz-checksum-sha256` header the
client must send, and it is part of what was signed, so the store refuses any body that does not hash
to the object the URL was cut for. A client holding an upload URL cannot put arbitrary bytes behind
it.

**The upload goes to a key only that repository was signed for**, under `.incoming/`, not to the
shared content key. That is what keeps possession meaning something: bytes arriving there prove the
repository had the object, where a write to the shared key would prove only that somebody with push
rights knew a digest, which is all a leaked pointer file is. On `verify` the server checks the size
that actually arrived against the declared one, the object ceiling and the repository's budget, then
moves the object into the shared keyspace with a copy inside the bucket and writes the marker.

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

The permission check does not move. A signature is cut only after the marker says this repository
holds the object, exactly as a plain download is refused without it, and the URL it signs is scoped
to one object and expires with the action. What changes is what the server can still see:
`lfsx_downloaded_bytes` stops counting bytes it no longer carries, and the bucket serves the ranges.

This is the one case where the batch response says `"authenticated": true`, and the one case where
it is true: see [below](#why-authentication-cannot-live-in-the-proxy) for why that field is a trap
everywhere else.

**Capacity is not reported.** `lfsx_objects_stored` and `lfsx_store_bytes` are not measured against
a bucket: there is no cheap answer for what one holds, and building it from a full listing would cost
a request per object on every scrape. The gauges are left alone rather than pinned to a zero a
dashboard would average as an empty store, and the server says so at startup. Read capacity from the
bucket itself. Per-repository figures still work, since a repository is a prefix.

**Four things do not apply, and say so** rather than reporting an empty success. Collection,
compression and verification are not implemented against a bucket yet and answer `501`.
Deduplication answers `501` too, for a different reason: content addressing already stores each
object once, so there is nothing left to fold in.

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
