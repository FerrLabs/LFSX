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
use lfsx_server::config::{Auth, Config};
use serde_json::json;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

struct Forge {
    calls: AtomicUsize,
    writer_can_push: Mutex<bool>,
}

async fn forge() -> (String, Arc<Forge>) {
    let forge = Arc::new(Forge {
        calls: AtomicUsize::new(0),
        writer_can_push: Mutex::new(true),
    });

    let router = Router::new()
        .route("/repos/{org}/{repo}", get(repository))
        .with_state(forge.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    (format!("http://{address}"), forge)
}

async fn repository(State(forge): State<Arc<Forge>>, headers: HeaderMap) -> Response {
    forge.calls.fetch_add(1, Ordering::SeqCst);

    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();

    match token {
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
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Bad credentials" })),
        )
            .into_response(),
    }
}

fn app(root: &tempfile::TempDir, api_url: &str, cache_ttl: Duration) -> Router {
    lfsx_server::app(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        storage_root: root.path().to_path_buf(),
        public_url: "https://lfs.example".into(),
        action_lifetime: 1800,
        auth: Auth::Github {
            api_url: api_url.to_owned(),
            cache_ttl,
        },
    })
}

fn credentials(token: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("git:{token}")))
}

async fn put(app: Router, token: Option<&str>, payload: &[u8]) -> Response {
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

async fn batch(app: Router, token: &str, operation: &str) -> StatusCode {
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

#[tokio::test]
async fn health_stays_open() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, forge) = forge().await;

    let response = app(&root, &api_url, Duration::from_secs(60))
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(forge.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_request_without_credentials_is_challenged() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let response = put(
        app(&root, &api_url, Duration::from_secs(60)),
        None,
        b"asset",
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        response.headers().contains_key("www-authenticate"),
        "git-lfs needs a challenge to ask the credential helper for a token"
    );
    assert!(response.headers().contains_key("lfs-authenticate"));
}

#[tokio::test]
async fn a_token_the_forge_rejects_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let response = put(
        app(&root, &api_url, Duration::from_secs(60)),
        Some("expired-pat"),
        b"asset",
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_token_without_access_to_the_repository_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let response = put(
        app(&root, &api_url, Duration::from_secs(60)),
        Some("stranger"),
        b"asset",
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_read_only_token_can_download_but_not_upload() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));

    let upload = put(app.clone(), Some("reader"), b"asset").await;

    assert_eq!(upload.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        batch(app.clone(), "reader", "upload").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(batch(app, "reader", "download").await, StatusCode::OK);
}

#[tokio::test]
async fn a_write_token_can_upload() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let response = put(
        app(&root, &api_url, Duration::from_secs(60)),
        Some("writer"),
        b"asset",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn permissions_are_resolved_again_once_the_cache_entry_expires() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_millis(50));

    assert_eq!(
        put(app.clone(), Some("writer"), b"first").await.status(),
        StatusCode::OK
    );

    *forge.writer_can_push.lock().unwrap() = false;

    assert_eq!(
        put(app.clone(), Some("writer"), b"second").await.status(),
        StatusCode::OK,
        "the cached permission should still be in force"
    );
    assert_eq!(
        forge.calls.load(Ordering::SeqCst),
        1,
        "the second upload must not have hit the forge"
    );

    tokio::time::sleep(Duration::from_millis(80)).await;

    assert_eq!(
        put(app, Some("writer"), b"third").await.status(),
        StatusCode::FORBIDDEN,
        "the revoked permission should be picked up once the entry expires"
    );
    assert_eq!(forge.calls.load(Ordering::SeqCst), 2);
}
