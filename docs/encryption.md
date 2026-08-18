# Encryption at rest

**Read this part first: for most self-hosted deployments the better answer is the volume.** LUKS, an
encrypted EBS volume, or a storage class that does it transparently costs nothing in this server, has
no key handling in our code to get wrong, and covers the same disks. Reach for what follows when the
storage itself is the thing you do not trust: a shared NAS, a bucket somebody else operates, a
laptop, a disk you will one day return under warranty.

```bash
head -c 32 /dev/urandom | xxd -p -c 64 > /etc/lfsx/key
LFSX_ENCRYPTION_KEY_FILE=/etc/lfsx/key
```

A path rather than the key itself, deliberately. A key in an environment variable is in the pod
spec, in `docker inspect`, and in every log that dumps the environment. A file comes from a
Kubernetes Secret mount or from FerrVault without any of that. The server refuses to start if the
file is missing or unreadable, because the alternative is a server that quietly writes plaintext
while an operator believes otherwise.

### What it protects, in the words that are true

It protects the bytes at rest: the disk, the snapshot, the backup, the volume you decommission, the
bucket whose provider is not you. **It does not protect against anyone who has the running server**,
because that process holds the key by construction. Anyone who can read its memory, its environment,
or its mounted Secret can read every object. "Encrypted" is exactly the word that gives false
comfort, so it is worth being blunt about where the line is.

### What it does to the store

**The object keeps its name.** The oid is the digest of the plaintext, the client declares it, and
that is the protocol. So the upload hashes the bytes as they stream past and encrypts them after: the
filename stays the plaintext digest, the contents become ciphertext. Reverse those two and every
object fails its own verification. It also means deduplication is untouched, since two repositories
pushing identical bytes still agree on the name.

**Nothing is buffered.** Objects are encrypted in the same four-megabyte frames compression uses,
each sealed on its own with ChaCha20-Poly1305, so serving the tail of a three-gigabyte asset costs
the frames it touches. A single pass over a whole object would put the authentication tag at the end
and force the server to hold the entire thing before serving a byte.

**`Content-Length` is the plaintext length**, read from the object header rather than from the file,
which is now longer than the object by a header and a tag per frame.

**Compression runs first** when both are on. Sealed bytes are indistinguishable from random and give
up no ground at any level, so the other order would spend the CPU for nothing. The usual caveat
applies: compressing before encrypting means the ciphertext length leaks how compressible the
plaintext was.

**A frame is bound to where it was written.** Its index, whether it is the last one, and the object
id are all authenticated, so frames cannot be reordered, an object cannot be truncated to a shorter
one that still opens, and a file cannot be moved on top of another in the shared content store.

### Turning it on, and rotating

Turning it on is not a flag day. Objects already written stay plaintext and keep being served; new
ones are encrypted. Both kinds live in one store because each object says in its first bytes which it
is. Objects written before the key existed are not rewritten, and re-pushing them is what converts
them today.

On Kubernetes the chart mounts an existing Secret rather than taking the key as a value, because a
Helm value ends up in the release secret and in whatever printed the command:

```bash
kubectl create secret generic lfsx-key --from-file=key=/dev/stdin <<< "$(head -c 32 /dev/urandom | xxd -p -c 64)"
helm upgrade lfsx oci://ghcr.io/ferrlabs/charts/lfsx --set encryption.existingSecret=lfsx-key
```

Rotation is a new line at the top of the file:

```
# the key new objects are written under
9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
# still here for everything already on disk
5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03
```

Every key in the file is accepted for reads and the first is used for writes, so rotating does not
mean re-encrypting the store. Each object records which key it was written under, by a hash of that
key rather than by a number, so an id can never come to name a different key than the one it was
written with. A key that is deleted while objects still reference it makes those objects unreadable,
and the server says so by name instead of reporting corruption.
