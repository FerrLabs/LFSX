pub mod auth;
pub mod config;
pub mod dashboard;
pub mod error;
#[cfg(feature = "fuzzing")]
pub mod fuzzing;
pub mod locks;
pub mod metrics;
pub mod model;
pub mod namespace;
pub mod oid;
pub mod page;
pub mod range;
pub mod routes;
pub mod state;
pub mod storage;
pub mod tls;

use std::sync::Arc;

use axum::Router;

use crate::auth::Authorizer;
use crate::config::Config;
use crate::locks::LockStore;
use crate::metrics::Metrics;
use crate::state::AppState;
use crate::storage::s3::{Keyspace, S3Config, S3Store};
use crate::storage::{LocalStore, Store};

pub fn app(config: Config) -> Router {
    // Here rather than in `backends`, which `reclaim` also calls: a reclaim pass
    // hands nobody a URL, and saying this twice at every boot teaches an operator
    // to skim it.
    //
    // The hrefs in a batch answer are where a client sends the object, and it
    // sends its credential with them. Unset, they are built from the `Host` and
    // `X-Forwarded-Proto` of whoever asked, which is a deployment fact only for
    // as long as something in front is rewriting both.
    //
    // Warned rather than refused. Every deployment that works today works without
    // it, and taking those down to close a hole most of them do not have is the
    // wrong trade.
    if config.public_url.is_none() && !matches!(config.auth, crate::config::Auth::Disabled) {
        tracing::warn!(
            "LFSX_PUBLIC_URL is not set, so the URLs handed to clients are built from the Host and \
             X-Forwarded-Proto headers of whoever asked. Behind a proxy that does not rewrite them, \
             a caller chooses where the next request goes and takes its token there. Set it to the \
             address clients actually use"
        );
    }

    let (store, locks) = backends(&config);
    let authorizer = Authorizer::new(&config.auth);

    routes::router(Arc::new(AppState {
        store,
        locks,
        config,
        authorizer,
        metrics: Metrics::new(),
    }))
}

// Everything an interrupted upload left behind, wherever it left it: a staging
// file on the volume, or bytes under an upload key nobody ever reported. Built
// from the same construction the server uses, so a bucket deployment does not
// end up sweeping only half of itself.
pub async fn reclaim(config: &Config) {
    let reclaimed = backends(config).0.reclaim(config.staging_max_age).await;

    if reclaimed.files > 0 {
        tracing::info!(
            files = reclaimed.files,
            bytes = reclaimed.bytes,
            "reclaimed what interrupted uploads left behind"
        );
    }
}

// Ask the bucket, once, whether it really refuses an upload whose body does not
// match the checksum its URL was signed for, and give up pre-signing if it does
// not say yes.
//
// Handing a client a write URL is safe only because of that refusal. Without it,
// anyone with push rights to any repository can put chosen bytes under a chosen
// digest, and objects are shared: bytes live once at `.content/{oid}`, so every
// repository that later pushes that digest gets a marker pointing at them and
// uploads nothing. One store that ignores the header decides what an object is
// for everybody.
//
// Losing pre-signing costs throughput and nothing else, because transfers fall
// back to coming through this server, which hashes what it is sent. That is why
// a store which cannot be asked loses it too: the question guards data, and an
// unanswered question is not a yes.
pub async fn verify_presign(config: &mut Config) {
    use crate::storage::s3::probe::{Checksums, checksums};

    let crate::config::Storage::Bucket { presign: true, .. } = &config.storage else {
        return;
    };

    let Some(keys) = keyspace(config) else {
        return;
    };

    let refusal = match checksums(&keys).await {
        Checksums::Enforced => return,
        Checksums::Ignored => {
            "this object store accepted an upload whose body did not match the checksum its own \
             signature named. A store that does not verify that header lets a client with push \
             rights put chosen bytes under a chosen digest, and every repository that later pushes \
             that digest would get a marker pointing at them"
        }
        Checksums::Unknown => {
            "this object store could not be asked whether it verifies upload checksums. Handing out \
             a write URL is only safe if the store refuses a body that does not match it, and that \
             has not been established"
        }
    };

    tracing::error!(
        "{refusal}, so LFSX_S3_PRESIGN is being ignored and uploads keep coming through this server"
    );

    if let crate::config::Storage::Bucket { presign, .. } = &mut config.storage {
        *presign = false;
    }
}

// Ask the bucket, once, whether it refuses the second of two conditional writes,
// and give up locking if it will not say yes.
//
// That refusal is the entirety of lock uniqueness here. Two clients race for the
// same path, both write, and the store is the only thing that can say one of them
// arrived second. A store that accepts `If-None-Match: *` without implementing it
// performs both writes and reports success twice, so both are told the lock is
// theirs, and nothing anywhere notices.
//
// There is no safe degraded mode for that, so taking a lock becomes a `501`
// instead. It is the loudest honest answer: a client sees a refusal at the moment
// it asks, rather than a lock somebody else also holds. Everything else about the
// deployment is untouched, objects included, because a team that never takes a
// lock should not lose a working server over this.
pub async fn verify_locking(config: &mut Config) {
    use crate::storage::s3::probe::{Conditional, conditional_writes};

    let Some(keys) = keyspace(config) else {
        return;
    };

    let refusal = match conditional_writes(&keys).await {
        Conditional::Enforced => return,
        Conditional::Ignored => {
            "this object store wrote the same key twice under a condition that should have refused \
             the second, so it cannot say which of two clients racing for a lock arrived first"
        }
        Conditional::Unknown => {
            "this object store could not be asked whether it refuses a conditional write, and lock \
             uniqueness is exactly that refusal"
        }
    };

    tracing::error!(
        "{refusal}, so taking a lock here answers 501. Objects are unaffected, and so is everything \
         else this server does"
    );

    if let crate::config::Storage::Bucket { locking, .. } = &mut config.storage {
        *locking = false;
    }
}

fn keyspace(config: &Config) -> Option<Keyspace> {
    let crate::config::Storage::Bucket {
        endpoint,
        bucket,
        region,
        access_key,
        secret_key,
        path_style,
        ..
    } = &config.storage
    else {
        return None;
    };

    Some(
        Keyspace::new(&S3Config {
            endpoint: endpoint.clone(),
            bucket: bucket.clone(),
            region: region.clone(),
            access_key: access_key.clone(),
            secret_key: secret_key.clone(),
            path_style: *path_style,
            lifetime: std::time::Duration::from_secs(config.action_lifetime.into()),
        })
        .expect("the bucket configuration is not usable"),
    )
}

fn backends(config: &Config) -> (Store, LockStore) {
    // Said out loud because it decides who can read the objects. It is off unless
    // asked for, so this line means somebody asked: it belongs in the log so a
    // deployment that inherited the flag from an older chart sees it rather than
    // discovers it.
    if let crate::config::Auth::Forge {
        anonymous_read: true,
        ..
    } = config.auth
    {
        tracing::info!(
            "anonymous read is on: a request with no credentials is resolved against the forge, so \
             objects in a repository the forge serves publicly can be read by anybody, and the \
             bandwidth is yours. Unset LFSX_ANONYMOUS_READ to require a token whatever the \
             repository's visibility"
        );
    }

    // Refusing to start beats starting without it. A server that silently wrote
    // plaintext because a Secret failed to mount is the one failure this feature
    // must never have: nothing downstream would notice, and the objects written
    // in the meantime are the ones the operator believed were covered.
    let keys = config.encryption_key_file.as_deref().map(|path| {
        std::sync::Arc::new(
            crate::storage::crypt::Keyring::load(path)
                .expect("the encryption key file is not usable"),
        )
    });

    let local = LocalStore::new(config.storage_root.clone())
        .with_max_object_size(config.max_object_size)
        .with_compression(config.compression)
        .with_encryption(keys);

    // The two backends are chosen together and the lock policy is applied once,
    // to both. Deciding it per arm is how `LFSX_LOCK_MAX_AGE` came to be silently
    // ignored in bucket mode: the arms are far apart, only one of them had it,
    // and nothing failed.
    let (store, lock_backend) = match &config.storage {
        crate::config::Storage::Local => (
            Store::local(local),
            LockStore::local(config.storage_root.clone()),
        ),
        crate::config::Storage::Bucket {
            presign, locking, ..
        } => {
            // Built once and shared: the objects and the locks are two ways of
            // using the same bucket, not two buckets. Signing, the connection
            // pool and the retry policy are settled here, and neither layer
            // reaches into the other to get at them.
            let keys = keyspace(config).expect("a bucket keyspace for a bucket store");

            tracing::warn!(
                "objects and locks are stored in a bucket: deduplication, rewriting and \
             verification answer 501, and the lfsx_objects_stored and lfsx_store_bytes \
             gauges are not measured: read capacity from the bucket itself"
            );

            if *presign {
                if config.encryption_key_file.is_some() || config.compression.is_some() {
                    tracing::warn!(
                        "LFSX_S3_PRESIGN=true, but a codec is configured, so downloads keep \
             streaming through this server: what sits in the bucket is a frame under \
             the plaintext digest, and a client handed that directly would hash it \
             and reject the object"
                    );
                } else {
                    tracing::warn!(
                        "LFSX_S3_PRESIGN=true, downloads are redirected to the bucket, so \
             lfsx_downloaded_bytes stops counting them and the bucket serves the ranges"
                    );
                }

                if config.encryption_key_file.is_some() {
                    tracing::warn!(
                        "LFSX_ENCRYPTION_KEY_FILE is set, so uploads keep coming through this \
             server rather than going straight to the bucket: an object a client \
             writes itself would arrive unencrypted"
                    );
                } else if config.compression.is_some() {
                    tracing::warn!(
                        "LFSX_COMPRESSION is set, and objects clients upload straight to the \
             bucket arrive uncompressed: only what passes through this server is \
             compressed"
                    );
                }
            }

            // The locks go with the objects. Left on the volume they would make
            // the bucket a half measure: capacity would be shared and the one
            // piece of state a second replica must agree on would not be.
            (
                Store::bucket(S3Store::new(keys.clone(), *presign), local),
                LockStore::bucket(keys).with_conditional_writes(*locking),
            )
        }
    };
    (store, lock_backend.with_max_age(config.lock_max_age))
}
