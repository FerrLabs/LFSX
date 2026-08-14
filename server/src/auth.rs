mod cache;
mod credentials;
mod github;

use std::collections::HashMap;

use axum::extract::{Path, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;

use crate::config::Auth;
use crate::error::Error;
use crate::namespace::Namespace;
use crate::state::Shared;
use cache::Cache;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
}

impl Permission {
    pub fn require_write(self) -> Result<(), Error> {
        matches!(self, Self::Write)
            .then_some(())
            .ok_or(Error::Forbidden)
    }
}

pub enum Authorizer {
    Github {
        client: reqwest::Client,
        api_url: String,
        cache: Cache,
    },
    Disabled,
}

impl Authorizer {
    pub fn new(auth: &Auth) -> Self {
        match auth {
            Auth::Disabled => Self::Disabled,
            Auth::Github { api_url, cache_ttl } => Self::Github {
                client: reqwest::Client::builder()
                    .user_agent(concat!("lfsx/", env!("CARGO_PKG_VERSION")))
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .expect("http client"),
                api_url: api_url.clone(),
                cache: Cache::new(*cache_ttl),
            },
        }
    }

    async fn permission(
        &self,
        headers: &HeaderMap,
        ns: &Namespace<'_>,
    ) -> Result<Permission, Error> {
        let Self::Github {
            client,
            api_url,
            cache,
        } = self
        else {
            return Ok(Permission::Write);
        };

        let token = credentials::token(headers).ok_or(Error::Unauthenticated)?;
        if let Some(permission) = cache.get(&token, ns) {
            return Ok(permission);
        }

        let permission = github::permission(client, api_url, &token, ns).await?;
        cache.insert(&token, ns, permission);

        Ok(permission)
    }
}

pub async fn authorize(
    State(state): State<Shared>,
    Path(params): Path<HashMap<String, String>>,
    mut request: Request,
    next: Next,
) -> Result<Response, Error> {
    let (org, repo) = match (params.get("org"), params.get("repo")) {
        (Some(org), Some(repo)) => (org, repo),
        _ => return Err(Error::MalformedNamespace),
    };
    let ns = Namespace::new(org, repo)?;

    let permission = state.authorizer.permission(request.headers(), &ns).await?;
    request.extensions_mut().insert(permission);

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests;
