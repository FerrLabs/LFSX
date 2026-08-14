use std::sync::Arc;

use crate::auth::Authorizer;
use crate::config::Config;
use crate::locks::LockStore;
use crate::metrics::Metrics;
use crate::storage::LocalStore;

pub struct AppState {
    pub store: LocalStore,
    pub locks: LockStore,
    pub config: Config,
    pub authorizer: Authorizer,
    pub metrics: Metrics,
}

pub type Shared = Arc<AppState>;
