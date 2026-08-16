use std::sync::Arc;

use crate::auth::Authorizer;
use crate::config::Config;
use crate::locks::LockStore;
use crate::metrics::Metrics;
use crate::storage::Store;

pub struct AppState {
    pub store: Store,
    pub locks: LockStore,
    pub config: Config,
    pub authorizer: Authorizer,
    pub metrics: Metrics,
}

pub type Shared = Arc<AppState>;
