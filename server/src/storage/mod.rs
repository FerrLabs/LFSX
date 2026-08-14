mod sweep;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::Error;
use crate::namespace::Namespace;

pub use sweep::SweepReport;

pub struct LocalStore {
    root: PathBuf,
    counter: AtomicU64,
}

impl LocalStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            counter: AtomicU64::new(0),
        }
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
        mut chunks: S,
    ) -> Result<(), Error>
    where
        S: Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::validate_oid(oid)?;

        let path = self.object_path(ns, oid);
        let parent = path.parent().expect("object paths always have a parent");
        fs::create_dir_all(parent).await?;

        let staged = self.staging_path(parent, oid);
        let outcome = self.stream_to(&staged, &mut chunks).await;

        match outcome {
            Ok((digest, written)) => {
                self.finish(&staged, &path, oid, expected_size, &digest, written)
                    .await
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

    async fn stream_to<S, E>(&self, staged: &Path, chunks: &mut S) -> Result<(String, u64), Error>
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

        fs::rename(staged, final_path).await?;
        Ok(())
    }
}
