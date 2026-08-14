use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use lfsx_server::config::Config;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

fn app(root: &tempfile::TempDir) -> Router {
    lfsx_server::app(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        storage_root: root.path().to_path_buf(),
        public_url: "https://lfs.example".into(),
        action_lifetime: 1800,
    })
}

fn oid_of(payload: &[u8]) -> String {
    hex::encode(Sha256::digest(payload))
}

async fn put(app: Router, oid: &str, payload: &[u8]) -> StatusCode {
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/Demo/objects/{oid}"))
        .header("content-length", payload.len())
        .body(Body::from(payload.to_vec()))
        .unwrap();

    app.oneshot(request).await.unwrap().status()
}

async fn batch(app: Router, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/FerrLabs/Demo/objects/batch")
        .header("content-type", "application/vnd.git-lfs+json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn stored_object_comes_back_byte_for_byte() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"some genuinely heavy assets".repeat(1000);
    let oid = oid_of(&payload);

    assert_eq!(put(app(&root), &oid, &payload).await, StatusCode::OK);

    let request = Request::builder()
        .uri(format!("/FerrLabs/Demo/objects/{oid}"))
        .body(Body::empty())
        .unwrap();
    let response = app(&root).oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(bytes.as_ref(), payload.as_slice());
}

#[tokio::test]
async fn content_that_does_not_hash_to_the_declared_oid_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let lie = oid_of(b"what the client claims to send");

    let status = put(app(&root), &lie, b"what it actually sends").await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let stored = walkdir(root.path());
    assert!(stored.is_empty(), "a corrupt object was kept");
}

#[tokio::test]
async fn a_truncated_upload_is_rejected_and_leaves_nothing_behind() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"the complete payload".to_vec();
    let oid = oid_of(&payload);

    let request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/Demo/objects/{oid}"))
        .header("content-length", payload.len() + 10)
        .body(Body::from(payload))
        .unwrap();

    let status = app(&root).oneshot(request).await.unwrap().status();

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(walkdir(root.path()).is_empty());
}

#[tokio::test]
async fn an_oid_that_is_not_a_sha256_digest_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let status = put(app(&root), "../../../etc/passwd", b"x").await;
    assert_ne!(status, StatusCode::OK);
}

#[tokio::test]
async fn downloading_an_unknown_object_reports_it_per_object() {
    let root = tempfile::tempdir().unwrap();
    let (status, body) = batch(
        app(&root),
        serde_json::json!({
            "operation": "download",
            "transfers": ["basic"],
            "objects": [{ "oid": oid_of(b"never pushed"), "size": 12 }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["objects"][0]["error"]["code"], 404);
    assert!(body["objects"][0]["actions"].is_null());
}

#[tokio::test]
async fn an_object_already_stored_is_not_asked_for_again() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"already here".to_vec();
    let oid = oid_of(&payload);
    put(app(&root), &oid, &payload).await;

    let (_, body) = batch(
        app(&root),
        serde_json::json!({
            "operation": "upload",
            "transfers": ["basic"],
            "objects": [{ "oid": oid, "size": payload.len() }]
        }),
    )
    .await;

    assert!(
        body["objects"][0]["actions"].is_null(),
        "the client would re-upload an object already stored"
    );
    assert!(body["objects"][0]["error"].is_null());
}

#[tokio::test]
async fn batch_never_claims_the_transfer_is_pre_authenticated() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"payload".to_vec();
    let oid = oid_of(&payload);
    put(app(&root), &oid, &payload).await;

    let (_, body) = batch(
        app(&root),
        serde_json::json!({
            "operation": "download",
            "transfers": ["basic"],
            "objects": [{ "oid": oid, "size": payload.len() }]
        }),
    )
    .await;

    assert!(
        body["objects"][0]["authenticated"].is_null(),
        "advertising authenticated without supplying a header makes the client send \
         transfers with no credentials, and it loops on 401"
    );
}

fn walkdir(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }

    found
}
