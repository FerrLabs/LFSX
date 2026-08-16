mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{app, credentials, forge, post, read_json};
use serde_json::json;
use tower::ServiceExt;

const SCENE: &str = "Assets/Scenes/Arena.unity";

async fn lock(app: Router, token: &str, path: &str) -> axum::response::Response {
    post(
        app,
        token,
        "/FerrLabs/Blastlands/locks",
        json!({ "path": path }),
    )
    .await
}

async fn unlock(app: Router, token: &str, id: &str, force: bool) -> StatusCode {
    post(
        app,
        token,
        &format!("/FerrLabs/Blastlands/locks/{id}/unlock"),
        json!({ "force": force }),
    )
    .await
    .status()
}

async fn list(app: Router, token: &str, query: &str) -> serde_json::Value {
    let request = Request::builder()
        .uri(format!("/FerrLabs/Blastlands/locks{query}"))
        .header("authorization", credentials(token))
        .body(Body::empty())
        .unwrap();

    read_json(app.oneshot(request).await.unwrap()).await
}

#[tokio::test]
async fn a_lock_is_created_and_then_listed() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));

    let created = lock(app.clone(), "writer", SCENE).await;

    assert_eq!(created.status(), StatusCode::CREATED);
    let body = read_json(created).await;
    assert_eq!(body["lock"]["path"], SCENE);
    assert_eq!(body["lock"]["owner"]["name"], "writer");
    assert!(
        !body["lock"]["locked_at"].as_str().unwrap().is_empty(),
        "git-lfs shows locked_at in `git lfs locks`"
    );

    let listed = list(app, "writer", "").await;
    assert_eq!(listed["locks"].as_array().unwrap().len(), 1);
    assert_eq!(listed["locks"][0]["path"], SCENE);
}

#[tokio::test]
async fn taking_a_lock_someone_else_holds_fails_and_names_them() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    lock(app.clone(), "writer", SCENE).await;

    let refused = lock(app, "admin", SCENE).await;

    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let body = read_json(refused).await;
    assert_eq!(
        body["lock"]["owner"]["name"], "writer",
        "the client shows who to go and talk to"
    );
}

#[tokio::test]
async fn the_owner_can_unlock() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let id = read_json(lock(app.clone(), "writer", SCENE).await).await["lock"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_eq!(
        unlock(app.clone(), "writer", &id, false).await,
        StatusCode::OK
    );
    assert!(
        list(app, "writer", "").await["locks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_third_party_cannot_unlock_what_they_do_not_hold() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let id = read_json(lock(app.clone(), "writer", SCENE).await).await["lock"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_eq!(
        unlock(app.clone(), "admin", &id, false).await,
        StatusCode::FORBIDDEN,
        "someone else's lock is not yours to drop by accident"
    );
    assert_eq!(
        list(app, "writer", "").await["locks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn only_an_administrator_can_force_a_lock_open() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let id = read_json(lock(app.clone(), "writer", SCENE).await).await["lock"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_eq!(
        unlock(app.clone(), "reader", &id, true).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(unlock(app, "admin", &id, true).await, StatusCode::OK);
}

#[tokio::test]
async fn verify_tells_each_caller_which_locks_are_theirs() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    lock(app.clone(), "writer", SCENE).await;
    lock(app.clone(), "admin", "Assets/Art/Hero.psd").await;

    let mine = read_json(
        post(
            app,
            "writer",
            "/FerrLabs/Blastlands/locks/verify",
            json!({}),
        )
        .await,
    )
    .await;

    assert_eq!(mine["ours"].as_array().unwrap().len(), 1);
    assert_eq!(mine["ours"][0]["path"], SCENE);
    assert_eq!(mine["theirs"].as_array().unwrap().len(), 1);
    assert_eq!(mine["theirs"][0]["owner"]["name"], "admin");
}

#[tokio::test]
async fn a_read_only_token_cannot_take_a_lock() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));

    let refused = lock(app.clone(), "reader", SCENE).await;

    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(
        list(app, "reader", "").await["locks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn locks_can_be_looked_up_by_path() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    lock(app.clone(), "writer", SCENE).await;
    lock(app.clone(), "writer", "Assets/Art/Hero.psd").await;

    let found = list(app.clone(), "writer", &format!("?path={SCENE}")).await;

    assert_eq!(found["locks"].as_array().unwrap().len(), 1);
    assert_eq!(found["locks"][0]["path"], SCENE);
    assert!(
        list(app, "writer", "?path=Assets/Never/Locked.png").await["locks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

async fn lock_many(app: Router, count: usize) {
    for n in 0..count {
        lock(app.clone(), "writer", &format!("Assets/Prop{n:03}.psd")).await;
    }
}

#[tokio::test]
async fn a_long_list_of_locks_is_paged_rather_than_sent_whole() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    lock_many(app.clone(), 7).await;

    let first = list(app.clone(), "writer", "?limit=3").await;
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();
    let second = list(app.clone(), "writer", &format!("?limit=3&cursor={cursor}")).await;

    assert_eq!(first["locks"].as_array().unwrap().len(), 3);
    assert_eq!(second["locks"].as_array().unwrap().len(), 3);
    assert!(
        !cursor.is_empty(),
        "a client that sent a limit and got no cursor believes it has seen every lock"
    );

    let seen: Vec<_> = first["locks"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["locks"].as_array().unwrap())
        .map(|lock| lock["id"].as_str().unwrap())
        .collect();
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(seen.len(), unique.len(), "a walk must not repeat a lock");
}

#[tokio::test]
async fn the_last_page_of_locks_carries_no_cursor() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    lock_many(app.clone(), 4).await;

    let page = list(app, "writer", "?limit=10").await;

    assert_eq!(page["locks"].as_array().unwrap().len(), 4);
    assert!(
        page["next_cursor"].is_null(),
        "an absent cursor is what ends the walk: {page}"
    );
}

#[tokio::test]
async fn verify_pages_over_the_whole_list_so_both_sides_agree() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    lock_many(app.clone(), 5).await;
    lock(app.clone(), "admin", "Assets/Hero.psd").await;

    let page = read_json(
        post(
            app,
            "writer",
            "/FerrLabs/Blastlands/locks/verify",
            json!({ "limit": 4 }),
        )
        .await,
    )
    .await;

    let ours = page["ours"].as_array().unwrap().len();
    let theirs = page["theirs"].as_array().unwrap().len();
    assert_eq!(
        ours + theirs,
        4,
        "the limit governs the page, not each half of it: {page}"
    );
    assert!(!page["next_cursor"].as_str().unwrap_or_default().is_empty());
}
