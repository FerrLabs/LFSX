use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use super::{Refused, admit};
use crate::auth::{Access, Namespaces};
use crate::config::{Auth, Config};
use crate::error::Error;
use crate::state::Shared;

const FILE: &str = "access.json";
const EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub anonymous_read: bool,
    pub restricted: Vec<String>,
    pub allowed: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct Shown {
    pub editable: bool,
    pub source: &'static str,
    pub anonymous_read: bool,
    pub restricted: Vec<String>,
    pub allowed: Option<Vec<String>>,
}

pub(crate) fn from_environment(config: &Config) -> Option<Access> {
    match &config.auth {
        Auth::Disabled => None,
        Auth::Forge {
            anonymous_read,
            restricted,
            allowed,
            ..
        } => Some(Access {
            anonymous_read: *anonymous_read,
            restricted: restricted.clone(),
            allowed: allowed.clone(),
        }),
    }
}

impl Saved {
    fn access(&self) -> Result<Access, Vec<String>> {
        let restricted = Namespaces::from_entries(&self.restricted);
        let allowed = self.allowed.as_deref().map(Namespaces::from_entries);

        match (restricted, allowed.transpose()) {
            (Ok(restricted), Ok(allowed)) => Ok(Access {
                anonymous_read: self.anonymous_read,
                restricted,
                allowed,
            }),
            (restricted, allowed) => Err(restricted
                .err()
                .into_iter()
                .chain(allowed.err())
                .flatten()
                .collect()),
        }
    }
}

async fn saved(state: &Shared) -> Result<Option<Saved>, Error> {
    let Some(bytes) = state.store.read_meta(FILE).await? else {
        return Ok(None);
    };

    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub(crate) async fn refresh(state: &Shared) {
    let Some(environment) = from_environment(&state.config) else {
        return;
    };

    let access = match saved(state).await {
        Ok(None) => environment,
        Ok(Some(saved)) => match saved.access() {
            Ok(access) => access,
            Err(unreadable) => {
                tracing::warn!(
                    ?unreadable,
                    "the access settings saved from the dashboard name entries that are not \
                     org/repo, so the environment's are in force"
                );
                environment
            }
        },
        Err(error) => {
            tracing::warn!(
                %error,
                "the access settings saved from the dashboard could not be read, so the ones in \
                 force are kept"
            );
            return;
        }
    };

    if state.authorizer.access().as_ref() != Some(&access) {
        state.authorizer.set_access(access);
    }
}

pub(crate) fn keep_fresh(state: Shared) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(EVERY);
        loop {
            tick.tick().await;
            refresh(&state).await;
        }
    });
}

async fn shown(state: &Shared) -> Result<Shown, Error> {
    let source = if saved(state).await?.is_some() {
        "dashboard"
    } else {
        "environment"
    };

    Ok(match state.authorizer.access() {
        Some(access) => Shown {
            editable: true,
            source,
            anonymous_read: access.anonymous_read,
            restricted: access.restricted.entries(),
            allowed: access.allowed.as_ref().map(Namespaces::entries),
        },
        None => Shown {
            editable: false,
            source: "environment",
            anonymous_read: true,
            restricted: Vec::new(),
            allowed: None,
        },
    })
}

pub(crate) async fn read(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> Result<Json<Shown>, Refused> {
    admit(&state, &headers).await?;

    Ok(Json(shown(&state).await?))
}

pub(crate) async fn write(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(saved): Json<Saved>,
) -> Result<Response, Refused> {
    admit(&state, &headers).await?;

    if state.authorizer.access().is_none() {
        return Ok(unchangeable());
    }

    let access = match saved.access() {
        Ok(access) => access,
        Err(unreadable) => {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "message": "these entries are not org/repo, org/prefix-* or org/*",
                    "unreadable": unreadable,
                })),
            )
                .into_response());
        }
    };

    state
        .store
        .write_meta(
            FILE,
            serde_json::to_vec_pretty(&saved).map_err(Error::from)?,
        )
        .await?;
    state.authorizer.set_access(access);
    tracing::info!(?saved, "access settings changed from the dashboard");

    Ok(Json(shown(&state).await?).into_response())
}

pub(crate) async fn reset(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> Result<Response, Refused> {
    admit(&state, &headers).await?;

    let Some(environment) = from_environment(&state.config) else {
        return Ok(unchangeable());
    };

    state.store.delete_meta(FILE).await?;
    state.authorizer.set_access(environment);
    tracing::info!("access settings reset to the environment from the dashboard");

    Ok(Json(shown(&state).await?).into_response())
}

fn unchangeable() -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "message": "authentication is disabled, so there is no access to change"
        })),
    )
        .into_response()
}
