use std::path::Path;

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::fs;

use super::LocalStore;
use crate::error::Error;
use crate::namespace::Namespace;

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct VerifyReport {
    pub checked: u64,
    pub bytes: u64,
    pub corrupt: Vec<String>,
    pub unreadable: Vec<String>,
}

impl LocalStore {
    // Every object is named after the digest of its own contents, so the store
    // checks itself without a manifest — the property a restore is confirmed
    // with. Compression at rest is what took it away from `sha256sum`: the file
    // is no longer the bytes it is named after, so reading it back has to go
    // through the same path a download does.
    pub async fn verify(&self, ns: &Namespace) -> Result<VerifyReport, Error> {
        let mut report = VerifyReport::default();

        let Ok(mut prefixes) = fs::read_dir(self.root.join(ns.org()).join(ns.repo())).await else {
            return Ok(report);
        };

        while let Some(prefix) = prefixes.next_entry().await? {
            let Ok(mut fanouts) = fs::read_dir(prefix.path()).await else {
                continue;
            };

            while let Some(fanout) = fanouts.next_entry().await? {
                self.verify_directory(&fanout.path(), ns, &mut report).await;
            }
        }

        Ok(report)
    }

    async fn verify_directory(&self, directory: &Path, ns: &Namespace, report: &mut VerifyReport) {
        let Ok(mut entries) = fs::read_dir(directory).await else {
            return;
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let oid = entry.file_name().to_string_lossy().into_owned();
            if Self::validate_oid(&oid).is_err() {
                continue;
            }

            report.checked += 1;

            match self.digest_of(ns, &oid).await {
                Ok((digest, read)) => {
                    report.bytes += read;

                    if digest != oid {
                        report.corrupt.push(oid);
                    }
                }
                // A file that cannot be read is its own kind of answer, and the
                // one a failing disk gives first. Reporting it as corrupt would
                // send an operator looking for the wrong problem.
                Err(error) => {
                    tracing::warn!(oid, %error, "object could not be read");
                    report.unreadable.push(oid);
                }
            }
        }
    }

    async fn digest_of(&self, ns: &Namespace, oid: &str) -> Result<(String, u64), Error> {
        let object = self.open(ns, oid).await?;
        let size = object.size();

        let mut hasher = Sha256::new();
        let mut read = 0u64;
        let mut chunks = object.stream(0, size).await?;

        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            read += chunk.len() as u64;
            hasher.update(&chunk);
        }

        Ok((hex::encode(hasher.finalize()), read))
    }
}

#[cfg(test)]
mod tests;
