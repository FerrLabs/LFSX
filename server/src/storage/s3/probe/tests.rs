use std::time::Duration;

use super::*;
use crate::storage::s3::tests::{
    bucket, bucket_ignoring_checksums, bucket_ignoring_conditions, keyspace,
};

// The good case, and the one that makes the rest meaningful: a store that
// compares the body against the header refuses the probe, so nothing lands.
#[tokio::test]
async fn a_store_that_refuses_a_body_which_does_not_match_is_trusted() {
    crate::tls::install_crypto_provider();

    let (endpoint, objects) = bucket().await;

    assert_eq!(checksums(&keyspace(&endpoint)).await, Checksums::Enforced);
    assert!(
        objects.lock().unwrap().is_empty(),
        "the probe body does not hash to the digest it was signed for, so a store that checks \
         keeps none of it"
    );
}

// And the case the whole thing exists for. A store that takes the header and
// never compares it lets a client put chosen bytes under a chosen digest, and
// since bytes live once and every repository pushing that digest inherits them,
// that is a poisoned object for everybody.
#[tokio::test]
async fn a_store_that_keeps_a_body_which_does_not_match_is_not_trusted() {
    crate::tls::install_crypto_provider();

    let (endpoint, objects) = bucket_ignoring_checksums().await;

    assert_eq!(checksums(&keyspace(&endpoint)).await, Checksums::Ignored);
    assert!(
        objects.lock().unwrap().is_empty(),
        "the probe wrote an object to find that out and has to take it back with it"
    );
}

// A store that cannot be asked has not said yes. Answering `Enforced` here would
// turn one unreachable moment at startup into a server that hands out write URLs
// it never established were safe.
#[tokio::test]
async fn a_store_that_cannot_be_asked_is_not_trusted() {
    crate::tls::install_crypto_provider();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = listener.local_addr().unwrap();
    drop(listener);

    let keys = keyspace(&format!("http://{closed}"));

    assert_eq!(checksums(&keys).await, Checksums::Unknown);
}

fn presigning(endpoint: &str) -> crate::config::Config {
    crate::config::Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        storage_root: std::path::PathBuf::from("."),
        public_url: Some("https://lfs.example".into()),
        action_lifetime: 1800,
        gc_grace: Duration::from_secs(0),
        staging_max_age: Duration::from_secs(0),
        lock_max_age: None,
        max_object_size: None,
        max_concurrent_transfers: 128,
        repo_quota: None,
        compression: None,
        encryption_key_file: None,
        storage: crate::config::Storage::Bucket {
            endpoint: endpoint.to_owned(),
            bucket: "assets".into(),
            region: "us-east-1".into(),
            access_key: "key".into(),
            secret_key: "secret".into(),
            path_style: true,
            presign: true,
            locking: true,
        },
        auth: crate::config::Auth::Disabled,
    }
}

fn presigns(config: &crate::config::Config) -> bool {
    matches!(
        config.storage,
        crate::config::Storage::Bucket { presign: true, .. }
    )
}

// The probe is only worth anything if its answer reaches the flag. Asserted end
// to end rather than on the probe alone, because the failure that matters is the
// server carrying on handing out write URLs after being told not to.
#[tokio::test]
async fn a_store_that_ignores_checksums_loses_pre_signing() {
    crate::tls::install_crypto_provider();

    let (endpoint, _objects) = bucket_ignoring_checksums().await;
    let mut config = presigning(&endpoint);
    assert!(presigns(&config));

    crate::verify_presign(&mut config).await;

    assert!(
        !presigns(&config),
        "uploads have to fall back to coming through this server, which hashes what it is sent"
    );
}

// And a store that does verify keeps it, so the check cannot quietly cost every
// deployment the feature it was written to protect.
#[tokio::test]
async fn a_store_that_verifies_them_keeps_pre_signing() {
    crate::tls::install_crypto_provider();

    let (endpoint, _objects) = bucket().await;
    let mut config = presigning(&endpoint);

    crate::verify_presign(&mut config).await;

    assert!(presigns(&config));
}

// Nothing is asked of a store that was never going to be handed a write URL, so
// the default deployment pays nothing for this.
#[tokio::test]
async fn a_store_that_was_not_going_to_pre_sign_is_never_probed() {
    crate::tls::install_crypto_provider();

    let (endpoint, objects) = bucket_ignoring_checksums().await;
    let mut config = presigning(&endpoint);
    if let crate::config::Storage::Bucket { presign, .. } = &mut config.storage {
        *presign = false;
    }

    crate::verify_presign(&mut config).await;

    assert!(
        objects.lock().unwrap().is_empty(),
        "a store that ignores checksums keeps whatever it is sent, so anything written here would \
         show up"
    );
}

// A store that implements the condition refuses the second write of the same
// key, which is the whole contract a lock rests on.
#[tokio::test]
async fn a_store_that_refuses_the_second_write_can_hold_locks() {
    crate::tls::install_crypto_provider();

    let (endpoint, objects) = bucket().await;

    assert_eq!(
        conditional_writes(&keyspace(&endpoint)).await,
        Conditional::Enforced
    );
    assert!(
        objects.lock().unwrap().is_empty(),
        "the probe wrote a key to find that out and has to take it back with it"
    );
}

// And the store this exists to find. It accepts `If-None-Match: *`, writes
// anyway, and answers success twice, so two clients racing for the same lock are
// both told it is theirs.
#[tokio::test]
async fn a_store_that_writes_twice_cannot() {
    crate::tls::install_crypto_provider();

    let (endpoint, objects) = bucket_ignoring_conditions().await;

    assert_eq!(
        conditional_writes(&keyspace(&endpoint)).await,
        Conditional::Ignored
    );
    assert!(objects.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_store_that_cannot_be_asked_about_conditions_is_not_trusted_either() {
    crate::tls::install_crypto_provider();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = listener.local_addr().unwrap();
    drop(listener);

    assert_eq!(
        conditional_writes(&keyspace(&format!("http://{closed}"))).await,
        Conditional::Unknown
    );
}

fn locks(config: &crate::config::Config) -> bool {
    matches!(
        config.storage,
        crate::config::Storage::Bucket { locking: true, .. }
    )
}

// The answer has to reach the lock store, because the failure that matters is a
// server that carries on handing out locks it cannot arbitrate.
#[tokio::test]
async fn a_store_that_ignores_conditions_loses_locking() {
    crate::tls::install_crypto_provider();

    let (endpoint, _objects) = bucket_ignoring_conditions().await;
    let mut config = presigning(&endpoint);
    assert!(locks(&config));

    crate::verify_locking(&mut config).await;

    assert!(!locks(&config));
}

#[tokio::test]
async fn a_store_that_honours_them_keeps_locking() {
    crate::tls::install_crypto_provider();

    let (endpoint, _objects) = bucket().await;
    let mut config = presigning(&endpoint);

    crate::verify_locking(&mut config).await;

    assert!(locks(&config));
}

// Anything left behind by a run that died mid-probe has to be inert, and a fresh
// key per probe is what makes it so. A stale one under the key this run was
// about to use would make its first write the second one, and the refusal would
// read as enforcement that was never tested.
#[tokio::test]
async fn a_probe_key_left_by_an_earlier_run_does_not_pass_for_enforcement() {
    crate::tls::install_crypto_provider();

    let (endpoint, objects) = bucket().await;
    for stale in [".probe/conditional", ".probe/conditional-0123456789abcdef"] {
        objects
            .lock()
            .unwrap()
            .insert(stale.to_owned(), b"left behind".to_vec());
    }

    assert_eq!(
        conditional_writes(&keyspace(&endpoint)).await,
        Conditional::Enforced,
        "what is measured has to be a fresh pair of writes, whatever an earlier run left"
    );
}

// The trap the review on #181 found, which a fixed key made permanent and
// silent. A leftover, from a run that died or from a store that has since been
// fixed, sits under the key the next run would have used: the compliant store
// refuses the fresh write, the `HEAD` finds the old object, and pre-signing is
// switched off for good with nothing said about why.
#[tokio::test]
async fn a_probe_object_left_by_an_earlier_run_does_not_condemn_a_compliant_store() {
    crate::tls::install_crypto_provider();

    let (endpoint, objects) = bucket().await;
    for stale in [".probe/checksum", ".probe/checksum-0123456789abcdef"] {
        objects
            .lock()
            .unwrap()
            .insert(stale.to_owned(), b"left behind".to_vec());
    }

    assert_eq!(
        checksums(&keyspace(&endpoint)).await,
        Checksums::Enforced,
        "this store refuses a body that does not match, and no amount of litter changes that"
    );
}

// And what a probe leaves is not litter forever: the reclaimer takes `.probe/`
// on the same schedule as an upload nobody reported.
#[tokio::test]
async fn what_a_probe_leaves_behind_is_reclaimed() {
    crate::tls::install_crypto_provider();

    let (endpoint, objects) = bucket_ignoring_checksums().await;
    objects.lock().unwrap().insert(
        ".probe/checksum-deadbeefdeadbeef".to_owned(),
        b"orphan".to_vec(),
    );

    let reclaimed = crate::storage::s3::tests::store(&endpoint)
        .reclaim_incoming(Duration::from_secs(0))
        .await
        .unwrap();

    assert_eq!(reclaimed.files, 1);
    assert!(objects.lock().unwrap().is_empty());
}
