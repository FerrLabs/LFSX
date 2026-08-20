use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use serde::Deserialize;

use super::Permission;
use crate::error::Error;
use crate::namespace::Namespace;

// Gitea and Forgejo are the same API. Forgejo is a fork of Gitea and answers the
// same routes with the same shapes, so one provider serves both and the only
// thing that distinguishes an instance is its API root.

#[derive(Deserialize)]
struct Repository {
    permissions: Option<Permissions>,
}

// No `pull`, for the reason GitHub's has none: read is settled by the answer
// arriving at all, so this block is only ever consulted for whether to grant
// more than that.
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

    let repository = send(client, &url, token)
        .await?
        .json::<Repository>()
        .await
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge response could not be parsed");
            Error::Forge
        })?;

    // The answer arriving at all is the proof of read access: Gitea hides a
    // repository it will not admit to a caller behind a 404 rather than refusing
    // it, exactly as GitHub does, and a public repository is readable by anyone
    // regardless. So this block only ever raises the level and can never be the
    // thing that refuses.
    //
    // That is GitHub's shape rather than GitLab's, and deliberately. GitLab's
    // module treats a missing block as a refusal because whether a CI job token
    // gets one there was never established. Here it was: the 404 is what carries
    // the refusal, so a body at all has already answered the question.
    match repository.permissions {
        Some(Permissions { admin: true, .. }) => Ok(Permission::Admin),
        Some(Permissions { push: true, .. }) => Ok(Permission::Write),
        _ => Ok(Permission::Read),
    }
}

// Is this repository readable by anybody? Asked with no credentials, which is the
// question an anonymous `git clone` asks. A private repository is a 401 rather
// than a 403, because a 403 tells git-lfs the answer will not change and it stops
// asking the credential helper.
pub async fn public(
    client: &reqwest::Client,
    api_url: &str,
    ns: &Namespace,
) -> Result<Permission, Error> {
    let url = format!("{api_url}/repos/{ns}");

    let response = client.get(&url).send().await.map_err(|error| {
        tracing::warn!(%error, %url, "forge request failed");
        Error::Forge
    })?;

    if response.status() == StatusCode::OK {
        return Ok(Permission::Read);
    }

    if let Some(retry_after) = throttled(&response) {
        tracing::warn!(%url, retry_after, "forge is rate-limiting this server");
        return Err(Error::RateLimited { retry_after });
    }

    match response.status() {
        // Split apart because they mean different things to whoever reads the
        // log. A 404 is the ordinary answer for a repository the forge will not
        // admit to a caller with no credentials, which is most of them. A 401
        // means the forge rejected this server's unauthenticated call itself,
        // which points at the endpoint rather than at the repository.
        StatusCode::NOT_FOUND => {
            tracing::info!(%url, "the forge will not admit this repository anonymously");
            Err(Error::Unauthenticated)
        }
        StatusCode::UNAUTHORIZED => {
            tracing::warn!(%url, "the forge refused this server's anonymous lookup");
            Err(Error::Unauthenticated)
        }
        // Anything else is the forge being unhelpful rather than the repository
        // being private, and a client that could have authenticated should try.
        status => {
            tracing::warn!(%status, %url, "unexpected forge response to an anonymous lookup");
            Err(Error::Unauthenticated)
        }
    }
}

pub async fn login(client: &reqwest::Client, api_url: &str, token: &str) -> Result<String, Error> {
    let url = format!("{api_url}/user");

    send(client, &url, token)
        .await?
        .json::<User>()
        .await
        .map(|user| user.login)
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge response could not be parsed");
            Error::Forge
        })
}

async fn send(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<reqwest::Response, Error> {
    let response = client
        .get(url)
        // `token`, not `Bearer`. It is the scheme Gitea documents for the access
        // tokens a user creates, it is what Forgejo inherited, and it has worked
        // since long before either accepted anything else. Bearer works on a
        // current instance and is the sort of thing that stops working on an old
        // one, which is most self-hosted instances.
        .header(AUTHORIZATION, format!("token {token}"))
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge request failed");
            Error::Forge
        })?;

    if response.status() == StatusCode::OK {
        return Ok(response);
    }

    if let Some(retry_after) = throttled(&response) {
        tracing::warn!(%url, retry_after, "forge is rate-limiting this server");
        return Err(Error::RateLimited { retry_after });
    }

    match response.status() {
        StatusCode::UNAUTHORIZED => Err(Error::Unauthenticated),
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            // Said out loud because the refusal a client sees is the same one a
            // readable repository with an unreadable body produces, and an
            // operator staring at a failing CI job has nothing else to go on.
            tracing::info!(%url, "the forge will not admit this repository to this token");
            Err(Error::Forbidden)
        }
        status => {
            tracing::warn!(%status, %url, "unexpected forge response");
            Err(Error::Forge)
        }
    }
}

// Neither Gitea nor Forgejo limits its own API, so throttling arrives from
// whatever sits in front of the instance, and it does not look like GitHub's 403
// or GitLab's 429 because it is not the forge speaking. nginx's `limit_req`
// answers 503 unless it has been told to answer 429, and both carry `Retry-After`
// when configured to.
//
// The distinction that matters is the header, not the status. A 503 carrying
// `Retry-After` is something saying come back later, which is a limit. A 503
// saying nothing is an instance that is down, and it stays a bad gateway so the
// outage reads as an outage.
fn throttled(response: &reqwest::Response) -> Option<u64> {
    let says_when = response.headers().contains_key("retry-after");

    match response.status() {
        StatusCode::TOO_MANY_REQUESTS => Some(super::backoff::retry_after(response)),
        StatusCode::SERVICE_UNAVAILABLE if says_when => Some(super::backoff::retry_after(response)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
