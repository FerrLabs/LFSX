use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client, Response};
use serde::Serialize;

pub struct Server {
    client: Client,
    base: String,
    token: Option<String>,
}

impl Server {
    pub fn new(url: &str, token: Option<String>) -> Result<Self> {
        // rustls needs a process-wide crypto provider before the first TLS
        // connection, and reqwest is built without one so the choice is this
        // project's: ring, as 0.12 used, rather than the aws-lc that 0.13 made
        // the default. It sits next to the client it is for.
        let _ = rustls::crypto::ring::default_provider().install_default();

        Ok(Self {
            client: Client::builder()
                .user_agent(concat!("lfsx/", env!("CARGO_PKG_VERSION")))
                .build()
                .context("could not build an http client")?,
            base: url.trim_end_matches('/').to_owned(),
            token,
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn get(&self, path: &str) -> Result<Response> {
        let url = format!("{}{path}", self.base);
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

pub fn resolve_token(explicit: Option<String>) -> Option<String> {
    explicit
        .or_else(|| std::env::var("LFSX_TOKEN").ok())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .filter(|token| !token.is_empty())
}

pub fn split_namespace(repository: &str) -> Result<(&str, &str)> {
    match repository.trim_matches('/').split('/').collect::<Vec<_>>()[..] {
        [org, repo] if !org.is_empty() && !repo.is_empty() => Ok((org, repo)),
        _ => bail!("expected a repository as org/repo, got {repository:?}"),
    }
}
