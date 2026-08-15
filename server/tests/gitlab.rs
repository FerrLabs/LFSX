mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router as AxumRouter};
use common::credentials;
use lfsx_server::config::{Auth, Config, Provider};
use serde_json::json;
use tower::ServiceExt;

async fn gitlab() -> String {
    let router = AxumRouter::new()
        .route("/api/v4/projects/{path}", get(project))
        .route("/api/v4/user", get(user));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    format!("http://{address}/api/v4")
}

fn token_of(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned()
}

async fn project(Path(path): Path<String>, headers: HeaderMap) -> Response {
    if path != "FerrLabs/Blastlands" {
        return (StatusCode::NOT_FOUND, Json(json!({ "message": "404" }))).into_response();
    }

    if token_of(&headers) == "group-developer" {
        return Json(
            json!({ "permissions": { "project_access": null, "group_access": { "access_level": 30 } } }),
        )
        .into_response();
    }

    let level = match token_of(&headers).as_str() {
        "owner" => 50,
        "maintainer" => 40,
        "developer" => 30,
        "reporter" => 20,
        "guest" => 10,
        "stranger" => return (StatusCode::NOT_FOUND, Json(json!({}))).into_response(),
        "throttled" => return (StatusCode::TOO_MANY_REQUESTS, Json(json!({}))).into_response(),
        _ => return (StatusCode::UNAUTHORIZED, Json(json!({}))).into_response(),
    };

    Json(json!({ "permissions": { "project_access": { "access_level": level }, "group_access": null } }))
        .into_response()
}

async fn user(headers: HeaderMap) -> Response {
    match token_of(&headers).as_str() {
        "" => (StatusCode::UNAUTHORIZED, Json(json!({}))).into_response(),
        token => Json(json!({ "username": token })).into_response(),
    }
}

fn app(root: &tempfile::TempDir, api_url: &str) -> Router {
    lfsx_server::app(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        storage_root: root.path().to_path_buf(),
        public_url: Some("https://lfs.example".into()),
        action_lifetime: 1800,
        gc_grace: Duration::ZERO,
        staging_max_age: Duration::from_secs(86400),
        max_object_size: None,
        repo_quota: None,
        auth: Auth::Forge {
            provider: Provider::Gitlab,
            api_url: api_url.to_owned(),
            cache_ttl: Duration::from_secs(60),
            rejection_ttl: Duration::from_secs(60),
        },
    })
}

async fn upload(app: Router, token: &str) -> StatusCode {
    let payload = b"a gitlab-hosted asset";
    let oid = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(payload));
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/Blastlands/objects/{oid}"))
        .header("content-length", payload.len())
        .header("authorization", credentials(token))
        .body(Body::from(payload.to_vec()))
        .unwrap();

    app.oneshot(request).await.unwrap().status()
}

async fn lock(app: Router, token: &str) -> Response {
    let request = Request::builder()
        .method("POST")
        .uri("/FerrLabs/Blastlands/locks")
        .header("content-type", "application/json")
        .header("authorization", credentials(token))
        .body(Body::from(
            json!({ "path": "Assets/Scene.unity" }).to_string(),
        ))
        .unwrap();

    app.oneshot(request).await.unwrap()
}

#[tokio::test]
async fn access_levels_map_onto_the_same_three_permissions() {
    let root = tempfile::tempdir().unwrap();
    let api_url = gitlab().await;

    for (token, expected) in [
        ("developer", StatusCode::OK),
        ("maintainer", StatusCode::OK),
        ("owner", StatusCode::OK),
        ("reporter", StatusCode::FORBIDDEN),
        ("guest", StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            upload(app(&root, &api_url), token).await,
            expected,
            "developer is the level that may push, everything below reads at most ({token})"
        );
    }
}

#[tokio::test]
async fn a_token_gitlab_refuses_is_refused_here() {
    let root = tempfile::tempdir().unwrap();
    let api_url = gitlab().await;

    assert_eq!(
        upload(app(&root, &api_url), "expired").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        upload(app(&root, &api_url), "stranger").await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_throttled_gitlab_is_an_outage_not_a_denial() {
    let root = tempfile::tempdir().unwrap();
    let api_url = gitlab().await;

    assert_eq!(
        upload(app(&root, &api_url), "throttled").await,
        StatusCode::BAD_GATEWAY,
        "GitLab answers 429 where GitHub answers 403, and both mean retry rather than denied"
    );
}

#[tokio::test]
async fn a_lock_belongs_to_the_gitlab_username() {
    let root = tempfile::tempdir().unwrap();
    let api_url = gitlab().await;

    let response = lock(app(&root, &api_url), "developer").await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        body["lock"]["owner"]["name"], "developer",
        "the identity comes from /user, which GitLab keys on username rather than login"
    );
}

#[tokio::test]
async fn a_grant_inherited_from_the_group_counts_as_much_as_a_project_one() {
    let root = tempfile::tempdir().unwrap();
    let api_url = gitlab().await;

    assert_eq!(
        upload(app(&root, &api_url), "group-developer").await,
        StatusCode::OK,
        "a project with no direct membership still grants through its group, which is how          most GitLab organisations are set up"
    );
}
