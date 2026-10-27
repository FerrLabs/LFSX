mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::Response;
use common::{config, forge, logged, read_json};
use lfsx_server::config::{Auth, Config, Dashboard};
use lfsx_server::console::tokens::{self, Refusal};
use lfsx_server::namespace::Namespace;
use lfsx_server::storage::Store;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Served {
    _root: tempfile::TempDir,
    _pages: tempfile::TempDir,
    forge: std::sync::Arc<common::Forge>,
    store: Store,
    app: Router,
}

async fn served(auth: Option<Auth>) -> Served {
    serve(|_, default| auth.unwrap_or(default)).await
}

async fn served_without_cache() -> Served {
    serve(|api_url, _| {
        common::anonymous_forge_auth(
            api_url,
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
            false,
        )
    })
    .await
}

async fn serve(auth: impl FnOnce(&str, Auth) -> Auth) -> Served {
    let root = tempfile::tempdir().unwrap();
    let (api_url, forge) = forge().await;
    let pages = tempfile::tempdir().unwrap();
    std::fs::write(pages.path().join("index.html"), "<lfsx-root></lfsx-root>").unwrap();
    let base = config(&root, &api_url);
    let config = Config {
        dashboard: Some(Dashboard {
            dir: pages.path().to_path_buf(),
            admins: Some(Namespace::new("FerrLabs", "Infra").unwrap()),
        }),
        auth: auth(&api_url, base.auth.clone()),
        ..base
    };

    Served {
        store: lfsx_server::store(&config),
        app: lfsx_server::app(config),
        forge,
        _root: root,
        _pages: pages,
    }
}

async fn sign_in(app: Router, token: &str) -> Response {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/-/api/session")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "token": token }).to_string()))
        .unwrap();

    app.oneshot(request).await.unwrap()
}

fn session_of(response: &Response) -> String {
    let cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap();

    cookie.split(';').next().unwrap().to_owned()
}

async fn overview(app: Router, cookie: Option<&str>) -> StatusCode {
    let mut request = Request::builder().uri("/-/api/overview");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }

    app.oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_dashboard_token_opens_a_session_the_dashboard_accepts() {
    let served = served(None).await;
    let token = tokens::create(&served.store, "alice").await.unwrap();

    let response = sign_in(served.app.clone(), &token).await;

    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Path=/-/"));
    assert!(cookie.contains("Secure"));
    let session = session_of(&response);
    assert_eq!(
        read_json(response).await,
        json!({ "via": "token", "name": "alice" })
    );
    assert_eq!(overview(served.app, Some(&session)).await, StatusCode::OK);
}

#[tokio::test]
async fn a_token_nobody_issued_opens_nothing() {
    let served = served(None).await;
    tokens::create(&served.store, "alice").await.unwrap();

    let response = sign_in(served.app, "lfsx_not-a-token-this-server-issued").await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get("set-cookie").is_none());
}

#[tokio::test]
async fn a_forge_admin_of_the_dashboard_repository_opens_a_session() {
    let served = served(None).await;

    let response = sign_in(served.app.clone(), "admin").await;

    assert_eq!(response.status(), StatusCode::OK);
    let session = session_of(&response);
    assert_eq!(
        read_json(response).await,
        json!({ "via": "forge", "name": "admin" })
    );
    assert_eq!(overview(served.app, Some(&session)).await, StatusCode::OK);
}

#[tokio::test]
async fn a_forge_user_who_is_not_an_admin_is_refused_a_session() {
    let served = served(None).await;

    let response = sign_in(served.app, "writer").await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response.headers().get("set-cookie").is_none());
}

#[tokio::test(start_paused = true)]
async fn revoking_a_token_ends_the_sessions_it_opened() {
    let served = served(None).await;
    let token = tokens::create(&served.store, "alice").await.unwrap();
    let session = session_of(&sign_in(served.app.clone(), &token).await);

    tokens::revoke(&served.store, "alice").await.unwrap();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;

    assert_eq!(
        overview(served.app, Some(&session)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test(start_paused = true)]
async fn a_token_created_again_under_a_revoked_name_does_not_revive_old_sessions() {
    let served = served(None).await;
    let leaked = tokens::create(&served.store, "alice").await.unwrap();
    let session = session_of(&sign_in(served.app.clone(), &leaked).await);

    tokens::revoke(&served.store, "alice").await.unwrap();
    let fresh = tokens::create(&served.store, "alice").await.unwrap();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;

    assert_eq!(
        overview(served.app.clone(), Some(&session)).await,
        StatusCode::UNAUTHORIZED
    );
    let renewed = session_of(&sign_in(served.app.clone(), &fresh).await);
    assert_eq!(overview(served.app, Some(&renewed)).await, StatusCode::OK);
}

#[tokio::test]
async fn a_forged_session_cookie_is_ignored() {
    let served = served(None).await;
    let token = tokens::create(&served.store, "alice").await.unwrap();
    let session = session_of(&sign_in(served.app.clone(), &token).await);
    let tampered = format!("{}x", session);

    assert_eq!(
        overview(served.app, Some(&tampered)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn with_authentication_disabled_only_a_dashboard_token_gets_in() {
    let served = served(Some(Auth::Disabled)).await;
    assert_eq!(
        overview(served.app.clone(), None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        sign_in(served.app.clone(), "admin").await.status(),
        StatusCode::UNAUTHORIZED
    );

    let token = tokens::create(&served.store, "ops").await.unwrap();
    let session = session_of(&sign_in(served.app.clone(), &token).await);

    assert_eq!(overview(served.app, Some(&session)).await, StatusCode::OK);
}

#[tokio::test]
async fn signing_out_clears_the_cookie() {
    let served = served(None).await;

    let response = served
        .app
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/-/api/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.starts_with("lfsx_session=;"));
    assert!(cookie.contains("Max-Age=0"));
}

#[tokio::test]
async fn the_current_session_names_who_opened_it() {
    let served = served(None).await;
    let token = tokens::create(&served.store, "alice").await.unwrap();
    let session = session_of(&sign_in(served.app.clone(), &token).await);

    let response = served
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/-/api/session")
                .header("cookie", &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        read_json(response).await,
        json!({ "via": "token", "name": "alice" })
    );

    let response = served
        .app
        .oneshot(
            Request::builder()
                .uri("/-/api/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn token_names_are_unique_and_readable() {
    let served = served(None).await;
    tokens::create(&served.store, "alice").await.unwrap();

    assert!(matches!(
        tokens::create(&served.store, "alice").await,
        Err(Refusal::Taken(_))
    ));
    for name in ["", "with space", "slash/name", &"x".repeat(65)] {
        assert!(
            matches!(
                tokens::create(&served.store, name).await,
                Err(Refusal::Unreadable)
            ),
            "{name}"
        );
    }
    assert!(matches!(
        tokens::revoke(&served.store, "bob").await,
        Err(Refusal::Unknown(_))
    ));
    let names: Vec<String> = tokens::issued(&served.store)
        .await
        .unwrap()
        .into_iter()
        .map(|issued| issued.name)
        .collect();
    assert_eq!(names, ["alice"]);
}

#[tokio::test]
async fn the_store_keeps_no_token_in_the_clear() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let store = lfsx_server::store(&config(&root, &api_url));

    let token = tokens::create(&store, "alice").await.unwrap();

    let kept = std::fs::read_to_string(root.path().join(".lfsx/dashboard-tokens.json")).unwrap();
    assert!(!kept.contains(&token));
    assert!(!kept.contains(token.trim_start_matches("lfsx_")));
    let _: Value = serde_json::from_str(&kept).unwrap();
}

#[tokio::test]
async fn signing_in_lands_on_the_audit_trail_naming_who_signed_in() {
    let served = served(None).await;
    let token = tokens::create(&served.store, "alice").await.unwrap();

    let (response, trail) = logged(sign_in(served.app, &token)).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(trail.contains("lfsx::audit"), "{trail}");
    assert!(trail.contains(r#"actor=Token("alice")"#), "{trail}");
    assert!(!trail.contains(&token), "{trail}");
}

#[tokio::test]
async fn creating_and_revoking_a_token_land_on_the_audit_trail_without_the_secret() {
    let served = served(None).await;

    let (token, created) =
        logged(async { tokens::create(&served.store, "alice").await.unwrap() }).await;
    let (_, revoked) =
        logged(async { tokens::revoke(&served.store, "alice").await.unwrap() }).await;

    assert!(
        created.contains("lfsx::audit") && created.contains(r#"name="alice""#),
        "{created}"
    );
    assert!(!created.contains(&token), "{created}");
    assert!(
        revoked.contains("lfsx::audit") && revoked.contains(r#"name="alice""#),
        "{revoked}"
    );
}

fn sign_out_request(session: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri("/-/api/session")
        .header("cookie", session)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn a_forge_session_ends_when_the_admin_loses_admin_rights() {
    let served = served_without_cache().await;
    let session = session_of(&sign_in(served.app.clone(), "admin").await);
    assert_eq!(
        overview(served.app.clone(), Some(&session)).await,
        StatusCode::OK
    );

    *served.forge.admin_is_admin.lock().unwrap() = false;

    assert_eq!(
        overview(served.app.clone(), Some(&session)).await,
        StatusCode::UNAUTHORIZED
    );
    *served.forge.admin_is_admin.lock().unwrap() = true;
    assert_eq!(
        overview(served.app, Some(&session)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_signed_out_forge_session_cannot_be_replayed() {
    let served = served(None).await;
    let session = session_of(&sign_in(served.app.clone(), "admin").await);

    let response = served
        .app
        .clone()
        .oneshot(sign_out_request(&session))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        overview(served.app, Some(&session)).await,
        StatusCode::UNAUTHORIZED
    );
}
