use axum::http::StatusCode;
use serde::Deserialize;

use super::Permission;
use crate::error::Error;
use crate::namespace::Namespace;

#[derive(Deserialize)]
struct Repository {
    permissions: Option<Permissions>,
}

// No `pull`. Read is settled by the response arriving at all, so the only thing
// this block is consulted for is whether to grant more than that.
#[derive(Deserialize)]
struct Permissions {
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

    // Said out loud because the refusal the client sees is the same one a
    // readable repository with an unreadable permissions block produces, and an
    // operator staring at a failing CI job has nothing else to tell them apart.
    let repository = send(
        client,
        &url,
        token,
        "the forge will not admit this repository to this token",
    )
    .await?
    .json::<Repository>()
    .await
    .map_err(|error| {
        tracing::warn!(%error, %url, "forge response could not be parsed");
        Error::Forge
    })?;

    // The answer arriving at all is the proof of read access: the forge returns
    // 404 to a token it will not admit the repository to, and a public
    // repository is readable by anyone regardless. So the block below only ever
    // raises the level, and can never be the thing that refuses.
    //
    // That is not a convenience. A GitHub App installation token, which is what
    // `GITHUB_TOKEN` is inside Actions, receives a block with every field false:
    //
    //     {"admin":false,"maintain":false,"push":false,"triage":false,"pull":false}
    //
    // because that field reports the authenticated *user's* permissions and an
    // installation token has no user behind it. Read literally it says "no
    // access" about a token that had just been handed the repository. Believing
    // it refused every CI job on a private repository, with a message about
    // credentials that were correct.
    //
    // Write is still only ever granted by the block saying so. Nothing here
    // infers it, because a job that uploads objects it was never granted is the
    // failure worth keeping impossible.
    match repository.permissions {
        Some(Permissions { admin: true, .. }) => Ok(Permission::Admin),
        Some(Permissions { push: true, .. }) => Ok(Permission::Write),
        _ => Ok(Permission::Read),
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

    let response = crate::telemetry::propagated(asking(client, &url))
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge request failed");
            Error::Forge
        })?;

    match response.status() {
        StatusCode::OK => Ok(Permission::Read),
        // Split apart because they mean different things to whoever is reading
        // the log. A 404 is the ordinary answer for a repository the forge will
        // not admit to a caller with no credentials, which is most of them. A
        // 401 means the forge rejected this server's unauthenticated call
        // itself, which points at the endpoint rather than at the repository.
        StatusCode::NOT_FOUND => {
            tracing::info!(%url, "the forge will not admit this repository anonymously");
            Err(Error::Unauthenticated)
        }
        StatusCode::UNAUTHORIZED => {
            tracing::warn!(%url, "the forge refused this server's anonymous lookup");
            Err(Error::Unauthenticated)
        }
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

    send(
        client,
        &url,
        token,
        "the forge will not say who this token belongs to",
    )
    .await?
    .json::<User>()
    .await
    .map(|user| user.login)
    .map_err(|error| {
        tracing::warn!(%error, %url, "forge response could not be parsed");
        Error::Forge
    })
}

fn asking(client: &reqwest::Client, url: &str) -> reqwest::RequestBuilder {
    client
        .get(url)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
}

async fn send(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    refusal: &'static str,
) -> Result<reqwest::Response, Error> {
    let response = crate::telemetry::propagated(asking(client, url).bearer_auth(token))
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge request failed");
            Error::Forge
        })?;

    match response.status() {
        StatusCode::OK => Ok(response),
        StatusCode::UNAUTHORIZED => Err(Error::Unauthenticated),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
            if let Some(retry_after) = super::backoff::rate_limited(&response) =>
        {
            tracing::warn!(%url, retry_after, "forge is rate-limiting this server");
            Err(Error::RateLimited { retry_after })
        }
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            tracing::info!(%url, "{refusal}");
            Err(Error::Forbidden)
        }
        status => {
            tracing::warn!(%status, %url, "unexpected forge response");
            Err(Error::Forge)
        }
    }
}

#[cfg(test)]
mod tests;
