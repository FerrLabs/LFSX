use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use futures_util::StreamExt;
use futures_util::stream;

use super::*;

const OID: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";
const CHUNK: usize = 1024;

fn namespace() -> Namespace {
    Namespace::new("FerrLabs", "Blastlands").unwrap()
}

fn body(
    chunks: usize,
) -> (
    impl Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
    Arc<AtomicUsize>,
) {
    let pulled = Arc::new(AtomicUsize::new(0));
    let counted = pulled.clone();

    let stream = stream::iter(0..chunks).map(move |_| {
        counted.fetch_add(1, Ordering::Relaxed);
        Ok(Bytes::from(vec![0u8; CHUNK]))
    });

    (stream, pulled)
}

pub(crate) fn staging_files(root: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut directories = vec![root.to_path_buf()];

    while let Some(directory) = directories.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "part")
            {
                found.push(path);
            }
        }
    }

    found
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_survives_a_collection_emptying_its_fanout() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalStore::new(root.path()));
    let ns = namespace();

    let collecting = {
        let store = store.clone();
        let ns = ns.clone();
        tokio::spawn(async move {
            for _ in 0..200 {
                let _ = store
                    .sweep(
                        &ns,
                        &std::collections::HashSet::new(),
                        Duration::ZERO,
                        false,
                    )
                    .await;
                tokio::task::yield_now().await;
            }
        })
    };

    for asset in 0..100u32 {
        let payload = format!("an asset worth pushing {asset}").into_bytes();
        let oid = hex::encode(Sha256::digest(&payload));

        let written = store
            .write(
                &ns,
                &oid,
                None,
                None,
                stream::iter([Ok::<_, std::io::Error>(Bytes::from(payload))]),
            )
            .await;

        assert!(
            written.is_ok(),
            "scheduled collection removes a fanout directory once it has emptied it, and an \
             upload that arrives in that window did nothing wrong: {written:?}"
        );
    }

    collecting.await.unwrap();
}

#[tokio::test]
async fn a_body_that_outgrows_the_limit_is_cut_off_rather_than_written_to_completion() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::new(root.path()).with_max_object_size(Some(4 * CHUNK as u64));
    let (body, pulled) = body(4096);

    let refused = store.write(&namespace(), OID, None, None, body).await;

    assert!(matches!(refused, Err(Error::TooLarge { limit: 4096 })));
    assert_eq!(
        pulled.load(Ordering::Relaxed),
        5,
        "the transfer has to stop at the chunk that crosses the line — reading to the end to \
         find out how big it was is the outage this limit exists to prevent"
    );
    assert!(
        staging_files(root.path()).is_empty(),
        "a refused transfer leaves nothing behind to reclaim later"
    );
}

#[tokio::test]
async fn a_declared_size_over_the_limit_never_touches_the_disk() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::new(root.path()).with_max_object_size(Some(1024));
    let (body, pulled) = body(1);

    let refused = store.write(&namespace(), OID, Some(4096), None, body).await;

    assert!(matches!(refused, Err(Error::TooLarge { limit: 1024 })));
    assert_eq!(
        pulled.load(Ordering::Relaxed),
        0,
        "the size is known before the body is read, so nothing is read"
    );
    assert!(!root.path().join("FerrLabs").exists());
}

#[tokio::test]
async fn an_object_at_the_limit_is_accepted() {
    let root = tempfile::tempdir().unwrap();
    let payload = vec![0u8; CHUNK];
    let oid = hex::encode(Sha256::digest(&payload));
    let store = LocalStore::new(root.path()).with_max_object_size(Some(CHUNK as u64));

    let written = store
        .write(
            &namespace(),
            &oid,
            Some(CHUNK as u64),
            None,
            stream::iter([Ok::<_, std::io::Error>(Bytes::from(payload))]),
        )
        .await;

    assert_eq!(
        written.unwrap().bytes,
        CHUNK as u64,
        "the limit is a ceiling, not a value to stay under"
    );
}

#[tokio::test]
async fn without_a_limit_a_large_object_still_goes_through() {
    let root = tempfile::tempdir().unwrap();
    let payload = vec![0u8; 64 * CHUNK];
    let oid = hex::encode(Sha256::digest(&payload));
    let store = LocalStore::new(root.path());

    let written = store
        .write(
            &namespace(),
            &oid,
            Some(payload.len() as u64),
            None,
            stream::iter([Ok::<_, std::io::Error>(Bytes::from(payload))]),
        )
        .await;

    assert_eq!(written.unwrap().bytes, 64 * CHUNK as u64);
}
