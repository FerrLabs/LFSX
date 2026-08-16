mod backend;
mod codec;
mod dedupe;
mod rewrite;
pub mod s3;
mod staging;
mod sweep;
mod verify;
mod walk;

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

pub use backend::Store;
pub use dedupe::DedupeReport;
use dedupe::shares_bytes_with;
pub use rewrite::CompressReport;
pub use verify::VerifyReport;

enum Sink {
    Raw(fs::File),
    Framed(Box<codec::Writer>),
}

impl Sink {
    async fn write(&mut self, chunk: &[u8]) -> Result<(), Error> {
        match self {
            Self::Raw(file) => Ok(file.write_all(chunk).await?),
            Self::Framed(writer) => writer.push(chunk).await,
        }
    }

    async fn finish(self) -> Result<(), Error> {
        match self {
            Self::Raw(mut file) => {
                file.flush().await?;
                Ok(file.sync_all().await?)
            }
            Self::Framed(writer) => writer.finish().await,
        }
    }
}

// A transfer that has passed every check and is waiting to be put somewhere.
pub struct Staged {
    pub path: PathBuf,
    destination: PathBuf,
    pub written: u64,
    fresh: bool,
}

// What a download reads from, whether or not the bytes on disk are the object.
pub enum Object {
    Raw {
        file: fs::File,
        size: u64,
    },
    Framed(codec::Framed),
    Remote {
        bucket: s3::S3Store,
        oid: String,
        size: u64,
    },
}

impl Object {
    pub fn size(&self) -> u64 {
        match self {
            Self::Raw { size, .. } => *size,
            Self::Framed(framed) => framed.plaintext(),
            Self::Remote { size, .. } => *size,
        }
    }

    pub async fn stream(
        self,
        start: u64,
        length: u64,
    ) -> Result<futures_util::stream::BoxStream<'static, Result<axum::body::Bytes, Error>>, Error>
    {
        use futures_util::StreamExt;
        use tokio::io::AsyncSeekExt;

        match self {
            Self::Raw { mut file, .. } => {
                file.seek(std::io::SeekFrom::Start(start)).await?;
                let reader =
                    tokio_util::io::ReaderStream::new(tokio::io::AsyncReadExt::take(file, length));

                Ok(reader.map(|chunk| chunk.map_err(Error::from)).boxed())
            }
            Self::Framed(framed) => Ok(framed.stream(start, length).boxed()),
            Self::Remote { bucket, oid, .. } => {
                let chunks = bucket.read(&oid, start, length).await?;

                Ok(chunks
                    .map(|chunk| {
                        chunk.map_err(|error| Error::Storage(std::io::Error::other(error)))
                    })
                    .boxed())
            }
        }
    }
}
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
    compression: Option<i32>,
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
            compression: None,
        }
    }

    pub fn with_compression(mut self, level: Option<i32>) -> Self {
        self.compression = level;
        self
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

    pub async fn open(&self, ns: &Namespace, oid: &str) -> Result<Object, Error> {
        Self::validate_oid(oid)?;
        let path = self.object_path(ns, oid);
        let file = fs::File::open(&path).await.map_err(|_| Error::NotFound)?;
        let on_disk = file.metadata().await?.len();

        match codec::Framed::open(file, on_disk).await? {
            Some(framed) => Ok(Object::Framed(framed)),
            None => Ok(Object::Raw {
                file: fs::File::open(&path).await.map_err(|_| Error::NotFound)?,
                size: on_disk,
            }),
        }
    }

    // Everything a transfer has to survive before it counts as an object: the
    // digest it claims, the size it declared, the ceiling on a single object and
    // the repository's remaining budget. It ends on local disk whatever the
    // backend is, because a bucket cannot be asked to hold bytes that might turn
    // out to be the wrong ones.
    pub async fn stage<S, E>(
        &self,
        ns: &Namespace,
        oid: &str,
        expected_size: Option<u64>,
        budget: Option<Budget>,
        mut chunks: S,
    ) -> Result<Staged, Error>
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

        match self.stream_to(&staged, budget, &mut chunks).await {
            Ok((digest, written)) => {
                if let Err(error) = Self::agrees(oid, expected_size, &digest, written) {
                    let _ = fs::remove_file(&staged).await;
                    return Err(error);
                }

                Ok(Staged {
                    path: staged,
                    destination: path,
                    written,
                    fresh,
                })
            }
            Err(error) => {
                let _ = fs::remove_file(&staged).await;
                Err(error)
            }
        }
    }

    fn agrees(
        oid: &str,
        expected_size: Option<u64>,
        digest: &str,
        written: u64,
    ) -> Result<(), Error> {
        if let Some(declared) = expected_size.filter(|declared| *declared != written) {
            return Err(Error::SizeMismatch {
                declared,
                actual: written,
            });
        }

        if digest != oid {
            return Err(Error::OidMismatch {
                declared: oid.to_owned(),
                actual: digest.to_owned(),
            });
        }

        Ok(())
    }

    pub async fn write<S, E>(
        &self,
        ns: &Namespace,
        oid: &str,
        expected_size: Option<u64>,
        budget: Option<Budget>,
        chunks: S,
    ) -> Result<u64, Error>
    where
        S: Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let staged = self.stage(ns, oid, expected_size, budget, chunks).await?;

        self.link_or_move(&staged.path, &staged.destination, oid)
            .await?;

        if staged.fresh {
            self.stored(ns, staged.written).await;
        }

        Ok(staged.written)
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
        let file = fs::File::create(staged).await?;
        // The digest, the declared size and the budget are all counted on the
        // plaintext going past, whatever the bytes look like once they land —
        // so compression is a different sink, not a different path.
        let mut sink = match self.compression {
            Some(level) => Sink::Framed(Box::new(codec::Writer::open(file, level).await?)),
            None => Sink::Raw(file),
        };
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

            sink.write(&chunk).await?;
        }

        sink.finish().await?;

        Ok((hex::encode(hasher.finalize()), written))
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
