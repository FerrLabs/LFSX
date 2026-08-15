mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{app_capped, credentials, forge, put, read_json};
use serde_json::json;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const LIMIT: u64 = 4096;

async fn negotiate(app: Router, objects: serde_json::Value) -> serde_json::Value {
    let request = Request::builder()
        .method("POST")
        .uri("/FerrLabs/LFSX/objects/batch")
        .header("content-type", "application/vnd.git-lfs+json")
        .header("authorization", credentials("writer"))
        .body(Body::from(
            json!({ "operation": "upload", "objects": objects }).to_string(),
        ))
        .unwrap();

    read_json(app.oneshot(request).await.unwrap()).await
}

fn object(payload: &[u8]) -> serde_json::Value {
    json!({ "oid": hex::encode(Sha256::digest(payload)), "size": payload.len() })
}

#[tokio::test]
async fn an_object_over_the_limit_is_refused_before_a_single_byte_moves() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app_capped(&root, &api_url, LIMIT);
    let oversized = vec![0u8; 8192];

    let batch = negotiate(app, json!([object(&oversized)])).await;

    assert_eq!(batch["objects"][0]["error"]["code"], 413);
    assert!(
        batch["objects"][0]["actions"].is_null(),
        "an href here is an invitation to spend an hour uploading something that will be \
         thrown away: {batch}"
    );
}

#[tokio::test]
async fn one_oversized_object_does_not_sink_the_rest_of_the_push() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app_capped(&root, &api_url, LIMIT);

    let batch = negotiate(
        app,
        json!([object(&vec![0u8; 8192]), object(b"a small asset")]),
    )
    .await;

    assert_eq!(batch["objects"][0]["error"]["code"], 413);
    assert!(
        !batch["objects"][1]["actions"]["upload"]["href"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "the limit refuses an object, not the commit it arrived with: {batch}"
    );
}

#[tokio::test]
async fn an_upload_declaring_more_than_the_limit_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app_capped(&root, &api_url, LIMIT);
    let oversized = vec![0u8; 8192];

    let response = put(app.clone(), Some("writer"), &oversized).await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        negotiate(app, json!([object(&oversized)])).await["objects"][0]["error"]["code"],
        413,
        "nothing was stored, so the object is still unknown to the server"
    );
}

#[tokio::test]
async fn a_body_that_lies_about_its_size_is_still_cut_off() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app_capped(&root, &api_url, LIMIT);
    let oversized = vec![0u8; 8192];
    let oid = hex::encode(Sha256::digest(&oversized));

    let request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/LFSX/objects/{oid}"))
        .header("authorization", credentials("writer"))
        .header("content-length", 1024)
        .body(Body::from(oversized))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "the declared size is a claim by the client, so the ceiling has to hold against a \
         body that ignores it"
    );
}

#[tokio::test]
async fn lowering_the_limit_does_not_strand_what_is_already_stored() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let payload = vec![7u8; 8192];
    let oid = hex::encode(Sha256::digest(&payload));

    let generous = app_capped(&root, &api_url, 16_384);
    assert_eq!(
        put(generous, Some("writer"), &payload).await.status(),
        StatusCode::OK
    );

    let request = Request::builder()
        .uri(format!("/FerrLabs/LFSX/objects/{oid}"))
        .header("authorization", credentials("reader"))
        .body(Body::empty())
        .unwrap();
    let response = app_capped(&root, &api_url, LIMIT)
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the limit governs what may arrive, not what a repository can still check out"
    );
}
