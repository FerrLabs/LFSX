# Allowed namespaces

**Set this on any server the internet can reach.** `LFSX_ALLOWED=acme/*` names the repositories this
server is for, and everything else is answered `404` before the forge is asked.

Without it the server mirrors the forge for every repository on it. That is the whole permission
model, and it has one consequence worth stating plainly: anybody with an account can create a
repository, is its admin, and can therefore push objects for it here. On `github.com` that is
everyone. The forge is right, it is their repository; the disk is yours.

```bash
LFSX_ALLOWED=acme/*,partner/shared-assets
```

An entry is `org/repo`, `org/prefix-*` for a run of repositories, or `org/*` for all of an
organisation's, the same syntax as [restricted namespaces](restricted-namespaces.md). Matching
ignores case. An entry that is not `org/repo` is dropped and logged rather than widened, and a list
where nothing parsed serves nothing rather than everything.

Being listed only lets a repository in. The caller still gets exactly what the forge grants, so a
token with pull on `acme/assets` still cannot push to it.

A repository outside the list is a `404` for everybody, its admin included and a caller with no
credentials too: there is nothing to authenticate for, and the forge is never asked, so a stranger
walking repository names spends none of the [lookup budget](configuration.md).

The server logs a warning at boot when it talks to a forge and no list is set.

The [dashboard](dashboard.md) can change the list without a restart. What it saves takes precedence
over `LFSX_ALLOWED` until it is reset.

## Doing it in the reverse proxy instead

It works, and it is the right place if the proxy already filters by path. The server answers on
`/health`, `/ready`, `/metrics`, under `/-/` when the [dashboard](dashboard.md) is on, and under
`/{org}/{repo}/` and nowhere else, so allowing the first three, `/-/` and the prefixes you want, and refusing the rest, is the whole rule. Match the organisation
without regard to case, since the forge does. `LFSX_ALLOWED` does the same thing without needing
the proxy to know the URL layout, and keeps working if a route is ever added.
