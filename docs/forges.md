# Several forges

One server can check access against more than one forge: GitHub and a self-hosted GitLab, or two
Gitea instances. Objects stay content-addressed in one store, so an asset pushed to a repository on
each forge is stored once.

The forge set by `LFSX_AUTH` stays where it is, at `/{org}/{repo}`. Every other forge gets a name and
is served under `/-/{name}/{org}/{repo}`:

```bash
LFSX_FORGES=work,home

LFSX_FORGE_WORK_AUTH=gitlab
LFSX_FORGE_WORK_API_URL=https://gitlab.work.example/api/v4
LFSX_FORGE_WORK_ALLOWED=platform/*

LFSX_FORGE_HOME_AUTH=forgejo
LFSX_FORGE_HOME_API_URL=https://git.home.example/api/v1
LFSX_FORGE_HOME_ALLOWED=me/*
```

A repository on `work` then uses:

```ini
# .lfsconfig
[lfs]
    url = https://lfs.example.com/-/work/platform/game-assets
```

The CLI takes the forge as part of its base URL: `lfsx --url https://lfs.example.com/-/work gc
--repo platform/game-assets`.

## Variables

`LFSX_FORGES` lists the names, comma-separated. A name is 1 to 32 lowercase letters, digits or
dashes, and cannot be `api` or `dashboard`, which the dashboard uses. Each name reads its own
variables, the name upper-cased with dashes as underscores (`self-hosted` reads
`LFSX_FORGE_SELF_HOSTED_*`):

| Variable | Default | Purpose |
|---|---|---|
| `LFSX_FORGE_<NAME>_AUTH` | none, required | `github`, `gitlab`, `gitea` or `forgejo` |
| `LFSX_FORGE_<NAME>_API_URL` | github.com or gitlab.com, required for Gitea | API root of that forge |
| `LFSX_FORGE_<NAME>_ALLOWED` | none | repositories served on that forge, as [`LFSX_ALLOWED`](allowed-namespaces.md) |
| `LFSX_FORGE_<NAME>_RESTRICTED` | none | as [`LFSX_RESTRICTED`](restricted-namespaces.md), for that forge |
| `LFSX_FORGE_<NAME>_ANONYMOUS_READ` | `false` | as [`LFSX_ANONYMOUS_READ`](anonymous-read.md), for that forge |

The cache lifetimes and the lookup budget (`LFSX_AUTH_CACHE_TTL`, `LFSX_AUTH_REJECTION_TTL`,
`LFSX_AUTH_LOOKUP_BUDGET`) apply to every forge, and each forge spends its own budget. A GitHub App
identity is only available to the forge set by `LFSX_AUTH`. `LFSX_FORGES` needs authentication
on: with `LFSX_AUTH=disabled` the server refuses to start.

## What stays apart

`acme/game` on `work` and `acme/game` on the main forge are two repositories. Each has its own
permissions, asked of its own forge, its own allow-list, quota, locks and statistics, and collection
on one never treats the other as a holder it can ignore. On the volume and in the bucket a named
forge's repositories sit under `{name}~{org}`, for example `work~acme/game`. `~` cannot appear in an
organisation name, so the main forge's layout is unchanged and nothing moves when a forge is added.

Renaming a forge in `LFSX_FORGES` therefore points it at a different directory: its repositories
read as empty until the directories are renamed to match.

## Dashboard

The [dashboard](dashboard.md) lists every forge under Settings. Its admins are checked against the
main forge, and the access settings it edits are the main forge's; the other forges take theirs from
the environment.
