pub mod auth;
pub mod config;
pub mod dashboard;
pub mod error;
pub mod locks;
pub mod metrics;
pub mod model;
pub mod namespace;
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
use crate::storage::s3::{S3Config, S3Store};
use crate::storage::{LocalStore, Store};

pub fn app(config: Config) -> Router {
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
            endpoint,
            bucket,
            region,
            access_key,
            secret_key,
            path_style,
            presign,
        } => {
            let bucket = S3Store::new(&S3Config {
                endpoint: endpoint.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
                access_key: access_key.clone(),
                secret_key: secret_key.clone(),
                path_style: *path_style,
                redirect: *presign,
                lifetime: std::time::Duration::from_secs(config.action_lifetime.into()),
            })
            .expect("the bucket configuration is not usable");

            tracing::warn!(
                "objects and locks are stored in a bucket: collection, deduplication, compression                  and verification answer 501, and the lfsx_objects_stored and lfsx_store_bytes                  gauges are not measured — read capacity from the bucket itself"
            );

            if *presign {
                tracing::warn!(
                    "LFSX_S3_PRESIGN=true — downloads are redirected to the bucket, so                      lfsx_downloaded_bytes stops counting them and the bucket serves the ranges"
                );
            }

            if config.encryption_key_file.is_some() {
                tracing::warn!(
                    "LFSX_ENCRYPTION_KEY_FILE is set and objects are stored in a bucket — the                      bucket holds them as they arrive, because an encrypted object is only                      readable through the local file the codec opens"
                );
            }

            if config.compression.is_some() {
                tracing::warn!(
                    "LFSX_COMPRESSION is set and objects are stored in a bucket — the bucket                      holds them uncompressed, because a compressed object is only readable                      through the local file the codec opens"
                );
            }

            // The locks go with the objects. Left on the volume they would make
            // the bucket a half measure: capacity would be shared and the one
            // piece of state a second replica must agree on would not be.
            (
                Store::bucket(bucket.clone(), local),
                LockStore::bucket(bucket),
            )
        }
    };
    let locks = lock_backend.with_max_age(config.lock_max_age);
    let authorizer = Authorizer::new(&config.auth);

    routes::router(Arc::new(AppState {
        store,
        locks,
        config,
        authorizer,
        metrics: Metrics::new(),
    }))
}
