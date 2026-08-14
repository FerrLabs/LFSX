pub mod config;
pub mod error;
pub mod model;
pub mod routes;
pub mod storage;

use std::sync::Arc;

use axum::Router;

use crate::config::Config;
use crate::routes::AppState;
use crate::storage::LocalStore;

pub fn app(config: Config) -> Router {
    let store = LocalStore::new(config.storage_root.clone());
    routes::router(Arc::new(AppState { store, config }))
}
