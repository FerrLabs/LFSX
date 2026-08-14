use axum::Router;
use axum::body::Body;
use std::time::Duration;

use axum::http::{Request, StatusCode};
use lfsx_server::config::{Auth, Config};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

fn app(root: &tempfile::TempDir) -> Router {
    app_with_grace(root, std::time::Duration::from_secs(14 * 24 * 60 * 60))
}

fn app_with_grace(root: &tempfile::TempDir, gc_grace: std::time::Duration) -> Router {
    lfsx_server::app(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        storage_root: root.path().to_path_buf(),
        public_url: "https://lfs.example".into(),
        action_lifetime: 1800,
        gc_grace,
        auth: Auth::Disabled,
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

async fn retain(app: Router, oids: &[&str], dry_run: bool) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({ "oids": oids, "dry_run": dry_run });
    let request = Request::builder()
        .method("POST")
        .uri("/FerrLabs/Demo/objects/retain")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn is_stored(app: Router, oid: &str) -> bool {
    let request = Request::builder()
        .uri(format!("/FerrLabs/Demo/objects/{oid}"))
        .body(Body::empty())
        .unwrap();

    app.oneshot(request).await.unwrap().status() == StatusCode::OK
}

#[tokio::test]
async fn an_object_nothing_references_any_more_is_swept() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"an asset a rewritten history left behind".repeat(10);
    let oid = oid_of(&payload);
    let app = app_with_grace(&root, Duration::ZERO);

    assert_eq!(put(app.clone(), &oid, &payload).await, StatusCode::OK);
    let (status, report) = retain(app.clone(), &[], false).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["swept"], 1);
    assert_eq!(report["bytes"], payload.len());
    assert!(!is_stored(app, &oid).await, "the object survived the sweep");
}

#[tokio::test]
async fn an_object_uploaded_but_not_yet_referenced_survives() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"pushed before the commit that references it".to_vec();
    let oid = oid_of(&payload);
    let app = app_with_grace(&root, Duration::from_secs(3600));

    assert_eq!(put(app.clone(), &oid, &payload).await, StatusCode::OK);
    let (_, report) = retain(app.clone(), &[], false).await;

    assert_eq!(report["swept"], 0);
    assert_eq!(report["within_grace"], 1);
    assert!(
        is_stored(app, &oid).await,
        "an object still inside the grace period was collected mid-push"
    );
}

#[tokio::test]
async fn a_referenced_object_is_kept_however_old_it_is() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"still pointed at by a commit".to_vec();
    let oid = oid_of(&payload);
    let app = app_with_grace(&root, Duration::ZERO);

    assert_eq!(put(app.clone(), &oid, &payload).await, StatusCode::OK);
    let (_, report) = retain(app.clone(), &[&oid], false).await;

    assert_eq!(report["swept"], 0);
    assert!(is_stored(app, &oid).await);
}

#[tokio::test]
async fn a_dry_run_reports_what_it_would_free_and_frees_nothing() {
    let root = tempfile::tempdir().unwrap();
    let first = b"the first orphan".to_vec();
    let second = b"the second orphan".to_vec();
    let app = app_with_grace(&root, Duration::ZERO);

    put(app.clone(), &oid_of(&first), &first).await;
    put(app.clone(), &oid_of(&second), &second).await;

    let (_, report) = retain(app.clone(), &[], true).await;

    assert_eq!(report["swept"], 2);
    assert_eq!(report["bytes"], first.len() + second.len());
    assert_eq!(report["dry_run"], true);
    assert!(is_stored(app.clone(), &oid_of(&first)).await);
    assert!(is_stored(app, &oid_of(&second)).await);
}

#[tokio::test]
async fn a_transfer_in_flight_is_not_swept_from_under_the_client() {
    let root = tempfile::tempdir().unwrap();
    let oid = oid_of(b"whatever it will hash to");
    let staging = root
        .path()
        .join("FerrLabs/Demo")
        .join(&oid[0..2])
        .join(&oid[2..4]);
    std::fs::create_dir_all(&staging).unwrap();
    let staged = staging.join(format!("{oid}.0.part"));
    std::fs::write(&staged, b"half an upload").unwrap();

    let (_, report) = retain(app_with_grace(&root, Duration::ZERO), &[], false).await;

    assert_eq!(report["swept"], 0);
    assert!(staged.exists(), "a staging file was collected mid-upload");
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
