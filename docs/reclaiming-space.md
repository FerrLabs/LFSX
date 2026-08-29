# Reclaiming space

Objects are written and never removed on their own. A repository that rewrites history, drops a
branch or replaces a large asset leaves the old blobs behind, and the disk only grows.

The server cannot decide what is still needed: it never sees your Git history. So you tell it.
The [`lfsx`](../cli/) command does it from a clone:

```bash
lfsx --url https://lfs.example.com gc --repo my-org/my-project --dry-run
```

It refuses to run from a shallow clone, which would retain a fraction of what it should and sweep
the rest. Same command without `--dry-run` to actually free the space.

Under it is one endpoint, if you would rather call it yourself. `retain` takes the set of object
ids the repository still references and sweeps everything else:

```bash
git lfs ls-files --all --long \
  | cut -d' ' -f1 \
  | jq -Rs '{ oids: (split("\n") - [""]), dry_run: true }' \
  | curl -sS -u "git:$GITHUB_TOKEN" -H 'content-type: application/json' \
      --data @- https://lfs.example.com/my-org/my-project/objects/retain
```

`--all` walks every ref, so the set covers every commit still reachable, which is the point: run
this from a full clone, not a shallow one, or you will retain a fraction of what you should.

It answers with what it would free, and frees nothing until you drop `dry_run`:

```json
{ "swept": 42, "bytes": 3221225472, "within_grace": 3, "dry_run": true }
```

Two safeguards, because this deletes data. An object is uploaded *before* the commit referencing
it is pushed, so anything touched within `LFSX_GC_GRACE` (two weeks by default, matching git's own
`gc.pruneExpire`) is never taken: that is the `within_grace` count. And a transfer still in
flight is skipped, since staging files are not objects yet.

Objects go, the fanout directories they lived in stay: an inode and a block per prefix, reused by
the next object that hashes into it. Removing them raced every upload: a push creates its fanout
directory, and until the staging file lands that directory is empty, so a collection running
alongside could take it and fail a push on a directory made moments earlier for that push.

An upload streams into a `.part` file next to its destination and is renamed on success. A process
kill or a host crash mid-transfer leaves one behind, and nothing used to reclaim it. The server now
sweeps them at boot (a crash is exactly what strands them) and hourly after that, logging the
count and the bytes it recovered.

`LFSX_STAGING_MAX_AGE` has to stay above the longest transfer you expect to serve: a `.part` file
younger than that is not litter, it is an upload in flight, and removing it would break a client
doing nothing wrong. A day is generous for a multi-gigabyte push on a home upstream.

Both are worth understanding before you shorten the grace period: an empty `oids` set legitimately
means *nothing is referenced any more*, and outside the grace window that sweeps the repository
clean. Run it dry first.

Which is why the two halves ask for different rights. The dry run needs push rights: it is a read
of what collection would free, and any contributor may ask. Collecting for real unlinks objects,
so it needs the level the forge treats as administrative, admin on GitHub and Gitea, Maintainer or
Owner on GitLab, the same level that force-opens a lock. A `$GITHUB_TOKEN` from a routine CI job
will preview collection and be refused the real thing.
