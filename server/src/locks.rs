use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::Error;
use crate::namespace::Namespace;
use crate::storage::s3::Keyspace;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    pub id: String,
    pub path: String,
    pub locked_at: String,
    pub owner: Owner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub name: String,
}

// Locks live wherever the objects do. On a volume that is a file per lock; in a
// bucket it is a key per lock, and the reason is the same one the bucket exists
// for: two replicas sharing storage have to agree on who holds what. A lock
// store on local disk behind a bucket means each replica has its own answer,
// and an artist told the scene is theirs while another replica hands it to
// somebody else is worse than no locking at all.
// Longer than any path a real repository carries, and git's own limit is far
// below it. What this refuses is the megabyte path, which is not a filename,
// it is a payload: stored verbatim per lock, listed on every list, and never
// collected, since a stale lock is takeable rather than removed.
const MAX_PATH_BYTES: usize = 4096;

// A backstop, not a workflow ceiling. A studio locking every binary asset in a
// large project sits orders of magnitude below this; the thing that reaches it
// is a loop. The refusal at create is what keeps `list`, and everything built
// on it, bounded.
const DEFAULT_CAPACITY: usize = 10_000;

pub struct LockStore {
    backend: Backend,
    max_age: Option<Duration>,
    capacity: usize,
    // Whether the store can be relied on to let exactly one writer win. A
    // filesystem always can: `create_new` either makes the file or does not. A
    // bucket can only if it implements `If-None-Match: *`, and that is asked at
    // startup rather than assumed, because a store which accepts the header and
    // writes anyway tells both callers they took the lock.
    conditional_writes: bool,
}

enum Backend {
    Local { root: PathBuf },
    Bucket(Box<Keyspace>),
}

impl LockStore {
    pub fn local(root: impl Into<PathBuf>) -> Self {
        Self::over(Backend::Local { root: root.into() })
    }

    pub fn bucket(keys: Keyspace) -> Self {
        Self::over(Backend::Bucket(Box::new(keys)))
    }

    fn over(backend: Backend) -> Self {
        Self {
            backend,
            max_age: None,
            capacity: DEFAULT_CAPACITY,
            conditional_writes: true,
        }
    }

    #[cfg(test)]
    fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    pub fn with_max_age(mut self, max_age: Option<Duration>) -> Self {
        self.max_age = max_age;
        self
    }

    // Set from what the store answered at startup. False turns taking a lock
    // into a `501` rather than into a lock two people can hold, which is the
    // only honest thing to do with a store that cannot arbitrate.
    pub fn with_conditional_writes(mut self, supported: bool) -> Self {
        self.conditional_writes = supported;
        self
    }

    pub fn max_age(&self) -> Option<Duration> {
        self.max_age
    }

    // How long a lock has gone untouched, once it is past the age an operator
    // said was too long. None while it is still somebody's.
    pub fn stale_for(&self, lock: &Lock) -> Option<Duration> {
        stale_for(lock, self.max_age)
    }

    pub fn id_of(path: &str) -> String {
        hex::encode(Sha256::digest(path.as_bytes()))[..32].to_owned()
    }

    // The same layout in both, so an operator reading a bucket sees what they
    // would see on the volume.
    fn prefix(ns: &Namespace) -> String {
        format!(".locks/{}/{}/", ns.org(), ns.repo())
    }

    fn key_of(ns: &Namespace, id: &str) -> String {
        format!("{}{id}.json", Self::prefix(ns))
    }

    pub async fn create(&self, ns: &Namespace, path: &str, owner: &str) -> Result<Lock, Error> {
        if path.is_empty() {
            return Err(Error::MalformedLockPath);
        }
        if path.len() > MAX_PATH_BYTES {
            return Err(Error::LockPathTooLong {
                actual: path.len(),
                limit: MAX_PATH_BYTES,
            });
        }

        let lock = Lock {
            id: Self::id_of(path),
            path: path.to_owned(),
            locked_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
            owner: Owner {
                name: owner.to_owned(),
            },
        };
        let encoded = serde_json::to_vec(&lock)?;

        // Enforced only for a lock that would be new: a path already locked
        // adds nothing to the count, and the caller retrying a stale lock they
        // are entitled to take deserves "held by X", not "the repository is
        // full". Two creates racing under the ceiling can land one over it,
        // and that is fine, this is a backstop against a loop, not an
        // invariant anything downstream leans on.
        if self.get(ns, &lock.id).await?.is_none() && self.list(ns).await?.len() >= self.capacity {
            return Err(Error::LockLimitReached {
                limit: self.capacity,
            });
        }

        if self.take(ns, &lock, &encoded).await? {
            return Ok(lock);
        }

        // Whoever holds it is the useful half of the answer, and reading it back
        // can race with a release. Naming the caller's own attempt is a worse
        // answer than none, so a lock that vanished underfoot is reported as
        // held by nobody in particular rather than by the caller.
        let Some(held) = self.get(ns, &lock.id).await? else {
            return Err(Error::LockHeld(Box::new(lock)));
        };

        let Some(age) = self.stale_for(&held) else {
            return Err(Error::LockHeld(Box::new(held)));
        };

        // Discarding it and taking it conditionally, rather than overwriting in
        // place, is what stops two replicas both claiming one abandoned lock:
        // whoever loses the create loses outright and is told who won. Somebody
        // taking it fresh in the gap wins for the same reason.
        self.discard(ns, &held.id).await?;

        if !self.take(ns, &lock, &encoded).await? {
            return match self.get(ns, &lock.id).await? {
                Some(other) => Err(Error::LockHeld(Box::new(other))),
                None => Err(Error::LockHeld(Box::new(lock))),
            };
        }

        tracing::info!(
            path = lock.path,
            previous_owner = held.owner.name,
            untouched_for_seconds = age.as_secs(),
            new_owner = lock.owner.name,
            "a lock nobody had touched was taken over"
        );

        Ok(lock)
    }

    async fn take(&self, ns: &Namespace, lock: &Lock, encoded: &[u8]) -> Result<bool, Error> {
        match &self.backend {
            Backend::Local { root } => {
                Self::write_new(&Self::path_in(root, ns, &lock.id), encoded).await
            }
            // Refused rather than attempted. The write would succeed and the
            // caller would be told the lock is theirs, which is the one answer
            // this must never give when the store cannot say whether somebody
            // else was told the same thing a moment earlier.
            Backend::Bucket(_) if !self.conditional_writes => Err(Error::Unsupported(
                "this object store does not refuse a conditional write, so a lock here could be \
                 held by two people at once",
            )),
            Backend::Bucket(bucket) => {
                bucket
                    .put_if_absent(&Self::key_of(ns, &lock.id), encoded.to_vec())
                    .await
            }
        }
    }

    // Removing something already gone is the normal case here: another replica
    // may have discarded the same abandoned lock a moment earlier.
    async fn discard(&self, ns: &Namespace, id: &str) -> Result<(), Error> {
        match self.remove(ns, id).await {
            Ok(()) | Err(Error::LockNotFound) => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn write_new(path: &Path, encoded: &[u8]) -> Result<bool, Error> {
        let parent = path.parent().expect("lock paths have a parent");
        fs::create_dir_all(parent).await?;

        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await
        {
            Ok(mut file) => {
                file.write_all(encoded).await?;
                file.sync_all().await?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn get(&self, ns: &Namespace, id: &str) -> Result<Option<Lock>, Error> {
        if !is_well_formed_id(id) {
            return Ok(None);
        }

        let encoded = match &self.backend {
            Backend::Local { root } => match fs::read(Self::path_in(root, ns, id)).await {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            },
            Backend::Bucket(bucket) => bucket.get_bytes(&Self::key_of(ns, id)).await?,
        };

        Ok(encoded.and_then(|bytes| serde_json::from_slice(&bytes).ok()))
    }

    pub async fn list(&self, ns: &Namespace) -> Result<Vec<Lock>, Error> {
        let mut locks = match &self.backend {
            Backend::Local { root } => Self::list_local(&Self::directory_in(root, ns)).await?,
            Backend::Bucket(bucket) => Self::list_bucket(bucket, ns).await?,
        };

        locks.sort_by(|a: &Lock, b: &Lock| a.path.cmp(&b.path));
        Ok(locks)
    }

    async fn list_local(directory: &Path) -> Result<Vec<Lock>, Error> {
        let Ok(mut entries) = fs::read_dir(directory).await else {
            return Ok(Vec::new());
        };

        let mut locks = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            if let Ok(bytes) = fs::read(entry.path()).await
                && let Ok(lock) = serde_json::from_slice(&bytes)
            {
                locks.push(lock);
            }
        }

        Ok(locks)
    }

    // A failure here is an error rather than an empty list. For a capacity
    // figure, answering zero when the store cannot be reached is merely
    // unhelpful; for locks it tells a client every file is free, which is the
    // one answer that loses somebody's work.
    async fn list_bucket(bucket: &Keyspace, ns: &Namespace) -> Result<Vec<Lock>, Error> {
        let mut locks = Vec::new();

        for key in bucket.keys(&Self::prefix(ns)).await? {
            if let Some(bytes) = bucket.get_bytes(&key).await?
                && let Ok(lock) = serde_json::from_slice(&bytes)
            {
                locks.push(lock);
            }
        }

        Ok(locks)
    }

    pub async fn remove(&self, ns: &Namespace, id: &str) -> Result<(), Error> {
        if !is_well_formed_id(id) {
            return Err(Error::LockNotFound);
        }

        let removed = match &self.backend {
            Backend::Local { root } => match fs::remove_file(Self::path_in(root, ns, id)).await {
                Ok(()) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error.into()),
            },
            Backend::Bucket(bucket) => bucket.delete(&Self::key_of(ns, id)).await?,
        };

        removed.then_some(()).ok_or(Error::LockNotFound)
    }

    fn directory_in(root: &Path, ns: &Namespace) -> PathBuf {
        root.join(".locks").join(ns.org()).join(ns.repo())
    }

    fn path_in(root: &Path, ns: &Namespace, id: &str) -> PathBuf {
        Self::directory_in(root, ns).join(format!("{id}.json"))
    }
}

// The clock runs from when the lock was taken, not from the last push to the
// object it covers. Creation is the claim, and it is the one this server can
// answer for without guessing which object a path maps to.
pub fn stale_for(lock: &Lock, max_age: Option<Duration>) -> Option<Duration> {
    let max_age = max_age?;
    let taken = OffsetDateTime::parse(&lock.locked_at, &Rfc3339).ok()?;

    // A negative age is a clock that moved, not a lock from the future.
    let age = Duration::try_from(OffsetDateTime::now_utc() - taken).ok()?;

    (age > max_age).then_some(age)
}

fn is_well_formed_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests;
