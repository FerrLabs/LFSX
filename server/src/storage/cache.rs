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
    // Being hashed right now. An entry is only in `verified` once it passed,
    // so a reader can never serve one on the strength of a check still running.
    verifying: Mutex<HashSet<String>>,
    // What the directory holds, kept as fills and evictions move it. The
    // alternative is a walk per scrape, thousands of syscalls spent on the disk
    // this exists to spare.
    bytes: AtomicU64,
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

        let held = std::fs::read_dir(&dir)
            .map(|listing| {
                listing
                    .flatten()
                    .filter(|entry| !skip(&entry.file_name().to_string_lossy()))
                    .filter_map(|entry| entry.metadata().ok())
                    .map(|metadata| metadata.len())
                    .sum()
            })
            .unwrap_or(0);

        Ok(Self {
            dir,
            ceiling,
            filling: Mutex::new(HashSet::new()),
            verified: Mutex::new(HashSet::new()),
            verifying: Mutex::new(HashSet::new()),
            bytes: AtomicU64::new(held),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        })
    }

    // An object bigger than the whole cache can never be kept: filling it would
    // evict everything else and then itself, so the next download does it all
    // again. Not fetching it at all is the only outcome that leaves the cache
    // useful to everybody else.
    pub fn fits(&self, size: u64) -> bool {
        size <= self.ceiling
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

        if !self.verified.lock().unwrap().contains(&oid.to_string()) {
            // Somebody else is already hashing this one. Waiting on them would
            // be the other answer, but taking the bucket path costs a round
            // trip where trusting an entry nobody has checked yet costs the
            // thing this check exists to prevent.
            if !self.verifying.lock().unwrap().insert(oid.to_string()) {
                self.misses.fetch_add(1, Ordering::Relaxed);
                return None;
            }

            let intact = self.intact(oid, &path).await;
            self.verifying.lock().unwrap().remove(&oid.to_string());

            if !intact {
                tracing::warn!(%oid, "a cached object did not match its digest and was discarded");
                self.discard(oid).await;
                self.misses.fetch_add(1, Ordering::Relaxed);
                return None;
            }

            self.verified.lock().unwrap().insert(oid.to_string());
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

    // Hashed in frames, never held whole. A cached pack is measured in
    // gigabytes and this runs on the first serve of one, so reading it into
    // memory would kill the pod for the crime of checking a file it already
    // had, and would give up the property the rest of the read path keeps:
    // the object never exists in memory all at once.
    async fn intact(&self, oid: &Oid, path: &Path) -> bool {
        use tokio::io::AsyncReadExt;

        let Ok(recorded) = tokio::fs::read_to_string(self.digest(oid)).await else {
            return false;
        };
        let Ok(file) = tokio::fs::File::open(path).await else {
            return false;
        };

        let mut reader = tokio::io::BufReader::new(file);
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0u8; 128 * 1024];
        loop {
            let Ok(read) = reader.read(&mut buffer).await else {
                return false;
            };
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }

        hasher.finalize().to_hex().as_str() == recorded.trim()
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
    pub async fn fill<S>(&self, oid: &Oid, size: u64, chunks: S) -> Result<(), Error>
    where
        S: futures_util::Stream<Item = Result<axum::body::Bytes, Error>>,
    {
        use futures_util::StreamExt;

        let incoming = self.dir.join(format!(".incoming-{oid}"));
        let mut file = tokio::fs::File::create(&incoming).await?;
        let mut hasher = blake3::Hasher::new();
        let mut written = 0;
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
            written += chunk.len() as u64;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);

        // A body that ended early without saying so would otherwise become a
        // permanent entry whose digest matches its own truncation, served later
        // against the length the bucket reports. The digest cannot catch that;
        // only the count can.
        if written != size {
            let _ = tokio::fs::remove_file(&incoming).await;
            tracing::warn!(
                %oid,
                written,
                expected = size,
                "the bucket sent fewer bytes than it said it had, so nothing was cached"
            );
            return Err(Error::Storage(std::io::Error::other(
                "a short read cannot be cached",
            )));
        }

        tokio::fs::write(self.digest(oid), hasher.finalize().to_hex().as_str()).await?;
        tokio::fs::rename(&incoming, self.object(oid)).await?;
        self.bytes.fetch_add(size, Ordering::Relaxed);

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
            if skip(&name) {
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
                self.bytes.fetch_sub(
                    size.min(self.bytes.load(Ordering::Relaxed)),
                    Ordering::Relaxed,
                );
                self.verified.lock().unwrap().remove(&name);
            }
        }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
        }
    }
}

// The bookkeeping beside an object, and a fill still in flight: neither is a
// cached object and neither counts against the ceiling.
fn skip(name: &str) -> bool {
    name.ends_with(".b3") || name.starts_with(".incoming-")
}

#[cfg(test)]
mod tests;
