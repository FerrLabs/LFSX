use axum::http::StatusCode;
use serde::Deserialize;

use super::Permission;
use crate::error::Error;
use crate::namespace::Namespace;

#[derive(Deserialize)]
struct Project {
    permissions: Option<Permissions>,
}

#[derive(Deserialize)]
struct Permissions {
    project_access: Option<Access>,
    group_access: Option<Access>,
}

#[derive(Deserialize)]
struct Access {
    access_level: u32,
}

#[derive(Deserialize)]
struct User {
    username: String,
}

const REPORTER: u32 = 20;
const DEVELOPER: u32 = 30;
const MAINTAINER: u32 = 40;

pub async fn permission(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    ns: &Namespace,
) -> Result<Permission, Error> {
    let url = format!(
        "{api_url}/projects/{}%2F{}",
        urlencoding(ns.org()),
        urlencoding(ns.repo())
    );

    let response = send(client, &url, token).await?;
    let project = response.json::<Project>().await.map_err(|error| {
        tracing::warn!(%error, %url, "forge response could not be parsed");
        Error::Forge
    })?;

    // Kept apart from a block that grants too little, because they are different
    // afternoons for whoever is reading the log: one is the forge saying no, the
    // other is the forge saying nothing. GitHub answers with no block at all for
    // an app installation token, and this shape would refuse the same request.
    // Whether GitLab does the same for a CI job token is untested, so the
    // decision is left alone and only the cause is written down.
    let declared = project.permissions.is_some();
    let level = project
        .permissions
        .map(|permissions| {
            let of = |access: Option<Access>| access.map(|a| a.access_level).unwrap_or_default();
            of(permissions.project_access).max(of(permissions.group_access))
        })
        .unwrap_or_default();

    match level {
        level if level >= MAINTAINER => Ok(Permission::Admin),
        level if level >= DEVELOPER => Ok(Permission::Write),
        level if level >= REPORTER => Ok(Permission::Read),
        _ if declared => {
            tracing::info!(%url, level, "the forge grants this token too little for this project");
            Err(Error::Forbidden)
        }
        _ => {
            tracing::info!(%url, "the forge sent no permissions block for this project");
            Err(Error::Forbidden)
        }
    }
}

// The same question as GitHub's, asked the same way: no credentials, and a
// project the server will not admit exists answers 404. A private project is a
// 401 rather than a 403, so git-lfs keeps asking the credential helper.
pub async fn public(
    client: &reqwest::Client,
    api_url: &str,
    ns: &Namespace,
) -> Result<Permission, Error> {
    let url = format!("{api_url}/projects/{}", urlencoding(&ns.to_string()));

    let response = client.get(&url).send().await.map_err(|error| {
        tracing::warn!(%error, %url, "forge request failed");
        Error::Forge
    })?;

    match response.status() {
        StatusCode::OK => Ok(Permission::Read),
        StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED => Err(Error::Unauthenticated),
        StatusCode::TOO_MANY_REQUESTS => {
            let retry_after = super::backoff::retry_after(&response);
            tracing::warn!(%url, retry_after, "forge is rate-limiting this server");
            Err(Error::RateLimited { retry_after })
        }
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
        .map(|user| user.username)
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
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, %url, "forge request failed");
            Error::Forge
        })?;

    match response.status() {
        StatusCode::OK => Ok(response),
        StatusCode::UNAUTHORIZED => Err(Error::Unauthenticated),
        StatusCode::TOO_MANY_REQUESTS => {
            let retry_after = super::backoff::retry_after(&response);
            tracing::warn!(%url, retry_after, "forge is rate-limiting this server");
            Err(Error::RateLimited { retry_after })
        }
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => Err(Error::Forbidden),
        status => {
            tracing::warn!(%status, %url, "unexpected forge response");
            Err(Error::Forge)
        }
    }
}

fn urlencoding(segment: &str) -> String {
    segment
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests;
