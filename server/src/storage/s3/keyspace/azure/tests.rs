use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, post};

use super::credential::{Identity, Source};
use super::*;

#[derive(Default)]
struct Container {
    blobs: HashMap<String, Vec<u8>>,
    blocks: HashMap<String, Vec<u8>>,
    block_lists: Vec<Vec<String>>,
    bearers: Vec<Option<String>>,
}

type Shared = Arc<Mutex<Container>>;

async fn blob(
    State(container): State<Shared>,
    Path((_container, name)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut container = container.lock().unwrap();
    container.bearers.push(
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
    );

    match (method, query.get("comp").map(String::as_str)) {
        (Method::PUT, Some("block")) => {
            let id = query.get("blockid").cloned().unwrap_or_default();
            container.blocks.insert(id, body.to_vec());
            StatusCode::CREATED.into_response()
        }
        (Method::PUT, Some("blocklist")) => {
            let list = String::from_utf8_lossy(&body).into_owned();
            let ids: Vec<String> = list
                .split("<Latest>")
                .skip(1)
                .filter_map(|part| part.split("</Latest>").next())
                .map(str::to_owned)
                .collect();
            let assembled = ids
                .iter()
                .flat_map(|id| container.blocks.get(id).cloned().unwrap_or_default())
                .collect();
            container.blobs.insert(name, assembled);
            container.block_lists.push(ids);
            StatusCode::CREATED.into_response()
        }
        (Method::PUT, None) => {
            if headers.contains_key("if-none-match") && container.blobs.contains_key(&name) {
                return StatusCode::CONFLICT.into_response();
            }
            container.blobs.insert(name, body.to_vec());
            StatusCode::CREATED.into_response()
        }
        (Method::DELETE, None) => match container.blobs.remove(&name) {
            Some(_) => StatusCode::ACCEPTED.into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        (Method::GET, None) => match container.blobs.get(&name) {
            Some(bytes) => bytes.clone().into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn token(State(issued): State<Arc<AtomicUsize>>, body: String) -> Response {
    if !body.contains("client_assertion=assertion-from-the-cluster") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let count = issued.fetch_add(1, Ordering::SeqCst) + 1;

    axum::Json(serde_json::json!({
        "access_token": format!("token-{count}"),
        "expires_in": 3600,
    }))
    .into_response()
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("http://{address}")
}

async fn store() -> (String, Shared) {
    let container = Shared::default();
    let endpoint = serve(
        Router::new()
            .route("/{container}/{*name}", any(blob))
            .with_state(container.clone()),
    )
    .await;

    (endpoint, container)
}

fn config(endpoint: &str) -> AzureConfig {
    AzureConfig {
        endpoint: endpoint.to_owned(),
        account: "account".into(),
        container: "assets".into(),
        credential: AzureCredential::Sas("sv=2022-11-02&sig=stub".into()),
        lifetime: Duration::from_secs(1800),
    }
}

fn keys(endpoint: &str) -> AzureKeys {
    AzureKeys::new(&config(endpoint)).unwrap()
}

#[tokio::test]
async fn an_object_goes_up_in_blocks_and_comes_back_whole() {
    let (endpoint, container) = store().await;
    let payload: Vec<u8> = (0..40_000u32).flat_map(u32::to_le_bytes).collect();
    let root = tempfile::tempdir().unwrap();
    let staged = root.path().join("staged");
    std::fs::write(&staged, &payload).unwrap();

    keys(&endpoint)
        .put_in_blocks("a/b/object", &staged, payload.len() as u64, 50_000)
        .await
        .unwrap();

    let container = container.lock().unwrap();
    assert_eq!(
        container.blobs.get("a/b/object"),
        Some(&payload),
        "a blob assembled out of order, or short a block, is a corrupt object under a digest that \
         says otherwise"
    );
    let ids = &container.block_lists[0];
    assert_eq!(ids.len(), 4, "160000 bytes in 50000-byte blocks");
    assert!(
        ids.iter().all(|id| id.len() == ids[0].len()),
        "Azure refuses a block list whose ids differ in length"
    );
}

#[tokio::test]
async fn two_uploads_of_one_blob_never_share_a_block() {
    let (endpoint, container) = store().await;
    let keys = keys(&endpoint);
    let root = tempfile::tempdir().unwrap();
    let staged = root.path().join("staged");
    std::fs::write(&staged, vec![1u8; 120_000]).unwrap();

    for _ in 0..2 {
        keys.put_in_blocks("a/b/object", &staged, 120_000, 50_000)
            .await
            .unwrap();
    }

    let container = container.lock().unwrap();
    let (first, second) = (&container.block_lists[0], &container.block_lists[1]);
    assert!(
        first.iter().all(|id| !second.contains(id)),
        "uncommitted blocks live on the blob, so two pushes of the same object over the ceiling \
         would overwrite each other's blocks and commit a mix: with encryption that is two salts \
         in one blob that never decrypts"
    );
    assert!(second.iter().all(|id| id.len() == first[0].len()));
}

#[tokio::test]
async fn the_second_writer_is_refused_with_azures_conflict() {
    let (endpoint, _container) = store().await;
    let keys = keys(&endpoint);

    assert!(
        keys.put_if_absent(".locks/one", b"first".to_vec())
            .await
            .unwrap()
    );
    assert!(
        !keys
            .put_if_absent(".locks/one", b"second".to_vec())
            .await
            .unwrap(),
        "Azure answers 409 rather than 412 to `If-None-Match: *` on an existing blob, and reading \
         that as an error is a lock both clients are told they hold"
    );
}

#[tokio::test]
async fn deleting_what_is_not_there_says_so() {
    let (endpoint, _container) = store().await;
    let keys = keys(&endpoint);

    keys.put("present", reqwest::Body::from(b"x".to_vec()), 1)
        .await
        .unwrap();

    assert!(keys.delete("present").await.unwrap());
    assert!(!keys.delete("present").await.unwrap());
}

#[tokio::test]
async fn a_sas_token_is_never_handed_to_a_client() {
    assert_eq!(
        keys("http://127.0.0.1:1").signed_download("objects/x"),
        None,
        "a configured SAS token covers the whole container, so a URL carrying it would give a \
         client every object rather than the one it is entitled to"
    );
}

async fn identity_store() -> (AzureKeys, Shared, Arc<AtomicUsize>) {
    let (endpoint, container) = store().await;
    let issued = Arc::new(AtomicUsize::new(0));
    let authority = serve(
        Router::new()
            .route("/{tenant}/oauth2/v2.0/token", post(token))
            .with_state(issued.clone()),
    )
    .await;

    let root = tempfile::tempdir().unwrap().keep();
    let token_file = root.join("token");
    std::fs::write(&token_file, "assertion-from-the-cluster\n").unwrap();

    let identity = Identity::new(Source::Workload {
        authority,
        tenant: "tenant".into(),
        client_id: "client".into(),
        token_file,
    });
    let keys =
        AzureKeys::with_credential(&config(&endpoint), Credential::Identity(identity)).unwrap();

    (keys, container, issued)
}

#[tokio::test]
async fn workload_identity_trades_the_cluster_token_for_one_bearer_and_reuses_it() {
    let (keys, container, issued) = identity_store().await;

    keys.put("one", reqwest::Body::from(b"1".to_vec()), 1)
        .await
        .unwrap();
    keys.put("two", reqwest::Body::from(b"2".to_vec()), 1)
        .await
        .unwrap();

    assert_eq!(
        issued.load(Ordering::SeqCst),
        1,
        "a token per request is a round trip to Entra ID per request, and a throttled tenant"
    );
    assert_eq!(
        container.lock().unwrap().bearers,
        vec![
            Some("Bearer token-1".to_owned()),
            Some("Bearer token-1".to_owned())
        ]
    );
}

#[tokio::test]
async fn an_identity_signs_no_download_url() {
    let (keys, _container, _issued) = identity_store().await;

    assert_eq!(keys.signed_download("objects/x"), None);
}

type Asked = Arc<Mutex<Vec<(Option<String>, HashMap<String, String>)>>>;

async fn managed_token(
    State(asked): State<Asked>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let metadata = headers
        .get("metadata")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    asked.lock().unwrap().push((metadata, query));

    axum::Json(serde_json::json!({
        "access_token": "managed-token",
        "expires_in": "3600",
    }))
    .into_response()
}

#[tokio::test]
async fn a_managed_identity_is_asked_the_way_the_instance_metadata_service_expects() {
    let (endpoint, container) = store().await;
    let asked = Asked::default();
    let metadata = serve(
        Router::new()
            .route(
                "/metadata/identity/oauth2/token",
                axum::routing::get(managed_token),
            )
            .with_state(asked.clone()),
    )
    .await;
    let identity = Identity::new(Source::Managed {
        endpoint: format!("{metadata}/metadata/identity/oauth2/token"),
        client_id: Some("user-assigned".into()),
    });
    let keys =
        AzureKeys::with_credential(&config(&endpoint), Credential::Identity(identity)).unwrap();

    keys.put("one", reqwest::Body::from(b"1".to_vec()), 1)
        .await
        .unwrap();

    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    let (metadata, query) = &asked[0];
    assert_eq!(
        metadata.as_deref(),
        Some("true"),
        "the metadata service refuses a request without it, to stop SSRF through a proxy"
    );
    assert_eq!(query["resource"], "https://storage.azure.com/");
    assert_eq!(query["client_id"], "user-assigned");
    assert!(query.contains_key("api-version"));
    assert_eq!(
        container.lock().unwrap().bearers,
        vec![Some("Bearer managed-token".to_owned())]
    );
}

#[test]
fn an_account_key_that_is_not_base64_refuses_to_start() {
    let refused = AzureKeys::new(&AzureConfig {
        credential: AzureCredential::AccountKey("not base64 at all!".into()),
        ..config("http://127.0.0.1:9")
    });

    assert!(
        matches!(refused, Err(Error::Misconfigured(message)) if message.contains("not base64")),
        "a key that cannot sign anything should stop the server, not every request"
    );
}
