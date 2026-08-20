# Locking

Binary assets cannot be merged. Two artists editing the same `.psd` or the same Unity scene means
one of them loses work, and locking is the only mechanism Git offers to stop that happening. It is
the difference between LFS being usable for a game project and being a hazard.

```bash
git lfs lock Assets/Scenes/Arena.unity
git lfs locks
git lfs unlock Assets/Scenes/Arena.unity
```

A lock belongs to the identity behind the token, resolved from the forge, so `git lfs locks` names
the person to go and talk to. Taking a lock someone else holds is refused with their name attached,
rather than silently overwritten.

Only the owner can release a lock. Anyone else needs `--force`, and force needs **admin** rights on
the repository — the same person who could rewrite the branch anyway.

`LFSX_LOCK_MAX_AGE` is the answer to the same situation without an administrator in it. Unset, a
lock lasts until someone releases it, which is what happened before this existed. Set, a lock nobody
has touched for that long can be taken by anyone who could have taken it in the first place:

```bash
LFSX_LOCK_MAX_AGE=1209600   # two weeks
```

**A stale lock is not deleted, it is taken.** Until somebody claims it, it is still listed and still
names its holder, because the useful answer is not "this is free" but "marie had this and has not
touched it in three weeks". The takeover is recorded in the log with the previous owner, the new one,
and how long it had been.

The clock runs from when the lock was taken. Last-touched is closer to what people mean by stale,
and it would mean guessing which object a path maps to; the claim is the thing this server can
answer for.

One honest limitation: **`git lfs locks` cannot show you any of this.** The protocol's lock is an id,
a path, a timestamp and an owner, with nowhere to put a "stale" flag, so no phrasing of the JSON
would make the client display it. The repository page shows it, which is where somebody goes to ask
why they cannot take a scene.

Locks live next to the objects, under `.locks/`, so they are covered by the same backup and
disappear with the repository. That means `$LFSX_STORAGE_ROOT/.locks/` on a volume and the same
prefix in the bucket when objects are in one: whatever holds the objects holds the locks, because a
second replica has to agree with the first about who is holding what.

### What makes a lock unique

Only one of two clients reaching for the same path may come away with it, and the store is what
decides which. On a volume that is `create_new`, a filesystem primitive that either makes the file or
does not. In a bucket it is `If-None-Match: *`, a conditional write the store has to refuse when the
key is already there.

Not every S3-compatible implementation performs that condition. AWS added it in August 2024 and MinIO
has it; some of the others accept the header and write anyway, answering success twice. Both callers
are then told the lock is theirs, nothing detects it, and locking has quietly become advisory, which
is exactly the situation it exists to prevent.

So the server asks at startup rather than assuming. It writes one key twice under the condition and
requires the second to be refused. A store that performs both writes, or that cannot be asked, loses
locking: taking a lock answers `501` with a message saying why, and objects, transfers and everything
else carry on untouched. A refusal at the moment somebody asks is the only honest answer available;
handing out a lock two people can hold is not.

This is why there is no table here of which providers work. Each deployment establishes it against
its own store, at boot, and says so in the log, which is worth more than a list somebody tested once.
