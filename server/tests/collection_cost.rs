// Not a correctness test: a stopwatch on collection, which had no measurement at
// all and so quietly grew to cost objects times repositories. Ignored by default
// because it builds a store of a few thousand files.
//
//   cargo test --test collection_cost --release -- --ignored --nocapture
//
// Run it on Linux to mean anything. The link count is what makes this cheap and
// only Unix has one, so on Windows this measures the fallback and moves barely
// at all between the two implementations.

use std::time::{Duration, Instant};

use lfsx_server::namespace::Namespace;
use lfsx_server::storage::LocalStore;
use sha2::{Digest, Sha256};

const REPOS: usize = 30;
const OBJECTS: usize = 400;

#[tokio::test]
#[ignore = "timing, not correctness"]
async fn a_dry_run_over_a_store_with_many_repositories() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::new(root.path());

    // One repository holds every object; the rest exist so the store has breadth.
    let swept = Namespace::new("FerrLabs", "Swept").unwrap();
    for index in 0..OBJECTS {
        let payload = format!("object {index}").into_bytes();
        let oid = lfsx_server::oid::Oid::parse(&hex::encode(Sha256::digest(&payload))).unwrap();
        store
            .write(
                &swept,
                &oid,
                None,
                None,
                futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                    payload,
                ))]),
            )
            .await
            .unwrap();
    }

    for repo in 0..REPOS {
        let ns = Namespace::new("FerrLabs", format!("Other{repo}").as_str()).unwrap();
        let payload = format!("filler {repo}").into_bytes();
        let oid = lfsx_server::oid::Oid::parse(&hex::encode(Sha256::digest(&payload))).unwrap();
        store
            .write(
                &ns,
                &oid,
                None,
                None,
                futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                    payload,
                ))]),
            )
            .await
            .unwrap();
    }

    let started = Instant::now();
    let report = store
        .sweep(&swept, &Default::default(), Duration::ZERO, true)
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(report.swept, OBJECTS);
    println!(
        "dry run over {OBJECTS} objects with {REPOS} other repositories: {:?} ({:?} per object)",
        elapsed,
        elapsed / OBJECTS as u32
    );
}
