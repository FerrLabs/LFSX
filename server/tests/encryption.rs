mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{credentials, forge, put};
use lfsx_server::config::Config;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

fn keyed(root: &tempfile::TempDir, api_url: &str, compression: Option<i32>) -> Router {
    let key = root.path().join("key");
    std::fs::write(&key, hex::encode([7u8; 32])).unwrap();

    lfsx_server::app(Config {
        compression,
        encryption_key: Some(lfsx_server::config::KeySource::File(key)),
        ..common::config(root, api_url)
    })
}

async fn push(app: Router, payload: &[u8]) -> StatusCode {
    put(app, Some("writer"), payload).await.status()
}

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
    let length = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();

    (status, body, length)
}

fn stored(root: &tempfile::TempDir, oid: &str) -> Vec<u8> {
    std::fs::read(
        root.path()
            .join("FerrLabs/LFSX")
            .join(&oid[0..2])
            .join(&oid[2..4])
            .join(oid),
    )
    .unwrap()
}

#[tokio::test]
async fn an_object_pushed_to_a_keyed_server_is_not_on_the_disk_in_the_clear() {
    let (api_url, _forge) = forge().await;
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(6 * 1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));

    assert_eq!(
        push(keyed(&root, &api_url, None), &payload).await,
        StatusCode::OK
    );

    let on_disk = stored(&root, &oid);
    assert!(
        !on_disk.windows(64).any(|window| window == &payload[..64]),
        "a stolen disk is the whole threat model — finding the plaintext on it means this feature \
         did nothing"
    );

    let (status, body, length) = download(keyed(&root, &api_url, None), &oid, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hex::encode(Sha256::digest(&body)), oid);
    assert_eq!(
        length,
        payload.len().to_string(),
        "the client is promised the plaintext length, not the file's — answering with the \
         ciphertext length breaks every download at the last byte"
    );
}

// The command source is the file source with stdout instead of a mount, so a
// server keyed by a hook has to serve exactly what a server keyed by a file
// does: encrypted at rest, plaintext on the way out.
#[tokio::test]
async fn a_server_keyed_by_a_command_round_trips_and_stores_nothing_in_the_clear() {
    let (api_url, _forge) = forge().await;
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));

    let keyed = || {
        lfsx_server::app(Config {
            encryption_key: Some(lfsx_server::config::KeySource::Command(format!(
                "echo {}",
                hex::encode([7u8; 32])
            ))),
            ..common::config(&root, &api_url)
        })
    };

    assert_eq!(push(keyed(), &payload).await, StatusCode::OK);

    let on_disk = stored(&root, &oid);
    assert!(
        !on_disk.windows(64).any(|window| window == &payload[..64]),
        "keys from a hook must protect the disk exactly as keys from a file do"
    );

    let (status, body, _) = download(keyed(), &oid, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        hex::encode(Sha256::digest(&body)),
        oid,
        "and a fresh keyring from the same hook must read what the first one wrote"
    );
}

// The oid is the digest of the plaintext and the client declares it, so the
// hash has to be taken on the way past and the bytes encrypted after. Reversed,
// every upload would fail its own verification.
#[tokio::test]
async fn the_digest_is_still_the_one_the_client_declared() {
    let (api_url, _forge) = forge().await;
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));

    assert_eq!(
        push(keyed(&root, &api_url, None), &payload).await,
        StatusCode::OK
    );

    let on_disk = stored(&root, &oid);
    assert_ne!(
        hex::encode(Sha256::digest(&on_disk)),
        oid,
        "the file is not the object any more, which is exactly why the name has to keep meaning \
         the plaintext"
    );
}

#[tokio::test]
async fn a_range_of_an_encrypted_object_is_served_from_the_frames_it_touches() {
    let (api_url, _forge) = forge().await;
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(9 * 1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));
    push(keyed(&root, &api_url, None), &payload).await;

    let (status, body, _) = download(
        keyed(&root, &api_url, None),
        &oid,
        Some("bytes=5000000-5004095"),
    )
    .await;

    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, payload[5_000_000..5_004_096]);
}

// Compression first, then encryption: sealed bytes are indistinguishable from
// random, so the other order would spend the CPU and save nothing.
#[tokio::test]
async fn compression_and_encryption_together_still_take_less_room() {
    let (api_url, _forge) = forge().await;
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(8 * 1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));

    push(keyed(&root, &api_url, Some(3)), &payload).await;

    let on_disk = stored(&root, &oid).len() as u64;
    assert!(
        on_disk < payload.len() as u64 / 4,
        "encrypting after compressing has to keep the compression: {on_disk} bytes for {}",
        payload.len()
    );

    let (_, body, _) = download(keyed(&root, &api_url, Some(3)), &oid, None).await;
    assert_eq!(hex::encode(Sha256::digest(&body)), oid);
}

// Reading every object back through the download path is what an audit is, and
// it is the check that still means something once the file is not the object.
#[tokio::test]
async fn the_audit_reads_encrypted_objects_through_their_own_bytes() {
    let (api_url, _forge) = forge().await;
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(5 * 1024 * 1024);
    push(keyed(&root, &api_url, Some(3)), &payload).await;

    let response = keyed(&root, &api_url, Some(3))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/FerrLabs/LFSX/objects/audit")
                .header("authorization", credentials("admin"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let report: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();

    assert_eq!(report["checked"], 1);
    assert_eq!(report["bytes"], payload.len());
    assert!(
        report["corrupt"].as_array().unwrap().is_empty(),
        "an encrypted object that reads back as its own digest is not corrupt: {report}"
    );
}

// Turning the key on for a running deployment leaves everything already pushed
// in plaintext. If those stopped reading, enabling encryption would be a flag
// day and nobody would do it.
#[tokio::test]
async fn objects_written_before_the_key_existed_still_read_after_it() {
    let (api_url, _forge) = forge().await;
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(2 * 1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));

    let plain = lfsx_server::app(Config {
        encryption_key: None,
        ..common::config(&root, &api_url)
    });
    assert_eq!(push(plain, &payload).await, StatusCode::OK);

    let (status, body, _) = download(keyed(&root, &api_url, None), &oid, None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(hex::encode(Sha256::digest(&body)), oid);
}

// And the other way: an object written under a key must not be served as
// ciphertext by a server that lost it. The client would receive bytes that do
// not hash to what it asked for and call the store corrupt.
#[tokio::test]
async fn an_encrypted_object_is_refused_by_a_server_without_the_key() {
    let (api_url, _forge) = forge().await;
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));
    push(keyed(&root, &api_url, None), &payload).await;

    let keyless = lfsx_server::app(Config {
        encryption_key: None,
        ..common::config(&root, &api_url)
    });
    let (status, body, _) = download(keyless, &oid, None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_ne!(
        hex::encode(Sha256::digest(&body)),
        oid,
        "and it certainly must not be the object"
    );
}
