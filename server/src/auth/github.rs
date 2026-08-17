use axum::http::StatusCode;
use serde::Deserialize;

use super::Permission;
use crate::error::Error;
use crate::namespace::Namespace;

#[derive(Deserialize)]
struct Repository {
    permissions: Option<Permissions>,
}

#[derive(Deserialize)]
struct Permissions {
    #[serde(default)]
    pull: bool,
    #[serde(default)]
    push: bool,
    #[serde(default)]
    admin: bool,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

pub async fn permission(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    ns: &Namespace,
) -> Result<Permission, Error> {
    let url = format!("{api_url}/repos/{ns}");

    let response = client
        .get(&url)
        .bearer_auth(token)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge request failed");
            Error::Forge
        })?;

    match response.status() {
        StatusCode::OK => {}
        StatusCode::UNAUTHORIZED => return Err(Error::Unauthenticated),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
            if let Some(retry_after) = super::backoff::rate_limited(&response) =>
        {
            tracing::warn!(%url, retry_after, "forge is rate-limiting this server");
            return Err(Error::RateLimited { retry_after });
        }
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => return Err(Error::Forbidden),
        status => {
            tracing::warn!(%status, %url, "unexpected forge response");
            return Err(Error::Forge);
        }
    }

    let repository = response.json::<Repository>().await.map_err(|error| {
        tracing::warn!(%error, %url, "forge response could not be parsed");
        Error::Forge
    })?;

    match repository.permissions {
        Some(Permissions { admin: true, .. }) => Ok(Permission::Admin),
        Some(Permissions { push: true, .. }) => Ok(Permission::Write),
        Some(Permissions { pull: true, .. }) => Ok(Permission::Read),
        _ => Err(Error::Forbidden),
    }
}

// Is this repository readable by anybody? Asked with no credentials at all, which
// is the same question an anonymous `git clone` asks: GitHub answers 200 for a
// public repository and 404 for one it will not admit exists.
//
// A private repository is a 401 rather than a 403, deliberately. A 403 tells
// git-lfs the answer will not change, so it stops asking the credential helper and
// an authenticated user can never get in.
pub async fn public(
    client: &reqwest::Client,
    api_url: &str,
    ns: &Namespace,
) -> Result<Permission, Error> {
    let url = format!("{api_url}/repos/{ns}");

    let response = client
        .get(&url)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge request failed");
            Error::Forge
        })?;

    match response.status() {
        StatusCode::OK => Ok(Permission::Read),
        StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED => Err(Error::Unauthenticated),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
            if let Some(retry_after) = super::backoff::rate_limited(&response) =>
        {
            tracing::warn!(%url, retry_after, "forge is rate-limiting this server");
            Err(Error::RateLimited { retry_after })
        }
        // Anything else is the forge being unhelpful rather than the repository
        // being private, and a client that could have authenticated should be
        // told to try.
        status => {
            tracing::warn!(%status, %url, "unexpected forge response to an anonymous lookup");
            Err(Error::Unauthenticated)
        }
    }
}

pub async fn login(client: &reqwest::Client, api_url: &str, token: &str) -> Result<String, Error> {
    let url = format!("{api_url}/user");

    let response = client
        .get(&url)
        .bearer_auth(token)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge request failed");
            Error::Forge
        })?;

    match response.status() {
        StatusCode::OK => {}
        StatusCode::UNAUTHORIZED => return Err(Error::Unauthenticated),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
            if let Some(retry_after) = super::backoff::rate_limited(&response) =>
        {
            tracing::warn!(%url, retry_after, "forge is rate-limiting this server");
            return Err(Error::RateLimited { retry_after });
        }
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => return Err(Error::Forbidden),
        status => {
            tracing::warn!(%status, %url, "unexpected forge response");
            return Err(Error::Forge);
        }
    }

    response
        .json::<User>()
        .await
        .map(|user| user.login)
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge response could not be parsed");
            Error::Forge
        })
}
