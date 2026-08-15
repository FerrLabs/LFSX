use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncReadExt;

use super::codec;
use super::{LocalStore, shares_bytes_with};
use crate::error::Error;
use crate::namespace::Namespace;

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct CompressReport {
    pub inspected: u64,
    pub compressed: u64,
    pub already: u64,
    pub left_alone: u64,
    pub refused: u64,
    pub before: u64,
    pub after: u64,
    pub dry_run: bool,
}

impl LocalStore {
    // Turning compression on only changes what arrives next. This is how a store
    // that predates it stops paying full price for what it already holds.
    pub async fn compress(&self, ns: &Namespace, dry_run: bool) -> Result<CompressReport, Error> {
        let level = self.compression.ok_or(Error::CompressionDisabled)?;

        let mut report = CompressReport {
            dry_run,
            ..CompressReport::default()
        };

        let Ok(mut prefixes) = fs::read_dir(self.root.join(ns.org()).join(ns.repo())).await else {
            return Ok(report);
        };

        while let Some(prefix) = prefixes.next_entry().await? {
            let Ok(mut fanouts) = fs::read_dir(prefix.path()).await else {
                continue;
            };

            while let Some(fanout) = fanouts.next_entry().await? {
                self.compress_directory(&fanout.path(), level, &mut report)
                    .await?;
            }
        }

        if !dry_run && report.compressed > 0 {
            self.forget(ns).await;
        }

        Ok(report)
    }

    async fn compress_directory(
        &self,
        directory: &Path,
        level: i32,
        report: &mut CompressReport,
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
            let on_disk = entry.metadata().await?.len();
            report.before += on_disk;

            if self.is_framed(&path, on_disk).await? {
                report.already += 1;
                report.after += on_disk;
                continue;
            }

            report.after += self
                .compress_object(&path, &oid, on_disk, level, report)
                .await?;
        }

        Ok(())
    }

    async fn is_framed(&self, path: &Path, on_disk: u64) -> Result<bool, Error> {
        let file = fs::File::open(path).await?;

        Ok(codec::Framed::open(file, on_disk).await?.is_some())
    }

    async fn compress_object(
        &self,
        path: &Path,
        oid: &str,
        on_disk: u64,
        level: i32,
        report: &mut CompressReport,
    ) -> Result<u64, Error> {
        let parent = path.parent().expect("objects live in a fanout directory");
        let staged = self.staging_path(parent, oid);

        let (digest, compressed) = match self.rewrite(path, &staged, level).await {
            Ok(measured) => measured,
            Err(error) => {
                let _ = fs::remove_file(&staged).await;
                return Err(error);
            }
        };

        // The name is the digest, so this is the same check an operator would
        // run by hand — and the last chance to run it, since afterwards the file
        // is no longer the bytes it is named after.
        if digest != oid {
            tracing::warn!(oid, %digest, "object does not hash to its own name, leaving it alone");
            let _ = fs::remove_file(&staged).await;
            report.refused += 1;
            return Ok(on_disk);
        }

        // An object that will not compress costs a header and an index to store
        // this way. Leaving it is not a failure, it is the right answer.
        if compressed >= on_disk || report.dry_run {
            let _ = fs::remove_file(&staged).await;
            if compressed >= on_disk {
                report.left_alone += 1;
                return Ok(on_disk);
            }

            report.compressed += 1;
            return Ok(compressed);
        }

        self.swap_in(path, &staged, oid).await?;
        report.compressed += 1;

        Ok(compressed)
    }

    // Every repository holding these bytes has a link to one file, so replacing
    // this repository's link with a compressed copy would break the sharing the
    // deduplication just built. The shared copy is what gets replaced, and this
    // repository is relinked to it. Repositories that have not run this yet keep
    // the old bytes alive through their own links until their turn.
    async fn swap_in(&self, path: &Path, staged: &Path, oid: &str) -> Result<(), Error> {
        let content = self.content_path(oid);

        if !shares_bytes_with(path, &content).await {
            return Ok(fs::rename(staged, path).await?);
        }

        fs::rename(staged, &content).await?;

        let parent = path.parent().expect("objects live in a fanout directory");
        let relink = self.staging_path(parent, oid);
        self.link(&content, &relink).await?;

        Ok(fs::rename(&relink, path).await?)
    }

    async fn rewrite(
        &self,
        path: &Path,
        staged: &Path,
        level: i32,
    ) -> Result<(String, u64), Error> {
        let mut source = fs::File::open(path).await?;
        let mut writer = codec::Writer::open(fs::File::create(staged).await?, level).await?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];

        loop {
            let read = source.read(&mut buffer).await?;
            if read == 0 {
                break;
            }

            hasher.update(&buffer[..read]);
            writer.push(&buffer[..read]).await?;
        }

        writer.finish().await?;

        Ok((
            hex::encode(hasher.finalize()),
            fs::metadata(staged).await?.len(),
        ))
    }
}

#[cfg(test)]
mod tests;
