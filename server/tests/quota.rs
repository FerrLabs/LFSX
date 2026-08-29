mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{credentials, forge, put, read_json};
use lfsx_server::config::Config;
use serde_json::json;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const QUOTA: u64 = 1024;
const HELD: usize = 900;

fn app(root: &tempfile::TempDir, api_url: &str) -> Router {
    app_collecting_after(root, api_url, Duration::from_secs(14 * 24 * 60 * 60))
}

fn app_collecting_after(root: &tempfile::TempDir, api_url: &str, gc_grace: Duration) -> Router {
    lfsx_server::app(Config {
        repo_quota: Some(QUOTA),
        gc_grace,
        ..common::config(root, api_url)
    })
}

fn object(payload: &[u8]) -> serde_json::Value {
    json!({ "oid": hex::encode(Sha256::digest(payload)), "size": payload.len() })
}

async fn negotiate(app: Router, operation: &str, payload: &[u8]) -> serde_json::Value {
    let request = Request::builder()
        .method("POST")
        .uri("/FerrLabs/LFSX/objects/batch")
        .header("content-type", "application/vnd.git-lfs+json")
        .header("authorization", credentials("writer"))
        .body(Body::from(
            json!({ "operation": operation, "objects": [object(payload)] }).to_string(),
        ))
        .unwrap();

    read_json(app.oneshot(request).await.unwrap()).await
}

async fn fill(app: Router) -> Vec<u8> {
    let held = vec![7u8; HELD];
    assert_eq!(
        put(app, Some("writer"), &held).await.status(),
        StatusCode::OK
    );
    held
}

#[tokio::test]
async fn an_object_that_would_not_fit_is_refused_at_negotiation() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    fill(app.clone()).await;

    let batch = negotiate(
        app,
        "upload",
        b"the two hundred bytes that do not fit"
            .repeat(6)
            .as_slice(),
    )
    .await;

    assert_eq!(batch["objects"][0]["error"]["code"], 507);
    assert!(
        batch["objects"][0]["actions"].is_null(),
        "handing out an upload link here promises room the repository does not have: {batch}"
    );
}

#[tokio::test]
async fn an_object_that_still_fits_is_accepted() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    fill(app.clone()).await;

    let batch = negotiate(
        app,
        "upload",
        b"under the remaining hundred and twenty four",
    )
    .await;

    assert!(
        !batch["objects"][0]["actions"]["upload"]["href"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "the budget is a ceiling on what the repository holds, not a round number to stay under: \
         {batch}"
    );
}

#[tokio::test]
async fn a_full_repository_still_serves_what_it_holds() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    let held = fill(app.clone()).await;
    let oid = hex::encode(Sha256::digest(&held));

    let batch = negotiate(app.clone(), "download", &held).await;
    let request = Request::builder()
        .uri(format!("/FerrLabs/LFSX/objects/{oid}"))
        .header("authorization", credentials("reader"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();

    assert!(
        batch["objects"][0]["error"].is_null(),
        "a quota governs what may arrive, not what a working copy can check out: {batch}"
    );
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_client_that_skips_negotiation_is_refused_too() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    fill(app.clone()).await;

    let refused = put(app, Some("writer"), &[9u8; 200]).await;

    assert_eq!(refused.status(), StatusCode::INSUFFICIENT_STORAGE);
}

#[tokio::test]
async fn an_object_already_held_is_never_refused_for_want_of_room() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    let held = fill(app.clone()).await;

    let batch = negotiate(app, "upload", &held).await;

    assert!(
        batch["objects"][0]["error"].is_null(),
        "re-announcing an object the repository already holds asks for no new room: {batch}"
    );
    assert!(
        batch["objects"][0]["actions"].is_null(),
        "and there is nothing to upload: {batch}"
    );
}

#[tokio::test]
async fn a_body_that_declares_no_size_is_cut_off_at_the_budget() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    let held = fill(app.clone()).await;
    let arriving = vec![9u8; 200];

    let request = Request::builder()
        .method("PUT")
        .uri(format!(
            "/FerrLabs/LFSX/objects/{}",
            hex::encode(Sha256::digest(&arriving))
        ))
        .header("authorization", credentials("writer"))
        .body(Body::from(arriving))
        .unwrap();
    let refused = app.clone().oneshot(request).await.unwrap();

    assert_eq!(
        refused.status(),
        StatusCode::INSUFFICIENT_STORAGE,
        "a client that skips negotiation may skip declaring a size too, and a budget checked \
         once against a number the client chose is not a budget"
    );
    assert_eq!(
        negotiate(app, "download", &held).await["objects"][0]["error"],
        serde_json::Value::Null,
        "and the repository is left exactly as it was"
    );
}

#[tokio::test]
async fn a_full_repository_can_still_be_sent_an_object_it_already_holds() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    let held = fill(app.clone()).await;

    assert_eq!(
        put(app, Some("writer"), &held).await.status(),
        StatusCode::OK,
        "a retried transfer of an object already stored asks for no new room, at this gate as \
         much as at negotiation"
    );
}

#[tokio::test]
async fn collecting_makes_room_the_next_push_can_use() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app_collecting_after(&root, &api_url, Duration::ZERO);
    fill(app.clone()).await;
    let arriving = vec![9u8; 200];

    assert_eq!(
        negotiate(app.clone(), "upload", &arriving).await["objects"][0]["error"]["code"],
        507
    );

    let collected = Request::builder()
        .method("POST")
        .uri("/FerrLabs/LFSX/objects/retain")
        .header("content-type", "application/json")
        .header("authorization", credentials("admin"))
        .body(Body::from(json!({ "oids": [] }).to_string()))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(collected).await.unwrap().status(),
        StatusCode::OK
    );

    assert_eq!(
        put(app, Some("writer"), &arriving).await.status(),
        StatusCode::OK,
        "the budget has to follow the disk within the push that freed it, not a cache expiry later"
    );
}
