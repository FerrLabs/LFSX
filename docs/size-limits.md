# Size limits

`LFSX_MAX_OBJECT_SIZE` caps a single object, in bytes. Unset, there is no ceiling, which is fine
when the server has its volume to itself. Set it when it does not: an upload with no limit can fill
the disk, and a full disk fails every other repository on the server, so one careless push becomes
everyone's outage.

```bash
LFSX_MAX_OBJECT_SIZE=5368709120   # 5 GiB
```

The size is declared during batch negotiation, so an object over the ceiling is refused there —
before a byte moves — with a per-object error the client prints by name. The rest of the push goes
through; the limit refuses an object, not the commit it arrived with.

The transfer is capped as well, because the declared size is a claim by the client and the ceiling
has to hold against a body that ignores it. A stream that outgrows the limit is cut off at the
chunk that crosses it and the staging file is dropped, rather than read to the end to find out how
big it was.

Lowering the limit later does not strand what is already stored: it governs what may arrive, not
what a repository can still check out.

`LFSX_REPO_QUOTA` is the same idea one level up: a budget, in bytes, that any single `{org}/{repo}`
may hold. Unset, there is none.

```bash
LFSX_REPO_QUOTA=53687091200   # 50 GiB per repository
```

A per-object ceiling does not stop a project committing its renders directory a gigabyte at a time,
and on a server hosting a team the first symptom is unrelated repositories failing to push. The
budget turns that into one repository being told, in its own client, that it is out of room.

Negotiation refuses each object that would not fit, with a `507` the client prints, and the direct
`PUT` is guarded too for clients that skip negotiation — including when they skip declaring a size,
since the budget travels with the transfer and cuts it off at the byte that crosses the line. An
object the repository already holds is never refused at either gate: re-sending it asks for no new
room. Downloads never are either: a repository over budget still serves every object it holds,
because refusing a checkout punishes the wrong person and fixes nothing.

The figure is what the repository holds on disk, the same one `stats` and the dashboard report — not
what it costs after deduplication, and not what it weighed before compression. Two projects sharing a pack each count it against their own
budget, which is the number an operator is actually handing out. Collection is the way back under:
`retain` frees the room and the next push sees it immediately, without waiting for a cache to
expire.
