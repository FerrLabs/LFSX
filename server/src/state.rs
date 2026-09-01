use std::sync::Arc;

use tokio::sync::Semaphore;

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
    // None when the cap is configured off. A permit is held for as long as a
    // transfer keeps bytes moving, which for a download means as long as the
    // client keeps reading the body, so the permit travels with the stream.
    pub transfers: Option<Arc<Semaphore>>,
}

impl AppState {
    // A permit, or the refusal that tells the client when to come back. Taken
    // without waiting: a saturated server queueing acceptances would hold the
    // connection open for the privilege of being slow later, and an immediate
    // answer with Retry-After lets the client spend the wait on its side.
    pub fn transfer_permit(
        &self,
    ) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, crate::error::Error> {
        match &self.transfers {
            None => Ok(None),
            Some(transfers) => match transfers.clone().try_acquire_owned() {
                Ok(permit) => Ok(Some(permit)),
                Err(_) => Err(crate::error::Error::TransfersSaturated { retry_after: 5 }),
            },
        }
    }
}

pub type Shared = Arc<AppState>;
