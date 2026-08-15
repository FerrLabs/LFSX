use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncReadExt;

use super::LocalStore;
use crate::error::Error;
use crate::namespace::Namespace;

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct DedupeReport {
    pub inspected: u64,
    pub already_shared: u64,
    pub adopted: u64,
    pub linked: u64,
    pub reclaimed: u64,
    pub refused: u64,
    pub dry_run: bool,
}

impl LocalStore {
    // Objects written before the shared store existed are ordinary files with a
    // single link. They serve correctly and they never collapse, so a server
    // that predates deduplication keeps paying full price for every pack two
    // projects share. This folds them in, one repository at a time.
    pub async fn dedupe(&self, ns: &Namespace, dry_run: bool) -> Result<DedupeReport, Error> {
        let mut report = DedupeReport {
            dry_run,
            ..DedupeReport::default()
        };

        let Ok(mut prefixes) = fs::read_dir(self.root.join(ns.org()).join(ns.repo())).await else {
            return Ok(report);
        };

        while let Some(prefix) = prefixes.next_entry().await? {
            let Ok(mut fanouts) = fs::read_dir(prefix.path()).await else {
                continue;
            };

            while let Some(fanout) = fanouts.next_entry().await? {
                self.dedupe_directory(&fanout.path(), &mut report).await?;
            }
        }

        if !dry_run && (report.adopted > 0 || report.linked > 0) {
            self.forget(ns).await;
        }

        Ok(report)
    }

    async fn dedupe_directory(
        &self,
        directory: &Path,
        report: &mut DedupeReport,
    ) -> Result<(), Error> {
        let Ok(mut entries) = fs::read_dir(directory).await else {
            return Ok(());
        };

        while let Some(entry) = entries.next_entry().await? {
            let oid = entry.file_name().to_string_lossy().into_owned();
            if Self::validate_oid(&oid).is_err() {
                continue;
            }

            report.inspected += 1;
            let path = entry.path();
            let content = self.content_path(&oid);

            if shares_bytes_with(&path, &content).await {
                report.already_shared += 1;
                continue;
            }

            match fs::metadata(&content).await {
                Ok(shared) => {
                    self.adopt(&path, &content, &oid, shared.len(), report)
                        .await?
                }
                Err(_) => self.promote(&path, &content, &oid, report).await?,
            }
        }

        Ok(())
    }

    // The shared store already holds these bytes. Replacing this repository's
    // copy with a link to them is what frees the disk — but only after checking
    // that what is there really is this object: linking to a corrupt entry would
    // spread it to a repository that had a good copy of its own.
    async fn adopt(
        &self,
        path: &Path,
        content: &Path,
        oid: &str,
        size: u64,
        report: &mut DedupeReport,
    ) -> Result<(), Error> {
        if report.dry_run {
            report.linked += 1;
            report.reclaimed += size;
            return Ok(());
        }

        if !hashes_to(content, oid).await {
            tracing::warn!(
                oid,
                "shared copy does not hash to its own name, leaving the repository's own file alone"
            );
            report.refused += 1;
            return Ok(());
        }

        let parent = path.parent().expect("objects live in a fanout directory");
        let staged = self.staging_path(parent, oid);

        self.link(content, &staged).await?;
        // Rename over the original rather than removing it first: a crash here
        // leaves either the old file or the new link, never a gap where the
        // repository has no object at all.
        fs::rename(&staged, path).await?;

        report.linked += 1;
        report.reclaimed += size;

        Ok(())
    }

    // Nothing shares these bytes yet, so this repository's copy becomes the
    // shared one and gets a link back in its place. Nothing is freed today; the
    // next repository to hold the same object is the one that stops paying.
    async fn promote(
        &self,
        path: &Path,
        content: &Path,
        oid: &str,
        report: &mut DedupeReport,
    ) -> Result<(), Error> {
        if report.dry_run {
            report.adopted += 1;
            return Ok(());
        }

        if !hashes_to(path, oid).await {
            tracing::warn!(
                oid,
                "object does not hash to its own name, leaving it out of the shared store"
            );
            report.refused += 1;
            return Ok(());
        }

        let parent = content.parent().expect("content paths have a parent");
        fs::create_dir_all(parent).await?;

        fs::rename(path, content).await?;
        if let Err(error) = self.link(content, path).await {
            // Put it back where the repository expects it rather than leave the
            // object reachable only from the shared store.
            fs::rename(content, path).await?;
            return Err(error.into());
        }

        report.adopted += 1;

        Ok(())
    }
}

// Two paths that resolve to the same inode are already one set of bytes with
// two names, which is exactly what deduplication produces — so this is how a
// second run knows there is nothing left to do.
#[cfg(unix)]
pub(super) async fn shares_bytes_with(path: &Path, content: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let (Ok(one), Ok(other)) = (fs::metadata(path).await, fs::metadata(content).await) else {
        return false;
    };

    (one.dev(), one.ino()) == (other.dev(), other.ino())
}

// Without inode numbers there is no way to tell a link from a copy, so every
// run relinks. The result is the same, the work is repeated. The server ships
// on Linux; this keeps the tests honest everywhere else.
#[cfg(not(unix))]
pub(super) async fn shares_bytes_with(_path: &Path, _content: &Path) -> bool {
    false
}

async fn hashes_to(path: &Path, oid: &str) -> bool {
    let Ok(mut file) = fs::File::open(path).await else {
        return false;
    };

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 128 * 1024];

    loop {
        match file.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(_) => return false,
        }
    }

    hex::encode(hasher.finalize()) == oid
}

#[cfg(test)]
mod tests;
