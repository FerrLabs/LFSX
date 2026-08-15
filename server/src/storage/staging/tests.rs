use super::*;

const OID: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

fn store_with_files(files: &[(&str, &[u8])]) -> (tempfile::TempDir, LocalStore) {
    let root = tempfile::tempdir().unwrap();
    let fanout = root
        .path()
        .join("FerrLabs/Blastlands")
        .join(&OID[0..2])
        .join(&OID[2..4]);
    std::fs::create_dir_all(&fanout).unwrap();

    for (name, contents) in files {
        std::fs::write(fanout.join(name), contents).unwrap();
    }

    let store = LocalStore::new(root.path());
    (root, store)
}

#[tokio::test]
async fn an_abandoned_staging_file_is_reclaimed_with_its_size() {
    let (root, store) = store_with_files(&[(&format!("{OID}.0.part"), b"half an upload")]);

    let reclaimed = store.reclaim_staging(Duration::ZERO).await;

    assert_eq!(
        reclaimed,
        Reclaimed {
            files: 1,
            bytes: 14
        }
    );
    assert!(
        !root
            .path()
            .join("FerrLabs/Blastlands")
            .join(&OID[0..2])
            .join(&OID[2..4])
            .join(format!("{OID}.0.part"))
            .exists()
    );
}

#[tokio::test]
async fn an_upload_still_in_flight_is_left_alone() {
    let (_root, store) = store_with_files(&[(&format!("{OID}.0.part"), b"still arriving")]);

    let reclaimed = store.reclaim_staging(Duration::from_secs(3600)).await;

    assert_eq!(
        reclaimed,
        Reclaimed::default(),
        "a staging file younger than the threshold belongs to a transfer in progress, and \
         removing it would break a client doing nothing wrong"
    );
}

#[tokio::test]
async fn objects_are_never_touched() {
    let (root, store) = store_with_files(&[
        (OID, b"a real object"),
        (&format!("{OID}.3.part"), b"litter"),
    ]);

    let reclaimed = store.reclaim_staging(Duration::ZERO).await;

    assert_eq!(reclaimed.files, 1);
    assert!(
        root.path()
            .join("FerrLabs/Blastlands")
            .join(&OID[0..2])
            .join(&OID[2..4])
            .join(OID)
            .exists(),
        "the sweep runs inside the object fanout, so mistaking an object for litter would \
         delete data no backup expects to lose"
    );
}

#[tokio::test]
async fn the_shared_content_store_is_not_walked() {
    let (root, store) = store_with_files(&[]);
    let content = root.path().join(".content").join(&OID[0..2]);
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join(format!("{OID}.0.part")), b"never happens").unwrap();

    let reclaimed = store.reclaim_staging(Duration::ZERO).await;

    assert_eq!(
        reclaimed,
        Reclaimed::default(),
        "uploads never stage under .content, so an hourly walk of it would cost I/O that \
         grows with the whole store instead of with the litter"
    );
}

#[tokio::test]
async fn a_repository_named_like_a_dot_directory_is_still_swept() {
    let root = tempfile::tempdir().unwrap();
    let fanout = root
        .path()
        .join("FerrLabs/.github")
        .join(&OID[0..2])
        .join(&OID[2..4]);
    std::fs::create_dir_all(&fanout).unwrap();
    std::fs::write(fanout.join(format!("{OID}.0.part")), b"litter").unwrap();

    let reclaimed = LocalStore::new(root.path())
        .reclaim_staging(Duration::ZERO)
        .await;

    assert_eq!(
        reclaimed.files, 1,
        "only the root carries directories worth skipping — .github is a real repository name"
    );
}

#[tokio::test]
async fn an_empty_store_reclaims_nothing_and_does_not_fail() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::new(root.path());

    assert_eq!(
        store.reclaim_staging(Duration::ZERO).await,
        Reclaimed::default()
    );
}
