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

pub async fn repository(
    State(forge): State<Arc<Forge>>,
    axum::extract::Path((_org, repo)): axum::extract::Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
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
        // A second person who can push and is nobody's administrator, which is
        // what "taking an abandoned lock needs no admin" has to be tested with.
        "colleague" => {
            Json(json!({ "permissions": { "pull": true, "push": true } })).into_response()
        }
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
        // A secondary limit: the forge says how long to wait rather than when the
        // window resets.
        "throttled" => (
            StatusCode::FORBIDDEN,
            [("retry-after", "42")],
            Json(json!({ "message": "You have exceeded a secondary rate limit" })),
        )
            .into_response(),
        "rate-limited" => (
            StatusCode::FORBIDDEN,
            [("x-ratelimit-remaining", "0")],
            Json(json!({ "message": "API rate limit exceeded" })),
        )
            .into_response(),
        // No credentials at all. GitHub answers 200 for a public repository and
        // 404 for one it will not admit exists, and the repository in the path is
        // what decides which.
        "" if repo == "Public" => {
            Json(json!({ "permissions": { "pull": true, "push": false } })).into_response()
        }
        "" => (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "Not Found" })),
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
        lock_max_age: None,
        max_object_size: None,
        max_concurrent_transfers: 128,
        repo_quota: None,
        compression: None,
        encryption_key_file: None,
        storage: lfsx_server::config::Storage::Local,
        auth: forge_auth(api_url, Duration::from_secs(60), Duration::from_secs(10)),
    }
}

fn forge_auth(api_url: &str, cache_ttl: Duration, rejection_ttl: Duration) -> Auth {
    // Off for the existing suite, which is about what a token buys. The cases
    // that are about anonymous access turn it on explicitly, so neither set can
    // pass because of the other's setting.
    anonymous_forge_auth(api_url, cache_ttl, rejection_ttl, false)
}

pub fn anonymous_forge_auth(
    api_url: &str,
    cache_ttl: Duration,
    rejection_ttl: Duration,
    anonymous_read: bool,
) -> Auth {
    Auth::Forge {
        provider: Provider::Github,
        api_url: api_url.to_owned(),
        cache_ttl,
        rejection_ttl,
        lookup_budget: None,
        anonymous_read,
        github_app: None,
    }
}

pub fn app_reading_anonymously(root: &tempfile::TempDir, api_url: &str) -> Router {
    lfsx_server::app(Config {
        auth: anonymous_forge_auth(api_url, Duration::ZERO, Duration::ZERO, true),
        ..config(root, api_url)
    })
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
        max_concurrent_transfers: 128,
        ..config(root, api_url)
    })
}

// A ceiling on forge lookups, which is what stops a caller rotating tokens from
// spending this server's standing with the forge.
pub fn app_with_lookup_budget(
    root: &tempfile::TempDir,
    api_url: &str,
    lookup_budget: u32,
) -> Router {
    lfsx_server::app(Config {
        auth: Auth::Forge {
            provider: Provider::Github,
            api_url: api_url.to_owned(),
            cache_ttl: Duration::from_secs(60),
            rejection_ttl: Duration::from_secs(10),
            lookup_budget: Some(lookup_budget),
            anonymous_read: false,
            github_app: None,
        },
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
