mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{credentials, forge, put};
use lfsx_server::config::Config;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

fn app(root: &tempfile::TempDir, api_url: &str) -> Router {
    lfsx_server::app(Config {
        compression: Some(3),
        encryption_key: None,
        ..common::config(root, api_url)
    })
}

// A mesh is float arrays, and float arrays repeat themselves — which is why an
// LFS store full of FBX compresses at all, where its PNGs do not.
fn mesh(len: usize) -> Vec<u8> {
    b"vertex 0.7071 0.0000 0.7071 normal 0.0000 1.0000 0.0000 "
        .iter()
        .cycle()
        .take(len)
        .copied()
        .collect()
}

async fn download(app: Router, oid: &str, range: Option<&str>) -> (StatusCode, Vec<u8>, String) {
    let mut request = Request::builder()
        .uri(format!("/FerrLabs/LFSX/objects/{oid}"))
        .header("authorization", credentials("reader"));

    if let Some(range) = range {
        request = request.header(header::RANGE, range);
    }

    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_range = response
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();

    (status, body, content_range)
}

fn on_disk(root: &tempfile::TempDir, oid: &str) -> u64 {
    std::fs::metadata(
        root.path()
            .join("FerrLabs/LFSX")
            .join(&oid[0..2])
            .join(&oid[2..4])
            .join(oid),
    )
    .unwrap()
    .len()
}

#[tokio::test]
async fn an_object_comes_back_exactly_as_it_went_in() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    let payload = mesh(9 * 1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));

    assert_eq!(
        put(app.clone(), Some("writer"), &payload).await.status(),
        StatusCode::OK
    );
    let (status, body, _) = download(app, &oid, None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        hex::encode(Sha256::digest(&body)),
        oid,
        "the client verifies the digest of what it receives, so anything but the original bytes \
         is a failed transfer"
    );
    assert!(
        on_disk(&root, &oid) < payload.len() as u64 / 4,
        "and the disk holds a fraction of it: {} of {}",
        on_disk(&root, &oid),
        payload.len()
    );
}

#[tokio::test]
async fn a_range_lands_on_the_right_bytes_of_a_compressed_object() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    let payload = mesh(10 * 1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));
    put(app.clone(), Some("writer"), &payload).await;

    let start = 9_000_000;
    let end = 9_001_023;
    let (status, body, content_range) =
        download(app, &oid, Some(&format!("bytes={start}-{end}"))).await;

    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        content_range,
        format!("bytes {start}-{end}/{}", payload.len()),
        "the range is negotiated in the object's own bytes, not the ones on disk"
    );
    assert_eq!(
        body,
        &payload[start..=end],
        "resuming a transfer near the end of a compressed object must not hand back the frame \
         boundary before it"
    );
}

#[tokio::test]
async fn a_store_written_before_compression_still_serves() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let payload = mesh(2 * 1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));

    let uncompressed = lfsx_server::app(common::config(&root, &api_url));
    assert_eq!(
        put(uncompressed, Some("writer"), &payload).await.status(),
        StatusCode::OK
    );
    let raw = on_disk(&root, &oid);

    let (status, body, _) = download(app(&root, &api_url), &oid, None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, payload);
    assert_eq!(
        on_disk(&root, &oid),
        raw,
        "turning compression on rewrites nothing that is already there — a mixed store is the \
         normal state of one that was upgraded"
    );
}

#[tokio::test]
async fn what_the_repository_holds_is_measured_on_disk() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url);
    let payload = mesh(4 * 1024 * 1024);
    put(app.clone(), Some("writer"), &payload).await;

    let request = Request::builder()
        .uri("/FerrLabs/LFSX/objects/stats")
        .header("authorization", credentials("reader"))
        .body(Body::empty())
        .unwrap();
    let stats = common::read_json(app.oneshot(request).await.unwrap()).await;

    assert!(
        stats["bytes"].as_u64().unwrap() < payload.len() as u64 / 4,
        "a budget is a budget on disk: reporting the size before compression would bill a \
         repository for room it is not taking: {stats}"
    );
}
