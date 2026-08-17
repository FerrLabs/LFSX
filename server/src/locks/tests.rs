use super::*;

fn namespace() -> Namespace {
    Namespace::new("FerrLabs", "Blastlands").unwrap()
}

#[tokio::test]
async fn a_lock_survives_a_round_trip_through_the_store() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path());
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
    let locks = LockStore::local(root.path());
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
    let locks = LockStore::local(root.path());
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
    let locks = LockStore::local(root.path());
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
    let locks = LockStore::local(root.path());

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

// A lock is stale by its own timestamp, so an abandoned one is written directly
// rather than waited for. That is also the honest shape of the test: the age is
// the only input the rule reads.
async fn abandoned(locks: &LockStore, ns: &Namespace, path: &str, owner: &str, ago: Duration) {
    let lock = Lock {
        id: LockStore::id_of(path),
        path: path.to_owned(),
        locked_at: (OffsetDateTime::now_utc() - ago).format(&Rfc3339).unwrap(),
        owner: Owner {
            name: owner.to_owned(),
        },
    };

    locks
        .take(ns, &lock, &serde_json::to_vec(&lock).unwrap())
        .await
        .unwrap();
}

const WEEK: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[tokio::test]
async fn without_a_maximum_age_a_lock_is_never_stale() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path());
    let ns = namespace();
    abandoned(&locks, &ns, "Arena.unity", "marie", 400 * WEEK).await;

    let held = locks.list(&ns).await.unwrap().remove(0);

    assert!(
        locks.stale_for(&held).is_none(),
        "a team that has not configured an expiry keeps getting what it had"
    );
    assert!(
        locks.create(&ns, "Arena.unity", "jane").await.is_err(),
        "and nobody can take it from marie, however long she has been gone"
    );
}

#[tokio::test]
async fn a_lock_past_the_maximum_age_can_be_taken_without_admin() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path()).with_max_age(Some(WEEK));
    let ns = namespace();
    abandoned(&locks, &ns, "Arena.unity", "marie", 3 * WEEK).await;

    let taken = locks.create(&ns, "Arena.unity", "jane").await.unwrap();

    assert_eq!(taken.owner.name, "jane");
    assert_eq!(
        locks.list(&ns).await.unwrap().len(),
        1,
        "the takeover replaces the claim rather than adding a second one"
    );
    assert_eq!(
        locks.get(&ns, &taken.id).await.unwrap().unwrap().owner.name,
        "jane"
    );
}

#[tokio::test]
async fn a_lock_inside_the_maximum_age_is_still_hers() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path()).with_max_age(Some(4 * WEEK));
    let ns = namespace();
    abandoned(&locks, &ns, "Arena.unity", "marie", WEEK).await;

    let outcome = locks.create(&ns, "Arena.unity", "jane").await;

    match outcome {
        Err(Error::LockHeld(held)) => assert_eq!(held.owner.name, "marie"),
        other => panic!("a week is not three weeks: {other:?}"),
    }
}

// The information an artist needs is not "it is free" but "marie had this and
// has not touched it in three weeks", so the previous holder has to survive
// long enough to be reported.
#[tokio::test]
async fn a_stale_lock_still_names_who_had_it_until_it_is_taken() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path()).with_max_age(Some(WEEK));
    let ns = namespace();
    abandoned(&locks, &ns, "Arena.unity", "marie", 3 * WEEK).await;

    let listed = locks.list(&ns).await.unwrap();

    assert_eq!(listed[0].owner.name, "marie");
    assert_eq!(
        locks
            .stale_for(&listed[0])
            .map(|age| age.as_secs() / WEEK.as_secs()),
        Some(3),
        "and how long it has been, so a dashboard can say it"
    );
}

#[tokio::test]
async fn a_timestamp_this_server_cannot_read_is_not_treated_as_ancient() {
    let lock = Lock {
        id: LockStore::id_of("Arena.unity"),
        path: "Arena.unity".to_owned(),
        locked_at: "whenever".to_owned(),
        owner: Owner {
            name: "marie".to_owned(),
        },
    };

    assert!(
        stale_for(&lock, Some(Duration::from_secs(1))).is_none(),
        "an unparseable timestamp is a lock of unknown age, and taking somebody's scene on the \
         strength of a guess is the wrong way to be wrong"
    );
}

#[tokio::test]
async fn a_clock_that_moved_backwards_does_not_make_a_lock_stale() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path()).with_max_age(Some(Duration::from_secs(1)));

    let future = Lock {
        id: LockStore::id_of("Arena.unity"),
        path: "Arena.unity".to_owned(),
        locked_at: (OffsetDateTime::now_utc() + 10 * WEEK)
            .format(&Rfc3339)
            .unwrap(),
        owner: Owner {
            name: "marie".to_owned(),
        },
    };

    assert!(locks.stale_for(&future).is_none());
}
