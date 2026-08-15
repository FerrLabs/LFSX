#![allow(dead_code)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use lfsx_server::config::{Auth, Config, Provider};
use serde_json::json;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

pub struct Forge {
    pub calls: AtomicUsize,
    pub writer_can_push: Mutex<bool>,
}

pub async fn forge() -> (String, Arc<Forge>) {
    let forge = Arc::new(Forge {
        calls: AtomicUsize::new(0),
        writer_can_push: Mutex::new(true),
    });

    let router = Router::new()
        .route("/repos/{org}/{repo}", get(repository))
        .route("/user", get(user))
        .with_state(forge.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    (format!("http://{address}"), forge)
}

pub async fn repository(State(forge): State<Arc<Forge>>, headers: HeaderMap) -> Response {
    forge.calls.fetch_add(1, Ordering::SeqCst);

    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();

    match token {
        "admin" => Json(json!({ "permissions": { "pull": true, "push": true, "admin": true } }))
            .into_response(),
        "writer" => {
            let push = *forge.writer_can_push.lock().unwrap();
            Json(json!({ "permissions": { "pull": true, "push": push } })).into_response()
        }
        "reader" => Json(json!({ "permissions": { "pull": true, "push": false } })).into_response(),
        "stranger" => (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "Not Found" })),
        )
            .into_response(),
        "outsider" => (
            StatusCode::FORBIDDEN,
            Json(json!({ "message": "Resource not accessible" })),
        )
            .into_response(),
        "rate-limited" => (
            StatusCode::FORBIDDEN,
            [("x-ratelimit-remaining", "0")],
            Json(json!({ "message": "API rate limit exceeded" })),
        )
            .into_response(),
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Bad credentials" })),
        )
            .into_response(),
    }
}

pub fn config(root: &tempfile::TempDir, api_url: &str) -> Config {
    Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        storage_root: root.path().to_path_buf(),
        public_url: Some("https://lfs.example".into()),
        action_lifetime: 1800,
        gc_grace: Duration::from_secs(14 * 24 * 60 * 60),
        staging_max_age: Duration::from_secs(86400),
        max_object_size: None,
        repo_quota: None,
        auth: forge_auth(api_url, Duration::from_secs(60), Duration::from_secs(10)),
    }
}

fn forge_auth(api_url: &str, cache_ttl: Duration, rejection_ttl: Duration) -> Auth {
    Auth::Forge {
        provider: Provider::Github,
        api_url: api_url.to_owned(),
        cache_ttl,
        rejection_ttl,
    }
}

pub fn app(root: &tempfile::TempDir, api_url: &str, cache_ttl: Duration) -> Router {
    app_with_rejection_ttl(root, api_url, cache_ttl, Duration::from_secs(10))
}

pub fn app_collecting_immediately(root: &tempfile::TempDir, api_url: &str) -> Router {
    lfsx_server::app(Config {
        gc_grace: Duration::ZERO,
        ..config(root, api_url)
    })
}

pub fn app_capped(root: &tempfile::TempDir, api_url: &str, max_object_size: u64) -> Router {
    lfsx_server::app(Config {
        max_object_size: Some(max_object_size),
        ..config(root, api_url)
    })
}

pub fn app_with_rejection_ttl(
    root: &tempfile::TempDir,
    api_url: &str,
    cache_ttl: Duration,
    rejection_ttl: Duration,
) -> Router {
    lfsx_server::app(Config {
        auth: forge_auth(api_url, cache_ttl, rejection_ttl),
        ..config(root, api_url)
    })
}

pub fn credentials(token: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("git:{token}")))
}

pub async fn put(app: Router, token: Option<&str>, payload: &[u8]) -> Response {
    let oid = hex::encode(Sha256::digest(payload));
    let mut request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/LFSX/objects/{oid}"))
        .header("content-length", payload.len());

    if let Some(token) = token {
        request = request.header("authorization", credentials(token));
    }

    app.oneshot(request.body(Body::from(payload.to_vec())).unwrap())
        .await
        .unwrap()
}

pub async fn batch(app: Router, token: &str, operation: &str) -> StatusCode {
    let body = json!({
        "operation": operation,
        "objects": [{ "oid": hex::encode(Sha256::digest(b"asset")), "size": 5 }],
    });

    let request = Request::builder()
        .method("POST")
        .uri("/FerrLabs/LFSX/objects/batch")
        .header("content-type", "application/vnd.git-lfs+json")
        .header("authorization", credentials(token))
        .body(Body::from(body.to_string()))
        .unwrap();

    app.oneshot(request).await.unwrap().status()
}

pub async fn user(headers: HeaderMap) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();

    match token {
        "" | "expired-pat" => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Bad credentials" })),
        )
            .into_response(),
        login => Json(json!({ "login": login })).into_response(),
    }
}

pub async fn post(app: Router, token: &str, path: &str, body: serde_json::Value) -> Response {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", credentials(token))
        .body(Body::from(body.to_string()))
        .unwrap();

    app.oneshot(request).await.unwrap()
}

pub async fn read_json(response: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    serde_json::from_slice(&bytes).unwrap()
}
