# What the protocol asks for and what this answers

**Transfers.** The batch response answers `basic`, which is the only adapter this server implements
and the one every client supports. It is chosen from what the client advertised rather than
assumed, so adding another is a change in one place.

**Locks are paged.** `GET /locks` and `POST /locks/verify` honour `limit` and `cursor`, defaulting to
100 and capped at 1000. A response carries `next_cursor` only when there is another page, so an
absent cursor ends the walk. The cursor is the id of the last lock returned and the list is ordered
by id, which means a lock released mid-walk is skipped rather than shifting everything after it out
of view. `verify` pages over the whole list before splitting it into yours and theirs, so both sides
agree on where the page ends.

Without this a studio that has locked an art directory received every lock in one body, and a client
that sent `limit` believed it had seen the list.

**`ref` is accepted and not acted on.** Clients send the branch they are working on, and this server
cannot make it change any answer: permissions come from the forge at repository granularity (pull,
push, admin), so there is nothing branch-shaped to consult. Refusing the field would break a client
sending exactly what the specification tells it to send, so it is parsed and ignored deliberately
rather than by omission. The day it matters is
[#46](https://github.com/FerrLabs/LFSX/issues/46): once a public repository can be read without a
token, write access has to say which refs, and that is where `ref` earns its keep.
