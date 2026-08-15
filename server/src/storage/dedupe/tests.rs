use std::path::{Path, PathBuf};

use super::*;

fn namespace(repo: &str) -> Namespace {
    Namespace::new("FerrLabs", repo).unwrap()
}

fn oid_of(payload: &[u8]) -> String {
    hex::encode(Sha256::digest(payload))
}

// An object as 0.17.x left it: a plain file at the repository path, with no
// counterpart in the shared store.
fn store_the_old_way(root: &Path, repo: &str, payload: &[u8]) -> PathBuf {
    let oid = oid_of(payload);
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

fn content_path(root: &Path, oid: &str) -> PathBuf {
    root.join(".content")
        .join(&oid[0..2])
        .join(&oid[2..4])
        .join(oid)
}

#[tokio::test]
async fn an_object_from_before_the_shared_store_is_folded_into_it() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"an asset pushed by an older server".repeat(4);
    let path = store_the_old_way(root.path(), "Blastlands", &payload);
    let store = LocalStore::new(root.path());

    let report = store.dedupe(&namespace("Blastlands"), false).await.unwrap();

    assert_eq!(report.inspected, 1);
    assert_eq!(report.adopted, 1);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        payload,
        "the repository still serves the same bytes from the same path"
    );
    assert!(content_path(root.path(), &oid_of(&payload)).exists());
}

#[tokio::test]
async fn a_second_repository_stops_paying_for_bytes_it_shares() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"the asset pack both projects use".repeat(64);
    store_the_old_way(root.path(), "Blastlands", &payload);
    let second = store_the_old_way(root.path(), "Arena", &payload);
    let store = LocalStore::new(root.path());

    store.dedupe(&namespace("Blastlands"), false).await.unwrap();
    let (_, before) = store.usage().await;
    let report = store.dedupe(&namespace("Arena"), false).await.unwrap();

    assert_eq!(report.linked, 1);
    assert_eq!(report.reclaimed, payload.len() as u64);
    assert_eq!(
        std::fs::read(&second).unwrap(),
        payload,
        "the second repository reads what it always did"
    );

    if cfg!(unix) {
        let (_, after) = store.usage().await;
        assert!(
            after < before,
            "the disk has to actually give the bytes back: {before} -> {after}"
        );
    }
}

#[tokio::test]
async fn running_it_again_finds_nothing_left_to_do() {
    let root = tempfile::tempdir().unwrap();
    store_the_old_way(root.path(), "Blastlands", b"an asset worth folding in");
    let store = LocalStore::new(root.path());
    let ns = namespace("Blastlands");

    store.dedupe(&ns, false).await.unwrap();
    let again = store.dedupe(&ns, false).await.unwrap();

    assert_eq!(again.inspected, 1);
    assert_eq!(
        (again.adopted, again.refused),
        (0, 0),
        "nothing is left to fold in, and nothing about it has become suspect: {again:?}"
    );

    #[cfg(unix)]
    assert_eq!(
        again,
        DedupeReport {
            inspected: 1,
            already_shared: 1,
            ..DedupeReport::default()
        },
        "a migration that cannot be re-run is one nobody dares run at all, and one that \
         re-reports the bytes it already freed cannot be trusted to say what it did"
    );
}

#[tokio::test]
async fn a_shared_copy_that_does_not_match_its_name_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"what this repository actually holds";
    let path = store_the_old_way(root.path(), "Blastlands", payload);
    let corrupt = content_path(root.path(), &oid_of(payload));
    std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
    std::fs::write(&corrupt, b"something else entirely").unwrap();

    let report = LocalStore::new(root.path())
        .dedupe(&namespace("Blastlands"), false)
        .await
        .unwrap();

    assert_eq!(report.refused, 1);
    assert_eq!(report.linked, 0);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        payload,
        "linking to a corrupt shared copy would spread it to a repository that had a good one"
    );
}

#[tokio::test]
async fn an_object_that_does_not_match_its_name_is_left_where_it_is() {
    let root = tempfile::tempdir().unwrap();
    let oid = oid_of(b"the name it was given");
    let fanout = root
        .path()
        .join("FerrLabs/Blastlands")
        .join(&oid[0..2])
        .join(&oid[2..4]);
    std::fs::create_dir_all(&fanout).unwrap();
    std::fs::write(fanout.join(&oid), b"not those bytes at all").unwrap();

    let report = LocalStore::new(root.path())
        .dedupe(&namespace("Blastlands"), false)
        .await
        .unwrap();

    assert_eq!(report.refused, 1);
    assert_eq!(report.adopted, 0);
    assert!(
        !content_path(root.path(), &oid).exists(),
        "the shared store is keyed by digest, so admitting a file that hashes to something else \
         would hand the wrong bytes to every repository that links to it later"
    );
}

#[tokio::test]
async fn a_dry_run_reports_without_touching_anything() {
    let root = tempfile::tempdir().unwrap();
    let payload = b"an asset that stays exactly where it is";
    let path = store_the_old_way(root.path(), "Blastlands", payload);

    let report = LocalStore::new(root.path())
        .dedupe(&namespace("Blastlands"), true)
        .await
        .unwrap();

    assert_eq!(report.adopted, 1);
    assert!(report.dry_run);
    assert!(!content_path(root.path(), &oid_of(payload)).exists());
    assert_eq!(std::fs::read(&path).unwrap(), payload);
}
