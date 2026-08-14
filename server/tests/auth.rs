mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{app, app_with_rejection_ttl, batch, credentials, forge, put};
use serde_json::json;
use tower::ServiceExt;

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
    let app = app(&root, &api_url, Duration::from_secs(1));

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

    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert_eq!(
        put(app, Some("writer"), b"third").await.status(),
        StatusCode::FORBIDDEN,
        "the revoked permission should be picked up once the entry expires"
    );
    assert_eq!(forge.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_read_only_token_cannot_collect_garbage() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let request = Request::builder()
        .method("POST")
        .uri("/FerrLabs/LFSX/objects/retain")
        .header("content-type", "application/json")
        .header("authorization", credentials("reader"))
        .body(Body::from(json!({ "oids": [] }).to_string()))
        .unwrap();

    let response = app(&root, &api_url, Duration::from_secs(60))
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_plain_forbidden_from_the_forge_stays_forbidden() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let response = put(
        app(&root, &api_url, Duration::from_secs(60)),
        Some("outsider"),
        b"asset",
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_rate_limited_forge_is_an_outage_not_a_denial() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let response = put(
        app(&root, &api_url, Duration::from_secs(60)),
        Some("rate-limited"),
        b"asset",
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_GATEWAY,
        "a rate-limited forge must read as an outage the client retries, not as denied access"
    );
}

#[tokio::test]
async fn a_rejected_token_stops_reaching_the_forge_then_recovers() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, forge) = forge().await;
    let app = app_with_rejection_ttl(
        &root,
        &api_url,
        Duration::from_secs(60),
        Duration::from_secs(1),
    );

    for _ in 0..3 {
        assert_eq!(
            put(app.clone(), Some("stranger"), b"asset").await.status(),
            StatusCode::FORBIDDEN
        );
    }

    assert_eq!(
        forge.calls.load(Ordering::SeqCst),
        1,
        "a token the forge already refused must not cost another API call on every retry"
    );

    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert_eq!(
        put(app, Some("stranger"), b"asset").await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        forge.calls.load(Ordering::SeqCst),
        2,
        "once the rejection lapses the forge is asked again, so newly granted access is picked up"
    );
}
