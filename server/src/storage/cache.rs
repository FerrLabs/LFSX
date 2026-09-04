use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::AsyncWriteExt;

use crate::error::Error;
use crate::oid::Oid;

// A local copy of what the bucket holds, so the second client to want an object
// reads it from this disk instead of paying the round trip again. The workload
// this exists for is a CI fleet pulling the same asset pack all day: without it
// every job spends the bucket egress on bytes that have not changed.
//
// What is cached is the stored form, byte for byte what the bucket has, not the
// plaintext. Everything downstream then reads a local file exactly the way the
// volume backend does, so compression, encryption and ranges need to know
// nothing about this, and a cache directory is no more sensitive than the
// bucket it mirrors.
pub struct Cache {
    dir: PathBuf,
    ceiling: u64,
    // Filling happens off the request path, so two clients racing for a cold
    // object must not both fetch it. Whoever arrives second finds the oid here
    // and leaves it to the first.
    filling: Mutex<HashSet<String>>,
    // A cached file is checked against its digest the first time this process
    // serves it. Bit rot does not appear between two reads a second apart, and
    // hashing gigabytes on every hit would spend the disk this exists to save.
    verified: Mutex<HashSet<String>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

pub struct Stats {
    pub hits: u64,
    pub misses: u64,
    pub bytes: u64,
}

impl Cache {
    pub fn new(dir: PathBuf, ceiling: u64) -> Result<Self, Error> {
        std::fs::create_dir_all(&dir).map_err(|error| {
            Error::Storage(std::io::Error::other(format!(
                "the cache directory at {} could not be created: {error}",
                dir.display()
            )))
        })?;

        Ok(Self {
            dir,
            ceiling,
            filling: Mutex::new(HashSet::new()),
            verified: Mutex::new(HashSet::new()),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        })
    }

    fn object(&self, oid: &Oid) -> PathBuf {
        self.dir.join(oid.to_string())
    }

    // The digest of the cached bytes, beside them. It is what makes a truncated
    // or rotted entry detectable, and its modification time is the recency
    // signal eviction sorts on: rewriting 32 bytes on a hit costs nothing and
    // survives a restart, where an in-memory clock would not.
    fn digest(&self, oid: &Oid) -> PathBuf {
        self.dir.join(format!("{oid}.b3"))
    }

    // The cached object, or None when it is not here or cannot be trusted. A
    // file that fails its digest is removed rather than served: the next reader
    // takes the bucket path and fills this again from it.
    pub async fn open(&self, oid: &Oid) -> Option<tokio::fs::File> {
        let path = self.object(oid);
        let Ok(file) = tokio::fs::File::open(&path).await else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };

        let first_time = self.verified.lock().unwrap().insert(oid.to_string());
        if first_time && !self.intact(oid, &path).await {
            tracing::warn!(%oid, "a cached object did not match its digest and was discarded");
            self.verified.lock().unwrap().remove(&oid.to_string());
            self.discard(oid).await;
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        }

        self.touch(oid).await;
        self.hits.fetch_add(1, Ordering::Relaxed);

        Some(file)
    }

    // The same file without the bookkeeping, for a reader that already counted
    // its hit and needs the handle again.
    pub async fn reopen(&self, oid: &Oid) -> Option<tokio::fs::File> {
        tokio::fs::File::open(self.object(oid)).await.ok()
    }

    async fn intact(&self, oid: &Oid, path: &Path) -> bool {
        let Ok(recorded) = tokio::fs::read_to_string(self.digest(oid)).await else {
            return false;
        };
        let Ok(bytes) = tokio::fs::read(path).await else {
            return false;
        };

        blake3::hash(&bytes).to_hex().as_str() == recorded.trim()
    }

    async fn touch(&self, oid: &Oid) {
        if let Ok(recorded) = tokio::fs::read_to_string(self.digest(oid)).await {
            let _ = tokio::fs::write(self.digest(oid), recorded).await;
        }
    }

    async fn discard(&self, oid: &Oid) {
        let _ = tokio::fs::remove_file(self.object(oid)).await;
        let _ = tokio::fs::remove_file(self.digest(oid)).await;
    }

    // Written under a temporary name and renamed, so a crash midway leaves
    // nothing a later reader could mistake for a whole object: an entry appears
    // complete or not at all.
    pub async fn fill<S>(&self, oid: &Oid, chunks: S) -> Result<(), Error>
    where
        S: futures_util::Stream<Item = Result<axum::body::Bytes, Error>>,
    {
        use futures_util::StreamExt;

        let incoming = self.dir.join(format!(".incoming-{oid}"));
        let mut file = tokio::fs::File::create(&incoming).await?;
        let mut hasher = blake3::Hasher::new();
        let mut chunks = std::pin::pin!(chunks);

        while let Some(chunk) = chunks.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    let _ = tokio::fs::remove_file(&incoming).await;
                    return Err(error);
                }
            };
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);

        tokio::fs::write(self.digest(oid), hasher.finalize().to_hex().as_str()).await?;
        tokio::fs::rename(&incoming, self.object(oid)).await?;

        self.evict().await;

        Ok(())
    }

    // True when this caller took responsibility for filling the object, false
    // when somebody else already has it in hand.
    pub fn claim(&self, oid: &Oid) -> bool {
        self.filling.lock().unwrap().insert(oid.to_string())
    }

    pub fn release(&self, oid: &Oid) {
        self.filling.lock().unwrap().remove(&oid.to_string());
    }

    // Oldest use first, until the ceiling is met. Recency is the digest file
    // modification time, which a hit rewrites, so an asset pack pulled every
    // day outlives one fetched once a month whatever their ages.
    async fn evict(&self) {
        let mut entries = Vec::new();
        let mut total = 0;

        let Ok(mut listing) = tokio::fs::read_dir(&self.dir).await else {
            return;
        };
        while let Ok(Some(entry)) = listing.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".b3") || name.starts_with(".incoming-") {
                continue;
            }
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };

            total += metadata.len();
            let used = match tokio::fs::metadata(self.dir.join(format!("{name}.b3"))).await {
                Ok(sidecar) => sidecar.modified().ok(),
                Err(_) => metadata.modified().ok(),
            };
            entries.push((used, metadata.len(), entry.path(), name));
        }

        if total <= self.ceiling {
            return;
        }

        entries.sort_by_key(|(used, ..)| *used);
        for (_, size, path, name) in entries {
            if total <= self.ceiling {
                break;
            }
            if tokio::fs::remove_file(&path).await.is_ok() {
                let _ = tokio::fs::remove_file(self.dir.join(format!("{name}.b3"))).await;
                total -= size;
                self.verified.lock().unwrap().remove(&name);
            }
        }
    }

    pub async fn stats(&self) -> Stats {
        let mut bytes = 0;
        if let Ok(mut listing) = tokio::fs::read_dir(&self.dir).await {
            while let Ok(Some(entry)) = listing.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".b3") || name.starts_with(".incoming-") {
                    continue;
                }
                if let Ok(metadata) = entry.metadata().await {
                    bytes += metadata.len();
                }
            }
        }

        Stats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            bytes,
        }
    }
}

#[cfg(test)]
mod tests;
