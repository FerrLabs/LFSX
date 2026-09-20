# Restricted namespaces

**Off unless you list something.** `LFSX_RESTRICTED=acme/assets,acme/game-*` names the repositories
whose objects take write access to read.

A forge grants read on a public repository to everyone who asks, and this server mirrors the forge,
so by default the assets of a public project are as public as its code. That is usually what you
want and occasionally the opposite of it: an engine you are happy to open with art you licensed and
cannot redistribute, a plugin whose source is MIT and whose sample scenes are not.

Listing the namespace changes one thing. The question the forge answers stays the same, and the
answer it gives is still the ceiling; what changes is the floor. `pull` is no longer enough, `push`
is, so the people who could have committed the asset are the people who can fetch it.

```bash
LFSX_RESTRICTED=acme/assets
```

An entry is `org/repo`, or `org/prefix-*` for a run of repositories, or `org/*` for all of an
organisation's. Matching ignores case, an entry naming no repository is dropped rather than widened
to the whole organisation, and anything not listed is untouched.

## Why write, rather than a list of people

Because the forge already keeps that list, and keeping a second one is how the two drift apart. On a
public repository a stranger gets `pull: true, push: false` and a collaborator gets `push: true`,
which is the line you were going to draw by hand. Taking it from the forge means no token this
server issues, no file of usernames to edit, and no second place to revoke: remove somebody upstream
and their access to the objects goes in the same breath, which is the property this server is built
around.

The cost is that a read-only collaborator is refused, and there are projects where that is the wrong
answer. If yours is one of them, say so on the issue tracker rather than working around it, because
the fix is a real allowlist and it should be designed once.

## What a client sees

`git clone` still works. The repository is public and that is the forge's business, not this
server's.

`git lfs pull` fails on the objects, and the working tree keeps the pointer files. That is the
intended outcome rather than a half-broken one: the caller has the repository, not the assets.

A caller with no credentials is answered `401` with the challenge even when
[anonymous read](anonymous-read.md) is on, never `403`. The distinction is the same one that page
makes: `403` tells git-lfs the answer will not change, so it stops asking the credential helper and
the person who does hold write access never gets to present it. The forge is not asked at all, since
no anonymous caller can hold write access and there is nothing to learn.

A caller presenting a token the forge admits but who cannot push is answered `403`, which is
accurate: that answer will not change until somebody upstream changes it.

## What it does not do

**The pointer files stay public.** They are committed to the repository, so the oid, the size and
the path of every asset are readable by anyone who can clone, listed or not. This hides the bytes,
not their existence.

**It is not retroactive.** Anyone who pulled before you listed the namespace still has what they
pulled. Restricting a repository is a change to who can fetch from here from now on, not a recall.

**Uploads were already restricted.** Writing has always needed write, so listing a namespace changes
nothing about pushing.

If you need an asset to be unreadable even to somebody who can reach the endpoint, this is the wrong
tool and so is [encryption at rest](encryption.md), which protects the disk rather than the caller.
What you want is encrypting the file before it becomes an object, so the repository carries
ciphertext and the oid is its digest. It works against a fully public endpoint, and it costs
deduplication and compression, which are most of the reason to run this server.
