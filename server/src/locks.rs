use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::Error;
use crate::namespace::Namespace;

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

pub struct LockStore {
    root: PathBuf,
}

impl LockStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn id_of(path: &str) -> String {
        hex::encode(Sha256::digest(path.as_bytes()))[..32].to_owned()
    }

    fn directory(&self, ns: &Namespace) -> PathBuf {
        self.root.join(".locks").join(ns.org()).join(ns.repo())
    }

    fn path_of(&self, ns: &Namespace, id: &str) -> PathBuf {
        self.directory(ns).join(format!("{id}.json"))
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

        let directory = self.directory(ns);
        fs::create_dir_all(&directory).await?;

        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path_of(ns, &lock.id))
            .await;

        match file {
            Ok(mut file) => {
                file.write_all(&serde_json::to_vec(&lock)?).await?;
                file.sync_all().await?;
                Ok(lock)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                match self.get(ns, &lock.id).await? {
                    Some(held) => Err(Error::LockHeld(Box::new(held))),
                    None => Err(Error::LockHeld(Box::new(lock))),
                }
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn get(&self, ns: &Namespace, id: &str) -> Result<Option<Lock>, Error> {
        if !is_well_formed_id(id) {
            return Ok(None);
        }

        match fs::read(self.path_of(ns, id)).await {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn list(&self, ns: &Namespace) -> Result<Vec<Lock>, Error> {
        let Ok(mut entries) = fs::read_dir(self.directory(ns)).await else {
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

        locks.sort_by(|a: &Lock, b: &Lock| a.path.cmp(&b.path));
        Ok(locks)
    }

    pub async fn remove(&self, ns: &Namespace, id: &str) -> Result<(), Error> {
        if !is_well_formed_id(id) {
            return Err(Error::LockNotFound);
        }

        fs::remove_file(self.path_of(ns, id))
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Error::LockNotFound,
                _ => error.into(),
            })
    }
}

fn is_well_formed_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests;
