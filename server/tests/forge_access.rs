mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::Response;
use common::{anonymous_forge_auth, batch_at, config, forge, read_json};
use lfsx_server::auth::Namespaces;
use lfsx_server::config::{Auth, Config, Dashboard, Forge as NamedForge, Provider};
use lfsx_server::namespace::Namespace;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Served {
    root: tempfile::TempDir,
    _pages: tempfile::TempDir,
    app: Router,
}

async fn served() -> Served {
    let root = tempfile::tempdir().unwrap();
    let pages = tempfile::tempdir().unwrap();
    std::fs::write(pages.path().join("index.html"), "<lfsx-root></lfsx-root>").unwrap();
    let (primary_url, _primary) = forge().await;
    let (work_url, _work) = forge().await;
    let app = lfsx_server::app(Config {
        dashboard: Some(Dashboard {
            dir: pages.path().to_path_buf(),
            admins: Some(Namespace::new("FerrLabs", "Infra").unwrap()),
        }),
        forges: vec![NamedForge {
            name: "work".into(),
            auth: Auth::Forge {
                provider: Provider::Github,
                api_url: work_url,
                cache_ttl: Duration::ZERO,
                rejection_ttl: Duration::ZERO,
                lookup_budget: None,
                github_app: None,
                anonymous_read: false,
                restricted: Namespaces::parse("LFSX_FORGE_WORK_RESTRICTED", None),
                allowed: None,
            },
        }],
        auth: anonymous_forge_auth(&primary_url, Duration::ZERO, Duration::ZERO, false),
        ..config(&root, &primary_url)
    });

    Served {
        root,
        _pages: pages,
        app,
    }
}

async fn access(app: Router, method: Method, query: &str, body: Option<Value>) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/-/api/access{query}"))
        .header("authorization", "Bearer admin");
    let body = match body {
        Some(body) => {
            request = request.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };

    app.oneshot(request.body(body).unwrap()).await.unwrap()
}

fn only_partner() -> Value {
    json!({ "anonymous_read": false, "restricted": [], "allowed": ["Partner/*"] })
}

#[tokio::test]
async fn a_named_forge_takes_its_own_access_without_touching_the_main_one() {
    let served = served().await;

    let saved = access(
        served.app.clone(),
        Method::PUT,
        "?forge=work",
        Some(only_partner()),
    )
    .await;

    assert_eq!(saved.status(), StatusCode::OK);
    assert_eq!(read_json(saved).await["source"], "dashboard");
    assert!(served.root.path().join(".lfsx/access-work.json").exists());
    assert!(!served.root.path().join(".lfsx/access.json").exists());

    assert_eq!(
        batch_at(
            served.app.clone(),
            "/-/work/FerrLabs/LFSX",
            "writer",
            "upload"
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        batch_at(
            served.app.clone(),
            "/-/work/Partner/Assets",
            "writer",
            "upload"
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        batch_at(served.app.clone(), "/FerrLabs/LFSX", "writer", "upload").await,
        StatusCode::OK,
        "the main forge keeps serving what its environment allows"
    );

    let main = read_json(access(served.app, Method::GET, "", None).await).await;
    assert_eq!(main["source"], "environment");
    assert_eq!(main["allowed"], Value::Null);
}

#[tokio::test]
async fn resetting_a_named_forge_brings_back_its_environment() {
    let served = served().await;
    access(
        served.app.clone(),
        Method::PUT,
        "?forge=work",
        Some(only_partner()),
    )
    .await;

    let reset = access(served.app.clone(), Method::DELETE, "?forge=work", None).await;

    assert_eq!(reset.status(), StatusCode::OK);
    assert_eq!(read_json(reset).await["source"], "environment");
    assert!(!served.root.path().join(".lfsx/access-work.json").exists());
    assert_eq!(
        batch_at(served.app, "/-/work/FerrLabs/LFSX", "writer", "upload").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_forge_nobody_configured_has_no_access_to_change() {
    let served = served().await;

    for method in [Method::GET, Method::DELETE] {
        let response = access(served.app.clone(), method.clone(), "?forge=elsewhere", None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method}");
    }
    let response = access(
        served.app.clone(),
        Method::PUT,
        "?forge=elsewhere",
        Some(only_partner()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(!served.root.path().join(".lfsx").exists());
}

#[tokio::test]
async fn a_server_starting_with_saved_forge_access_uses_it() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(".lfsx")).unwrap();
    std::fs::write(
        root.path().join(".lfsx/access-work.json"),
        only_partner().to_string(),
    )
    .unwrap();
    let pages = tempfile::tempdir().unwrap();
    let (primary_url, _primary) = forge().await;
    let (work_url, _work) = forge().await;
    let app = lfsx_server::app(Config {
        dashboard: Some(Dashboard {
            dir: pages.path().to_path_buf(),
            admins: Some(Namespace::new("FerrLabs", "Infra").unwrap()),
        }),
        forges: vec![NamedForge {
            name: "work".into(),
            auth: anonymous_forge_auth(&work_url, Duration::ZERO, Duration::ZERO, false),
        }],
        auth: anonymous_forge_auth(&primary_url, Duration::ZERO, Duration::ZERO, false),
        ..config(&root, &primary_url)
    });

    let mut status = StatusCode::OK;
    for _ in 0..50 {
        status = batch_at(app.clone(), "/-/work/FerrLabs/LFSX", "writer", "upload").await;
        if status == StatusCode::NOT_FOUND {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(status, StatusCode::NOT_FOUND);
}
