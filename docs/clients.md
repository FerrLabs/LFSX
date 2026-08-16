# Clients

## What is tested, and on what

Every push to this repository runs the full end-to-end script — push a 16 MiB asset with the real
`git lfs`, clone it back, compare the bytes, take a lock, fail to steal it, release it — against:

| Platform | git-lfs |
|---|---|
| Linux | whatever the runner ships |
| macOS | whatever the runner ships |
| Windows | whatever Git for Windows ships |
| Linux | 3.0.2, the oldest version supported |

**3.0 is the floor**, because that is where the locking API settled. Older clients will do ordinary
transfers, and nothing here goes out of its way to break them, but they are not exercised and a
protocol detail that only they hit will not be caught.

That matters more than a version table usually does: the clients a studio actually runs are rarely
the newest. Git for Windows, GitHub Desktop, Sourcetree, Rider and Unity's own integration each
bundle a copy, and a project can easily be a year behind without anyone choosing that.

## The graphical clients

Those cannot be automated here — they are what the artists run, so they are worth checking by hand
after a change to the transfer or locking paths. The whole list is ten minutes.

- [ ] **Clone** a repository holding a large asset, and confirm the file opens in its editor rather
      than arriving as a 130-byte pointer. This is the failure that looks like corruption and is
      really a missing `git lfs install`.
- [ ] **Push** a new asset over a hundred megabytes, and watch the progress bar move. A stalled bar
      that then completes usually means the server buffered rather than streamed.
- [ ] **Interrupt** that push halfway — pull the network, not the process — and start it again. It
      should resume rather than restart.
- [ ] **Take a lock** on a scene or a PSD from the client's own UI, and confirm a second machine is
      refused and told who holds it.
- [ ] **Release** it, and confirm the first machine's UI notices.
- [ ] **Check out an older commit** that references an asset the working copy does not have, and
      confirm it downloads rather than failing the checkout.

Worth recording which client and version each check was run against, since the answer changes with
the bundled copy rather than with this server.
