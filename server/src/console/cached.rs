use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::error::Error;

pub struct Cached<T> {
    ttl: Duration,
    slot: Mutex<Option<(Instant, Arc<T>)>>,
}

impl<T> Cached<T> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            slot: Mutex::new(None),
        }
    }

    pub async fn get<F, Fut>(&self, load: F) -> Result<Arc<T>, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, Error>>,
    {
        let mut slot = self.slot.lock().await;
        if let Some((loaded, value)) = slot.as_ref()
            && loaded.elapsed() < self.ttl
        {
            return Ok(value.clone());
        }

        let value = Arc::new(load().await?);
        *slot = Some((Instant::now(), value.clone()));

        Ok(value)
    }

    pub async fn invalidate(&self) {
        *self.slot.lock().await = None;
    }
}

#[cfg(test)]
mod tests;
