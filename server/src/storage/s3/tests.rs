use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use futures_util::StreamExt;

use super::*;

pub(crate) type Objects = Arc<Mutex<HashMap<String, Vec<u8>>>>;

// Enough of S3 to prove the layout and the wire format this store depends on:
// PUT, HEAD, ranged GET and a prefix listing. The same shape as the stub forge
// the authentication tests run against — a real MinIO belongs in CI, not in the
// path of every `cargo test`.
pub(crate) async fn bucket() -> (String, Objects) {
    let objects: Objects = Arc::new(Mutex::new(HashMap::new()));

    let app = Router::new()
        .route(
            "/{*key}",
            any(
                |State(objects): State<Objects>,
                 Path(key): Path<String>,
                 axum::extract::RawQuery(query): axum::extract::RawQuery,
                 headers: HeaderMap,
                 method: axum::http::Method,
                 body: axum::body::Body| async move {
                    let query = query.unwrap_or_default();

                    // Path-style addressing puts the bucket in the path, so the
                    // stub drops it to keep the same keyspace a real bucket has.
                    let key = key.strip_prefix("assets/").unwrap_or(&key).to_owned();

                    if query.contains("list-type=2") {
                        return list(&objects, &query);
                    }

                    match method {
                        axum::http::Method::PUT => {
                            let bytes = axum::body::to_bytes(body, usize::MAX)
                                .await
                                .unwrap_or_default();
                            objects.lock().unwrap().insert(key, bytes.to_vec());
                            StatusCode::OK.into_response()
                        }
                        axum::http::Method::HEAD => match objects.lock().unwrap().get(&key) {
                            Some(object) => (
                                StatusCode::OK,
                                [(header::CONTENT_LENGTH, object.len().to_string())],
                            )
                                .into_response(),
                            None => StatusCode::NOT_FOUND.into_response(),
                        },
                        _ => get(&objects, &key, &headers),
                    }
                },
            ),
        )
        .with_state(objects.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    (format!("http://{address}"), objects)
}

fn get(objects: &Objects, key: &str, headers: &HeaderMap) -> Response {
    let held = objects.lock().unwrap();
    let Some(object) = held.get(key) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.split_once('-'));

    let Some((start, end)) = range else {
        return (StatusCode::OK, object.clone()).into_response();
    };

    let start: usize = start.parse().unwrap_or_default();
    let end: usize = end.parse().unwrap_or(object.len() - 1);
    let slice = object[start.min(object.len())..(end + 1).min(object.len())].to_vec();

    (StatusCode::PARTIAL_CONTENT, slice).into_response()
}

fn list(objects: &Objects, query: &str) -> Response {
    let prefix = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("prefix="))
        .map(|prefix| prefix.replace("%2F", "/"))
        .unwrap_or_default();

    let held = objects.lock().unwrap();
    let matching: Vec<_> = held.keys().filter(|key| key.starts_with(&prefix)).collect();
    let keys: String = matching
        .iter()
        .map(|key| format!(
            "<Contents><Key>{key}</Key><Size>0</Size><ETag>\"d41d8\"</ETag><LastModified>2026-08-15T00:00:00.000Z</LastModified><StorageClass>STANDARD</StorageClass></Contents>"
        ))
        .collect();

    (
        StatusCode::OK,
        format!(
            "<?xml version=\"1.0\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>assets</Name><KeyCount>{}</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>{keys}</ListBucketResult>",
            matching.len()
        ),
    )
        .into_response()
}

pub(crate) fn store(endpoint: &str) -> S3Store {
    S3Store::new(&S3Config {
        endpoint: endpoint.to_owned(),
        bucket: "assets".into(),
        region: "us-east-1".into(),
        access_key: "key".into(),
        secret_key: "secret".into(),
        path_style: true,
    })
    .unwrap()
}

fn namespace(repo: &str) -> Namespace {
    Namespace::new("FerrLabs", repo).unwrap()
}

async fn staged(payload: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("staged");
    tokio::fs::write(&path, payload).await.unwrap();

    (root, path)
}

async fn read_all(store: &S3Store, oid: &str, start: u64, length: u64) -> Vec<u8> {
    let mut chunks = Box::pin(store.read(oid, start, length).await.unwrap());
    let mut out = Vec::new();

    while let Some(chunk) = chunks.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }

    out
}

const OID: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

#[tokio::test]
async fn an_object_goes_up_once_and_comes_back_whole() {
    let (endpoint, objects) = bucket().await;
    let store = store(&endpoint);
    let payload = b"an asset that lives in a bucket".repeat(64);
    let (_root, path) = staged(&payload).await;

    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    assert_eq!(
        read_all(&store, OID, 0, payload.len() as u64).await,
        payload
    );
    assert_eq!(
        objects.lock().unwrap().len(),
        2,
        "the bytes under their digest, and one empty marker saying this repository holds them"
    );
}

#[tokio::test]
async fn a_second_repository_adds_a_marker_and_not_the_bytes() {
    let (endpoint, objects) = bucket().await;
    let store = store(&endpoint);
    let payload = b"the asset pack both projects use".repeat(32);
    let (_root, path) = staged(&payload).await;

    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();
    store.store(&namespace("Arena"), OID, &path).await.unwrap();

    let held = objects.lock().unwrap();
    assert_eq!(
        held.len(),
        3,
        "one copy of the bytes, two markers — deduplication is what content addressing gives for \
         free here, and paying twice would throw it away"
    );
    assert_eq!(held.values().filter(|object| !object.is_empty()).count(), 1);
}

#[tokio::test]
async fn a_repository_that_never_pushed_an_object_does_not_hold_it() {
    let (endpoint, _objects) = bucket().await;
    let store = store(&endpoint);
    let (_root, path) = staged(b"an asset one project pushed").await;
    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    assert!(store.exists(&namespace("Blastlands"), OID).await);
    assert!(
        !store.exists(&namespace("Arena"), OID).await,
        "the marker is the proof of possession — without it, guessing a digest would be enough to \
         read another project's assets out of the shared keyspace"
    );
}

#[tokio::test]
async fn a_range_asks_the_bucket_for_that_range() {
    let (endpoint, _objects) = bucket().await;
    let store = store(&endpoint);
    let payload: Vec<u8> = (0..=255u8).cycle().take(8192).collect();
    let (_root, path) = staged(&payload).await;
    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    assert_eq!(read_all(&store, OID, 4096, 100).await, payload[4096..4196]);
}

#[tokio::test]
async fn what_a_repository_holds_is_counted_from_its_markers() {
    let (endpoint, _objects) = bucket().await;
    let store = store(&endpoint);
    let (_root, path) = staged(b"an asset worth counting").await;
    store
        .store(&namespace("Blastlands"), OID, &path)
        .await
        .unwrap();

    let (objects, bytes) = store.usage_of(&namespace("Blastlands")).await;

    assert_eq!(objects, 1);
    assert_eq!(
        bytes, 23,
        "the marker is empty, so the size has to come from the content it points at — counting          the marker would report a repository holding nothing"
    );
}

#[tokio::test]
async fn an_object_id_that_could_not_be_one_is_refused_rather_than_slicing_into_it() {
    let (endpoint, _objects) = bucket().await;
    let store = store(&endpoint);
    let (_root, path) = staged(b"bytes nobody will store").await;

    // The fanout takes the first four characters of the digest, so anything
    // shorter is an index out of bounds — a panic where the client deserves a
    // refusal.
    for short in ["", "ab", "abc"] {
        assert!(store.size_of(short).await.is_err());
        assert!(store.read(short, 0, 1).await.is_err());
        assert!(
            store
                .store(&namespace("Blastlands"), short, &path)
                .await
                .is_err()
        );
    }
}
