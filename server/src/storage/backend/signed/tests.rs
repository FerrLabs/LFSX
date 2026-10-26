use super::*;
use crate::storage::LocalStore;

const KEY: [u8; 32] = [7; 32];

fn store(root: &tempfile::TempDir, key: Option<[u8; 32]>) -> Store {
    Store::local(LocalStore::new(root.path())).with_signing(key)
}

#[tokio::test]
async fn a_signed_file_reads_back_as_it_was_written() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root, Some(KEY));

    store
        .write_signed("access.json", br#"{"anonymous_read":false}"#.to_vec())
        .await
        .unwrap();

    let read = store.read_signed("access.json").await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&read).unwrap(),
        serde_json::json!({"anonymous_read": false})
    );
    assert_eq!(store.read_signed("missing.json").await.unwrap(), None);
}

#[tokio::test]
async fn a_signed_file_whose_payload_was_edited_in_the_store_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root, Some(KEY));
    store
        .write_signed("access.json", br#"{"anonymous_read":false}"#.to_vec())
        .await
        .unwrap();

    let path = root.path().join(".lfsx/access.json");
    let forged = std::fs::read_to_string(&path)
        .unwrap()
        .replace("false", "true");
    std::fs::write(&path, forged).unwrap();

    assert!(matches!(
        store.read_signed("access.json").await,
        Err(Error::Tampered)
    ));
}

#[tokio::test]
async fn a_file_signed_for_another_name_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root, Some(KEY));
    store
        .write_signed("dashboard-tokens.json", b"[]".to_vec())
        .await
        .unwrap();

    std::fs::copy(
        root.path().join(".lfsx/dashboard-tokens.json"),
        root.path().join(".lfsx/access.json"),
    )
    .unwrap();

    assert!(matches!(
        store.read_signed("access.json").await,
        Err(Error::Tampered)
    ));
}

#[tokio::test]
async fn a_file_signed_with_another_key_is_refused() {
    let root = tempfile::tempdir().unwrap();
    store(&root, Some([9; 32]))
        .write_signed("access.json", b"{}".to_vec())
        .await
        .unwrap();

    assert!(matches!(
        store(&root, Some(KEY)).read_signed("access.json").await,
        Err(Error::Tampered)
    ));
}

#[tokio::test]
async fn an_unsigned_file_is_refused_once_a_key_is_configured() {
    let root = tempfile::tempdir().unwrap();
    store(&root, None)
        .write_signed("access.json", br#"{"anonymous_read":true}"#.to_vec())
        .await
        .unwrap();

    assert!(matches!(
        store(&root, Some(KEY)).read_signed("access.json").await,
        Err(Error::Misconfigured(_))
    ));
}

#[tokio::test]
async fn without_a_key_files_stay_plain_json() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root, None);

    store
        .write_signed("access.json", br#"{"anonymous_read":true}"#.to_vec())
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(root.path().join(".lfsx/access.json")).unwrap(),
        br#"{"anonymous_read":true}"#
    );
    assert_eq!(
        store.read_signed("access.json").await.unwrap().unwrap(),
        br#"{"anonymous_read":true}"#
    );
}
