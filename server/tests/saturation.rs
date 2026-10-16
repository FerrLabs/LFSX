use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use lfsx_server::config::{Auth, Config};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

fn app(root: &tempfile::TempDir, cap: usize) -> Router {
    app_idling(root, cap, Duration::from_secs(60))
}

fn app_idling(root: &tempfile::TempDir, cap: usize, idle: Duration) -> Router {
    lfsx_server::app_with_transfer_idle(
        Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            metrics_bind: None,
            dashboard: None,
            forges: Vec::new(),
            storage_root: root.path().to_path_buf(),
            public_url: Some("https://lfs.example".into()),
            action_lifetime: 1800,
            gc_grace: Duration::from_secs(14 * 24 * 60 * 60),
            staging_max_age: Duration::from_secs(86400),
            lock_max_age: None,
            max_object_size: None,
            max_concurrent_transfers: cap,
            repo_quota: None,
            compression: None,
            encryption_key: None,
            storage: lfsx_server::config::Storage::Local,
            auth: Auth::Disabled,
        },
        idle,
    )
}

fn upload(oid: &str, body: Body, length: usize) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/Demo/objects/{oid}"))
        .header("content-length", length)
        .body(body)
        .unwrap()
}

fn oid_of(payload: &[u8]) -> String {
    hex::encode(Sha256::digest(payload))
}

// The slot is held for as long as the transfer moves bytes: an upload whose
// body has not finished arriving keeps its permit, and the next transfer is
// told to come back rather than queued into a server that is already full.
#[tokio::test]
async fn a_transfer_beyond_the_cap_is_told_to_come_back() {
    let root = tempfile::tempdir().unwrap();
    let app = app(&root, 1);

    let (writer, receiver) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(1);
    let payload = b"held open".to_vec();
    let slow = Body::from_stream(futures_util::stream::unfold(receiver, |mut chunks| async {
        chunks.recv().await.map(|chunk| (chunk, chunks))
    }));

    let holder = tokio::spawn({
        let app = app.clone();
        let oid = oid_of(&payload);
        let length = payload.len();
        async move { app.oneshot(upload(&oid, slow, length)).await.unwrap() }
    });

    // Once the first chunk is consumed, the handler has its permit.
    writer.send(Ok(b"held ".to_vec())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let refused = app
        .clone()
        .oneshot(upload(
            &oid_of(b"second"),
            Body::from("second".as_bytes()),
            6,
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        refused.headers().contains_key("retry-after"),
        "a saturated server has to say when to come back"
    );

    // Finishing the held transfer frees the slot for the next one.
    writer.send(Ok(b"open".to_vec())).await.unwrap();
    drop(writer);
    assert_eq!(holder.await.unwrap().status(), StatusCode::OK);

    let payload = b"after the slot freed".to_vec();
    let accepted = app
        .oneshot(upload(
            &oid_of(&payload),
            Body::from(payload.clone()),
            payload.len(),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
}

// A download's permit has to live as long as the client keeps reading the
// body, not as long as the handler ran, or every large download would free
// its slot at the first byte.
#[tokio::test]
async fn a_download_holds_its_slot_until_the_body_is_dropped() {
    let root = tempfile::tempdir().unwrap();
    let app = app(&root, 1);
    let payload = vec![3u8; 64 * 1024];
    let oid = oid_of(&payload);

    let stored = app
        .clone()
        .oneshot(upload(&oid, Body::from(payload.clone()), payload.len()))
        .await
        .unwrap();
    assert_eq!(stored.status(), StatusCode::OK);

    let get = |oid: &str| {
        Request::builder()
            .uri(format!("/FerrLabs/Demo/objects/{oid}"))
            .body(Body::empty())
            .unwrap()
    };

    // Headers arrive, the body stays unread: the slot stays occupied.
    let held = app.clone().oneshot(get(&oid)).await.unwrap();
    assert_eq!(held.status(), StatusCode::OK);

    let refused = app.clone().oneshot(get(&oid)).await.unwrap();
    assert_eq!(
        refused.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "the first download's unread body is still holding the only slot"
    );

    // Dropping the body releases the permit.
    drop(held);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let after = app.oneshot(get(&oid)).await.unwrap();
    assert_eq!(after.status(), StatusCode::OK);
}

// Zero is the off switch, for the operator whose proxy already does this job.
#[tokio::test]
async fn a_cap_of_zero_never_refuses() {
    let root = tempfile::tempdir().unwrap();
    let app = app(&root, 0);
    let payload = b"unlimited".to_vec();

    let stored = app
        .oneshot(upload(
            &oid_of(&payload),
            Body::from(payload.clone()),
            payload.len(),
        ))
        .await
        .unwrap();
    assert_eq!(stored.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_keep_list_is_refused_before_its_body_is_read_when_the_cap_is_reached() {
    let root = tempfile::tempdir().unwrap();
    let app = app(&root, 1);

    let (writer, receiver) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(1);
    let slow = Body::from_stream(futures_util::stream::unfold(receiver, |mut chunks| async {
        chunks.recv().await.map(|chunk| (chunk, chunks))
    }));
    let holder = tokio::spawn({
        let app = app.clone();
        async move {
            app.oneshot(upload(&oid_of(b"held"), slow, 4))
                .await
                .unwrap()
        }
    });
    writer.send(Ok(b"he".to_vec())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let retain = Request::builder()
        .method("POST")
        .uri("/FerrLabs/Demo/objects/retain")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"oids":[],"dry_run":true}"#))
        .unwrap();
    let refused = app.oneshot(retain).await.unwrap();

    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(refused.headers().contains_key("retry-after"));
    holder.abort();
}

const BIG: usize = 64 * 1024;

async fn stored_big(app: &Router) -> (String, Vec<u8>) {
    let payload = vec![7u8; BIG];
    let oid = oid_of(&payload);
    let stored = app
        .clone()
        .oneshot(upload(&oid, Body::from(payload.clone()), payload.len()))
        .await
        .unwrap();
    assert_eq!(stored.status(), StatusCode::OK);
    (oid, payload)
}

fn get(oid: &str) -> Request<Body> {
    Request::builder()
        .uri(format!("/FerrLabs/Demo/objects/{oid}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn a_download_nobody_reads_gives_its_slot_back_after_the_idle_timeout() {
    let root = tempfile::tempdir().unwrap();
    let app = app_idling(&root, 1, Duration::from_millis(200));
    let (oid, _) = stored_big(&app).await;

    let stalled = app.clone().oneshot(get(&oid)).await.unwrap();
    assert_eq!(stalled.status(), StatusCode::OK);

    let refused = app.clone().oneshot(get(&oid)).await.unwrap();
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);

    tokio::time::sleep(Duration::from_millis(600)).await;

    let after = app.oneshot(get(&oid)).await.unwrap();
    assert_eq!(after.status(), StatusCode::OK);
    drop(stalled);
}

#[tokio::test]
async fn a_slow_download_that_keeps_progressing_is_not_cut() {
    let root = tempfile::tempdir().unwrap();
    let app = app_idling(&root, 1, Duration::from_millis(300));
    let (oid, payload) = stored_big(&app).await;

    let response = app.oneshot(get(&oid)).await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let mut received = 0;
    let started = std::time::Instant::now();

    while let Some(chunk) = futures_util::StreamExt::next(&mut body).await {
        received += chunk
            .expect("the body must not be cut while it progresses")
            .len();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert_eq!(received, payload.len());
    assert!(started.elapsed() > Duration::from_millis(300));
}

#[tokio::test]
async fn an_upload_that_stops_sending_gives_its_slot_back_after_the_idle_timeout() {
    let root = tempfile::tempdir().unwrap();
    let app = app_idling(&root, 1, Duration::from_millis(200));

    let (writer, receiver) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(1);
    let stalled = Body::from_stream(futures_util::stream::unfold(receiver, |mut chunks| async {
        chunks.recv().await.map(|chunk| (chunk, chunks))
    }));
    let holder = tokio::spawn({
        let app = app.clone();
        async move {
            app.oneshot(upload(&oid_of(b"held open"), stalled, 9))
                .await
                .unwrap()
        }
    });
    writer.send(Ok(b"held ".to_vec())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let refused = app
        .clone()
        .oneshot(upload(&oid_of(b"second"), Body::from("second"), 6))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);

    let cut = holder.await.unwrap();
    assert_ne!(cut.status(), StatusCode::OK);

    let after = app
        .oneshot(upload(&oid_of(b"second"), Body::from("second"), 6))
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::OK);
    drop(writer);
}
