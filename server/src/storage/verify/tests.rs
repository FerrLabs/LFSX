use std::path::{Path, PathBuf};

use super::*;

fn namespace() -> Namespace {
    Namespace::new("FerrLabs", "Blastlands").unwrap()
}

fn mesh(len: usize) -> Vec<u8> {
    b"vertex 0.7071 0.0000 0.7071 normal 0.0000 1.0000 0.0000 "
        .iter()
        .cycle()
        .take(len)
        .copied()
        .collect()
}

fn fanout(root: &Path, oid: &str) -> PathBuf {
    let fanout = root
        .join("FerrLabs/Blastlands")
        .join(&oid[0..2])
        .join(&oid[2..4]);
    std::fs::create_dir_all(&fanout).unwrap();
    fanout
}

fn store_uncompressed(root: &Path, payload: &[u8]) -> PathBuf {
    let oid = hex::encode(Sha256::digest(payload));
    let path = fanout(root, &oid).join(&oid);
    std::fs::write(&path, payload).unwrap();
    path
}

#[tokio::test]
async fn an_intact_store_reports_nothing() {
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(3 * 1024 * 1024);
    store_uncompressed(root.path(), &payload);

    let report = LocalStore::new(root.path())
        .verify(&namespace())
        .await
        .unwrap();

    assert_eq!(
        report,
        VerifyReport {
            checked: 1,
            bytes: payload.len() as u64,
            corrupt: Vec::new(),
            unreadable: Vec::new(),
            incomplete: false,
        }
    );
}

#[tokio::test]
async fn a_compressed_object_is_checked_through_its_own_bytes() {
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(9 * 1024 * 1024);
    store_uncompressed(root.path(), &payload);
    let store = LocalStore::new(root.path()).with_compression(Some(3));
    store.compress(&namespace(), false).await.unwrap();

    let report = store.verify(&namespace()).await.unwrap();

    assert_eq!(
        (report.checked, report.bytes),
        (1, payload.len() as u64),
        "the check has to see the object, not the file — a compressed store is exactly where \
         rehashing the file on disk stops meaning anything: {report:?}"
    );
    assert!(report.corrupt.is_empty());
}

#[tokio::test]
async fn a_corrupt_object_is_named() {
    let root = tempfile::tempdir().unwrap();
    let oid = hex::encode(Sha256::digest(b"what this object is supposed to be"));
    std::fs::write(
        fanout(root.path(), &oid).join(&oid),
        b"and what it now holds",
    )
    .unwrap();

    let report = LocalStore::new(root.path())
        .verify(&namespace())
        .await
        .unwrap();

    assert_eq!(report.corrupt, vec![oid]);
    assert!(
        report.unreadable.is_empty(),
        "it read fine, it just is not what it claims"
    );
}

#[tokio::test]
async fn a_compressed_object_whose_frames_are_damaged_is_reported_rather_than_thrown() {
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(5 * 1024 * 1024);
    let path = store_uncompressed(root.path(), &payload);
    let store = LocalStore::new(root.path()).with_compression(Some(3));
    store.compress(&namespace(), false).await.unwrap();

    let mut damaged = std::fs::read(&path).unwrap();
    let middle = damaged.len() / 2;
    damaged[middle] ^= 0xFF;
    std::fs::write(&path, &damaged).unwrap();

    let report = store.verify(&namespace()).await.unwrap();

    assert_eq!(report.checked, 1);
    assert_eq!(
        report.corrupt.len() + report.unreadable.len(),
        1,
        "a damaged frame either decompresses to the wrong bytes or refuses to decompress, and \
         both are answers this has to survive giving: {report:?}"
    );
}

#[tokio::test]
async fn a_repository_it_could_not_fully_read_is_not_reported_as_clean() {
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(1024 * 1024);
    store_uncompressed(root.path(), &payload);
    // A file where a fanout directory belongs: whatever put it there, the audit
    // cannot see what that prefix holds.
    std::fs::write(
        root.path().join("FerrLabs/Blastlands/ff"),
        b"not a directory",
    )
    .unwrap();

    let report = LocalStore::new(root.path())
        .verify(&namespace())
        .await
        .unwrap();

    assert!(
        report.incomplete,
        "an audit that skipped part of the repository and said nothing would be read as a clean \
         bill of health, which is the one thing this command must never hand out: {report:?}"
    );
    assert_eq!(report.checked, 1, "and it still reports what it did read");
}
