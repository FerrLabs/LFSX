// What opening a framed object in a bucket costs against a raw one. The format
// needs the header and the index before the first frame, so a framed read is
// three ranged GETs where a raw read is one. Ignored: it needs a bucket and it
// is a stopwatch, not an assertion.
//
//   LFSX_TEST_S3_ENDPOINT=http://127.0.0.1:9100 \
//     cargo test --test framed_cost --release -- --ignored --nocapture

mod common;

use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use lfsx_server::config::{Auth, Config, Storage};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const ROUNDS: u32 = 20;

fn app(root: &tempfile::TempDir, endpoint: &str, compression: Option<i32>) -> Router {
    lfsx_server::app(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        storage_root: root.path().to_path_buf(),
        public_url: Some("https://lfs.example".into()),
        action_lifetime: 1800,
        gc_grace: Duration::from_secs(14 * 24 * 60 * 60),
        staging_max_age: Duration::from_secs(86400),
        lock_max_age: None,
        max_object_size: None,
        repo_quota: None,
        compression,
        encryption_key_file: None,
        storage: Storage::Bucket {
            endpoint: endpoint.to_owned(),
            bucket: "lfsx-test".into(),
            region: "us-east-1".into(),
            access_key: "lfsxkey".into(),
            secret_key: "lfsxsecret".into(),
            path_style: true,
            presign: false,
            locking: true,
        },
        auth: Auth::Disabled,
    })
}

#[tokio::test]
#[ignore = "timing, needs a bucket"]
async fn a_framed_read_against_a_raw_one() {
    let Ok(endpoint) = std::env::var("LFSX_TEST_S3_ENDPOINT") else {
        eprintln!("skipped: set LFSX_TEST_S3_ENDPOINT");
        return;
    };

    // Three shapes, because the answer depends on the payload: framing is two
    // extra ranged GETs, and whether that matters is set against what the frames
    // save on the wire. An already-compressed asset saves nothing and still pays.
    for (label, compression, compressible) in [
        ("raw", None, true),
        ("framed", Some(3), true),
        ("framed-incompressible", Some(3), false),
    ] {
        let root = tempfile::tempdir().unwrap();
        let run = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let payload = if compressible {
            format!("a mesh for {label} on {run}: ")
                .repeat(40_000)
                .into_bytes()
        } else {
            let mut state = 0x2545_F491_4F6C_DD1Du64 ^ run as u64;
            (0..1_200_000)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state >> 24) as u8
                })
                .collect()
        };
        let oid = hex::encode(Sha256::digest(&payload));
        let repo = format!("Cost{label}{run}");

        let request = Request::builder()
            .method("PUT")
            .uri(format!("/FerrLabs/{repo}/objects/{oid}"))
            .header("content-length", payload.len())
            .body(Body::from(payload.clone()))
            .unwrap();
        app(&root, &endpoint, compression)
            .oneshot(request)
            .await
            .unwrap();

        let started = Instant::now();
        for _ in 0..ROUNDS {
            let request = Request::builder()
                .uri(format!("/FerrLabs/{repo}/objects/{oid}"))
                .body(Body::empty())
                .unwrap();
            let response = app(&root, &endpoint, compression)
                .oneshot(request)
                .await
                .unwrap();
            let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        }

        println!(
            "{label}: {:?} per download of {} bytes",
            started.elapsed() / ROUNDS,
            payload.len()
        );
    }
}
