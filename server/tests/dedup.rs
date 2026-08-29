mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{app, app_collecting_immediately, credentials, forge};
use serde_json::json;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

async fn put_into(app: Router, repo: &str, payload: &[u8]) -> StatusCode {
    let oid = hex::encode(Sha256::digest(payload));
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/{repo}/objects/{oid}"))
        .header("content-length", payload.len())
        .header("authorization", credentials("writer"))
        .body(Body::from(payload.to_vec()))
        .unwrap();

    app.oneshot(request).await.unwrap().status()
}

async fn get_from(app: Router, repo: &str, payload: &[u8]) -> StatusCode {
    let oid = hex::encode(Sha256::digest(payload));
    let request = Request::builder()
        .uri(format!("/FerrLabs/{repo}/objects/{oid}"))
        .header("authorization", credentials("writer"))
        .body(Body::empty())
        .unwrap();

    app.oneshot(request).await.unwrap().status()
}

// Collected as the administrator, because a real run unlinks files and asks
// for that level since #233. What these tests exercise is what happens to the
// shared bytes, and the pushes above them stay ordinary writers.
async fn retain(app: Router, repo: &str, oids: &[&str]) -> serde_json::Value {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/FerrLabs/{repo}/objects/retain"))
        .header("content-type", "application/json")
        .header("authorization", credentials("admin"))
        .body(Body::from(json!({ "oids": oids }).to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    serde_json::from_slice(&bytes).unwrap()
}

fn stored_bytes(root: &std::path::Path) -> u64 {
    let content = root.join(".content");
    let mut total = 0;
    let mut stack = vec![content];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(metadata) = entry.metadata() {
                total += metadata.len();
            }
        }
    }

    total
}

#[tokio::test]
async fn the_same_asset_in_two_projects_is_stored_once() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let pack = b"a texture pack shared between two games".repeat(100);

    assert_eq!(
        put_into(app.clone(), "Blastlands", &pack).await,
        StatusCode::OK
    );
    let after_first = stored_bytes(root.path());

    assert_eq!(
        put_into(app.clone(), "RogueLite", &pack).await,
        StatusCode::OK
    );

    assert_eq!(
        stored_bytes(root.path()),
        after_first,
        "the second project must cost no disk, which is the entire point"
    );
    assert_eq!(after_first, pack.len() as u64);
}

#[tokio::test]
async fn each_project_still_reads_its_own_copy() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let pack = b"a shared asset".to_vec();
    put_into(app.clone(), "Blastlands", &pack).await;
    put_into(app.clone(), "RogueLite", &pack).await;

    assert_eq!(
        get_from(app.clone(), "Blastlands", &pack).await,
        StatusCode::OK
    );
    assert_eq!(get_from(app, "RogueLite", &pack).await, StatusCode::OK);
}

#[tokio::test]
async fn a_project_cannot_see_an_object_it_never_pushed() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let secret = b"an asset only one project has".to_vec();
    put_into(app.clone(), "Blastlands", &secret).await;

    assert_eq!(
        get_from(app, "RogueLite", &secret).await,
        StatusCode::NOT_FOUND,
        "sharing the bytes on disk must not share them over the API"
    );
}

#[tokio::test]
async fn collecting_one_project_leaves_the_other_readable() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app_collecting_immediately(&root, &api_url);
    let pack = b"a shared asset pack".repeat(50);
    put_into(app.clone(), "Blastlands", &pack).await;
    put_into(app.clone(), "RogueLite", &pack).await;

    let report = retain(app.clone(), "Blastlands", &[]).await;

    assert_eq!(report["swept"], 1);
    assert_eq!(
        report["bytes"], 0,
        "nothing was freed: the other project still holds those bytes, and saying otherwise \
         would promise space that does not exist"
    );
    assert_eq!(
        get_from(app.clone(), "RogueLite", &pack).await,
        StatusCode::OK
    );
    assert_eq!(
        get_from(app, "Blastlands", &pack).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(stored_bytes(root.path()), pack.len() as u64);
}

#[tokio::test]
async fn the_last_project_to_let_go_frees_the_disk() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app_collecting_immediately(&root, &api_url);
    let pack = b"a shared asset pack".repeat(50);
    put_into(app.clone(), "Blastlands", &pack).await;
    put_into(app.clone(), "RogueLite", &pack).await;
    retain(app.clone(), "Blastlands", &[]).await;

    let report = retain(app.clone(), "RogueLite", &[]).await;

    assert_eq!(report["bytes"], pack.len());
    assert_eq!(
        stored_bytes(root.path()),
        0,
        "once no project references it, the content itself has to go"
    );
}

#[tokio::test]
async fn the_capacity_gauge_reports_the_disk_not_the_sum_of_projects() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let pack = b"a pack three games share".repeat(100);

    for repo in ["Blastlands", "RogueLite", "IdlerSurvivor"] {
        assert_eq!(put_into(app.clone(), repo, &pack).await, StatusCode::OK);
    }

    let request = Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let exposition = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();

    let reported: u64 = exposition
        .lines()
        .find(|line| line.starts_with("lfsx_store_bytes"))
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value as u64)
        .expect("lfsx_store_bytes is missing");

    assert_eq!(
        reported,
        pack.len() as u64,
        "three projects share one copy, so the capacity metric must show one copy —          counting per-repository links would report the pre-deduplication total and          grow with every project that links the same pack"
    );
}

async fn dedupe(app: Router, token: &str, repo: &str) -> axum::response::Response {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/FerrLabs/{repo}/objects/dedupe"))
        .header("content-type", "application/json")
        .header("authorization", credentials(token))
        .body(Body::from(json!({ "dry_run": false }).to_string()))
        .unwrap();

    app.oneshot(request).await.unwrap()
}

// What a server that predates the shared store left behind: a plain file at the
// repository path, holding the only copy of its bytes.
fn store_the_old_way(root: &tempfile::TempDir, repo: &str, payload: &[u8]) {
    let oid = hex::encode(Sha256::digest(payload));
    let fanout = root
        .path()
        .join("FerrLabs")
        .join(repo)
        .join(&oid[0..2])
        .join(&oid[2..4]);
    std::fs::create_dir_all(&fanout).unwrap();
    std::fs::write(fanout.join(&oid), payload).unwrap();
}

#[tokio::test]
async fn objects_from_an_older_server_are_folded_in_and_still_served() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let payload = b"an asset pushed long before deduplication existed".repeat(8);
    store_the_old_way(&root, "Blastlands", &payload);

    let report = common::read_json(dedupe(app.clone(), "admin", "Blastlands").await).await;

    assert_eq!(report["adopted"], 1);
    assert_eq!(
        get_from(app, "Blastlands", &payload).await,
        StatusCode::OK,
        "the migration is only worth running if the repository keeps serving what it held"
    );
}

#[tokio::test]
async fn folding_a_repository_in_needs_more_than_push_rights() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let payload = b"an asset nobody should rewrite on a writer's say-so";
    store_the_old_way(&root, "Blastlands", &payload[..]);

    let refused = dedupe(app.clone(), "writer", "Blastlands").await;

    assert_eq!(
        refused.status(),
        StatusCode::FORBIDDEN,
        "this rewrites every object in place, so it asks for the rights of someone who could \
         delete them instead"
    );
    assert_eq!(
        get_from(app, "Blastlands", payload).await,
        StatusCode::OK,
        "and it changed nothing on the way out"
    );
}
