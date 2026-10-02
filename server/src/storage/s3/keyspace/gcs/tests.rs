use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};

use super::credential::{ServiceAccount, Source};
use super::*;

#[derive(Default)]
struct Bucket {
    objects: HashMap<String, Vec<u8>>,
    partial: Vec<u8>,
    ranges: Vec<String>,
    authorizations: Vec<Option<String>>,
}

type Shared = Arc<Mutex<Bucket>>;

fn seen(bucket: &mut Bucket, headers: &HeaderMap) {
    bucket.authorizations.push(
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
    );
}

async fn upload(
    State(bucket): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut bucket = bucket.lock().unwrap();
    seen(&mut bucket, &headers);
    let name = query.get("name").cloned().unwrap_or_default();

    match query.get("uploadType").map(String::as_str) {
        Some("resumable") => {
            bucket.partial.clear();
            (StatusCode::OK, [("location", format!("/session/{name}"))]).into_response()
        }
        Some("media") => {
            if query.get("ifGenerationMatch").map(String::as_str) == Some("0")
                && bucket.objects.contains_key(&name)
            {
                return StatusCode::PRECONDITION_FAILED.into_response();
            }
            bucket.objects.insert(name, body.to_vec());
            StatusCode::OK.into_response()
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn session(
    State(bucket): State<Shared>,
    axum::extract::Path(name): axum::extract::Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut bucket = bucket.lock().unwrap();
    seen(&mut bucket, &headers);
    let range = headers
        .get("content-range")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    bucket.ranges.push(range.clone());
    bucket.partial.extend_from_slice(&body);

    let total: usize = range
        .rsplit('/')
        .next()
        .and_then(|total| total.parse().ok())
        .unwrap_or(0);
    if bucket.partial.len() < total {
        return StatusCode::PERMANENT_REDIRECT.into_response();
    }

    let whole = std::mem::take(&mut bucket.partial);
    bucket.objects.insert(name, whole);
    StatusCode::OK.into_response()
}

async fn list(State(bucket): State<Shared>, headers: HeaderMap) -> Response {
    let mut bucket = bucket.lock().unwrap();
    seen(&mut bucket, &headers);
    axum::Json(serde_json::json!({ "items": [] })).into_response()
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("http://{address}")
}

async fn store() -> (String, Shared) {
    let bucket = Shared::default();
    let endpoint = serve(
        Router::new()
            .route("/upload/storage/v1/b/{bucket}/o", post(upload))
            .route("/session/{*name}", put(session))
            .route("/storage/v1/b/{bucket}/o", get(list))
            .with_state(bucket.clone()),
    )
    .await;

    (endpoint, bucket)
}

fn config(endpoint: &str) -> GcsConfig {
    GcsConfig {
        endpoint: endpoint.to_owned(),
        bucket: "assets".into(),
        credential: GcsCredential::Anonymous,
        lifetime: Duration::from_secs(1800),
    }
}

#[tokio::test]
async fn a_large_object_goes_up_in_chunks_that_name_their_place() {
    let (endpoint, bucket) = store().await;
    let keys = GcsKeys::new(&config(&endpoint)).unwrap();
    let payload: Vec<u8> = (0..150_000u32).flat_map(u32::to_le_bytes).collect();
    let root = tempfile::tempdir().unwrap();
    let staged = root.path().join("staged");
    std::fs::write(&staged, &payload).unwrap();

    keys.put_resumable(
        ".content/ab/cd/abcd",
        &staged,
        payload.len() as u64,
        256 * 1024,
    )
    .await
    .unwrap();

    let bucket = bucket.lock().unwrap();
    assert_eq!(
        bucket.objects.get(".content/ab/cd/abcd"),
        Some(&payload),
        "an object reassembled short of a chunk is a corrupt object under a digest that says \
         otherwise"
    );
    assert_eq!(
        bucket.ranges,
        vec![
            "bytes 0-262143/600000".to_owned(),
            "bytes 262144-524287/600000".to_owned(),
            "bytes 524288-599999/600000".to_owned(),
        ]
    );
}

#[tokio::test]
async fn the_second_writer_is_refused_by_generation_zero() {
    let (endpoint, _bucket) = store().await;
    let keys = GcsKeys::new(&config(&endpoint)).unwrap();

    assert!(
        keys.put_if_absent(".locks/one", b"first".to_vec())
            .await
            .unwrap()
    );
    assert!(
        !keys
            .put_if_absent(".locks/one", b"second".to_vec())
            .await
            .unwrap()
    );
}

async fn token_endpoint(State(issued): State<Arc<AtomicUsize>>, body: String) -> Response {
    let grant = body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer");
    let assertion = body
        .split("assertion=")
        .nth(1)
        .is_some_and(|jwt| jwt.split('.').count() == 3);
    if !grant || !assertion {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let count = issued.fetch_add(1, Ordering::SeqCst) + 1;
    axum::Json(serde_json::json!({ "access_token": format!("sa-{count}"), "expires_in": 3599 }))
        .into_response()
}

async fn metadata(State(issued): State<Arc<AtomicUsize>>, headers: HeaderMap) -> Response {
    if headers
        .get("metadata-flavor")
        .and_then(|value| value.to_str().ok())
        != Some("Google")
    {
        return StatusCode::FORBIDDEN.into_response();
    }

    let count = issued.fetch_add(1, Ordering::SeqCst) + 1;
    axum::Json(serde_json::json!({ "access_token": format!("md-{count}"), "expires_in": 3599 }))
        .into_response()
}

fn throwaway_account(token_uri: &str) -> ServiceAccount {
    use rsa::pkcs1::EncodeRsaPrivateKey;

    let key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, 2048).expect("a throwaway test key");
    let private = key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF).unwrap();

    ServiceAccount::from_json(
        &serde_json::json!({
            "client_email": "lfsx@project.iam.gserviceaccount.com",
            "private_key": private.as_str(),
            "token_uri": token_uri,
        })
        .to_string(),
    )
    .unwrap()
}

#[tokio::test]
async fn a_service_account_trades_a_signed_assertion_for_one_token_and_reuses_it() {
    let (endpoint, bucket) = store().await;
    let issued = Arc::new(AtomicUsize::new(0));
    let authority = serve(
        Router::new()
            .route("/token", post(token_endpoint))
            .with_state(issued.clone()),
    )
    .await;
    let account = throwaway_account(&format!("{authority}/token"));
    let keys = GcsKeys::with_credential(
        &config(&endpoint),
        Credential::from_source(Source::ServiceAccount(account)),
    )
    .unwrap();

    keys.reachable().await.unwrap();
    keys.reachable().await.unwrap();

    assert_eq!(issued.load(Ordering::SeqCst), 1);
    assert_eq!(
        bucket.lock().unwrap().authorizations,
        vec![
            Some("Bearer sa-1".to_owned()),
            Some("Bearer sa-1".to_owned())
        ]
    );
    assert!(
        keys.signed_download("objects/x").is_some(),
        "a service account key is what signs a download URL"
    );
}

#[tokio::test]
async fn workload_identity_asks_the_metadata_server_and_signs_nothing() {
    let (endpoint, bucket) = store().await;
    let issued = Arc::new(AtomicUsize::new(0));
    let server = serve(
        Router::new()
            .route("/token", get(metadata))
            .with_state(issued.clone()),
    )
    .await;
    let keys = GcsKeys::with_credential(
        &config(&endpoint),
        Credential::from_source(Source::Metadata {
            endpoint: format!("{server}/token"),
        }),
    )
    .unwrap();

    keys.reachable().await.unwrap();

    assert_eq!(
        bucket.lock().unwrap().authorizations,
        vec![Some("Bearer md-1".to_owned())]
    );
    assert_eq!(keys.signed_download("objects/x"), None);
}

#[tokio::test]
async fn an_emulator_is_asked_with_no_credentials() {
    let (endpoint, bucket) = store().await;

    GcsKeys::new(&config(&endpoint))
        .unwrap()
        .reachable()
        .await
        .unwrap();

    assert_eq!(bucket.lock().unwrap().authorizations, vec![None]);
}
