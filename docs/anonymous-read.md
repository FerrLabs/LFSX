# Anonymous read

Cloning a public repository pulls its LFS objects with no credentials anywhere else, so it does here
too. A request with no token is resolved against the forge rather than refused: an unauthenticated
lookup answers for a repository the forge serves publicly and denies one it will not admit exists, so
a public project can be cloned by CI without a token, by a contributor with no credential helper, and
by anyone who just wants to read.

**Reading only.** A public repository grants exactly that; uploading needs a token whatever the
visibility.

**A private repository still answers `401` with the challenge**, never `403`. That distinction is not
cosmetic: `403` tells git-lfs the answer will not change, so it stops asking the credential helper and
a user who does have access can never get in.

**The two resolutions are cached apart.** An anonymous decision and a token's decision are hashed
under different domains, so one is never served in place of the other in either direction.

Set `LFSX_ANONYMOUS_READ=false` to turn it off, and a credential-free request is refused without the
forge being asked at all. Worth doing if you are self-hosting precisely to keep large assets private
behind public code, which is a real pattern and the case this default does not suit. The server logs
which way it is configured at startup.

`LFSX_AUTH=disabled` turns the server into an open one. It exists for local development and closed
networks, it is logged loudly at startup, and it is never the right setting for anything reachable
from the internet.
