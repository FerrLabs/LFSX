use super::*;
use crate::storage::LocalStore;

fn store() -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::local(LocalStore::new(root.path()));
    (root, store)
}

async fn admitted(registry: &Registry, store: &Store, token: &str) -> bool {
    registry.holder(store, token).await.unwrap().is_some()
}

#[tokio::test(start_paused = true)]
async fn a_token_revoked_here_stops_working_at_once() {
    let (_root, store) = store();
    let registry = Registry::default();
    let token = registry.create(&store, "alice").await.unwrap();
    let hash = registry.holder(&store, &token).await.unwrap().unwrap().hash;
    assert!(registry.still_issued(&store, &hash).await.unwrap());

    registry.revoke(&store, "alice").await.unwrap();

    assert!(!admitted(&registry, &store, &token).await);
    assert!(!registry.still_issued(&store, &hash).await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn a_token_created_elsewhere_is_picked_up_after_the_refresh() {
    let (_root, store) = store();
    let registry = Registry::default();
    assert!(!admitted(&registry, &store, "lfsx_unknown").await);

    let token = create(&store, "alice").await.unwrap();
    assert!(!admitted(&registry, &store, &token).await);

    tokio::time::advance(EVERY).await;
    assert!(admitted(&registry, &store, &token).await);
}

#[tokio::test(start_paused = true)]
async fn a_token_revoked_elsewhere_stops_working_after_the_refresh() {
    let (_root, store) = store();
    let registry = Registry::default();
    let token = create(&store, "alice").await.unwrap();
    assert!(admitted(&registry, &store, &token).await);

    revoke(&store, "alice").await.unwrap();
    tokio::time::advance(EVERY).await;

    assert!(!admitted(&registry, &store, &token).await);
}

#[tokio::test(start_paused = true)]
async fn unknown_tokens_do_not_read_the_store_each_time() {
    let (_root, store) = store();
    let registry = Registry::default();
    assert!(!admitted(&registry, &store, "lfsx_unknown").await);
    store.write_meta(FILE, b"not json".to_vec()).await.unwrap();

    for attempt in 0..25 {
        assert!(
            !admitted(&registry, &store, &format!("lfsx_guess{attempt}")).await,
            "{attempt}"
        );
    }
}
