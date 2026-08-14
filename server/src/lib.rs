pub mod auth;
pub mod config;
pub mod dashboard;
pub mod error;
pub mod locks;
pub mod metrics;
pub mod model;
pub mod namespace;
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
use crate::storage::LocalStore;

pub fn app(config: Config) -> Router {
    let store = LocalStore::new(config.storage_root.clone());
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
