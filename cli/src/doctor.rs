use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::client::{Server, split_namespace};

pub struct Report {
    checks: Vec<(bool, String)>,
}

impl Report {
    fn new() -> Self {
        Self { checks: Vec::new() }
    }

    fn pass(&mut self, message: impl Into<String>) {
        self.checks.push((true, message.into()));
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.checks.push((false, message.into()));
    }

    pub fn print(&self) {
        for (ok, message) in &self.checks {
            println!("{} {message}", if *ok { "ok  " } else { "FAIL" });
        }
    }

    pub fn healthy(&self) -> bool {
        self.checks.iter().all(|(ok, _)| *ok)
    }
}

pub fn run(server: &Server, repository: Option<&str>) -> Result<Report> {
    let mut report = Report::new();

    match server.get("/health") {
        Ok(response) if response.status().is_success() => report.pass("the server is up"),
        Ok(response) => report.fail(format!("/health answered {}", response.status())),
        Err(error) => report.fail(format!("{error:#}")),
    }

    match server.get("/ready") {
        Ok(response) if response.status().is_success() => {
            report.pass("the storage root is writable")
        }
        Ok(response) => report.fail(format!(
            "/ready answered {} — the volume is missing, full or read only",
            response.status()
        )),
        Err(error) => report.fail(format!("{error:#}")),
    }

    if let Some(repository) = repository {
        check_repository(server, repository, &mut report)?;
    }

    Ok(report)
}

fn check_repository(server: &Server, repository: &str, report: &mut Report) -> Result<()> {
    let (org, repo) = split_namespace(repository)?;
    let batch = json!({
        "operation": "upload",
        "objects": [{ "oid": "0".repeat(64), "size": 1 }],
    });

    let response = server.post(&format!("/{org}/{repo}/objects/batch"), &batch)?;
    let status = response.status();

    if status == reqwest::StatusCode::UNAUTHORIZED {
        report.fail("the server refused these credentials".to_owned());
        return Ok(());
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        report.fail(format!(
            "these credentials have no write access to {repository}, so the advertised URL could not be checked"
        ));
        return Ok(());
    }
    if !status.is_success() {
        report.fail(format!("negotiation for {repository} answered {status}"));
        return Ok(());
    }

    report.pass(format!("negotiation succeeds for {repository}"));

    let body: Value = response.json().context("the batch response was not json")?;
    match advertised_origin(&body) {
        Some(advertised) if origin_of(server.base()) == Some(advertised.clone()) => {
            report.pass(format!("transfers are advertised at {advertised}"));
        }
        Some(advertised) => report.fail(format!(
            "the server advertises transfers at {advertised} but was reached at {} — \
             negotiation will keep succeeding and every transfer will fail. Set LFSX_PUBLIC_URL \
             to the URL clients actually use",
            server.base()
        )),
        None => report.fail("the batch response carried no upload link to check".to_owned()),
    }

    Ok(())
}

fn advertised_origin(body: &Value) -> Option<String> {
    let href = body["objects"][0]["actions"]["upload"]["href"].as_str()?;
    origin_of(href)
}

fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next()?;

    Some(format!("{scheme}://{authority}"))
}

#[cfg(test)]
mod tests;
