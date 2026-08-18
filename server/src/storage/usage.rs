use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::namespace::Namespace;

// A quota is checked on every negotiation, so the figure behind it can afford
// neither a fresh measurement each time nor a minute of staleness: stale in one
// direction lets a repository push past its budget, and in the other it refuses
// space the client has just freed. So it is remembered briefly, a stored object
// adds to what is remembered, and anything that rewrites a repository drops it
// so the next reader measures what is really left rather than trusting
// arithmetic across hard links.
//
// It sits on the seam rather than in one backend. It used to live in the local
// store, which meant a volume answered a batch from memory while a bucket
// listed the repository and issued a `HEAD` per object for every object in the
// batch. Two implementations of the same policy is one implementation and one
// that forgot.
const TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Usage {
    per_namespace: Mutex<HashMap<String, (Instant, u64, u64)>>,
}

impl Usage {
    pub(super) async fn cached(&self, ns: &Namespace) -> Option<(u64, u64)> {
        self.per_namespace
            .lock()
            .await
            .get(&ns.to_string())
            .filter(|(measured_at, _, _)| measured_at.elapsed() < TTL)
            .map(|(_, objects, bytes)| (*objects, *bytes))
    }

    pub(super) async fn remember(&self, ns: &Namespace, objects: u64, bytes: u64) {
        self.per_namespace
            .lock()
            .await
            .insert(ns.to_string(), (Instant::now(), objects, bytes));
    }

    // Only when something is already remembered: measuring from nothing but the
    // one object just written would report a repository as holding only that.
    pub(super) async fn stored(&self, ns: &Namespace, bytes: u64) {
        if let Some((_, objects, held)) = self.per_namespace.lock().await.get_mut(&ns.to_string()) {
            *objects += 1;
            *held += bytes;
        }
    }

    pub(super) async fn forget(&self, ns: &Namespace) {
        self.per_namespace.lock().await.remove(&ns.to_string());
    }
}
