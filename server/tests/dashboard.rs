mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{app, credentials, forge, put, read_json};
use tower::ServiceExt;

async fn get(app: Router, path: &str, token: Option<&str>) -> axum::response::Response {
    let mut request = Request::builder().uri(path);
    if let Some(token) = token {
        request = request.header("authorization", credentials(token));
    }

    app.oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn stats_report_what_this_repository_holds() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let payload = b"an asset that occupies space".repeat(10);
    put(app.clone(), Some("writer"), &payload).await;

    let body = read_json(get(app, "/FerrLabs/LFSX/objects/stats", Some("reader")).await).await;

    assert_eq!(body["objects"], 1);
    assert_eq!(body["bytes"], payload.len());
    assert_eq!(body["locks"], 0);
}

#[tokio::test]
async fn stats_are_scoped_to_the_repository_asked_about() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    put(app.clone(), Some("writer"), b"an asset of FerrLabs/LFSX").await;

    let elsewhere =
        read_json(get(app, "/FerrLabs/Other/objects/stats", Some("writer")).await).await;

    assert_eq!(
        elsewhere["objects"], 0,
        "one repository must not report another's usage"
    );
}

#[tokio::test]
async fn the_page_is_served_to_someone_who_can_read_the_repository() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    put(app.clone(), Some("writer"), b"an asset").await;

    let response = get(app, "/FerrLabs/LFSX", Some("reader")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    let page = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(page.contains("FerrLabs/LFSX"));
    assert!(
        page.contains("read</dd>"),
        "a reader is told they cannot write"
    );
}

#[tokio::test]
async fn the_page_needs_credentials_like_everything_else() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let response = get(
        app(&root, &api_url, Duration::from_secs(60)),
        "/FerrLabs/LFSX",
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        response.headers().contains_key("www-authenticate"),
        "the browser's own credential prompt is the login screen"
    );
}

#[tokio::test]
async fn a_stranger_is_refused_the_page() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;

    let response = get(
        app(&root, &api_url, Duration::from_secs(60)),
        "/FerrLabs/LFSX",
        Some("stranger"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
