# Looking at a repository

Open `https://lfs.example.com/my-org/my-project` in a browser. It shows how many objects the
repository holds, how much disk they take, and what is locked and by whom: the questions that
otherwise need a shell.

There is no login screen and no session. The page sits behind the same permission check as every
transfer, so the browser asks for credentials itself and you give it the same token git uses. Read
access is enough to see it; nothing on the page changes anything, deletion stays an explicit API
call. `/{org}/{repo}/objects/stats` serves the same numbers as JSON.

Its `locks` figure counts the lock entries in storage without reading them, so a lock file that no
longer decodes is counted there while the page and `GET /{org}/{repo}/locks` leave it out. A gap
between the two numbers means a damaged lock entry worth a look.
