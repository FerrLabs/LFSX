use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::client::{Server, split_namespace};

const REMOTE: &str = "origin";
const REVIEW_REFSPECS: [&str; 2] = [
    "+refs/pull/*:refs/lfsx/pull/*",
    "+refs/merge-requests/*:refs/lfsx/merge-requests/*",
];

struct Identity {
    base: String,
    repo: String,
}

pub fn resolve_url(dir: &Path, explicit: Option<&str>) -> Result<String> {
    match (explicit, clone_identity(dir)?) {
        (Some(url), Some(identity)) if !same_base(url, &identity.base) => bail!(
            "--url {url} is not the server this clone's lfs.url points at ({}): collecting here \
             would retain this clone's objects on another server and sweep everything else",
            identity.base
        ),
        (Some(url), _) => Ok(url.to_owned()),
        (None, Some(identity)) => Ok(identity.base),
        (None, None) => bail!("no --url given and lfs.url is not set in this clone"),
    }
}

fn resolve_repo(dir: &Path, explicit: Option<&str>) -> Result<String> {
    match (explicit, clone_identity(dir)?) {
        (Some(repo), Some(identity))
            if !identity.repo.eq_ignore_ascii_case(repo.trim_matches('/')) =>
        {
            bail!(
                "--repo {repo} is not the repository this clone's lfs.url points at ({}): \
                 collecting here would sweep that repository's objects",
                identity.repo
            )
        }
        (Some(repo), _) => Ok(repo.to_owned()),
        (None, Some(identity)) => Ok(identity.repo),
        (None, None) => bail!("no --repo given and lfs.url is not set in this clone"),
    }
}

fn same_base(a: &str, b: &str) -> bool {
    a.trim_end_matches('/')
        .eq_ignore_ascii_case(b.trim_end_matches('/'))
}

fn clone_identity(dir: &Path) -> Result<Option<Identity>> {
    let Some(url) = lfs_url(dir)? else {
        return Ok(None);
    };

    parse_lfs_url(&url)
        .map(Some)
        .with_context(|| format!("lfs.url {url:?} is not <server>/<org>/<repo>"))
}

fn lfs_url(dir: &Path) -> Result<Option<String>> {
    if let Some(url) = config_values(dir, &[], "lfs.url")?.pop() {
        return Ok(Some(url));
    }

    let toplevel = git(dir, &["rev-parse", "--show-toplevel"])?;
    let lfsconfig = Path::new(toplevel.trim()).join(".lfsconfig");

    Ok(config_values(dir, &["-f", &lfsconfig.to_string_lossy()], "lfs.url")?.pop())
}

fn parse_lfs_url(url: &str) -> Option<Identity> {
    let url = url.trim_end_matches('/');
    let url = url.strip_suffix("/info/lfs").unwrap_or(url);
    let url = url.strip_suffix(".git").unwrap_or(url);
    let (rest, repo) = url.rsplit_once('/')?;
    let (base, org) = rest.rsplit_once('/')?;

    (base.contains("://") && !org.is_empty() && !repo.is_empty()).then(|| Identity {
        base: base.to_owned(),
        repo: format!("{org}/{repo}"),
    })
}

pub fn run(
    server: &Server,
    repository: Option<&str>,
    dry_run: bool,
    include_review_refs: bool,
) -> Result<Value> {
    let dir = Path::new(".");
    let repository = resolve_repo(dir, repository)?;
    prepare(dir, include_review_refs)?;

    let oids = referenced_oids()?;
    let (org, repo) = split_namespace(&repository)?;

    let response = server.post(
        &format!("/{org}/{repo}/objects/retain"),
        &json!({ "oids": oids, "dry_run": dry_run }),
    )?;

    let status = response.status();
    if status.as_u16() == 403 && !dry_run {
        bail!(
            "the server answered 403: collecting for real needs the level the forge treats as \
             administrative (admin on GitHub and Gitea, Maintainer or Owner on GitLab). \
             --dry-run shows what collection would free and works with push rights"
        );
    }
    if status.as_u16() == 413 {
        bail!(
            "the server answered 413: the keep list names {} objects, more than it reads in one \
             request. Servers from before the fix for FerrLabs/LFSX#451 stop at 2 MiB, about \
             31,000 objects; current ones read up to 64 MiB, about a million",
            oids.len()
        );
    }
    if !status.is_success() {
        bail!("the server answered {status}");
    }

    response.json().context("the report was not json")
}

fn prepare(dir: &Path, include_review_refs: bool) -> Result<()> {
    if git(dir, &["rev-parse", "--is-shallow-repository"])?.trim() == "true" {
        bail!(
            "this is a shallow clone, so it does not know every object the repository references. \
             Collecting from here would sweep objects that are still in use: run it from a full clone"
        );
    }

    let fetch_key = format!("remote.{REMOTE}.fetch");
    if !config_values(dir, &[], &fetch_key)?
        .iter()
        .any(|spec| covers_heads(spec))
    {
        bail!(
            "{fetch_key} does not fetch every branch, as a --single-branch clone does not, so this \
             clone does not know every object the repository references. Collecting from here \
             would sweep objects that other branches still use: run it from a clone whose fetch \
             refspec is +refs/heads/*:refs/remotes/{REMOTE}/*"
        );
    }

    git(dir, &["fetch", "--prune", "--tags", REMOTE])?;
    if include_review_refs {
        git(
            dir,
            &[&["fetch", "--prune", REMOTE], &REVIEW_REFSPECS[..]].concat(),
        )?;
    }

    Ok(())
}

fn covers_heads(refspec: &str) -> bool {
    matches!(
        refspec.trim_start_matches('+').split(':').next(),
        Some("refs/heads/*" | "refs/*")
    )
}

fn referenced_oids() -> Result<Vec<String>> {
    Ok(parse_ls_files(&git(
        Path::new("."),
        &["lfs", "ls-files", "--all", "--long"],
    )?))
}

fn parse_ls_files(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|oid| oid.len() == 64 && oid.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .collect()
}

fn run_git(dir: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .with_context(|| format!("could not run git {}", args.join(" ")))
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = run_git(dir, args)?;

    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    String::from_utf8(output.stdout).context("git printed something that is not utf-8")
}

fn config_values(dir: &Path, scope: &[&str], key: &str) -> Result<Vec<String>> {
    let output = run_git(dir, &[&["config"], scope, &["--get-all", key]].concat())?;

    match output.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect()),
        Some(1) => Ok(Vec::new()),
        _ => bail!(
            "git config {key} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

#[cfg(test)]
mod tests;
