use std::collections::HashMap;

use futures_util::StreamExt;

use super::{S3Store, SIZES_AT_ONCE, sizes};
use crate::namespace::Namespace;

// What a repository holds, which is the size index read back. It lives beside
// the index rather than among the object operations, because the two only make
// sense together: one writes the numbers and the other adds them up.

impl S3Store {
    // What the bucket holds for this repository, counted from its markers and
    // the sizes recorded beside them. The markers are empty, so their own size
    // says nothing, and the objects they claim live under names this prefix
    // never reaches: the index is what closes that gap.
    //
    // The figure is still cached the way the local one is. A listing is cheap
    // and a quota is checked on every negotiation, which is often enough that
    // cheap is not the same as free.
    pub async fn usage_of(&self, ns: &Namespace) -> (u64, u64) {
        // One listing, which returns the markers and the size index together
        // because they share the repository's prefix. In a bucket this server
        // wrote, that is the whole measurement: no request is spent per object.
        let keys = match self.keys.keys(&Self::own_prefix(ns)).await {
            Ok(keys) => keys,
            Err(error) => {
                // A capacity figure that silently reads zero is worse than one
                // that is missing, because it looks like an answer.
                tracing::warn!(%error, "the object store could not be listed");
                return (0, 0);
            }
        };

        let mut indexed = HashMap::new();
        let mut held = Vec::new();

        for key in keys {
            if let Some((oid, size)) = sizes::read(&key) {
                indexed.insert(oid, size);
            } else if let Some(oid) = key.rsplit('/').next()
                && crate::storage::LocalStore::validate_oid(oid).is_ok()
            {
                held.push(oid.to_owned());
            }
        }

        // Only what a marker claims is counted. An index entry whose marker has
        // gone is inert rather than wrong, which is why a sweep that fails to
        // tidy one costs an empty key and nothing else.
        let objects = held.len() as u64;
        let mut bytes = held.iter().filter_map(|oid| indexed.get(oid)).sum();

        let unindexed: Vec<String> = held
            .into_iter()
            .filter(|oid| !indexed.contains_key(oid))
            .collect();

        if !unindexed.is_empty() {
            bytes += self.measure_and_index(ns, unindexed).await;
        }

        (objects, bytes)
    }

    // The old way, for the objects the index does not cover, and it writes what
    // it learns so it covers them next time.
    //
    // That is the whole migration. A bucket written before the index has markers
    // and no sizes, and the first reading measures it exactly as this server
    // always did and leaves the answer behind. There is nothing to run and no
    // flag to set: it converges by being used.
    async fn measure_and_index(&self, ns: &Namespace, oids: Vec<String>) -> u64 {
        tracing::info!(
            count = oids.len(),
            "measuring objects the size index does not cover yet, and indexing them"
        );

        futures_util::stream::iter(oids)
            .map(|oid| {
                let store = &self;
                async move {
                    let size = store.size_of(&oid).await.unwrap_or_default();

                    if let Err(error) = sizes::write(&store.keys, ns, &oid, size).await {
                        tracing::warn!(%error, oid, "an object could not be added to the size index");
                    }

                    size
                }
            })
            .buffer_unordered(SIZES_AT_ONCE)
            .fold(0, |held, size| async move { held + size })
            .await
    }
}
