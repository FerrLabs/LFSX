use futures_util::StreamExt;

use super::keyspace::Keyspace;
use crate::error::Error;
use crate::namespace::Namespace;
use crate::oid::Oid;

// One empty object per repository holding an oid, so the question collection
// actually asks (does anybody else still claim these bytes?) is a listing of
// one short prefix instead of a listing of the entire bucket. S3 has no query
// for "any key ending in this oid", and the oid is the suffix of a marker, so
// without this the sweep has to read every key there is.
//
// The index is only ever allowed to drift in one direction: it may name a
// holder that has gone, never miss one that is still there. A ref too many
// leaves an object nobody reads. A ref too few deletes an object somebody does.
// Every ordering below is chosen to make the second impossible, which is why
// the write goes in before the marker and the delete goes out after it.

// Written once the whole bucket has been walked and every marker seen has been
// given a ref. Until it exists the index proves nothing: a marker written before
// the index existed has no ref beside it, and would read as an object that
// nobody claims.
const COMPLETE: &str = ".refs/.complete";

const CONCURRENCY: usize = 16;

pub(crate) fn key(ns: &Namespace, oid: &Oid) -> String {
    format!(".refs/{oid}/{}/{}", ns.org(), ns.repo())
}

fn prefix(oid: &Oid) -> String {
    format!(".refs/{oid}/")
}

pub(crate) async fn ready(keys: &Keyspace) -> bool {
    keys.head(COMPLETE).await.is_ok()
}

pub(crate) async fn write(keys: &Keyspace, ns: &Namespace, oid: &Oid) -> Result<(), Error> {
    keys.put(&key(ns, oid), reqwest::Body::from(Vec::new()), 0)
        .await
}

// Answered by listing a prefix that holds one key per holder.
//
// Every failure answers yes. This is asked to decide whether to delete bytes,
// and the cost of being wrong is not symmetric: a false yes leaves an object
// nobody reads, a false no destroys one somebody does.
pub(crate) async fn claimed_by_another(keys: &Keyspace, ns: &Namespace, oid: &Oid) -> bool {
    let ours = key(ns, oid);

    match keys.keys(&prefix(oid)).await {
        Ok(holders) => holders.iter().any(|holder| holder != &ours),
        Err(error) => {
            tracing::warn!(
                %error,
                %oid,
                "the claim index could not be read, so the object is kept"
            );
            true
        }
    }
}

// A marker is `{org}/{repo}/{aa}/{bb}/{oid}`, and nothing else in the bucket has
// that shape. Anything that does not parse is not a marker and gets no ref,
// which is what keeps a stray key from inventing a holder.
pub(crate) fn from_marker(marker: &str) -> Option<String> {
    let mut parts = marker.split('/');
    let org = parts.next()?;
    let repo = parts.next()?;
    let aa = parts.next()?;
    let bb = parts.next()?;
    let oid = parts.next()?;

    if parts.next().is_some() || Oid::parse(oid).is_err() {
        return None;
    }

    (oid.starts_with(aa) && oid[2..].starts_with(bb) && !org.is_empty() && !repo.is_empty())
        .then(|| format!(".refs/{oid}/{org}/{repo}"))
}

// Give every marker in the bucket a ref, then record that it happened. It takes
// the listing collection has already paid for, so migrating a bucket that
// predates the index costs no extra listing: one write per marker, once.
//
// The mark goes last, and only if every write landed. A run that dies halfway
// leaves refs that are correct but incomplete, and no claim that they are, so
// the next sweep does the whole thing again rather than trusting a half-built
// index and deleting against it.
pub(crate) async fn backfill(keys: &Keyspace, markers: &[String]) -> Result<(), Error> {
    let refs: Vec<String> = markers.iter().filter_map(|key| from_marker(key)).collect();

    tracing::info!(
        count = refs.len(),
        "building the claim index for a bucket that predates it"
    );

    // Each write owns its key and its own handle to the store. Borrowing them
    // across `buffer_unordered` builds a future that is Send only for the
    // lifetime it was built with, and axum needs one that is Send for any.
    let mut writes = futures_util::stream::iter(refs.into_iter().map(|key| {
        let keys = keys.clone();
        async move { keys.put(&key, reqwest::Body::from(Vec::new()), 0).await }
    }))
    .buffer_unordered(CONCURRENCY);

    while let Some(written) = writes.next().await {
        written?;
    }

    keys.put(COMPLETE, reqwest::Body::from(Vec::new()), 0).await
}
