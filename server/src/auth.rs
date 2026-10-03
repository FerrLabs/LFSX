mod backoff;
mod budget;
mod cache;
mod credentials;
mod gitea;
mod github;
mod gitlab;
mod namespaces;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use axum::extract::{Path, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;

use crate::config::{Auth, Provider};
use crate::error::Error;
use crate::namespace::Namespace;
use crate::state::Shared;
use budget::Budget;
use cache::{Cache, Caller, Decision, IdentityCache};
pub use namespaces::Namespaces;

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

// What decides who reaches which repository, beyond what the forge answers. Held
// behind a lock rather than fixed at boot, so the dashboard can change it on a
// running server and every request after sees the new rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    pub anonymous_read: bool,
    pub restricted: Namespaces,
    pub allowed: Option<Namespaces>,
}

pub enum Authorizer {
    Forge {
        provider: Provider,
        client: reqwest::Client,
        api_url: String,
        // Boxed because this variant carries four sizeable things and the other
        // carries nothing, so every `Authorizer` in the process would pay for the
        // difference. The same reason `Backend::Bucket` boxes its handle.
        cache: Box<Cache>,
        identities: IdentityCache,
        // Spent only on a lookup the caches could not answer, which is what
        // makes it a ceiling on forge traffic rather than on requests: a push of
        // two hundred objects under one token costs one.
        budget: Budget,
        access: Arc<RwLock<Access>>,
        app: Option<Box<github::app::App>>,
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
                lookup_budget,
                anonymous_read,
                restricted,
                allowed,
                github_app,
            } => Self::Forge {
                provider: *provider,
                client: reqwest::Client::builder()
                    .user_agent(concat!("lfsx/", env!("CARGO_PKG_VERSION")))
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .expect("http client"),
                api_url: api_url.clone(),
                cache: Box::new(Cache::new(*cache_ttl, *rejection_ttl)),
                identities: IdentityCache::new(*cache_ttl),
                budget: Budget::new(*lookup_budget),
                access: Arc::new(RwLock::new(Access {
                    anonymous_read: *anonymous_read,
                    restricted: restricted.clone(),
                    allowed: allowed.clone(),
                })),
                app: github_app.as_ref().map(|configured| {
                    Box::new(github::app::App::load(
                        &configured.app_id,
                        &configured.key_file,
                        *rejection_ttl,
                    ))
                }),
            },
        }
    }

    pub fn access(&self) -> Option<Access> {
        match self {
            Self::Forge { access, .. } => Some(access.read().unwrap().clone()),
            Self::Disabled => None,
        }
    }

    pub fn set_access(&self, replacement: Access) {
        if let Self::Forge { access, .. } = self {
            *access.write().unwrap() = replacement;
        }
    }

    pub(crate) async fn permission(
        &self,
        headers: &HeaderMap,
        ns: &Namespace,
    ) -> Result<Permission, Error> {
        self.served(ns)?;
        self.forge_permission(headers, ns).await
    }

    fn served(&self, ns: &Namespace) -> Result<(), Error> {
        let Self::Forge { access, .. } = self else {
            return Ok(());
        };

        match &access.read().unwrap().allowed {
            Some(allowed) if !allowed.covers(ns) => Err(Error::NotServed),
            _ => Ok(()),
        }
    }

    #[tracing::instrument(skip_all, fields(namespace = %ns))]
    pub(crate) async fn forge_permission(
        &self,
        headers: &HeaderMap,
        ns: &Namespace,
    ) -> Result<Permission, Error> {
        let Self::Forge {
            provider,
            client,
            api_url,
            cache,
            budget,
            access,
            app,
            ..
        } = self
        else {
            return Ok(Permission::Admin);
        };

        let (anonymous_read, writers_only) = {
            let access = access.read().unwrap();
            (access.anonymous_read, access.restricted.covers(ns))
        };
        let decided = |outcome: Result<Permission, Error>| {
            if writers_only {
                let permission = outcome?;
                permission.require_write()?;
                return Ok(permission);
            }
            outcome
        };

        // A request with no credentials is the one an anonymous `git clone` makes.
        // The forge already knows whether that should be allowed, so it is asked
        // rather than refused outright, and the answer is cached under its own
        // key so it can never be handed to somebody presenting a token.
        let Some(token) = credentials::token(headers) else {
            // A restricted namespace wants write access, which nobody anonymous
            // has, so the forge is not asked. Unauthenticated rather than
            // Forbidden for the reason github.rs records about private
            // repositories: a 403 tells git-lfs the answer is final and it stops
            // asking the credential helper, so the caller who does hold write
            // access could never present it.
            if !anonymous_read || writers_only {
                return Err(Error::Unauthenticated);
            }

            if let Some(decision) = cache.get(Caller::Anonymous, ns) {
                return decision.into();
            }

            budget.afford()?;

            let outcome = match provider {
                Provider::Github => github::public(client, api_url, app.as_deref(), ns).await,
                Provider::Gitlab => gitlab::public(client, api_url, ns).await,
                Provider::Gitea => gitea::public(client, api_url, ns).await,
            };
            if let Some(decision) = Decision::of(&outcome) {
                cache.insert(Caller::Anonymous, ns, decision);
            }

            return outcome;
        };

        if let Some(decision) = cache.get(Caller::Token(&token), ns) {
            return decided(decision.into());
        }

        // Only here, past both caches. Everything above this line was answered
        // without asking anybody.
        budget.afford()?;

        let outcome = match provider {
            Provider::Github => github::permission(client, api_url, &token, ns).await,
            Provider::Gitlab => gitlab::permission(client, api_url, &token, ns).await,
            Provider::Gitea => gitea::permission(client, api_url, &token, ns).await,
        };
        if let Some(decision) = Decision::of(&outcome) {
            cache.insert(Caller::Token(&token), ns, decision);
        }

        decided(outcome)
    }
}

impl Authorizer {
    #[tracing::instrument(skip_all)]
    pub async fn actor(&self, headers: &HeaderMap) -> Result<Actor, Error> {
        let Self::Forge {
            provider,
            client,
            api_url,
            identities,
            budget,
            ..
        } = self
        else {
            return Ok(Actor("anonymous".to_owned()));
        };

        let token = credentials::token(headers).ok_or(Error::Unauthenticated)?;
        if let Some(login) = identities.get(&token) {
            return Ok(Actor(login));
        }

        budget.afford()?;

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
    let ns = match params.get("forge") {
        Some(forge) => Namespace::on(forge.as_str(), org.as_str(), repo.as_str())
            .map_err(|_| Error::NotServed)?,
        None => Namespace::new(org.as_str(), repo.as_str())?,
    };

    let permission = state
        .authorizer_for(&ns)?
        .permission(request.headers(), &ns)
        .await?;
    request.extensions_mut().insert(permission);
    request.extensions_mut().insert(ns);

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests;
