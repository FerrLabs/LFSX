use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::client::{Server, split_namespace};

pub fn run(server: &Server, repository: &str, dry_run: bool) -> Result<Value> {
    call(server, repository, "dedupe", dry_run)
}

pub fn compress(server: &Server, repository: &str, dry_run: bool) -> Result<Value> {
    call(server, repository, "compress", dry_run)
}

pub fn verify(server: &Server, repository: &str) -> Result<Value> {
    call(server, repository, "audit", false)
}

fn call(server: &Server, repository: &str, action: &str, dry_run: bool) -> Result<Value> {
    let (org, repo) = split_namespace(repository)?;

    let response = server.post(
        &format!("/{org}/{repo}/objects/{action}"),
        &json!({ "dry_run": dry_run }),
    )?;

    let status = response.status();
    if !status.is_success() {
        bail!("the server answered {status}");
    }

    response.json().context("the report was not json")
}
