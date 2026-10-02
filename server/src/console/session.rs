use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::{Refused, tokens};
use crate::config::Auth;
use crate::error::Error;
use crate::state::Shared;

const KEY: &str = "session.key";
const COOKIE_NAME: &str = "lfsx_session";
const LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "via", content = "name", rename_all = "lowercase")]
pub enum Viewer {
    Token(String),
    Forge(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    viewer: Viewer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issued: Option<String>,
    expires: i64,
}

#[derive(Debug, Deserialize)]
pub struct SignIn {
    token: String,
}

async fn key(state: &Shared) -> Result<&[u8], Error> {
    state
        .session_key
        .get_or_try_init(|| kept_key(state))
        .await
        .map(Vec::as_slice)
}

async fn kept_key(state: &Shared) -> Result<Vec<u8>, Error> {
    if let Some(key) = state.store.read_meta(KEY).await? {
        return Ok(key);
    }

    let mut key = vec![0u8; 32];
    getrandom::fill(&mut key).expect("the operating system has a random number generator");
    state.store.write_meta(KEY, key).await?;

    state
        .store
        .read_meta(KEY)
        .await?
        .ok_or(Error::Misconfigured(
            "the session key could not be kept in the store",
        ))
}

fn mac(key: &[u8]) -> Hmac<Sha256> {
    Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length")
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn seal(key: &[u8], viewer: Viewer, issued: Option<String>) -> Result<String, Error> {
    let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Claims {
        viewer,
        issued,
        expires: now() + LIFETIME.as_secs() as i64,
    })?);
    let mut mac = mac(key);
    mac.update(claims.as_bytes());

    Ok(format!(
        "{claims}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

fn open(key: &[u8], sealed: &str) -> Option<Claims> {
    let (claims, signature) = sealed.split_once('.')?;
    let mut mac = mac(key);
    mac.update(claims.as_bytes());
    mac.verify_slice(&URL_SAFE_NO_PAD.decode(signature).ok()?)
        .ok()?;

    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).ok()?).ok()?;
    (claims.expires > now()).then_some(claims)
}

fn presented(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(COOKIE_NAME)?.strip_prefix('='))
}

pub(crate) async fn viewer(state: &Shared, headers: &HeaderMap) -> Result<Option<Viewer>, Error> {
    let Some(sealed) = presented(headers) else {
        return Ok(None);
    };

    let Some(claims) = open(key(state).await?, sealed) else {
        return Ok(None);
    };

    match (&claims.viewer, &claims.issued) {
        (Viewer::Forge(_), _) => Ok(Some(claims.viewer)),
        (Viewer::Token(_), Some(hash)) if tokens::still_issued(&state.store, hash).await? => {
            Ok(Some(claims.viewer))
        }
        (Viewer::Token(_), _) => Ok(None),
    }
}

fn cookie(state: &Shared, value: &str, max_age: u64) -> HeaderValue {
    let secure = state
        .config
        .public_url
        .as_deref()
        .is_some_and(|url| url.starts_with("https://"));

    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={value}; Path=/-/; Max-Age={max_age}; HttpOnly; SameSite=Strict{}",
        if secure { "; Secure" } else { "" }
    ))
    .expect("a cookie built from base64 and fixed text is a valid header")
}

async fn admitted(state: &Shared, token: &str) -> Result<(Viewer, Option<String>), Error> {
    if let Some(issued) = tokens::holder(&state.store, token).await? {
        let hash = issued.hash().to_owned();
        return Ok((Viewer::Token(issued.name), Some(hash)));
    }

    let (Auth::Forge { .. }, Some(admins)) = (
        &state.config.auth,
        state
            .config
            .dashboard
            .as_ref()
            .and_then(|dashboard| dashboard.admins.as_ref()),
    ) else {
        return Err(Error::Unauthenticated);
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| Error::Unauthenticated)?,
    );
    state
        .authorizer
        .forge_permission(&headers, admins)
        .await?
        .require_admin()?;

    Ok((
        Viewer::Forge(state.authorizer.actor(&headers).await?.0),
        None,
    ))
}

pub(crate) async fn sign_in(
    State(state): State<Shared>,
    Json(SignIn { token }): Json<SignIn>,
) -> Result<Response, Refused> {
    if state.config.dashboard.is_none() {
        return Err(Error::NotServed.into());
    }

    let (viewer, issued) = admitted(&state, token.trim()).await?;
    let sealed = seal(key(&state).await?, viewer.clone(), issued)?;
    tracing::info!(?viewer, "signed in to the dashboard");

    Ok((
        [(SET_COOKIE, cookie(&state, &sealed, LIFETIME.as_secs()))],
        Json(viewer),
    )
        .into_response())
}

pub(crate) async fn current(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> Result<Json<Viewer>, Refused> {
    if state.config.dashboard.is_none() {
        return Err(Error::NotServed.into());
    }

    Ok(Json(
        viewer(&state, &headers)
            .await?
            .ok_or(Error::Unauthenticated)?,
    ))
}

pub(crate) async fn sign_out(State(state): State<Shared>) -> Response {
    (
        StatusCode::NO_CONTENT,
        [(SET_COOKIE, cookie(&state, "", 0))],
    )
        .into_response()
}

#[cfg(test)]
mod tests;
