use std::sync::Arc;

use crate::auth::Authorizer;
use crate::config::Config;
use crate::storage::LocalStore;

pub struct AppState {
    pub store: LocalStore,
    pub config: Config,
    pub authorizer: Authorizer,
}

pub type Shared = Arc<AppState>;
