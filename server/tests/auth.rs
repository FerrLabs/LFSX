mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{app, app_with_rejection_ttl, batch, credentials, forge, put};
use serde_json::json;
use sha2::{Digest, Sha256};
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
        StatusCode::SERVICE_UNAVAILABLE,
        "a throttled forge is not a broken one, and 502 invites the immediate retry that spends          another request on the same exhausted quota"
    );
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("60"),
        "the forge said when to come back, so the answer has to carry it or the client guesses"
    );
}

// The forge sends the duration itself for a secondary limit, and that is what
// the client should be told rather than this server's fallback.
#[tokio::test]
async fn the_wait_the_forge_asked_for_is_the_one_passed_on() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let response = put(
        app(&root, &api_url, Duration::from_secs(60)),
        Some("throttled"),
        b"asset",
    )
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("42")
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

// Cloning a public repository pulls its LFS objects with no credentials at all,
// everywhere else. A public project pointed at LFSX used to fail every anonymous
// clone on a 401: CI without a token, a contributor with no credential helper,
// anyone who just wants to read.
fn oid_of(payload: &[u8]) -> String {
    hex::encode(Sha256::digest(payload))
}

async fn download_from(app: Router, repo: &str, oid: &str, token: Option<&str>) -> StatusCode {
    let mut request = Request::builder().uri(format!("/FerrLabs/{repo}/objects/{oid}"));
    if let Some(token) = token {
        request = request.header("authorization", credentials(token));
    }

    app.oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_public_repository_can_be_read_without_credentials() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let payload = b"an asset in a public project".to_vec();
    let oid = oid_of(&payload);

    let request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/Public/objects/{oid}"))
        .header("authorization", credentials("writer"))
        .header("content-length", payload.len())
        .body(Body::from(payload.clone()))
        .unwrap();
    assert_eq!(
        common::app_reading_anonymously(&root, &api_url)
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    assert_eq!(
        download_from(
            common::app_reading_anonymously(&root, &api_url),
            "Public",
            &oid,
            None
        )
        .await,
        StatusCode::OK
    );
}

// A private repository must answer 401 with the challenge, not 403. A 403 tells
// git-lfs the answer will not change, so it stops asking the credential helper
// and a user who does have access can never get in.
#[tokio::test]
async fn a_private_repository_still_challenges_an_anonymous_caller() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let request = Request::builder()
        .uri(format!("/FerrLabs/Private/objects/{}", "a".repeat(64)))
        .body(Body::empty())
        .unwrap();
    let response = common::app_reading_anonymously(&root, &api_url)
        .oneshot(request)
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        response.headers().contains_key("www-authenticate"),
        "without the challenge git-lfs never asks for a token"
    );
    assert!(response.headers().contains_key("lfs-authenticate"));
}

#[tokio::test]
async fn anonymous_read_does_not_mean_anonymous_write() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let payload = b"an asset nobody signed for".to_vec();

    let request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/Public/objects/{}", oid_of(&payload)))
        .header("content-length", payload.len())
        .body(Body::from(payload))
        .unwrap();

    assert_eq!(
        common::app_reading_anonymously(&root, &api_url)
            .oneshot(request)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN,
        "a public repository grants reading, and nothing else"
    );
}

// The two resolutions are cached under different keys, so one can never be served
// in place of the other. Reading anonymously first must not let a token inherit
// read-only access, nor the reverse.
#[tokio::test]
async fn an_anonymous_resolution_is_never_handed_to_a_token() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let payload = b"an asset both callers want".to_vec();
    let oid = oid_of(&payload);

    let app = || {
        lfsx_server::app(lfsx_server::config::Config {
            auth: common::anonymous_forge_auth(
                &api_url,
                Duration::from_secs(60),
                Duration::from_secs(60),
                true,
            ),
            ..common::config(&root, &api_url)
        })
    };

    // Anonymous first, so the cache holds a read-only decision for this namespace.
    assert_eq!(
        download_from(app(), "Public", &oid, None).await,
        StatusCode::NOT_FOUND,
        "readable, and the object is not there yet"
    );

    let request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/Public/objects/{oid}"))
        .header("authorization", credentials("writer"))
        .header("content-length", payload.len())
        .body(Body::from(payload))
        .unwrap();

    assert_eq!(
        app().oneshot(request).await.unwrap().status(),
        StatusCode::OK,
        "a writer must not inherit the anonymous caller's read-only decision"
    );
}

// The switch, tested on the case that separates the two settings: a public
// repository, where anonymous read would otherwise be granted. Without this the
// option could do nothing and every test would still pass, since a private
// repository is refused either way.
#[tokio::test]
async fn the_option_actually_closes_anonymous_read() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, forge) = forge().await;

    let closed = lfsx_server::app(lfsx_server::config::Config {
        auth: common::anonymous_forge_auth(&api_url, Duration::ZERO, Duration::ZERO, false),
        ..common::config(&root, &api_url)
    });

    let before = forge.calls.load(Ordering::SeqCst);
    assert_eq!(
        download_from(closed, "Public", &"a".repeat(64), None).await,
        StatusCode::UNAUTHORIZED,
        "with the option off a public repository is no more readable than a private one"
    );
    assert_eq!(
        forge.calls.load(Ordering::SeqCst),
        before,
        "and the forge is not even asked: refusing outright is the old behaviour"
    );
}
