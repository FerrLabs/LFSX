pub(crate) mod keyspace;

use std::time::Duration;

use axum::body::Bytes;
use futures_util::Stream;

use base64::Engine;

use crate::error::Error;
use crate::namespace::Namespace;
use crate::storage::{Reclaimed, SweepReport};

pub use keyspace::{Keyspace, Presigned};

const CHECKSUM: &str = "x-amz-checksum-sha256";

pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub path_style: bool,
    // How long a signature is good for. It is the same number the batch
    // response advertises as `expires_in`, because a client told it has half an
    // hour and handed a URL that dies in five minutes will fail a resume it had
    // every reason to expect to work.
    pub lifetime: Duration,
}

// The same layout as the local store, for the same reasons. The bytes live once
// under a key derived from their digest, and a repository that holds them owns
// an empty marker beside it — the object store's answer to a hard link. It is
// what keeps two projects sharing an asset pack from paying twice, and what
// stops a repository reading an object it never pushed: the marker is the proof
// of possession, and it is the only thing the permission check consults.
//
// Everything below is object semantics. What it takes to talk to the store at
// all — signing, retrying, listing, the client — is the keyspace underneath.
#[derive(Clone)]
pub struct S3Store {
    keys: Keyspace,
    redirect: bool,
}

impl S3Store {
    pub fn new(keys: Keyspace, redirect: bool) -> Self {
        Self { keys, redirect }
    }

    fn content_key(oid: &str) -> String {
        format!(".content/{}/{}/{oid}", &oid[0..2], &oid[2..4])
    }

    // Where a client uploads to when the bytes never pass through this server.
    // Per repository on purpose: the shared content key would take bytes from
    // anyone allowed to write, and then nothing distinguishes a repository that
    // uploaded an object from one that merely knew its digest. A key only this
    // repository was handed a signature for is the proof of possession that the
    // marker stands for everywhere else.
    fn incoming_key(ns: &Namespace, oid: &str) -> String {
        format!(
            ".incoming/{}/{}/{}/{}/{oid}",
            ns.org(),
            ns.repo(),
            &oid[0..2],
            &oid[2..4]
        )
    }

    fn marker_key(ns: &Namespace, oid: &str) -> String {
        format!(
            "{}/{}/{}/{}/{oid}",
            ns.org(),
            ns.repo(),
            &oid[0..2],
            &oid[2..4]
        )
    }

    pub async fn reachable(&self) -> Result<(), Error> {
        self.keys.reachable().await
    }

    pub async fn exists(&self, ns: &Namespace, oid: &str) -> bool {
        if crate::storage::LocalStore::validate_oid(oid).is_err() {
            return false;
        }

        self.keys.head(&Self::marker_key(ns, oid)).await.is_ok()
    }

    pub async fn size_of(&self, oid: &str) -> Result<u64, Error> {
        // Every entry point validates before slicing an oid into a key: the
        // fanout takes the first four characters, so a short one is a panic
        // rather than a refusal, and a panic is a 500 for something that should
        // have been a 422.
        crate::storage::LocalStore::validate_oid(oid)?;

        self.keys.head(&Self::content_key(oid)).await
    }

    // A download is streamed through this server rather than redirected, so the
    // features that live in the byte path — the counters, the ranges, and the
    // compression that will follow — keep working. The pre-signed redirect is a
    // separate mode for operators who would rather spend the object store's
    // bandwidth than their own.
    pub async fn read(
        &self,
        oid: &str,
        start: u64,
        length: u64,
    ) -> Result<impl Stream<Item = Result<Bytes, reqwest::Error>> + use<>, Error> {
        crate::storage::LocalStore::validate_oid(oid)?;

        self.keys
            .get_range(&Self::content_key(oid), start, length)
            .await
    }

    // A URL the client fetches from the bucket directly, so the bytes never
    // cross this server. Whether the caller is entitled to them has already been
    // settled by the marker before this is called: the signature is scoped to
    // one content key and expires, and it grants nothing the batch response was
    // not about to grant anyway.
    pub fn presigned_download(&self, oid: &str) -> Option<String> {
        if !self.redirect || crate::storage::LocalStore::validate_oid(oid).is_err() {
            return None;
        }

        Some(self.keys.signed_download(&Self::content_key(oid)))
    }

    // A URL the client PUTs the object to, and the headers it has to send with
    // it. The digest is bound into the signature, so the store refuses anything
    // that does not hash to the object it was signed for: a client with this URL
    // cannot put arbitrary bytes anywhere, which is what makes handing one out
    // safe at all.
    pub fn presigned_upload(&self, ns: &Namespace, oid: &str) -> Option<Presigned> {
        if !self.redirect || crate::storage::LocalStore::validate_oid(oid).is_err() {
            return None;
        }

        let digest = base64::engine::general_purpose::STANDARD.encode(hex::decode(oid).ok()?);

        Some(self.keys.signed_upload(
            &Self::incoming_key(ns, oid),
            vec![(CHECKSUM.to_owned(), digest)],
        ))
    }

    // How big the object a client uploaded actually is, which is the first thing
    // this server learns about it: nothing measured the bytes on the way past.
    pub async fn uploaded_size(&self, ns: &Namespace, oid: &str) -> Result<u64, Error> {
        crate::storage::LocalStore::validate_oid(oid)?;

        self.keys.head(&Self::incoming_key(ns, oid)).await
    }

    // Take an upload that landed under this repository's own key into the shared
    // keyspace. The bytes are already known to hash to the oid, because the store
    // refused everything else.
    pub async fn adopt(&self, ns: &Namespace, oid: &str) -> Result<(), Error> {
        crate::storage::LocalStore::validate_oid(oid)?;

        let incoming = Self::incoming_key(ns, oid);
        let content = Self::content_key(oid);

        // Already there means another repository pushed the same object, and the
        // bytes are identical by construction.
        if self.keys.head(&content).await.is_err() {
            self.keys.copy(&incoming, &content).await?;
        }

        self.keys
            .put(
                &Self::marker_key(ns, oid),
                reqwest::Body::from(Vec::new()),
                0,
            )
            .await?;

        // Leaving it would pay for the object twice. A failure here is not worth
        // failing the push over: the object is adopted, and what is left is a key
        // the operator can see.
        if let Err(error) = self.keys.delete(&incoming).await {
            tracing::warn!(%error, key = incoming, "an adopted upload could not be cleaned up");
        }

        Ok(())
    }

    // The upload has already been streamed to a staging file, hashed and checked
    // against everything the server enforces, so that file is what goes up —
    // streamed from disk rather than read into memory, because an object here is
    // measured in gigabytes and the whole storage layer is built on holding at
    // most a few megabytes of one at a time.
    //
    // The bytes go up once, keyed by their digest, and the marker records that
    // this repository holds them. Content that is already there is skipped: the
    // key would receive the same bytes it already has.
    pub async fn store(
        &self,
        ns: &Namespace,
        oid: &str,
        staged: &std::path::Path,
    ) -> Result<(), Error> {
        crate::storage::LocalStore::validate_oid(oid)?;

        if self.keys.head(&Self::content_key(oid)).await.is_err() {
            let file = tokio::fs::File::open(staged).await?;
            let length = file.metadata().await?.len();
            let stream = tokio_util::io::ReaderStream::new(file);

            self.keys
                .put(
                    &Self::content_key(oid),
                    reqwest::Body::wrap_stream(stream),
                    length,
                )
                .await?;
        }

        self.keys
            .put(
                &Self::marker_key(ns, oid),
                reqwest::Body::from(Vec::new()),
                0,
            )
            .await
    }

    // What an interrupted upload leaves behind. A client can negotiate, PUT the
    // object, and never report it: the bytes sit under its own upload key and
    // nothing else will ever look at them. The local path has had a reclaimer for
    // this since the beginning, and a bucket had none, so the cost was unbounded
    // over time and invisible.
    pub async fn reclaim_incoming(&self, older_than: Duration) -> Result<Reclaimed, Error> {
        let mut reclaimed = Reclaimed::default();

        for entry in self.keys.entries(".incoming/").await? {
            // A slow client on a bad connection is not an abandoned one.
            if entry.age().is_none_or(|age| age < older_than) {
                continue;
            }

            if self.keys.delete(&entry.key).await.is_ok() {
                reclaimed.files += 1;
                reclaimed.bytes += entry.size;
            }
        }

        Ok(reclaimed)
    }

    // What the bucket holds for this repository, counted from its markers and
    // the content they point at. The markers are empty, so their own size says
    // nothing — this is a listing plus one head per object, which is why the
    // figure is cached the same way the local one is.
    pub async fn usage_of(&self, ns: &Namespace) -> (u64, u64) {
        let prefix = format!("{}/{}/", ns.org(), ns.repo());
        let mut objects = 0;
        let mut bytes = 0;

        for oid in self.list(&prefix).await {
            objects += 1;
            bytes += self.size_of(&oid).await.unwrap_or_default();
        }

        (objects, bytes)
    }

    async fn list(&self, prefix: &str) -> Vec<String> {
        // A capacity figure that silently reads zero is worse than one that is
        // missing, because it looks like an answer.
        let keys = match self.keys.keys(prefix).await {
            Ok(keys) => keys,
            Err(error) => {
                tracing::warn!(%error, "the object store could not be listed");
                return Vec::new();
            }
        };

        keys.into_iter()
            .filter_map(|key| key.rsplit('/').next().map(str::to_owned))
            .filter(|oid| crate::storage::LocalStore::validate_oid(oid).is_ok())
            .collect()
    }
}

impl S3Store {
    // Collection, with the marker keyspace standing in for the link count a
    // filesystem keeps. A repository's markers are its claim on the bytes; the
    // bytes go when the last claim anywhere does.
    //
    // The expensive question is that "anywhere". A filesystem answers it with one
    // stat, because it counts links itself. A bucket has to be asked, and asking
    // per object would cost objects times repositories, which is the shape the
    // local sweep was rewritten to avoid. So every marker in the bucket is listed
    // once, into the set of oids somebody else still claims, and each candidate
    // is then decided in memory.
    //
    // Cost, for a sweep of one repository: one listing of that repository's
    // markers, one listing of every marker in the bucket, then per object freed a
    // HEAD for its size and two deletes. Listings page a thousand keys at a time,
    // so the whole thing is linear in what the bucket holds rather than in the
    // product, and nothing is held in memory but the oids.
    pub async fn sweep(
        &self,
        ns: &Namespace,
        retained: &std::collections::HashSet<String>,
        grace: Duration,
        dry_run: bool,
    ) -> Result<SweepReport, Error> {
        let mut report = SweepReport {
            dry_run,
            ..SweepReport::default()
        };

        let prefix = format!("{}/{}/", ns.org(), ns.repo());
        let mine = self.keys.entries(&prefix).await?;

        let mut candidates = Vec::new();
        for entry in mine {
            let Some(oid) = entry.key.rsplit('/').next() else {
                continue;
            };
            if crate::storage::LocalStore::validate_oid(oid).is_err() || retained.contains(oid) {
                continue;
            }

            // A slow client on a bad connection is not an abandoned one, and the
            // same goes for an object pushed a moment ago by somebody whose
            // retained set was built before it existed.
            if entry.age().is_none_or(|age| age < grace) {
                report.within_grace += 1;
                continue;
            }

            candidates.push((entry.key.clone(), oid.to_owned()));
        }

        if candidates.is_empty() {
            return Ok(report);
        }

        // Only asked when something is actually going, so a sweep that frees
        // nothing costs one listing rather than two.
        let elsewhere = self.claimed_elsewhere(&prefix).await;

        for (marker, oid) in candidates {
            report.swept += 1;

            let last_claim = match &elsewhere {
                Some(claimed) => !claimed.contains(&oid),
                // The listing could not be finished, so nothing can be proven
                // about who else holds this object. The marker still goes, which
                // is this repository's own business; the bytes stay, because
                // deleting them on an incomplete answer is how a sweep takes an
                // asset another project was still using.
                None => false,
            };

            if dry_run {
                if last_claim {
                    report.bytes += self.size_of(&oid).await.unwrap_or_default();
                }
                continue;
            }

            self.keys.delete(&marker).await?;

            if !last_claim {
                continue;
            }

            let size = self.size_of(&oid).await.unwrap_or_default();

            // Counted only when this call is the one that removed them, so two
            // repositories dropping their last claim at once cannot both report
            // the same space.
            if self
                .keys
                .delete(&Self::content_key(&oid))
                .await
                .unwrap_or(false)
            {
                report.bytes += size;
            }
        }

        report.incomplete = elsewhere.is_none();

        Ok(report)
    }

    // Every oid still claimed by a repository other than the one being swept.
    // None when the listing could not be finished, which is not the same as
    // nobody claiming anything and must not be read as it.
    async fn claimed_elsewhere(&self, mine: &str) -> Option<std::collections::HashSet<String>> {
        let entries = match self.keys.entries("").await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(%error, "the markers could not be listed, so no content is collected");
                return None;
            }
        };

        Some(
            entries
                .into_iter()
                .filter(|entry| !entry.key.starts_with('.') && !entry.key.starts_with(mine))
                .filter_map(|entry| entry.key.rsplit('/').next().map(str::to_owned))
                .filter(|oid| crate::storage::LocalStore::validate_oid(oid).is_ok())
                .collect(),
        )
    }
}

#[cfg(test)]
pub(crate) mod tests;
