mod staging;
mod sweep;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::sync::Mutex;

use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::Error;
use crate::namespace::Namespace;

pub use staging::{Reclaimed, reclaim};
pub use sweep::SweepReport;

// What is left of a repository's budget for one transfer. It travels with the
// upload because a client that skips negotiation may also skip declaring a
// size, and a budget checked once against a number the client chose is not a
// budget.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub used: u64,
    pub limit: u64,
}

impl Budget {
    pub fn exceeded_by(&self, arriving: u64) -> bool {
        self.used + arriving > self.limit
    }

    pub fn refusal(&self) -> Error {
        Error::OverQuota {
            used: self.used,
            limit: self.limit,
        }
    }
}

pub struct LocalStore {
    root: PathBuf,
    counter: AtomicU64,
    usage: Mutex<Option<(Instant, u64, u64)>>,
    per_namespace: Mutex<HashMap<String, (Instant, u64, u64)>>,
    scans: AtomicU64,
    max_object_size: Option<u64>,
}

impl LocalStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            counter: AtomicU64::new(0),
            usage: Mutex::new(None),
            per_namespace: Mutex::new(HashMap::new()),
            scans: AtomicU64::new(0),
            max_object_size: None,
        }
    }

    pub fn with_max_object_size(mut self, limit: Option<u64>) -> Self {
        self.max_object_size = limit;
        self
    }

    pub fn validate_oid(oid: &str) -> Result<(), Error> {
        let well_formed = oid.len() == 64
            && oid
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));

        well_formed.then_some(()).ok_or(Error::MalformedOid)
    }

    fn object_path(&self, ns: &Namespace, oid: &str) -> PathBuf {
        self.root
            .join(ns.org())
            .join(ns.repo())
            .join(&oid[0..2])
            .join(&oid[2..4])
            .join(oid)
    }

    fn content_path(&self, oid: &str) -> PathBuf {
        self.root
            .join(".content")
            .join(&oid[0..2])
            .join(&oid[2..4])
            .join(oid)
    }

    pub fn scans(&self) -> u64 {
        self.scans.load(Ordering::Relaxed)
    }

    pub async fn writable(&self) -> Result<(), Error> {
        fs::create_dir_all(&self.root).await?;

        let ticket = self.counter.fetch_add(1, Ordering::Relaxed);
        let probe = self.root.join(format!(".readiness.{ticket}"));

        fs::write(&probe, b"").await?;
        fs::remove_file(&probe).await?;

        Ok(())
    }

    pub async fn exists(&self, ns: &Namespace, oid: &str) -> bool {
        Self::validate_oid(oid).is_ok() && fs::metadata(self.object_path(ns, oid)).await.is_ok()
    }

    pub async fn open(&self, ns: &Namespace, oid: &str) -> Result<(fs::File, u64), Error> {
        Self::validate_oid(oid)?;
        let path = self.object_path(ns, oid);
        let file = fs::File::open(&path).await.map_err(|_| Error::NotFound)?;
        let size = file.metadata().await?.len();
        Ok((file, size))
    }

    pub async fn write<S, E>(
        &self,
        ns: &Namespace,
        oid: &str,
        expected_size: Option<u64>,
        budget: Option<Budget>,
        mut chunks: S,
    ) -> Result<u64, Error>
    where
        S: Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::validate_oid(oid)?;

        if let Some(limit) = self.max_object_size
            && expected_size.is_some_and(|declared| declared > limit)
        {
            return Err(Error::TooLarge { limit });
        }

        let path = self.object_path(ns, oid);
        let parent = path.parent().expect("object paths always have a parent");
        fs::create_dir_all(parent).await?;

        // A retried transfer of an object this repository already holds costs it
        // no room, so it must not count against the budget a second time.
        let fresh = fs::metadata(&path).await.is_err();

        let staged = self.staging_path(parent, oid);
        let outcome = self.stream_to(&staged, budget, &mut chunks).await;

        match outcome {
            Ok((digest, written)) => {
                self.finish(&staged, &path, oid, expected_size, &digest, written)
                    .await?;

                if fresh {
                    self.stored(ns, written).await;
                }

                Ok(written)
            }
            Err(error) => {
                let _ = fs::remove_file(&staged).await;
                Err(error)
            }
        }
    }

    fn staging_path(&self, parent: &Path, oid: &str) -> PathBuf {
        let ticket = self.counter.fetch_add(1, Ordering::Relaxed);
        parent.join(format!("{oid}.{ticket}.part"))
    }

    async fn stream_to<S, E>(
        &self,
        staged: &Path,
        budget: Option<Budget>,
        chunks: &mut S,
    ) -> Result<(String, u64), Error>
    where
        S: Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let mut file = fs::File::create(staged).await?;
        let mut hasher = Sha256::new();
        let mut written = 0u64;

        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(std::io::Error::other)?;
            hasher.update(&chunk);
            written += chunk.len() as u64;

            // The declared size is a claim by the client, so the ceiling has to
            // hold against a body that ignores it. Stopping at the chunk that
            // crosses the line is the point: reading to the end to find out how
            // big it was would be the outage this limit exists to prevent.
            if let Some(limit) = self.max_object_size.filter(|limit| written > *limit) {
                return Err(Error::TooLarge { limit });
            }

            if let Some(budget) = budget.filter(|budget| budget.exceeded_by(written)) {
                return Err(budget.refusal());
            }

            file.write_all(&chunk).await?;
        }

        file.flush().await?;
        file.sync_all().await?;

        Ok((hex::encode(hasher.finalize()), written))
    }

    async fn finish(
        &self,
        staged: &Path,
        final_path: &Path,
        oid: &str,
        expected_size: Option<u64>,
        digest: &str,
        written: u64,
    ) -> Result<(), Error> {
        if let Some(declared) = expected_size.filter(|declared| *declared != written) {
            let _ = fs::remove_file(staged).await;
            return Err(Error::SizeMismatch {
                declared,
                actual: written,
            });
        }

        if digest != oid {
            let _ = fs::remove_file(staged).await;
            return Err(Error::OidMismatch {
                declared: oid.to_owned(),
                actual: digest.to_owned(),
            });
        }

        self.link_or_move(staged, final_path, oid).await
    }

    // One copy of the bytes under .content, and a hard link per repository that
    // holds them. Two projects sharing an asset pack cost the disk once, and the
    // link count is the reference count — the filesystem does the bookkeeping, so
    // nothing can leak a repository's contents to another and nothing needs a
    // migration: objects already sitting at their repository path keep working as
    // ordinary files with one link.
    async fn link_or_move(&self, staged: &Path, final_path: &Path, oid: &str) -> Result<(), Error> {
        let content = self.content_path(oid);
        let parent = content.parent().expect("content paths have a parent");
        fs::create_dir_all(parent).await?;

        if fs::metadata(&content).await.is_err() {
            fs::rename(staged, &content).await?;
        }

        match self.link(&content, final_path).await {
            // The content was collected between finding it and linking to it:
            // a concurrent retain on another repository dropped its last other
            // reference. The staged copy is still here precisely for this, so
            // put it back and link again rather than failing a push that did
            // nothing wrong.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::rename(staged, &content).await?;
                self.link(&content, final_path).await?;
            }
            outcome => outcome?,
        }

        let _ = fs::remove_file(staged).await;
        Ok(())
    }

    async fn link(&self, content: &Path, final_path: &Path) -> Result<(), std::io::Error> {
        let from = content.to_path_buf();
        let to = final_path.to_path_buf();
        let linked = tokio::task::spawn_blocking(move || std::fs::hard_link(&from, &to))
            .await
            .map_err(std::io::Error::other)?;

        match linked {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(error),
            // A filesystem without hard links, or one crossing a device
            // boundary: fall back to a full copy so the transfer still
            // succeeds. The disk pays for it, the client never notices.
            Err(_) => fs::copy(content, final_path).await.map(|_| ()),
        }
    }
}
