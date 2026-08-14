use super::*;

fn namespace() -> Namespace {
    Namespace::new("FerrLabs", "Blastlands").unwrap()
}

#[tokio::test]
async fn a_lock_survives_a_round_trip_through_the_store() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::new(root.path());
    let ns = namespace();

    let created = locks
        .create(&ns, "Assets/Scenes/Arena.unity", "jane")
        .await
        .unwrap();

    assert_eq!(created.path, "Assets/Scenes/Arena.unity");
    assert_eq!(created.owner.name, "jane");
    assert_eq!(locks.get(&ns, &created.id).await.unwrap(), Some(created));
}

#[tokio::test]
async fn taking_a_held_lock_fails_and_reports_who_holds_it() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::new(root.path());
    let ns = namespace();
    locks
        .create(&ns, "Assets/Scenes/Arena.unity", "jane")
        .await
        .unwrap();

    let refused = locks
        .create(&ns, "Assets/Scenes/Arena.unity", "john")
        .await
        .unwrap_err();

    match refused {
        Error::LockHeld(held) => assert_eq!(held.owner.name, "jane"),
        other => panic!("expected the lock to be refused, got {other:?}"),
    }
}

#[tokio::test]
async fn locks_are_scoped_to_their_repository() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::new(root.path());
    let elsewhere = Namespace::new("FerrLabs", "RogueLite").unwrap();
    locks
        .create(&namespace(), "Assets/Scenes/Arena.unity", "jane")
        .await
        .unwrap();

    locks
        .create(&elsewhere, "Assets/Scenes/Arena.unity", "john")
        .await
        .expect("the same path in another repository is a different file");

    assert_eq!(locks.list(&namespace()).await.unwrap().len(), 1);
    assert_eq!(locks.list(&elsewhere).await.unwrap().len(), 1);
}

#[tokio::test]
async fn removing_a_lock_frees_the_path() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::new(root.path());
    let ns = namespace();
    let lock = locks
        .create(&ns, "Assets/Art/Hero.psd", "jane")
        .await
        .unwrap();

    locks.remove(&ns, &lock.id).await.unwrap();

    assert!(locks.list(&ns).await.unwrap().is_empty());
    locks
        .create(&ns, "Assets/Art/Hero.psd", "john")
        .await
        .expect("the path is free once the lock is gone");
}

#[tokio::test]
async fn removing_something_that_is_not_a_lock_is_reported_as_missing() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::new(root.path());

    assert!(matches!(
        locks.remove(&namespace(), "not-an-id").await,
        Err(Error::LockNotFound)
    ));
    assert!(matches!(
        locks
            .remove(&namespace(), &LockStore::id_of("never/locked"))
            .await,
        Err(Error::LockNotFound)
    ));
}
