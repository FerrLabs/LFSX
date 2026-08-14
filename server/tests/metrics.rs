mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{app, forge, put};
use sha2::Digest;
use tower::ServiceExt;

async fn scrape(app: Router) -> String {
    let request = Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    String::from_utf8(bytes.to_vec()).unwrap()
}

fn sample(exposition: &str, metric: &str) -> f64 {
    exposition
        .lines()
        .find(|line| line.starts_with(metric))
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("{metric} is missing from:\n{exposition}"))
}

#[tokio::test]
async fn metrics_are_served_without_credentials() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, forge) = forge().await;

    let exposition = scrape(app(&root, &api_url, Duration::from_secs(60))).await;

    assert!(exposition.contains("lfsx_store_bytes"));
    assert!(exposition.contains("lfsx_objects_stored"));
    assert_eq!(
        forge.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "an orchestrator scraping metrics has no forge token"
    );
}

#[tokio::test]
async fn an_upload_moves_the_byte_counters_and_the_store_gauges() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let payload = b"an asset worth measuring".repeat(100);

    assert_eq!(
        put(app.clone(), Some("writer"), &payload).await.status(),
        StatusCode::OK
    );
    let exposition = scrape(app).await;

    assert_eq!(
        sample(&exposition, "lfsx_uploaded_bytes_total"),
        payload.len() as f64
    );
    assert_eq!(
        sample(
            &exposition,
            r#"lfsx_requests_total{route="/{org}/{repo}/objects/{oid}",status="200"}"#
        ),
        1.0
    );
    assert_eq!(sample(&exposition, "lfsx_object_size_bytes_count"), 1.0);
    assert_eq!(
        sample(&exposition, "lfsx_store_bytes"),
        payload.len() as f64
    );
    assert_eq!(sample(&exposition, "lfsx_objects_stored"), 1.0);
}

#[tokio::test]
async fn a_refusal_is_counted_under_its_cause() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));

    put(app.clone(), Some("reader"), b"asset").await;

    let exposition = scrape(app).await;
    assert!(
        exposition.contains(r#"lfsx_rejections_total{cause="forbidden"}"#),
        "a refusal has to be attributable, not just a 4xx count:\n{exposition}"
    );
}

#[tokio::test]
async fn the_object_id_never_becomes_a_label() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let payload = b"one object among millions".to_vec();
    let oid = hex::encode(sha2::Sha256::digest(&payload));

    put(app.clone(), Some("writer"), &payload).await;

    let exposition = scrape(app).await;
    assert!(
        !exposition.contains(&oid),
        "labelling by object id would grow the series count without bound"
    );
    assert!(
        exposition.contains(r#"route="/{org}/{repo}/objects/{oid}""#),
        "the route template is the label, not the path:\n{exposition}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_scrapes_walks_the_store_once() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    put(app.clone(), Some("writer"), b"an asset").await;

    let burst: Vec<_> = (0..8)
        .map(|_| {
            let app = app.clone();
            tokio::spawn(async move { scrape(app).await })
        })
        .collect();
    for handle in burst {
        handle.await.unwrap();
    }

    let exposition = scrape(app).await;
    assert_eq!(
        sample(&exposition, "lfsx_store_scans"),
        1.0,
        "concurrent scrapes must queue behind one walk, not each start their own:
{exposition}"
    );
}

#[tokio::test]
async fn downloaded_bytes_count_what_was_actually_streamed() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let app = app(&root, &api_url, Duration::from_secs(60));
    let payload = b"bytes that have to leave the building".to_vec();
    let oid = hex::encode(sha2::Sha256::digest(&payload));
    put(app.clone(), Some("writer"), &payload).await;

    let request = Request::builder()
        .uri(format!("/FerrLabs/LFSX/objects/{oid}"))
        .header("authorization", common::credentials("writer"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();

    assert_eq!(
        sample(&scrape(app.clone()).await, "lfsx_downloaded_bytes_total"),
        0.0,
        "nothing has been read off the response yet"
    );

    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    assert_eq!(
        sample(&scrape(app).await, "lfsx_downloaded_bytes_total"),
        payload.len() as f64
    );
}
