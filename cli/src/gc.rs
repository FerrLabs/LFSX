use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::client::{Server, split_namespace};

pub fn run(server: &Server, repository: &str, dry_run: bool) -> Result<Value> {
    if is_shallow()? {
        bail!(
            "this is a shallow clone, so it does not know every object the repository references. \
             Collecting from here would sweep objects that are still in use — run it from a full clone"
        );
    }

    let oids = referenced_oids()?;
    let (org, repo) = split_namespace(repository)?;

    let response = server.post(
        &format!("/{org}/{repo}/objects/retain"),
        &json!({ "oids": oids, "dry_run": dry_run }),
    )?;

    let status = response.status();
    if status.as_u16() == 403 && !dry_run {
        bail!(
            "the server answered 403: collecting for real needs the level the forge treats as              administrative (admin on GitHub and Gitea, Maintainer or Owner on GitLab).              --dry-run shows what collection would free and works with push rights"
        );
    }
    if !status.is_success() {
        bail!("the server answered {status}");
    }

    response.json().context("the report was not json")
}

fn is_shallow() -> Result<bool> {
    let output = git(&["rev-parse", "--is-shallow-repository"])?;

    Ok(output.trim() == "true")
}

pub fn referenced_oids() -> Result<Vec<String>> {
    Ok(parse_ls_files(&git(&[
        "lfs", "ls-files", "--all", "--long",
    ])?))
}

fn parse_ls_files(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|oid| oid.len() == 64 && oid.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_owned)
        .collect()
}

fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("could not run git {}", args.join(" ")))?;

    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    String::from_utf8(output.stdout).context("git printed something that is not utf-8")
}

#[cfg(test)]
mod tests;
