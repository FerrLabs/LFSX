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
use cache::{Cache, Decision, IdentityCache};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor(pub String);

impl Permission {
    pub fn require_write(self) -> Result<(), Error> {
        matches!(self, Self::Write | Self::Admin)
            .then_some(())
            .ok_or(Error::Forbidden)
    }

    pub fn require_admin(self) -> Result<(), Error> {
        matches!(self, Self::Admin)
            .then_some(())
            .ok_or(Error::Forbidden)
    }
}

pub enum Authorizer {
    Github {
        client: reqwest::Client,
        api_url: String,
        cache: Cache,
        identities: IdentityCache,
    },
    Disabled,
}

impl Authorizer {
    pub fn new(auth: &Auth) -> Self {
        match auth {
            Auth::Disabled => Self::Disabled,
            Auth::Github {
                api_url,
                cache_ttl,
                rejection_ttl,
            } => Self::Github {
                client: reqwest::Client::builder()
                    .user_agent(concat!("lfsx/", env!("CARGO_PKG_VERSION")))
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .expect("http client"),
                api_url: api_url.clone(),
                cache: Cache::new(*cache_ttl, *rejection_ttl),
                identities: IdentityCache::new(*cache_ttl),
            },
        }
    }

    async fn permission(&self, headers: &HeaderMap, ns: &Namespace) -> Result<Permission, Error> {
        let Self::Github {
            client,
            api_url,
            cache,
            ..
        } = self
        else {
            return Ok(Permission::Admin);
        };

        let token = credentials::token(headers).ok_or(Error::Unauthenticated)?;
        if let Some(decision) = cache.get(&token, ns) {
            return decision.into();
        }

        let outcome = github::permission(client, api_url, &token, ns).await;
        if let Some(decision) = Decision::of(&outcome) {
            cache.insert(&token, ns, decision);
        }

        outcome
    }
}

impl Authorizer {
    pub async fn actor(&self, headers: &HeaderMap) -> Result<Actor, Error> {
        let Self::Github {
            client,
            api_url,
            identities,
            ..
        } = self
        else {
            return Ok(Actor("anonymous".to_owned()));
        };

        let token = credentials::token(headers).ok_or(Error::Unauthenticated)?;
        if let Some(login) = identities.get(&token) {
            return Ok(Actor(login));
        }

        let login = github::login(client, api_url, &token).await?;
        identities.insert(&token, &login);

        Ok(Actor(login))
    }
}

pub async fn authorize(
    State(state): State<Shared>,
    Path(params): Path<HashMap<String, String>>,
    mut request: Request,
    next: Next,
) -> Result<Response, Error> {
    let (Some(org), Some(repo)) = (params.get("org"), params.get("repo")) else {
        return Err(Error::MalformedNamespace);
    };
    let ns = Namespace::new(org.as_str(), repo.as_str())?;

    let permission = state.authorizer.permission(request.headers(), &ns).await?;
    request.extensions_mut().insert(permission);
    request.extensions_mut().insert(ns);

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests;
