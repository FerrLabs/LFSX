use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path as UrlPath, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::any;

use super::*;
use crate::storage::s3::tests::keyspace;

#[derive(Default)]
struct Store {
    parts: HashMap<String, Vec<(u16, Vec<u8>)>>,
    objects: HashMap<String, Vec<u8>>,
    aborted: usize,
}

type Shared = Arc<Mutex<Store>>;

// Enough of the multipart API to prove the sequence: start, parts, assemble,
// abort. Kept apart from the object stub because none of this is a key
// operation, and because what has to be asserted here is the order rather than
// the contents of one key.
async fn bucket(refuse_part: Option<u16>) -> (String, Shared) {
    let store: Shared = Arc::new(Mutex::new(Store::default()));

    let app = Router::new()
        .route(
            "/{*key}",
            any(
                move |State(store): State<Shared>,
                      UrlPath(key): UrlPath<String>,
                      RawQuery(query): RawQuery,
                      method: axum::http::Method,
                      body: axum::body::Body| async move {
                    let query = query.unwrap_or_default();
                    let key = key.strip_prefix("assets/").unwrap_or(&key).to_owned();
                    let bytes = axum::body::to_bytes(body, usize::MAX)
                        .await
                        .unwrap_or_default();

                    if query.contains("uploads") && !query.contains("uploadId") {
                        store.lock().unwrap().parts.insert(key, Vec::new());

                        return (
                            StatusCode::OK,
                            "<?xml version=\"1.0\"?><InitiateMultipartUploadResult \
                             xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><UploadId>an-upload\
                             </UploadId></InitiateMultipartUploadResult>",
                        )
                            .into_response();
                    }

                    match method {
                        axum::http::Method::PUT => {
                            let number = part_number(&query);

                            if refuse_part == Some(number) {
                                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                            }

                            store
                                .lock()
                                .unwrap()
                                .parts
                                .entry(key)
                                .or_default()
                                .push((number, bytes.to_vec()));

                            (
                                StatusCode::OK,
                                [(axum::http::header::ETAG, format!("\"part-{number}\""))],
                            )
                                .into_response()
                        }
                        // Assembled in the order the parts were numbered, not the
                        // order they arrived, which is the guarantee the client
                        // is buying by naming them.
                        axum::http::Method::POST => {
                            let mut held = store.lock().unwrap();
                            let mut parts = held.parts.remove(&key).unwrap_or_default();
                            parts.sort_by_key(|(number, _)| *number);

                            let object: Vec<u8> =
                                parts.into_iter().flat_map(|(_, bytes)| bytes).collect();
                            held.objects.insert(key, object);

                            (StatusCode::OK, "<CompleteMultipartUploadResult/>").into_response()
                        }
                        axum::http::Method::DELETE => {
                            let mut held = store.lock().unwrap();
                            held.parts.remove(&key);
                            held.aborted += 1;

                            StatusCode::NO_CONTENT.into_response()
                        }
                        _ => StatusCode::NOT_FOUND.into_response(),
                    }
                },
            ),
        )
        .with_state(store.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    (format!("http://{address}"), store)
}

fn part_number(query: &str) -> u16 {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("partNumber="))
        .and_then(|number| number.parse().ok())
        .unwrap_or_default()
}

async fn staged(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("staged");
    tokio::fs::write(&path, bytes).await.unwrap();

    (root, path)
}

const KEY: &str = ".content/aa/bb/an-object";

// The whole point: the bytes come back in the right order, unaltered, from parts
// the store assembled. Driven with a small part size so the sequence is a real
// one rather than a single part pretending to be several.
#[tokio::test]
async fn an_object_goes_up_in_parts_and_the_store_puts_it_back_together() {
    crate::tls::install_crypto_provider();

    let payload: Vec<u8> = (0..40_000u32).flat_map(u32::to_le_bytes).collect();
    let (_root, path) = staged(&payload).await;
    let (endpoint, store) = bucket(None).await;
    let keys = keyspace(&endpoint);

    let upload = begin(&keys, KEY).await.unwrap();
    let etags = parts(&keys, KEY, &upload, &path, payload.len() as u64, 50_000)
        .await
        .unwrap();

    assert_eq!(etags.len(), 4, "160000 bytes in 50000-byte parts");

    finish(&keys, KEY, &upload, etags).await.unwrap();

    assert_eq!(
        store.lock().unwrap().objects.get(KEY),
        Some(&payload),
        "an object assembled out of order, or short a part, is a corrupt object under a digest that \
         says otherwise"
    );
}

// A part that fails takes the whole upload with it, and the upload has to be
// abandoned rather than left open. Parts of an incomplete upload are charged for
// and appear in no listing, so nothing else would ever find them.
#[tokio::test]
async fn a_failed_part_aborts_the_upload() {
    crate::tls::install_crypto_provider();

    let payload = vec![7u8; 160_000];
    let (_root, path) = staged(&payload).await;
    let (endpoint, store) = bucket(Some(3)).await;

    let failed = put_in_parts(
        &keyspace(&endpoint),
        KEY,
        &path,
        payload.len() as u64,
        50_000,
    )
    .await;

    assert!(failed.is_err(), "{failed:?}");

    let held = store.lock().unwrap();
    assert_eq!(held.aborted, 1, "the upload has to be abandoned explicitly");
    assert!(
        !held.objects.contains_key(KEY),
        "a key that appeared would be an object missing its third part"
    );
}

// The part size is what keeps the part count under S3's cap, so it grows with
// the object rather than staying put. A fixed size would be a second ceiling
// wearing a different number, which is the thing this module removes.
#[test]
fn the_part_size_grows_so_the_count_never_exceeds_the_cap() {
    for length in [
        SINGLE_PUT_CEILING + 1,
        100 * 1024 * 1024 * 1024,
        5 * 1024 * 1024 * 1024 * 1024,
    ] {
        let size = part_size(length);

        assert!(
            size >= SMALLEST_PART,
            "{length} gave a part below S3's floor"
        );
        assert!(
            length.div_ceil(size) <= MOST_PARTS,
            "{length} needs {} parts, and S3 allows {MOST_PARTS}",
            length.div_ceil(size)
        );
    }
}

// And it does not shrink below the default for an object that clears the ceiling
// by a little, which would turn a 6 GiB push into a thousand round trips.
#[test]
fn a_small_object_over_the_ceiling_still_uses_the_default_part_size() {
    assert_eq!(part_size(6 * 1024 * 1024 * 1024), PART);
}
