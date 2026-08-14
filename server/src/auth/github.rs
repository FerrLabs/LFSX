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
}

pub async fn permission(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    ns: &Namespace<'_>,
) -> Result<Permission, Error> {
    let url = format!("{api_url}/repos/{}/{}", ns.org(), ns.repo());

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
        Some(Permissions { push: true, .. }) => Ok(Permission::Write),
        Some(Permissions { pull: true, .. }) => Ok(Permission::Read),
        _ => Err(Error::Forbidden),
    }
}
