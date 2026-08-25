use futures_util::StreamExt;

use super::*;
use crate::storage::crypt::Keyring;
use crate::storage::s3::tests::{bucket, redirecting, store};

fn keyring() -> std::sync::Arc<Keyring> {
    std::sync::Arc::new(Keyring::parse(&hex::encode([7u8; crate::storage::crypt::KEY])).unwrap())
}

fn namespace() -> Namespace {
    Namespace::new("FerrLabs", "Blastlands").unwrap()
}

fn bucket_store(root: &tempfile::TempDir, endpoint: &str) -> Store {
    Store::bucket(store(endpoint), LocalStore::new(root.path()))
}

async fn read_back(store: &Store, ns: &Namespace, oid: &str) -> Vec<u8> {
    let object = store.open(ns, oid).await.unwrap();
    let size = object.size();
    let mut chunks = object.stream(0, size).await.unwrap();
    let mut out = Vec::new();

    while let Some(chunk) = chunks.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }

    out
}

#[tokio::test]
async fn an_upload_lands_in_the_bucket_and_reads_back_through_the_same_seam() {
    let root = tempfile::tempdir().unwrap();
    let (endpoint, _objects) = bucket().await;
    let store = bucket_store(&root, &endpoint);
    let payload = b"an asset that never touches this disk for long".repeat(32);
    let oid = hex::encode(sha2::Sha256::digest(&payload));

    let written = store
        .write(
            &namespace(),
            &oid,
            Some(payload.len() as u64),
            None,
            futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                payload.clone(),
            ))]),
        )
        .await
        .unwrap();

    assert_eq!(written, payload.len() as u64);
    assert!(store.exists(&namespace(), &oid).await);

    assert_eq!(read_back(&store, &namespace(), &oid).await, payload);
}

// Compression and a bucket are configured independently, and a studio that
// turns both on gets no warning from either. What lands under the digest
// has to be the object, because the only thing that will ever read it back
// is a client that asked for those bytes by that name.
#[tokio::test]
async fn a_bucket_holds_the_object_even_when_the_server_was_told_to_compress() {
    let root = tempfile::tempdir().unwrap();
    let (endpoint, _objects) = bucket().await;
    let store = Store::bucket(
        store(&endpoint),
        LocalStore::new(root.path()).with_compression(Some(3)),
    );
    let payload = b"a mesh that gives up most of its ground to zstd ".repeat(4096);
    let oid = hex::encode(sha2::Sha256::digest(&payload));

    store
        .write(
            &namespace(),
            &oid,
            Some(payload.len() as u64),
            None,
            futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                payload.clone(),
            ))]),
        )
        .await
        .unwrap();

    let restored = read_back(&store, &namespace(), &oid).await;

    assert_eq!(
        hex::encode(sha2::Sha256::digest(&restored)),
        oid,
        "the client asked for the object named by this digest and has no way to know the              server framed it on the way past: {} bytes came back",
        restored.len()
    );
    assert_eq!(restored, payload);
}

#[tokio::test]
async fn the_staging_file_does_not_outlive_the_upload() {
    let root = tempfile::tempdir().unwrap();
    let (endpoint, _objects) = bucket().await;
    let store = bucket_store(&root, &endpoint);
    let payload = b"an asset passing through".to_vec();
    let oid = hex::encode(sha2::Sha256::digest(&payload));

    store
        .write(
            &namespace(),
            &oid,
            None,
            None,
            futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(payload))]),
        )
        .await
        .unwrap();

    let leftovers = crate::storage::tests::staging_files(root.path());
    assert!(
        leftovers.is_empty(),
        "local disk is a write buffer here, and one that is never emptied is a disk that \
         fills: {leftovers:?}"
    );
}

#[tokio::test]
async fn a_bucket_reports_no_capacity_rather_than_an_empty_one() {
    let root = tempfile::tempdir().unwrap();
    let (endpoint, _objects) = bucket().await;

    assert!(
        bucket_store(&root, &endpoint).capacity().await.is_none(),
        "zero would be read as an empty store by every dashboard that averages it"
    );
    assert!(
        Store::local(LocalStore::new(root.path()))
            .capacity()
            .await
            .is_some()
    );
}

#[tokio::test]
async fn the_maintenance_commands_say_they_do_not_apply_rather_than_lying() {
    let root = tempfile::tempdir().unwrap();
    let (endpoint, _objects) = bucket().await;
    let store = bucket_store(&root, &endpoint);
    let ns = namespace();

    for outcome in [
        store.dedupe(&ns, true).await.err(),
        store.compress(&ns, true).await.err(),
        store.verify(&ns).await.err(),
    ] {
        assert!(
            matches!(outcome, Some(Error::Unsupported(_))),
            "an operator running one of these against a bucket has to be told it did nothing,                  not handed an empty report that reads like success: {outcome:?}"
        );
    }

    // Collection is the one that no longer belongs in that list.
    assert!(
        store
            .sweep(&ns, &std::collections::HashSet::new(), Duration::ZERO, true)
            .await
            .is_ok(),
        "collection is implemented for a bucket and must not answer Unsupported"
    );
}

// The bug this guards. A pre-signed download hands the client the bucket key
// itself, which is only the object while nothing framed it on the way in.
// `presigned_upload` has refused to sign an upload under a key since
// encryption landed, for exactly this reason; the download side had no such
// guard and handed out frames.
//
// Asserted against what is actually in the bucket rather than against the
// flag, so this fails if framing ever stops happening and the guard becomes
// theatre.
#[tokio::test]
async fn a_download_is_never_redirected_to_a_frame() {
    for label in ["compressed", "encrypted"] {
        let root = tempfile::tempdir().unwrap();
        let (endpoint, objects) = bucket().await;
        let staging = match label {
            "compressed" => LocalStore::new(root.path()).with_compression(Some(3)),
            _ => LocalStore::new(root.path()).with_encryption(Some(keyring())),
        };
        let store = Store::bucket(redirecting(&endpoint), staging);

        let payload = b"a scene file that compresses and must still come back whole ".repeat(512);
        let oid = hex::encode(sha2::Sha256::digest(&payload));

        store
            .write(
                &namespace(),
                &oid,
                Some(payload.len() as u64),
                None,
                futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                    payload.clone(),
                ))]),
            )
            .await
            .unwrap();

        let stored = objects
            .lock()
            .unwrap()
            .values()
            .find(|object| !object.is_empty())
            .cloned()
            .unwrap();

        assert_ne!(
            stored, payload,
            "{label}: the bucket holds a frame, which is the premise of the rest of this test"
        );
        assert_eq!(
            store.redirect(&oid),
            None,
            "{label}: a client sent to the bucket would hash {} bytes of frame and reject the \
             object it asked for",
            stored.len()
        );
        assert_eq!(
            read_back(&store, &namespace(), &oid).await,
            payload,
            "{label}: giving up the redirect is only correct because the streamed path decodes"
        );
    }
}

// And the guard does not quietly disable the feature it protects. With no
// codec configured the bucket holds the object itself, so the redirect is
// exactly what the operator asked for.
#[tokio::test]
async fn a_bucket_holding_the_object_itself_still_redirects() {
    let root = tempfile::tempdir().unwrap();
    let (endpoint, _objects) = bucket().await;
    let store = Store::bucket(redirecting(&endpoint), LocalStore::new(root.path()));

    let payload = b"an object stored as it arrived".repeat(32);
    let oid = hex::encode(sha2::Sha256::digest(&payload));

    store
        .write(
            &namespace(),
            &oid,
            Some(payload.len() as u64),
            None,
            futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                payload.clone(),
            ))]),
        )
        .await
        .unwrap();

    assert!(store.redirect(&oid).is_some());
}
