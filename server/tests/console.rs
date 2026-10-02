mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::Response;
use common::{batch, config, forge, put, read_json};
use lfsx_server::config::{Auth, Config, Dashboard};
use lfsx_server::namespace::Namespace;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Served {
    root: tempfile::TempDir,
    _pages: tempfile::TempDir,
    app: Router,
}

fn pages() -> tempfile::TempDir {
    let pages = tempfile::tempdir().unwrap();
    std::fs::write(pages.path().join("index.html"), "<lfsx-root></lfsx-root>").unwrap();
    pages
}

fn dashboard(pages: &tempfile::TempDir, admins: Option<Namespace>) -> Option<Dashboard> {
    Some(Dashboard {
        dir: pages.path().to_path_buf(),
        admins,
    })
}

async fn served() -> Served {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let pages = pages();
    let app = lfsx_server::app(Config {
        dashboard: dashboard(&pages, Some(Namespace::new("FerrLabs", "Infra").unwrap())),
        ..config(&root, &api_url)
    });

    Served {
        root,
        _pages: pages,
        app,
    }
}

async fn call(
    app: Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(body) => {
            request = request.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };

    app.oneshot(request.body(body).unwrap()).await.unwrap()
}

async fn overview(app: Router, token: Option<&str>) -> Response {
    call(app, Method::GET, "/-/api/overview", token, None).await
}

async fn save(app: Router, body: Value) -> Response {
    call(app, Method::PUT, "/-/api/access", Some("admin"), Some(body)).await
}

#[tokio::test]
async fn nothing_is_served_under_the_dashboard_unless_it_is_turned_on() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = lfsx_server::app(config(&root, &api_url));

    assert_eq!(
        overview(app.clone(), Some("admin")).await.status(),
        StatusCode::NOT_FOUND
    );
    let page = call(app, Method::GET, "/-/dashboard/", None, None).await;
    assert_eq!(page.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_caller_without_a_token_is_asked_to_sign_in_without_a_browser_prompt() {
    let served = served().await;

    let response = overview(served.app, None).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get("www-authenticate").is_none());
}

#[tokio::test]
async fn only_the_admins_of_the_named_repository_see_the_overview() {
    let served = served().await;

    for token in ["reader", "writer"] {
        let status = overview(served.app.clone(), Some(token)).await.status();
        assert_eq!(status, StatusCode::FORBIDDEN, "{token}");
    }
    assert_eq!(
        overview(served.app.clone(), Some("stranger"))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        overview(served.app, Some("admin")).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_overview_counts_what_was_stored_and_sent() {
    let served = served().await;
    let payload = b"an asset the overview has to count".repeat(8);
    assert!(
        put(served.app.clone(), Some("writer"), &payload)
            .await
            .status()
            .is_success()
    );

    let body = read_json(overview(served.app, Some("admin")).await).await;

    assert_eq!(body["storage"]["kind"], "local");
    assert_eq!(body["storage"]["objects"], 1);
    assert_eq!(body["storage"]["bytes"], payload.len());
    assert_eq!(body["traffic"]["uploaded_bytes"], payload.len());
    assert_eq!(body["traffic"]["server_errors"], 0);
    let sizes = body["object_sizes"].as_array().unwrap();
    assert_eq!(
        sizes
            .iter()
            .map(|bucket| bucket["objects"].as_u64().unwrap())
            .sum::<u64>(),
        1
    );
    assert_eq!(sizes.last().unwrap()["up_to"], Value::Null);
}

#[tokio::test]
async fn any_dashboard_route_falls_back_to_the_application() {
    let served = served().await;

    let response = call(served.app, Method::GET, "/-/dashboard/settings", None, None).await;

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"<lfsx-root></lfsx-root>");
}

#[tokio::test]
async fn access_starts_from_the_environment() {
    let served = served().await;

    let body = read_json(
        call(
            served.app,
            Method::GET,
            "/-/api/access",
            Some("admin"),
            None,
        )
        .await,
    )
    .await;

    assert_eq!(body["editable"], true);
    assert_eq!(body["source"], "environment");
    assert_eq!(body["allowed"], Value::Null);
}

#[tokio::test]
async fn saved_access_applies_at_once_and_is_kept_in_the_store() {
    let served = served().await;
    assert_eq!(
        batch(served.app.clone(), "writer", "upload").await,
        StatusCode::OK
    );

    let response = save(
        served.app.clone(),
        json!({ "anonymous_read": false, "restricted": [], "allowed": ["Elsewhere/*"] }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = read_json(response).await;
    assert_eq!(body["source"], "dashboard");
    assert_eq!(body["allowed"], json!(["elsewhere/*"]));
    assert_eq!(
        batch(served.app.clone(), "writer", "upload").await,
        StatusCode::NOT_FOUND
    );
    let kept: Value = serde_json::from_slice(
        &std::fs::read(served.root.path().join(".lfsx/access.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(kept["allowed"], json!(["Elsewhere/*"]));
}

#[tokio::test]
async fn an_allow_list_without_the_dashboard_repository_keeps_its_admins_in() {
    let served = served().await;
    save(
        served.app.clone(),
        json!({ "anonymous_read": false, "restricted": [], "allowed": ["Elsewhere/*"] }),
    )
    .await;

    assert_eq!(
        overview(served.app, Some("admin")).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn an_unreadable_entry_is_named_and_nothing_is_saved() {
    let served = served().await;

    let response = save(
        served.app.clone(),
        json!({ "anonymous_read": false, "restricted": ["not a repo"], "allowed": ["FerrLabs/*", "nope"] }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = read_json(response).await;
    assert_eq!(body["unreadable"], json!(["not a repo", "nope"]));
    assert!(!served.root.path().join(".lfsx/access.json").exists());
    assert_eq!(batch(served.app, "writer", "upload").await, StatusCode::OK);
}

#[tokio::test]
async fn a_reader_cannot_change_access() {
    let served = served().await;

    let response = call(
        served.app.clone(),
        Method::PUT,
        "/-/api/access",
        Some("writer"),
        Some(json!({ "anonymous_read": true, "restricted": [], "allowed": ["Elsewhere/*"] })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(!served.root.path().join(".lfsx/access.json").exists());
    assert_eq!(batch(served.app, "writer", "upload").await, StatusCode::OK);
}

#[tokio::test]
async fn reset_goes_back_to_the_environment_and_forgets_the_saved_access() {
    let served = served().await;
    save(
        served.app.clone(),
        json!({ "anonymous_read": false, "restricted": [], "allowed": ["Elsewhere/*"] }),
    )
    .await;

    let response = call(
        served.app.clone(),
        Method::DELETE,
        "/-/api/access",
        Some("admin"),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(read_json(response).await["source"], "environment");
    assert!(!served.root.path().join(".lfsx/access.json").exists());
    assert_eq!(batch(served.app, "writer", "upload").await, StatusCode::OK);
}

#[tokio::test]
async fn a_server_starting_on_a_store_with_saved_access_uses_it() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let pages = pages();
    std::fs::create_dir_all(root.path().join(".lfsx")).unwrap();
    std::fs::write(
        root.path().join(".lfsx/access.json"),
        json!({ "anonymous_read": false, "restricted": [], "allowed": ["Elsewhere/*"] })
            .to_string(),
    )
    .unwrap();

    let app = lfsx_server::app(Config {
        dashboard: dashboard(&pages, Some(Namespace::new("FerrLabs", "Infra").unwrap())),
        ..config(&root, &api_url)
    });

    let mut status = StatusCode::OK;
    for _ in 0..50 {
        status = batch(app.clone(), "writer", "upload").await;
        if status == StatusCode::NOT_FOUND {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn with_authentication_disabled_there_is_no_access_to_change() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let pages = pages();
    let app = lfsx_server::app(Config {
        auth: Auth::Disabled,
        dashboard: dashboard(&pages, None),
        ..config(&root, &api_url)
    });

    let shown = read_json(call(app.clone(), Method::GET, "/-/api/access", None, None).await).await;
    assert_eq!(shown["editable"], false);

    let response = call(
        app.clone(),
        Method::PUT,
        "/-/api/access",
        None,
        Some(json!({ "anonymous_read": false, "restricted": [], "allowed": ["Elsewhere/*"] })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = call(app, Method::DELETE, "/-/api/access", None, None).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}
