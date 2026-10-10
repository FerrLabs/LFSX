use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use super::{Refused, admit};
use crate::auth::{Access, Authorizer, Namespaces};
use crate::config::Auth;
use crate::error::Error;
use crate::state::Shared;

const MAIN_FILE: &str = "access.json";
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

#[derive(Debug, Default, Deserialize)]
pub struct Which {
    forge: Option<String>,
}

struct Target<'a> {
    authorizer: &'a Authorizer,
    auth: &'a Auth,
    file: String,
}

fn targets(state: &Shared) -> Vec<Target<'_>> {
    let mut all = vec![main_target(state)];
    all.extend(
        state
            .config
            .forges
            .iter()
            .filter_map(|forge| named_target(state, &forge.name)),
    );
    all
}

fn main_target(state: &Shared) -> Target<'_> {
    Target {
        authorizer: &state.authorizer,
        auth: &state.config.auth,
        file: MAIN_FILE.to_owned(),
    }
}

fn named_target<'a>(state: &'a Shared, name: &str) -> Option<Target<'a>> {
    let forge = state
        .config
        .forges
        .iter()
        .find(|forge| forge.name == name)?;

    Some(Target {
        authorizer: state.forges.get(name)?,
        auth: &forge.auth,
        file: format!("access-{name}.json"),
    })
}

fn target<'a>(state: &'a Shared, which: &Which) -> Result<Target<'a>, Error> {
    match which.forge.as_deref().filter(|forge| !forge.is_empty()) {
        None => Ok(main_target(state)),
        Some(name) => named_target(state, name).ok_or(Error::NotServed),
    }
}

fn from_auth(auth: &Auth) -> Option<Access> {
    match auth {
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

async fn saved(state: &Shared, file: &str) -> Result<Option<Saved>, Error> {
    let Some(bytes) = state.store.read_meta(file).await? else {
        return Ok(None);
    };

    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub(crate) async fn refresh(state: &Shared) {
    for target in targets(state) {
        refresh_one(state, &target).await;
    }
}

async fn refresh_one(state: &Shared, target: &Target<'_>) {
    let Some(environment) = from_auth(target.auth) else {
        return;
    };

    let seen = target.authorizer.revision();
    let access = match saved(state, &target.file).await {
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

    if target.authorizer.access().as_ref() != Some(&access)
        && !target.authorizer.refresh_access(seen, access)
    {
        tracing::debug!(
            file = %target.file,
            "access changed while it was being read back, so the change in force is kept"
        );
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

async fn shown(state: &Shared, target: &Target<'_>) -> Result<Shown, Error> {
    let source = if saved(state, &target.file).await?.is_some() {
        "dashboard"
    } else {
        "environment"
    };

    Ok(match target.authorizer.access() {
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
    Query(which): Query<Which>,
    headers: HeaderMap,
) -> Result<Json<Shown>, Refused> {
    admit(&state, &headers).await?;
    let target = target(&state, &which)?;

    Ok(Json(shown(&state, &target).await?))
}

pub(crate) async fn write(
    State(state): State<Shared>,
    Query(which): Query<Which>,
    headers: HeaderMap,
    Json(saved): Json<Saved>,
) -> Result<Response, Refused> {
    admit(&state, &headers).await?;
    let target = target(&state, &which)?;

    if target.authorizer.access().is_none() {
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
            &target.file,
            serde_json::to_vec_pretty(&saved).map_err(Error::from)?,
        )
        .await?;
    target.authorizer.set_access(access);
    tracing::info!(file = %target.file, ?saved, "access settings changed from the dashboard");

    Ok(Json(shown(&state, &target).await?).into_response())
}

pub(crate) async fn reset(
    State(state): State<Shared>,
    Query(which): Query<Which>,
    headers: HeaderMap,
) -> Result<Response, Refused> {
    admit(&state, &headers).await?;
    let target = target(&state, &which)?;

    let Some(environment) = from_auth(target.auth) else {
        return Ok(unchangeable());
    };

    state.store.delete_meta(&target.file).await?;
    target.authorizer.set_access(environment);
    tracing::info!(
        file = %target.file,
        "access settings reset to the environment from the dashboard"
    );

    Ok(Json(shown(&state, &target).await?).into_response())
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
