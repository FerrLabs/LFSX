use super::*;
use crate::storage::s3::tests::{Objects, bucket, keyspace};

const OID: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

fn marker(org: &str, repo: &str) -> String {
    format!("{org}/{repo}/58/91/{OID}")
}

fn keys_of(objects: &Objects) -> Vec<String> {
    let mut keys: Vec<String> = objects.lock().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn every_shape_that_carries_a_namespace_is_lowercased() {
    for (from, to) in [
        (
            marker("FerrLabs", "Blastlands"),
            marker("ferrlabs", "blastlands"),
        ),
        (
            format!("work~Acme/Repo/.sizes/{OID}.12"),
            format!("work~acme/repo/.sizes/{OID}.12"),
        ),
        (
            format!(".refs/{OID}/FerrLabs/Blastlands"),
            format!(".refs/{OID}/ferrlabs/blastlands"),
        ),
        (
            ".locks/FerrLabs/Blastlands/0123.json".to_owned(),
            ".locks/ferrlabs/blastlands/0123.json".to_owned(),
        ),
    ] {
        assert_eq!(canonical_key(&from), Some(to), "{from}");
    }
}

#[test]
fn keys_that_are_not_a_namespace_are_left_alone() {
    for key in [
        marker("ferrlabs", "blastlands"),
        format!(".content/58/91/{OID}"),
        ".lfsx/Tokens".to_owned(),
        ".refs/.complete".to_owned(),
        format!(".incoming/FerrLabs/Blastlands/58/91/{OID}"),
        "Stray".to_owned(),
        "Org/Repo".to_owned(),
        ".locks/FerrLabs/Blastlands".to_owned(),
    ] {
        assert_eq!(canonical_key(&key), None, "{key}");
    }
}

#[test]
fn only_the_namespace_is_lowercased_never_what_follows_it() {
    let key = ".locks/Org/Repo/Mixed.json";

    assert_eq!(canonical_key(key).unwrap(), ".locks/org/repo/Mixed.json");
}

#[tokio::test]
async fn a_bucket_is_folded_once_and_marked() {
    let (endpoint, objects) = bucket().await;
    let keys = keyspace(&endpoint);
    for key in [
        marker("FerrLabs", "Blastlands"),
        marker("ferrlabs", "blastlands"),
        marker("FERRLABS", "BlastLands"),
        format!(".refs/{OID}/FerrLabs/Blastlands"),
        format!(".content/58/91/{OID}"),
    ] {
        keys.put(&key, Vec::new()).await.unwrap();
    }
    keys.put(".locks/FerrLabs/Blastlands/0123.json", b"lock".to_vec())
        .await
        .unwrap();

    assert_eq!(fold_bucket(&keys).await.unwrap(), 4);

    assert_eq!(
        keys_of(&objects),
        vec![
            ".content/58/91/5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
                .to_owned(),
            ".lfsx/namespace-case".to_owned(),
            ".locks/ferrlabs/blastlands/0123.json".to_owned(),
            format!(".refs/{OID}/ferrlabs/blastlands"),
            marker("ferrlabs", "blastlands"),
        ]
    );
    assert_eq!(
        keys.get_bytes(".locks/ferrlabs/blastlands/0123.json")
            .await
            .unwrap()
            .unwrap(),
        b"lock"
    );
}

#[tokio::test]
async fn a_folded_bucket_is_not_listed_again() {
    let (endpoint, objects) = bucket().await;
    let keys = keyspace(&endpoint);
    assert_eq!(fold_bucket(&keys).await.unwrap(), 0);

    keys.put(&marker("FerrLabs", "Late"), Vec::new())
        .await
        .unwrap();

    assert_eq!(fold_bucket(&keys).await.unwrap(), 0);
    assert!(keys_of(&objects).contains(&marker("FerrLabs", "Late")));
}

#[tokio::test]
async fn a_bucket_that_cannot_be_reached_is_an_error_and_is_not_marked() {
    let keys = crate::storage::s3::tests::unreachable_keyspace();

    assert!(fold_bucket(&keys).await.is_err());
}

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn files(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    found.sort();
    found
}

#[tokio::test]
async fn a_directory_with_capitals_is_renamed() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        &format!("FerrLabs/Blastlands/58/91/{OID}"),
        b"bytes",
    );
    write(root.path(), "work~Acme/Game/aa/bb/x", b"x");

    assert_eq!(fold_local(root.path()).await.unwrap(), 4);

    assert_eq!(
        files(root.path()),
        vec![
            "ferrlabs/blastlands/58/91/5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03",
            "work~acme/game/aa/bb/x",
        ]
    );
}

#[tokio::test]
async fn variants_that_both_hold_data_are_merged_and_nothing_is_lost() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "FerrLabs/Blastlands/aa/aa/one", b"one");
    write(root.path(), "ferrlabs/blastlands/bb/bb/two", b"two");
    write(root.path(), "FERRLABS/blastlands/cc/cc/three", b"three");
    write(root.path(), "ferrlabs/BLASTLANDS/aa/aa/one", b"one");
    write(root.path(), "ferrlabs/Other/dd/dd/four", b"four");

    fold_local(root.path()).await.unwrap();

    assert_eq!(
        files(root.path()),
        vec![
            "ferrlabs/blastlands/aa/aa/one",
            "ferrlabs/blastlands/bb/bb/two",
            "ferrlabs/blastlands/cc/cc/three",
            "ferrlabs/other/dd/dd/four",
        ]
    );
    assert_eq!(
        std::fs::read(root.path().join("ferrlabs/blastlands/bb/bb/two")).unwrap(),
        b"two"
    );
}

#[tokio::test]
async fn a_lock_already_under_the_canonical_name_wins() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        ".locks/FerrLabs/Blastlands/id.json",
        b"variant",
    );
    write(
        root.path(),
        ".locks/ferrlabs/blastlands/id.json",
        b"canonical",
    );
    write(
        root.path(),
        ".locks/FerrLabs/Blastlands/other.json",
        b"moved",
    );

    fold_local(root.path()).await.unwrap();

    assert_eq!(
        files(root.path()),
        vec![
            ".locks/ferrlabs/blastlands/id.json",
            ".locks/ferrlabs/blastlands/other.json",
        ]
    );
    assert_eq!(
        std::fs::read(root.path().join(".locks/ferrlabs/blastlands/id.json")).unwrap(),
        b"canonical"
    );
}

#[tokio::test]
async fn shared_content_and_hidden_directories_are_not_touched() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &format!(".content/58/91/{OID}"), b"bytes");
    write(root.path(), ".lfsx/Tokens", b"t");

    assert_eq!(fold_local(root.path()).await.unwrap(), 0);
    assert_eq!(files(root.path()).len(), 2);
}

#[tokio::test]
async fn folding_twice_changes_nothing_the_second_time() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "FerrLabs/Blastlands/aa/aa/one", b"one");

    fold_local(root.path()).await.unwrap();
    let after_first = files(root.path());

    assert_eq!(fold_local(root.path()).await.unwrap(), 0);
    assert_eq!(files(root.path()), after_first);
}

#[tokio::test]
async fn a_fold_the_process_died_in_is_finished_on_the_next_start() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), ".fold-ferrlabs/blastlands/aa/aa/one", b"one");
    write(root.path(), "ferrlabs/blastlands/bb/bb/two", b"two");

    fold_local(root.path()).await.unwrap();

    assert_eq!(
        files(root.path()),
        vec![
            "ferrlabs/blastlands/aa/aa/one",
            "ferrlabs/blastlands/bb/bb/two",
        ]
    );
}

#[tokio::test]
async fn a_missing_root_is_an_empty_one() {
    let root = tempfile::tempdir().unwrap();

    assert_eq!(fold_local(&root.path().join("absent")).await.unwrap(), 0);
}
