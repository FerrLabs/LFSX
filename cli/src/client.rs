use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::Url;
use reqwest::blocking::{Client, Response};
use serde::Serialize;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Server {
    client: Client,
    base: String,
    token: Option<String>,
}

impl Server {
    pub fn new(url: &str, token: Option<String>, timeout: Option<Duration>) -> Result<Self> {
        // rustls needs a process-wide crypto provider before the first TLS
        // connection, and reqwest is built without one so the choice is this
        // project's: ring, as 0.12 used, rather than the aws-lc that 0.13 made
        // the default. It sits next to the client it is for.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let token = token.filter(|token| !token.is_empty());
        if token.is_some() {
            ensure_credential_is_safe_to_send(url)?;
        }

        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("lfsx/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(timeout)
                .build()
                .context("could not build an http client")?,
            base: url.trim_end_matches('/').to_owned(),
            token,
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn get_from_root(&self, path: &str) -> Result<Response> {
        self.fetch(format!("{}{path}", service_root(&self.base)))
    }

    fn fetch(&self, url: String) -> Result<Response> {
        self.authenticated(self.client.get(&url))
            .send()
            .with_context(|| format!("could not reach {url}"))
    }

    pub fn post<B: Serialize>(&self, path: &str, body: &B) -> Result<Response> {
        let url = format!("{}{path}", self.base);
        self.authenticated(self.client.post(&url))
            .header("content-type", "application/vnd.git-lfs+json")
            .json(body)
            .send()
            .with_context(|| format!("could not reach {url}"))
    }

    fn authenticated(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match &self.token {
            Some(token) => request.basic_auth("git", Some(token)),
            None => request,
        }
    }
}

pub fn service_root(base: &str) -> &str {
    match base.rsplit_once("/-/") {
        Some((root, forge)) if !forge.is_empty() && !forge.contains('/') => root,
        _ => base,
    }
}

fn ensure_credential_is_safe_to_send(url: &str) -> Result<()> {
    let parsed = Url::parse(url).with_context(|| format!("{url:?} is not a valid URL"))?;
    let loopback = parsed.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback) {
        Ok(())
    } else {
        bail!("refusing to send a token to {url}: use an https URL, or drop the token")
    }
}

pub fn split_namespace(repository: &str) -> Result<(&str, &str)> {
    match repository.trim_matches('/').split('/').collect::<Vec<_>>()[..] {
        [org, repo] if !org.is_empty() && !repo.is_empty() => Ok((org, repo)),
        _ => bail!("expected a repository as org/repo, got {repository:?}"),
    }
}

#[cfg(test)]
mod tests;
