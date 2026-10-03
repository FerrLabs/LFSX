mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::Response;
use common::{Forge, anonymous_forge_auth, config, credentials, forge, read_json};
use lfsx_server::auth::Namespaces;
use lfsx_server::config::{Auth, Config, Forge as NamedForge, Provider};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

struct Served {
    root: tempfile::TempDir,
    app: Router,
    primary: Arc<Forge>,
    work: Arc<Forge>,
}

async fn served(work_allowed: Option<&str>) -> Served {
    let root = tempfile::tempdir().unwrap();
    let (primary_url, primary) = forge().await;
    let (work_url, work) = forge().await;
    let app = lfsx_server::app(Config {
        forges: vec![NamedForge {
            name: "work".into(),
            auth: Auth::Forge {
                provider: Provider::Github,
                api_url: work_url.clone(),
                cache_ttl: Duration::ZERO,
                rejection_ttl: Duration::ZERO,
                lookup_budget: None,
                github_app: None,
                anonymous_read: false,
                restricted: Namespaces::parse("LFSX_FORGE_WORK_RESTRICTED", None),
                allowed: work_allowed
                    .map(|entries| Namespaces::parse("LFSX_FORGE_WORK_ALLOWED", Some(entries))),
            },
        }],
        auth: anonymous_forge_auth(&primary_url, Duration::ZERO, Duration::ZERO, false),
        ..config(&root, &primary_url)
    });

    Served {
        root,
        app,
        primary,
        work,
    }
}

async fn send(app: Router, method: Method, path: &str, body: Option<Value>) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", credentials("writer"));
    let body = match body {
        Some(body) => {
            request = request.header("content-type", "application/vnd.git-lfs+json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };

    app.oneshot(request.body(body).unwrap()).await.unwrap()
}

async fn upload(app: Router, prefix: &str, payload: &[u8]) -> StatusCode {
    let oid = hex::encode(Sha256::digest(payload));
    let request = Request::builder()
        .method(Method::PUT)
        .uri(format!("{prefix}/FerrLabs/LFSX/objects/{oid}"))
        .header("authorization", credentials("writer"))
        .header("content-length", payload.len())
        .body(Body::from(payload.to_vec()))
        .unwrap();

    app.oneshot(request).await.unwrap().status()
}

async fn stats(app: Router, prefix: &str) -> Value {
    read_json(
        send(
            app,
            Method::GET,
            &format!("{prefix}/FerrLabs/LFSX/objects/stats"),
            None,
        )
        .await,
    )
    .await
}

#[tokio::test]
async fn the_same_repository_on_two_forges_holds_its_own_objects() {
    let served = served(None).await;
    let payload = b"an asset pushed to the work forge only".repeat(4);

    assert!(
        upload(served.app.clone(), "/-/work", &payload)
            .await
            .is_success()
    );

    assert_eq!(stats(served.app.clone(), "/-/work").await["objects"], 1);
    assert_eq!(stats(served.app.clone(), "").await["objects"], 0);
    assert!(served.root.path().join("work~FerrLabs/LFSX").is_dir());
    assert!(!served.root.path().join("FerrLabs").exists());
}

#[tokio::test]
async fn a_request_under_a_forge_is_checked_with_that_forge() {
    let served = served(None).await;

    let status = common::batch_at(
        served.app.clone(),
        "/-/work/FerrLabs/LFSX",
        "writer",
        "upload",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(served.work.calls.load(Ordering::SeqCst) > 0);
    assert_eq!(served.primary.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn transfer_links_keep_the_forge_prefix() {
    let served = served(None).await;
    let payload = b"an object to download from the work forge".repeat(4);
    upload(served.app.clone(), "/-/work", &payload).await;
    let oid = hex::encode(Sha256::digest(&payload));

    let body = read_json(
        send(
            served.app,
            Method::POST,
            "/-/work/FerrLabs/LFSX/objects/batch",
            Some(json!({
                "operation": "download",
                "objects": [{ "oid": oid, "size": payload.len() }],
            })),
        )
        .await,
    )
    .await;

    assert_eq!(
        body["objects"][0]["actions"]["download"]["href"],
        format!("https://lfs.example/-/work/FerrLabs/LFSX/objects/{oid}")
    );
}

#[tokio::test]
async fn a_forge_nobody_configured_is_not_served() {
    let served = served(None).await;

    let status =
        common::batch_at(served.app, "/-/elsewhere/FerrLabs/LFSX", "writer", "upload").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn each_forge_applies_its_own_allow_list() {
    let served = served(Some("Partner/*")).await;

    assert_eq!(
        common::batch_at(
            served.app.clone(),
            "/-/work/FerrLabs/LFSX",
            "writer",
            "upload"
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        common::batch_at(
            served.app.clone(),
            "/-/work/Partner/Assets",
            "writer",
            "upload"
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        common::batch_at(served.app, "/FerrLabs/LFSX", "writer", "upload").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn locks_on_two_forges_do_not_meet() {
    let served = served(None).await;
    let take = |prefix: &'static str| {
        let app = served.app.clone();
        async move {
            send(
                app,
                Method::POST,
                &format!("{prefix}/FerrLabs/LFSX/locks"),
                Some(json!({ "path": "Arena.unity" })),
            )
            .await
            .status()
        }
    };

    assert_eq!(take("").await, StatusCode::CREATED);
    assert_eq!(take("/-/work").await, StatusCode::CREATED);
    assert_eq!(take("/-/work").await, StatusCode::CONFLICT);
}
