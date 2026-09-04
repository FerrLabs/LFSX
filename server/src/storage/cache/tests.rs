use super::Cache;
use crate::oid::Oid;

fn oid(seed: u8) -> Oid {
    Oid::parse(&hex::encode([seed; 32])).expect("a digest")
}

fn cache(root: &tempfile::TempDir, ceiling: u64) -> Cache {
    Cache::new(root.path().join("cache"), ceiling).expect("a cache directory")
}

async fn fill(cache: &Cache, oid: &Oid, bytes: &[u8]) {
    let chunk = axum::body::Bytes::copy_from_slice(bytes);
    cache
        .fill(
            oid,
            bytes.len() as u64,
            futures_util::stream::once(async { Ok(chunk) }),
        )
        .await
        .expect("the object is cached");
}

async fn read(file: tokio::fs::File) -> Vec<u8> {
    use tokio::io::AsyncReadExt;

    let mut bytes = Vec::new();
    tokio::io::BufReader::new(file)
        .read_to_end(&mut bytes)
        .await
        .expect("the cached file reads");

    bytes
}

#[tokio::test]
async fn an_object_comes_back_byte_for_byte() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache(&root, 1 << 20);
    let payload = b"a mesh nobody wants to fetch twice".repeat(64);

    fill(&cache, &oid(1), &payload).await;
    let cached = cache.open(&oid(1)).await.expect("the object is cached");

    assert_eq!(read(cached).await, payload);
}

#[tokio::test]
async fn an_object_that_was_never_cached_is_a_miss() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache(&root, 1 << 20);

    assert!(cache.open(&oid(2)).await.is_none());
    assert_eq!(cache.stats().misses, 1);
}

// The one failure a cache must not turn into a corrupt download. Nothing here
// hashes to the object identifier, because what is cached is the stored form,
// so the digest written beside the bytes is the only thing that can catch a
// truncated or rotted entry.
#[tokio::test]
async fn a_corrupted_entry_is_discarded_rather_than_served() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache(&root, 1 << 20);
    let payload = b"an asset that will be tampered with".to_vec();

    fill(&cache, &oid(3), &payload).await;
    let path = root.path().join("cache").join(oid(3).to_string());
    tokio::fs::write(&path, b"not what was stored")
        .await
        .unwrap();

    assert!(
        cache.open(&oid(3)).await.is_none(),
        "a cached file that fails its digest must never reach a client"
    );
    assert!(
        !path.exists(),
        "and it must be gone, so the next reader fills it from the bucket again"
    );
}

// A truncated file is what a crash mid-write would leave if the rename were not
// the thing that publishes an entry, so it is worth pinning separately.
#[tokio::test]
async fn a_truncated_entry_is_discarded_too() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache(&root, 1 << 20);

    fill(&cache, &oid(4), &b"a whole object".repeat(32)).await;
    let path = root.path().join("cache").join(oid(4).to_string());
    tokio::fs::write(&path, b"a whole").await.unwrap();

    assert!(cache.open(&oid(4)).await.is_none());
}

// The ceiling is the promise that this cannot fill the volume the server also
// stages uploads on.
#[tokio::test]
async fn the_ceiling_evicts_what_was_used_longest_ago() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache(&root, 200);
    let payload = [7u8; 100];

    fill(&cache, &oid(5), &payload).await;
    fill(&cache, &oid(6), &payload).await;

    // Reaching for the older one makes it the recent one, so the next fill has
    // to take the other.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(cache.open(&oid(5)).await.is_some());

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    fill(&cache, &oid(7), &payload).await;

    assert!(
        cache.open(&oid(5)).await.is_some(),
        "the object somebody just read is the one worth keeping"
    );
    assert!(
        cache.open(&oid(6)).await.is_none(),
        "and the one nobody has touched since it arrived is the one to drop"
    );
    assert!(cache.stats().bytes <= 200);
}

// Two clients racing for the same cold object must produce one fetch, not two.
#[tokio::test]
async fn only_one_caller_fills_an_object() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache(&root, 1 << 20);

    assert!(cache.claim(&oid(8)));
    assert!(
        !cache.claim(&oid(8)),
        "the second reader leaves the fetch to the first"
    );

    cache.release(&oid(8));
    assert!(
        cache.claim(&oid(8)),
        "and once it is done the object can be filled again"
    );
}

// An object nothing could keep must not be fetched at all: filling it would
// evict every other entry and then itself, and the next download would do it
// again.
#[tokio::test]
async fn an_object_larger_than_the_ceiling_is_not_worth_caching() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache(&root, 100);

    assert!(cache.fits(100));
    assert!(!cache.fits(101));
}

// A body that stops early without an error would otherwise be cached as a whole
// object whose digest matches its own truncation.
#[tokio::test]
async fn a_short_body_is_refused_rather_than_cached() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache(&root, 1 << 20);
    let chunk = axum::body::Bytes::from_static(b"only half of it");

    let filled = cache
        .fill(
            &oid(9),
            1024,
            futures_util::stream::once(async { Ok(chunk) }),
        )
        .await;

    assert!(filled.is_err());
    assert!(cache.open(&oid(9)).await.is_none());
    assert_eq!(cache.stats().bytes, 0);
}
