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
    let local = LocalStore::new(config.storage_root.clone())
        .with_max_object_size(config.max_object_size)
        .with_compression(config.compression);

    let store = match &config.storage {
        crate::config::Storage::Local => Store::local(local),
        crate::config::Storage::Bucket {
            endpoint,
            bucket,
            region,
            access_key,
            secret_key,
            path_style,
        } => {
            let bucket = S3Store::new(&S3Config {
                endpoint: endpoint.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
                access_key: access_key.clone(),
                secret_key: secret_key.clone(),
                path_style: *path_style,
            })
            .expect("the bucket configuration is not usable");

            tracing::warn!(
                "objects are stored in a bucket: collection, deduplication, compression and                  verification answer 501, and the lfsx_objects_stored and lfsx_store_bytes gauges                  are not measured — read capacity from the bucket itself"
            );

            if config.compression.is_some() {
                tracing::warn!(
                    "LFSX_COMPRESSION is set and objects are stored in a bucket — the bucket                      holds them uncompressed, because a compressed object is only readable                      through the local file the codec opens"
                );
            }

            Store::bucket(bucket, local)
        }
    };
    let locks = LockStore::new(config.storage_root.clone());
    let authorizer = Authorizer::new(&config.auth);

    routes::router(Arc::new(AppState {
        store,
        locks,
        config,
        authorizer,
        metrics: Metrics::new(),
    }))
}
