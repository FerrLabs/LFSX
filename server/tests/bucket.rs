// These run against a real S3 implementation, which is the only place the
// claims this backend makes can be checked at all. The stub the unit tests use
// proves the layout and the wire format; it cannot prove that SigV4 binds the
// method, that a pre-signed URL is honoured with no credentials, or that a
// prefix listing answers the way the usage figures assume — because it is a
// stub, and it agrees with whatever it is sent.
//
// Point LFSX_TEST_S3_ENDPOINT at MinIO, Garage or AWS and they run. CI always
// sets it. Locally:
//
//   docker run -d -p 9100:9000 -e MINIO_ROOT_USER=lfsxkey \
//     -e MINIO_ROOT_PASSWORD=lfsxsecret quay.io/minio/minio server /data
//   LFSX_TEST_S3_ENDPOINT=http://127.0.0.1:9100 cargo test --test bucket

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use lfsx_server::config::{Auth, Config, Storage};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

struct Bucket {
    endpoint: String,
    bucket: String,
    access_key: String,
    secret_key: String,
}

fn configured() -> Option<Bucket> {
    let endpoint = std::env::var("LFSX_TEST_S3_ENDPOINT")
        .ok()
        .filter(|value| !value.is_empty())?;

    Some(Bucket {
        endpoint,
        bucket: std::env::var("LFSX_TEST_S3_BUCKET").unwrap_or_else(|_| "lfsx-test".into()),
        access_key: std::env::var("LFSX_TEST_S3_ACCESS_KEY").unwrap_or_else(|_| "lfsxkey".into()),
        secret_key: std::env::var("LFSX_TEST_S3_SECRET_KEY")
            .unwrap_or_else(|_| "lfsxsecret".into()),
    })
}

macro_rules! bucket_or_skip {
    () => {
        match configured() {
            Some(bucket) => bucket,
            None => {
                eprintln!("skipped: set LFSX_TEST_S3_ENDPOINT to run this against a real bucket");
                return;
            }
        }
    };
}

fn app(root: &tempfile::TempDir, bucket: &Bucket, presign: bool) -> Router {
    lfsx_server::app(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        storage_root: root.path().to_path_buf(),
        public_url: Some("https://lfs.example".into()),
        action_lifetime: 1800,
        gc_grace: Duration::from_secs(14 * 24 * 60 * 60),
        staging_max_age: Duration::from_secs(86400),
        max_object_size: None,
        repo_quota: None,
        compression: None,
        encryption_key_file: None,
        storage: Storage::Bucket {
            endpoint: bucket.endpoint.clone(),
            bucket: bucket.bucket.clone(),
            region: "us-east-1".into(),
            access_key: bucket.access_key.clone(),
            secret_key: bucket.secret_key.clone(),
            path_style: true,
            presign,
        },
        auth: Auth::Disabled,
    })
}

// Distinct bytes per test and per run. Per test so nothing here depends on the
// bucket being empty when it starts; per run because an object already under
// its content key is not uploaded again, so a fixed payload would quietly stop
// exercising the upload path from the second run onwards.
fn payload(what: &str) -> Vec<u8> {
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    format!("an asset that only {what} pushes, run {run}: ")
        .repeat(4096)
        .into_bytes()
}

fn oid_of(payload: &[u8]) -> String {
    hex::encode(Sha256::digest(payload))
}

async fn push(app: Router, repo: &str, oid: &str, payload: &[u8]) -> StatusCode {
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/FerrLabs/{repo}/objects/{oid}"))
        .header("content-length", payload.len())
        .body(Body::from(payload.to_vec()))
        .unwrap();

    app.oneshot(request).await.unwrap().status()
}

async fn batch(app: Router, repo: &str, oid: &str, size: usize) -> serde_json::Value {
    let body = serde_json::json!({
        "operation": "download",
        "transfers": ["basic"],
        "objects": [{ "oid": oid, "size": size }]
    });
    let request = Request::builder()
        .method("POST")
        .uri(format!("/FerrLabs/{repo}/objects/batch"))
        .header("content-type", "application/vnd.git-lfs+json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn an_object_pushed_through_the_server_comes_back_byte_for_byte() {
    let bucket = bucket_or_skip!();
    let root = tempfile::tempdir().unwrap();
    let payload = payload("the streamed path");
    let oid = oid_of(&payload);

    assert_eq!(
        push(app(&root, &bucket, false), "Streamed", &oid, &payload).await,
        StatusCode::OK
    );

    let request = Request::builder()
        .uri(format!("/FerrLabs/Streamed/objects/{oid}"))
        .body(Body::empty())
        .unwrap();
    let response = app(&root, &bucket, false).oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let restored = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(oid_of(&restored), oid);
}

// The whole point of the redirect: the client fetches from the bucket with no
// credentials of any kind, because the URL carries its own. If this ever comes
// back needing an Authorization header, `authenticated: true` becomes the lie
// that sends git-lfs into a 401 loop.
#[tokio::test]
async fn a_pre_signed_download_is_fetched_with_no_credentials_at_all() {
    let bucket = bucket_or_skip!();
    let root = tempfile::tempdir().unwrap();
    let payload = payload("the redirected path");
    let oid = oid_of(&payload);

    push(app(&root, &bucket, true), "Redirected", &oid, &payload).await;
    let answer = batch(app(&root, &bucket, true), "Redirected", &oid, payload.len()).await;

    assert_eq!(
        answer["objects"][0]["authenticated"],
        serde_json::json!(true)
    );
    let href = answer["objects"][0]["actions"]["download"]["href"]
        .as_str()
        .expect("a redirecting server hands out an href");
    assert!(
        href.starts_with(&bucket.endpoint),
        "the client is being sent to the bucket, not back here: {href}"
    );

    let fetched = reqwest::Client::new()
        .get(href)
        .send()
        .await
        .expect("the bucket answered");

    assert_eq!(fetched.status(), 200, "{href}");
    assert_eq!(oid_of(&fetched.bytes().await.unwrap()), oid);
}

// A range against the pre-signed URL is what a resumed clone does, and the
// bucket is the one serving it once the redirect is on.
#[tokio::test]
async fn a_pre_signed_url_serves_a_range() {
    let bucket = bucket_or_skip!();
    let root = tempfile::tempdir().unwrap();
    let payload = payload("a resumed clone");
    let oid = oid_of(&payload);

    push(app(&root, &bucket, true), "Resumed", &oid, &payload).await;
    let answer = batch(app(&root, &bucket, true), "Resumed", &oid, payload.len()).await;
    let href = answer["objects"][0]["actions"]["download"]["href"]
        .as_str()
        .unwrap()
        .to_owned();

    let fetched = reqwest::Client::new()
        .get(&href)
        .header("range", "bytes=4096-8191")
        .send()
        .await
        .unwrap();

    assert_eq!(fetched.status(), 206, "{href}");
    assert_eq!(fetched.bytes().await.unwrap(), payload[4096..8192]);
}

// The marker is the proof of possession, and a signature is cut after it is
// checked rather than instead of it. Otherwise knowing a digest — which is all
// a leaked pointer file is — would be enough to read another project's assets
// out of a keyspace they share.
#[tokio::test]
async fn a_repository_that_never_pushed_the_object_is_signed_nothing() {
    let bucket = bucket_or_skip!();
    let root = tempfile::tempdir().unwrap();
    let payload = payload("one project only");
    let oid = oid_of(&payload);

    push(app(&root, &bucket, true), "Owner", &oid, &payload).await;
    let answer = batch(app(&root, &bucket, true), "Stranger", &oid, payload.len()).await;

    assert_eq!(answer["objects"][0]["error"]["code"], 404);
    assert!(answer["objects"][0]["actions"].is_null());
    assert!(answer["objects"][0]["authenticated"].is_null());
}

// Two repositories pushing the same bytes pay the bucket once, and each holds
// its own marker. The unit tests assert this against a stub that stores what it
// is sent; here it is the real keyspace.
#[tokio::test]
async fn two_repositories_sharing_an_asset_store_it_once() {
    let bucket = bucket_or_skip!();
    let root = tempfile::tempdir().unwrap();
    let payload = payload("both projects");
    let oid = oid_of(&payload);

    push(app(&root, &bucket, false), "First", &oid, &payload).await;
    push(app(&root, &bucket, false), "Second", &oid, &payload).await;

    for repo in ["First", "Second"] {
        let answer = batch(app(&root, &bucket, false), repo, &oid, payload.len()).await;
        assert!(
            answer["objects"][0]["error"].is_null(),
            "{repo} pushed it and should hold it: {answer}"
        );
    }
}
