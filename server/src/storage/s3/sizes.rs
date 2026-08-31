use super::keyspace::Keyspace;
use crate::error::Error;
use crate::namespace::Namespace;
use crate::oid::Oid;

// What a repository holds, answerable from a listing instead of a request per
// object.
//
// A listing reports each key's own length, and a marker is empty, so the size of
// the object a marker claims is not in the listing at all. It is on the content
// key, which lives somewhere else entirely and under a name the repository
// prefix does not reach. That is why measuring a repository cost one `HEAD` per
// object, and why one holding fifty thousand of them cost fifty thousand
// requests every time the cached figure expired.
//
// So the number goes into a key name, where a listing can read it: one empty
// object per marker at `{org}/{repo}/.sizes/{oid}.{size}`, and the total is the
// sum of what the names say.
//
// The same move `.refs/` made for the reverse lookup, for the same reason. The
// bucket is the only database here, and an index is what a database would have
// given for nothing.

const PREFIX: &str = ".sizes/";

pub(crate) fn key(ns: &Namespace, oid: &Oid, size: u64) -> String {
    format!("{}/{}/{PREFIX}{oid}.{size}", ns.org(), ns.repo())
}

// A key of this index rather than a marker. Both live under the repository's
// prefix, and everything that walks that prefix has to tell them apart: read as
// a marker, one of these is a claim on an object whose name ends in a number.
pub(crate) fn is_one(key: &str) -> bool {
    key.contains(&format!("/{PREFIX}"))
}

// The oid and the size a key carries, or None if it carries neither.
pub(crate) fn read(key: &str) -> Option<(Oid, u64)> {
    let (oid, size) = key.rsplit('/').next()?.rsplit_once('.')?;

    Some((Oid::parse(oid).ok()?, size.parse().ok()?))
}

// After the marker, always. The marker is what a repository holding an object
// means, and this only says how big it is: a size with no marker is counted by
// nobody, where a marker with no size is measured the old way and indexed on the
// next reading. Neither costs anything but a request.
pub(crate) async fn write(
    keys: &Keyspace,
    ns: &Namespace,
    oid: &Oid,
    size: u64,
) -> Result<(), Error> {
    keys.put(&key(ns, oid, size), reqwest::Body::from(Vec::new()), 0)
        .await
}

#[cfg(test)]
mod tests;
