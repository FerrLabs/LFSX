use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::LocalStore;
use crate::error::Error;
use crate::namespace::Namespace;

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct VerifyReport {
    pub checked: u64,
    pub bytes: u64,
    pub corrupt: Vec<String>,
    pub unreadable: Vec<String>,
    // An audit that could not see the whole repository must not read like one
    // that found nothing wrong. Silence is the result here, so anything that
    // makes the silence partial has to be said out loud.
    pub incomplete: bool,
}

impl LocalStore {
    // Every object is named after the digest of its own contents, so the store
    // checks itself without a manifest — the property a restore is confirmed
    // with. Compression at rest is what took it away from `sha256sum`: the file
    // is no longer the bytes it is named after, so reading it back has to go
    // through the same path a download does.
    pub async fn verify(&self, ns: &Namespace) -> Result<VerifyReport, Error> {
        let walk = self.objects_of(ns).await;
        let mut report = VerifyReport {
            incomplete: !walk.complete,
            ..VerifyReport::default()
        };

        for found in walk.objects {
            report.checked += 1;

            match self.digest_of(ns, &found.oid).await {
                Ok((digest, read)) => {
                    report.bytes += read;

                    if digest != found.oid {
                        report.corrupt.push(found.oid);
                    }
                }
                // A file that cannot be read is its own kind of answer, and the
                // one a failing disk gives first. Reporting it as corrupt would
                // send an operator looking for the wrong problem.
                Err(error) => {
                    tracing::warn!(oid = found.oid, %error, "object could not be read");
                    report.unreadable.push(found.oid);
                }
            }
        }

        Ok(report)
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
