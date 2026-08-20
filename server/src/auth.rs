mod backoff;
mod cache;
mod credentials;
mod gitea;
mod github;
mod gitlab;

use std::collections::HashMap;

use axum::extract::{Path, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;

use crate::config::{Auth, Provider};
use crate::error::Error;
use crate::namespace::Namespace;
use crate::state::Shared;
use cache::{Cache, Caller, Decision, IdentityCache};

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
    Forge {
        provider: Provider,
        client: reqwest::Client,
        api_url: String,
        cache: Cache,
        identities: IdentityCache,
        anonymous_read: bool,
    },
    Disabled,
}

impl Authorizer {
    pub fn new(auth: &Auth) -> Self {
        crate::tls::install_crypto_provider();

        match auth {
            Auth::Disabled => Self::Disabled,
            Auth::Forge {
                provider,
                api_url,
                cache_ttl,
                rejection_ttl,
                anonymous_read,
            } => Self::Forge {
                provider: *provider,
                client: reqwest::Client::builder()
                    .user_agent(concat!("lfsx/", env!("CARGO_PKG_VERSION")))
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .expect("http client"),
                api_url: api_url.clone(),
                cache: Cache::new(*cache_ttl, *rejection_ttl),
                identities: IdentityCache::new(*cache_ttl),
                anonymous_read: *anonymous_read,
            },
        }
    }

    async fn permission(&self, headers: &HeaderMap, ns: &Namespace) -> Result<Permission, Error> {
        let Self::Forge {
            provider,
            client,
            api_url,
            cache,
            anonymous_read,
            ..
        } = self
        else {
            return Ok(Permission::Admin);
        };

        // A request with no credentials is the one an anonymous `git clone` makes.
        // The forge already knows whether that should be allowed, so it is asked
        // rather than refused outright, and the answer is cached under its own
        // key so it can never be handed to somebody presenting a token.
        let Some(token) = credentials::token(headers) else {
            if !*anonymous_read {
                return Err(Error::Unauthenticated);
            }

            if let Some(decision) = cache.get(Caller::Anonymous, ns) {
                return decision.into();
            }

            let outcome = match provider {
                Provider::Github => github::public(client, api_url, ns).await,
                Provider::Gitlab => gitlab::public(client, api_url, ns).await,
                Provider::Gitea => gitea::public(client, api_url, ns).await,
            };
            if let Some(decision) = Decision::of(&outcome) {
                cache.insert(Caller::Anonymous, ns, decision);
            }

            return outcome;
        };

        if let Some(decision) = cache.get(Caller::Token(&token), ns) {
            return decision.into();
        }

        let outcome = match provider {
            Provider::Github => github::permission(client, api_url, &token, ns).await,
            Provider::Gitlab => gitlab::permission(client, api_url, &token, ns).await,
            Provider::Gitea => gitea::permission(client, api_url, &token, ns).await,
        };
        if let Some(decision) = Decision::of(&outcome) {
            cache.insert(Caller::Token(&token), ns, decision);
        }

        outcome
    }
}

impl Authorizer {
    pub async fn actor(&self, headers: &HeaderMap) -> Result<Actor, Error> {
        let Self::Forge {
            provider,
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

        let login = match provider {
            Provider::Github => github::login(client, api_url, &token).await?,
            Provider::Gitlab => gitlab::login(client, api_url, &token).await?,
            Provider::Gitea => gitea::login(client, api_url, &token).await?,
        };
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
