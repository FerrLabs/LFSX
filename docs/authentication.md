# Authentication

LFSX does not manage accounts. It asks the forge that hosts the repository what the caller is
allowed to do, so the answer is always the same one the repository gives.

The client presents the token it would use to clone over HTTPS (a personal access token, or the
`GITHUB_TOKEN` a CI job already has) as the password of an HTTP Basic credential, or as a bearer
token. LFSX resolves it against `GET /repos/{org}/{repo}` and maps the result:

| GitHub | GitLab | Gitea, Forgejo | Objects |
|---|---|---|---|
| admin | Maintainer, Owner | admin | download, upload, and force a lock open |
| push | Developer | push | download, upload, and take locks |
| pull only | Reporter | pull only | download |
| none, or an unusable token | Guest, or none | none, or an unusable token | rejected |

GitLab grants inherited from a group count the same as ones set on the project, which is how most
organisations there are arranged. Developer is the level that may push, matching what GitLab itself
requires to write to the repository.

`LFSX_AUTH` picks the forge: `github`, which is the default, `gitlab`, or `gitea`. `forgejo` names
the same provider as `gitea`, because Forgejo is a fork of Gitea and answers the same API. Each has
an API root that can be pointed at a self-hosted instance, `LFSX_GITHUB_API_URL`,
`LFSX_GITLAB_API_URL` and `LFSX_GITEA_API_URL`, which is how GitHub Enterprise and a private GitLab
are reached.

For Gitea that variable is required, and the server refuses to start without it. github.com and
gitlab.com are where a repository is unless somebody says otherwise, so defaulting there is nearly
always right. Gitea is software rather than a place, and an operator who names it is almost
certainly running their own. Quietly resolving their namespaces against gitea.com would be worse
than not starting: a public repository there that happened to share a name would hand out anonymous
read on objects it has nothing to do with.

`/health` stays open. Everything under `/{org}/{repo}/objects/` requires a token, and each answer
is cached for `LFSX_AUTH_CACHE_TTL` seconds so a push of two hundred objects costs one API call
rather than two hundred. That cache is also the delay before a revocation takes effect: shorten
it if that matters more than the round trips.

`LFSX_AUTH_LOOKUP_BUDGET` is the ceiling underneath all of that: the number of lookups a minute this
server will spend on the forge, counting only the ones neither cache could answer. A push of two
hundred objects under one token costs one, so an ordinary client never meets it however busy it is.

It exists because the caches are keyed by the token. A caller sending a different one every request
misses every entry and costs a lookup each time, and inventing a token that does not exist is free.
What runs out then is not this server: the forge counts a failed authentication against the address
that made it, so the budget being drained is the one every real lookup shares, and legitimate pushes
start being refused because somebody else spent it. Past the ceiling a caller gets a `503` with
`Retry-After` from here, which is the same answer the forge would eventually give and a very
different afternoon behind it, because this one lifts the moment the flood stops.

It is a ceiling on this server rather than fairness between callers. Telling two callers apart needs
the address a request came from, and behind a reverse proxy that is the proxy: per-client limiting
belongs there, where the real address is known. What a proxy cannot know is which requests cost a
forge lookup, which is exactly what this counts. `lfsx_rejections_total{cause="lookup_budget_spent"}`
is how you see it happening, kept apart from `forge_rate_limited` because one is this server saying
no and the other is the forge.

Refusals are remembered too, for the shorter `LFSX_AUTH_REJECTION_TTL`. Without that, a CI job
retrying with a revoked token spends one API call per attempt, forever, against the same budget
the server needs for real lookups, and an unauthenticated caller could drive that load on
purpose. The window is short on purpose: it is how long you keep being refused after being granted
access. A forge that cannot be reached is never cached, so an outage stays an outage rather than
becoming a lasting denial.

Git already sends the token if it is in the credential store for that host:

```bash
git config --global credential.https://lfs.example.com.username git
printf 'protocol=https\nhost=lfs.example.com\nusername=git\npassword=%s\n' "$GITHUB_TOKEN" \
  | git credential approve
```

In CI, the token the workflow already holds is enough, with no secret to provision:

```yaml
- run: |
    printf 'protocol=https\nhost=lfs.example.com\nusername=git\npassword=%s\n' "${{ secrets.GITHUB_TOKEN }}" \
      | git credential approve
    git lfs pull
```

### In CI

`actions/checkout` with `lfs: true` runs its LFS fetch against the endpoint the remote implies,
GitHub's own, and never reads `.lfsconfig`, which is only honoured by git-lfs invoked afterwards
against a working tree that already contains it. So the fetch has to be its own step:

```yaml
- uses: actions/checkout@v5
  with:
    lfs: false

- name: Fetch LFS objects
  env:
    LFS_TOKEN: ${{ github.token }}
  run: |
    git config --global credential.helper '!f() { echo username=x; echo "password=$LFS_TOKEN"; }; f'
    git lfs pull
```

`github.token` is enough to **read**. It is a GitHub App installation token, and the forge reports
`{"admin":false,"push":false,"pull":false}` for one however much access it actually has, because
that field describes the authenticated user's permissions and an installation token has no user
behind it. So this server does not read it as a refusal: the answer arriving at all is the proof of
access, since the forge returns 404 to a token that cannot see the repository.

It is not enough to **push**. Nothing in that answer says the token may write, and this server will
not assume it. A job that uploads objects needs a personal access token with write access to the
repository, kept in a secret and passed the same way.

### Adding a forge

Three providers exist, so the shape is settled rather than guessed. A provider is one module under
`server/src/auth/` exposing three functions, `permission(client, api_url, token, namespace)`,
`public(client, api_url, namespace)` and `login(client, api_url, token)`, plus a variant on
`config::Provider` carrying its environment variable and its default API root if it has one, and
three arms in `auth.rs`. Nothing else: the caching, the challenge handling and the rejection
accounting are shared and provider-blind.

The part worth care is the error mapping, because it is where the two existing providers already
disagree. GitHub answers `403` when rate-limited, GitLab answers `429`; both mean "ask again
later" and must map to `Error::RateLimited`, never to `Forbidden`. Getting that wrong tells a user
with full rights that they have none.

`Error::RateLimited` and `Error::Forge` are separate on purpose, and a new provider should keep them
apart. A throttled forge is working: it has said when to come back, and the answer is a `503`
carrying `Retry-After` so the client waits. `502` reads as a transient upstream failure and git-lfs
comes straight back, spending another request on the same exhausted quota, which turns one rate
limit into a CI run's worth of them. The duration comes from `Retry-After` when the forge sends one,
from `x-ratelimit-reset` when it sends an absolute reset instead, and from a one-minute default when
it says neither: never from zero. `lfsx_rejections_total{cause="forge_rate_limited"}` counts these
separately from `forge_unreachable`, because "the forge is throttling us" and "the forge is broken"
are different afternoons.

The three disagree in exactly that place, which is why it is worth naming. GitHub answers `403` when
rate-limited and GitLab answers `429`. Gitea limits nothing itself, so a throttled instance is
whatever sits in front of it speaking, and nginx answers `503` unless it was told otherwise. What
the Gitea provider keys on is the `Retry-After` header rather than the status: something that says
when to come back is a limit, and a `503` with nothing to say is an instance that is down.

Bitbucket and Azure DevOps are the two that are left. Neither overlaps with the people who
self-host the way Gitea does, so neither is queued behind anything.

### Why authentication cannot live in the proxy

This is not obvious, and it rules out the approach most people reach for first.

The batch response carries the URLs the client will use for each object transfer. If the server
advertises them as already authenticated (`"authenticated": true`) without supplying an
`Authorization` header alongside, git-lfs treats those URLs as pre-signed storage links and calls
them with **no credentials at all**. Behind an authenticating proxy, every one of those calls
returns `401`, and the client retries in a loop.

This is exactly what makes [rudolfs](https://github.com/jasonwhite/rudolfs) unusable behind
Traefik BasicAuth: it answers `"authenticated": true` with `"header": null` and offers no way to
change that.

LFSX never emits `authenticated` for a URL that points back at itself, so the client authenticates
each transfer itself. A test pins the behaviour down,
`batch_never_claims_a_transfer_through_this_server_is_pre_authenticated`, and it will fail if
anyone sets the field on the ordinary path.

The exception proves the rule rather than bending it. With `LFSX_S3_PRESIGN=true` the href is a
pre-signed bucket URL, which genuinely carries its own credentials and genuinely must be called
without an `Authorization` header: the proxy in front of this server is not even in that path. The
field is not a claim about the server's own authentication; it is a claim about the URL, and it is
set only when the URL was signed.

Authentication therefore lives in the server, which is what the section above describes.

## A GitHub App identity for the server's own calls

Permission lookups authenticate with the client's own token, and that is the model: the quota
being spent is the caller's, and the answer is about the caller. The one call with nobody behind
it is the anonymous public-repository lookup, which spends GitHub's 60-an-hour unauthenticated
budget. A GitHub App gives that call the server's own identity instead.

1. Create an App on your org (Settings, Developer settings, GitHub Apps). It needs no webhook and
   a single permission: repository **Metadata, read-only**.
2. Install it on the organizations this server serves.
3. Generate a private key, mount the `.pem` where the server can read it, and set both variables:

```bash
LFSX_GITHUB_APP_ID=41
LFSX_GITHUB_APP_KEY_FILE=/etc/lfsx/github-app/private-key.pem
```

The server signs a short-lived JWT, exchanges it for an installation token, and caches the token
per organization until shortly before expiry, so a busy server exchanges once an hour rather than
once a request. A repository the App is not installed on falls back to the plain anonymous ask.

One behaviour is deliberate and worth knowing: an installation token is admitted to every private
repository the App covers, so when asking as the App the server grants anonymous read only if the
repository says it is public, never merely because the answer arrived. Installing the App on
private repositories does not open them.

Nothing else changes. The permission check for an authenticated client still uses that client's
token, because "what may this token do here" is a question only that token can answer. There is
no OIDC and no login flow: this changes which quota the server's own questions spend, nothing
about whose repository it is.
