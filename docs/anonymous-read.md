# Anonymous read

**Off unless you ask for it.** Set `LFSX_ANONYMOUS_READ=true` to turn it on.

With it on, a request carrying no token is resolved against the forge rather than refused: an
unauthenticated lookup answers for a repository the forge serves publicly and denies one it will not
admit exists. So a public project can be cloned by CI without a token, by a contributor with no
credential helper, and by anyone who just wants to read, which is what cloning a public repository
does everywhere else.

It is off by default because of what it costs rather than what it exposes. Nothing confidential is at
stake: a private repository is still refused, and a repository's objects are only served to a caller
the marker says holds them, so a public project is never a way into a private one that happens to
share the same bytes. What it does mean is that anyone who finds the endpoint can pull from it, and on
a server whose job is to move files measured in gigabytes, that is your bandwidth and your
availability. Inheriting that silently is the mistake this default avoids.

**Reading only.** A public repository grants exactly that; uploading needs a token whatever the
visibility.

**A private repository still answers `401` with the challenge**, never `403`. That distinction is not
cosmetic: `403` tells git-lfs the answer will not change, so it stops asking the credential helper and
a user who does have access can never get in.

**The two resolutions are cached apart.** An anonymous decision and a token's decision are hashed
under different domains, so one is never served in place of the other in either direction.

Left unset, a credential-free request is refused without the forge being asked at all. Only the exact
string `true` opens it: a typo, an empty value or a `1` leaves it closed, because the failure worth
guarding against is the one that opens the door when nobody meant to.

The server logs at startup when it is on, so a deployment that inherited the flag from an older chart
sees it rather than discovers it.

`LFSX_AUTH=disabled` turns the server into an open one. It exists for local development and closed
networks, it is logged loudly at startup, and it is never the right setting for anything reachable
from the internet.
