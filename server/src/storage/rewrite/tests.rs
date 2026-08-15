use std::path::{Path, PathBuf};

use super::*;

fn namespace(repo: &str) -> Namespace {
    Namespace::new("FerrLabs", repo).unwrap()
}

fn mesh(len: usize) -> Vec<u8> {
    b"vertex 0.7071 0.0000 0.7071 normal 0.0000 1.0000 0.0000 "
        .iter()
        .cycle()
        .take(len)
        .copied()
        .collect()
}

fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

fn store_uncompressed(root: &Path, repo: &str, payload: &[u8]) -> PathBuf {
    let oid = hex::encode(Sha256::digest(payload));
    let fanout = root
        .join("FerrLabs")
        .join(repo)
        .join(&oid[0..2])
        .join(&oid[2..4]);
    std::fs::create_dir_all(&fanout).unwrap();

    let path = fanout.join(&oid);
    std::fs::write(&path, payload).unwrap();
    path
}

async fn read_back(store: &LocalStore, repo: &str, payload: &[u8]) -> Vec<u8> {
    use futures_util::StreamExt;

    let oid = hex::encode(Sha256::digest(payload));
    let object = store.open(&namespace(repo), &oid).await.unwrap();
    let size = object.size();

    let mut out = Vec::new();
    let mut chunks = object.stream(0, size).await.unwrap();
    while let Some(chunk) = chunks.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }

    out
}

fn store(root: &tempfile::TempDir) -> LocalStore {
    LocalStore::new(root.path()).with_compression(Some(3))
}

#[tokio::test]
async fn an_object_written_before_compression_is_folded_in_and_still_reads() {
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(9 * 1024 * 1024);
    let path = store_uncompressed(root.path(), "Blastlands", &payload);
    let store = store(&root);

    let report = store
        .compress(&namespace("Blastlands"), false)
        .await
        .unwrap();

    assert_eq!(report.compressed, 1);
    assert!(
        report.after < report.before / 4,
        "{} of {}",
        report.after,
        report.before
    );
    assert!(std::fs::metadata(&path).unwrap().len() < payload.len() as u64 / 4);
    assert_eq!(
        read_back(&store, "Blastlands", &payload).await,
        payload,
        "an object nobody can read back is not compressed, it is lost"
    );
}

#[tokio::test]
async fn running_it_again_finds_the_work_already_done() {
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(5 * 1024 * 1024);
    store_uncompressed(root.path(), "Blastlands", &payload);
    let store = store(&root);
    let ns = namespace("Blastlands");

    store.compress(&ns, false).await.unwrap();
    let again = store.compress(&ns, false).await.unwrap();

    assert_eq!(again.already, 1);
    assert_eq!((again.compressed, again.refused), (0, 0));
}

#[tokio::test]
async fn an_object_that_will_not_compress_keeps_its_bytes() {
    let root = tempfile::tempdir().unwrap();
    let payload = noise(3 * 1024 * 1024);
    let path = store_uncompressed(root.path(), "Blastlands", &payload);
    let before = std::fs::metadata(&path).unwrap().len();

    let report = store(&root)
        .compress(&namespace("Blastlands"), false)
        .await
        .unwrap();

    assert_eq!(report.left_alone, 1);
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        before,
        "wrapping an incompressible object costs a header and an index, so leaving it alone is \
         the right answer rather than a failure"
    );
}

#[tokio::test]
async fn an_object_that_does_not_hash_to_its_name_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let oid = hex::encode(Sha256::digest(b"the name it carries"));
    let fanout = root
        .path()
        .join("FerrLabs/Blastlands")
        .join(&oid[0..2])
        .join(&oid[2..4]);
    std::fs::create_dir_all(&fanout).unwrap();
    std::fs::write(fanout.join(&oid), mesh(2 * 1024 * 1024)).unwrap();

    let report = store(&root)
        .compress(&namespace("Blastlands"), false)
        .await
        .unwrap();

    assert_eq!(report.refused, 1);
    assert_eq!(report.compressed, 0);
    assert!(
        !fanout.join(format!("{oid}.0.part")).exists(),
        "and it leaves nothing behind"
    );
}

#[tokio::test]
async fn a_dry_run_measures_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(6 * 1024 * 1024);
    let path = store_uncompressed(root.path(), "Blastlands", &payload);

    let report = store(&root)
        .compress(&namespace("Blastlands"), true)
        .await
        .unwrap();

    assert_eq!(report.compressed, 1);
    assert!(
        report.after < report.before / 4,
        "the point of the dry run is the number it reports: {report:?}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        payload,
        "and the object it measured is untouched"
    );
}

#[tokio::test]
async fn a_server_that_does_not_compress_refuses_to_rewrite_a_store() {
    let root = tempfile::tempdir().unwrap();
    store_uncompressed(root.path(), "Blastlands", &mesh(1024 * 1024));

    let refused = LocalStore::new(root.path())
        .compress(&namespace("Blastlands"), false)
        .await;

    assert!(
        matches!(refused, Err(Error::CompressionDisabled)),
        "rewriting a store into a format the server was not told to use is not a decision to \
         take on the operator's behalf"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_shared_object_stays_shared_and_both_repositories_read_it() {
    let root = tempfile::tempdir().unwrap();
    let payload = mesh(5 * 1024 * 1024);
    let oid = hex::encode(Sha256::digest(&payload));
    let first = store_uncompressed(root.path(), "Blastlands", &payload);
    let second = store_uncompressed(root.path(), "Arena", &payload);
    let store = store(&root);

    store.dedupe(&namespace("Blastlands"), false).await.unwrap();
    store.dedupe(&namespace("Arena"), false).await.unwrap();
    store
        .compress(&namespace("Blastlands"), false)
        .await
        .unwrap();

    assert_eq!(read_back(&store, "Blastlands", &payload).await, payload);
    assert_eq!(
        read_back(&store, "Arena", &payload).await,
        payload,
        "compressing one repository must not leave another serving bytes that no longer exist"
    );
    let _ = (first, second, oid);
}
