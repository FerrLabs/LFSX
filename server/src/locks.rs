use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::Error;
use crate::namespace::Namespace;
use crate::storage::s3::S3Store;

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
pub struct LockStore(Backend);

enum Backend {
    Local { root: PathBuf },
    Bucket(Box<S3Store>),
}

impl LockStore {
    pub fn local(root: impl Into<PathBuf>) -> Self {
        Self(Backend::Local { root: root.into() })
    }

    pub fn bucket(bucket: S3Store) -> Self {
        Self(Backend::Bucket(Box::new(bucket)))
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

        let taken = match &self.0 {
            Backend::Local { root } => {
                Self::write_new(&Self::path_in(root, ns, &lock.id), &encoded).await?
            }
            Backend::Bucket(bucket) => {
                bucket
                    .put_if_absent(&Self::key_of(ns, &lock.id), encoded)
                    .await?
            }
        };

        if taken {
            return Ok(lock);
        }

        // Whoever holds it is the useful half of the answer, and reading it back
        // can race with a release. Naming the caller's own attempt is a worse
        // answer than none, so a lock that vanished underfoot is reported as
        // held by nobody in particular rather than by the caller.
        match self.get(ns, &lock.id).await? {
            Some(held) => Err(Error::LockHeld(Box::new(held))),
            None => Err(Error::LockHeld(Box::new(lock))),
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

        let encoded = match &self.0 {
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
        let mut locks = match &self.0 {
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
    async fn list_bucket(bucket: &S3Store, ns: &Namespace) -> Result<Vec<Lock>, Error> {
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

        let removed = match &self.0 {
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

fn is_well_formed_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests;
