use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use lfsx_server::config::{Auth, Config};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

fn app(root: &tempfile::TempDir, cap: usize) -> Router {
    lfsx_server::app(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
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
    })
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
    let payload = b"an asset somebody is downloading".to_vec();
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
