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

// The bucket lock path, which nothing exercised until the stub learned to refuse
// a conditional write. Two callers reach for the same path and the store is what
// decides the second one lost.
#[tokio::test]
async fn a_bucket_lets_exactly_one_of_two_callers_take_a_lock() {
    crate::tls::install_crypto_provider();

    let (endpoint, _objects) = crate::storage::s3::tests::bucket().await;
    let locks = LockStore::bucket(crate::storage::s3::tests::keyspace(&endpoint));
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

// And a store that cannot arbitrate is refused outright rather than handed the
// write. Attempting it would succeed and John would be told the scene is his,
// which is the one answer locking must never give.
#[tokio::test]
async fn a_bucket_that_cannot_arbitrate_refuses_to_take_a_lock_at_all() {
    crate::tls::install_crypto_provider();

    let (endpoint, objects) = crate::storage::s3::tests::bucket_ignoring_conditions().await;
    let locks = LockStore::bucket(crate::storage::s3::tests::keyspace(&endpoint))
        .with_conditional_writes(false);

    let refused = locks
        .create(&namespace(), "Assets/Scenes/Arena.unity", "jane")
        .await
        .unwrap_err();

    assert!(matches!(refused, Error::Unsupported(_)), "{refused:?}");
    assert!(
        objects.lock().unwrap().is_empty(),
        "nothing may be written for a lock that was not granted, or a later reader would find one"
    );
}

// Locking on a volume never depended on the store answering anything: create_new
// is a filesystem primitive. The flag exists for buckets and must not reach it.
#[tokio::test]
async fn a_volume_is_never_held_back_by_what_a_bucket_could_not_prove() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path()).with_conditional_writes(false);

    assert!(
        locks
            .create(&namespace(), "Assets/Scenes/Arena.unity", "jane")
            .await
            .is_ok()
    );
}

// Why the flag above is not decoration. Given the same store and no guard, both
// callers are told the lock is theirs and the second write lands on top of the
// first, so the bucket now says John holds a scene Jane was told she had. This is
// the sequential shape of what two clients racing would get, and it is the reason
// taking a lock is refused outright rather than attempted.
#[tokio::test]
async fn without_the_guard_that_store_hands_the_same_lock_to_both() {
    crate::tls::install_crypto_provider();

    let (endpoint, _objects) = crate::storage::s3::tests::bucket_ignoring_conditions().await;
    let locks = LockStore::bucket(crate::storage::s3::tests::keyspace(&endpoint));
    let ns = namespace();

    let jane = locks
        .create(&ns, "Assets/Scenes/Arena.unity", "jane")
        .await
        .unwrap();
    let john = locks
        .create(&ns, "Assets/Scenes/Arena.unity", "john")
        .await
        .expect("this store refuses nothing, which is the whole problem");

    assert_eq!(jane.id, john.id);
    assert_eq!(
        locks.get(&ns, &jane.id).await.unwrap().unwrap().owner.name,
        "john",
        "Jane was told the scene was hers and the store quietly gave it to John"
    );
}

// A megabyte path is not a filename, it is a payload: stored per lock, listed
// on every list, never collected. 4096 refuses nothing a repository really
// carries, git's own limit sits far below it.
#[tokio::test]
async fn a_path_longer_than_any_real_one_is_refused_and_the_longest_real_one_is_not() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path());
    let ns = namespace();

    let refused = locks
        .create(&ns, &"a/".repeat(2049), "jane")
        .await
        .unwrap_err();
    assert!(
        matches!(
            refused,
            Error::LockPathTooLong {
                actual: 4098,
                limit: 4096
            }
        ),
        "{refused:?}"
    );

    locks
        .create(&ns, &"a/".repeat(2048), "jane")
        .await
        .expect("a path at the limit is a real path");
}

#[tokio::test]
async fn a_full_repository_refuses_a_new_lock_and_not_a_retry_of_a_held_one() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path()).with_capacity(2);
    let ns = namespace();
    locks.create(&ns, "a.unity", "jane").await.unwrap();
    locks.create(&ns, "b.unity", "jane").await.unwrap();

    let refused = locks.create(&ns, "c.unity", "jane").await.unwrap_err();
    assert!(
        matches!(refused, Error::LockLimitReached { limit: 2 }),
        "{refused:?}"
    );

    // The path already holds a lock, so this attempt adds nothing to the
    // count, and the caller deserves to be told who has it, not that the
    // repository is full.
    let held = locks.create(&ns, "b.unity", "john").await.unwrap_err();
    assert!(matches!(held, Error::LockHeld(_)), "{held:?}");
}

// Taking over a stale lock swaps one lock for another, so a full repository
// must not stand in the way of the one operation that never grows it.
#[tokio::test]
async fn a_stale_lock_is_still_takeable_in_a_full_repository() {
    let root = tempfile::tempdir().unwrap();
    let locks = LockStore::local(root.path())
        .with_capacity(1)
        .with_max_age(Some(Duration::ZERO));
    let ns = namespace();
    locks.create(&ns, "a.unity", "jane").await.unwrap();

    let taken = locks.create(&ns, "a.unity", "john").await.unwrap();

    assert_eq!(taken.owner.name, "john");
}

// The review finding on the capacity guard: routed through `list`, taking one
// lock read the body of every lock already held, a GET per key on a bucket,
// paid on every genuinely new create. The guard counts keys instead, so the
// only body read is the existence probe for the lock being taken.
#[tokio::test]
async fn creating_a_lock_counts_the_others_and_reads_none_of_them() {
    crate::tls::install_crypto_provider();
    let (endpoint, _objects, reads) = crate::storage::s3::tests::bucket_counting_key_reads().await;
    let locks = LockStore::bucket(crate::storage::s3::tests::keyspace(&endpoint));
    let ns = namespace();
    for scene in 0..25 {
        locks
            .create(&ns, &format!("Assets/{scene:02}.unity"), "jane")
            .await
            .unwrap();
    }

    let before = reads.load(std::sync::atomic::Ordering::SeqCst);
    locks.create(&ns, "Assets/new.unity", "jane").await.unwrap();
    let cost = reads.load(std::sync::atomic::Ordering::SeqCst) - before;

    assert!(
        cost <= 1,
        "taking one lock read {cost} lock bodies against 25 already held: the capacity \
         guard is fetching every lock where a key listing would answer"
    );
}

#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// The takeover is on the audit trail: routed by its own target so an operator
// can ship it without turning everything up, and naming both owners, because
// "who took marie's lock" is the question the trail exists to answer. Captured
// under a fresh subscriber per attempt for the same global max-level reasons
// as the refusal-wording test in auth::github.
#[tokio::test]
async fn a_takeover_lands_on_the_audit_trail_naming_both_owners() {
    let mut logged = String::new();
    for _ in 0..5 {
        let captured = Captured::default();
        {
            let _guard = tracing::subscriber::set_default(
                tracing_subscriber::fmt()
                    .with_writer(captured.clone())
                    .with_max_level(tracing::Level::INFO)
                    .with_ansi(false)
                    .finish(),
            );

            let root = tempfile::tempdir().unwrap();
            let locks = LockStore::local(root.path()).with_max_age(Some(WEEK));
            let ns = namespace();
            abandoned(&locks, &ns, "Arena.unity", "marie", 3 * WEEK).await;
            locks.create(&ns, "Arena.unity", "jane").await.unwrap();
        }

        logged = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        if !logged.is_empty() {
            break;
        }
    }

    assert!(
        logged.contains("lfsx::audit"),
        "the takeover has to be routable on its own target: {logged}"
    );
    assert!(
        logged.contains(r#"actor="jane""#) && logged.contains(r#"previous_owner="marie""#),
        "the trail answers who took the lock and from whom: {logged}"
    );
}
