use futures_util::Stream;

use super::s3::S3Store;
use super::{Budget, CompressReport, DedupeReport, LocalStore, Object, SweepReport, VerifyReport};
use crate::error::Error;
use crate::namespace::Namespace;
use crate::oid::Oid;
#[cfg(test)]
use sha2::Digest;
#[cfg(test)]
use std::time::Duration;

// Where the objects live. A bucket decouples capacity from the machine, at the
// price of the things a filesystem gave for nothing: hard links, a directory
// walk, and a rename that is atomic. Each of those is answered here or refused
// out loud; none of them is quietly skipped.
pub struct Store {
    backend: Backend,
    usage: super::usage::Usage,
}

enum Backend {
    Local(LocalStore),
    // Even with a bucket the local store stays, because a transfer has to land
    // somewhere before anyone can tell whether it is the object it claims to be.
    // It is a write buffer, not the store.
    // Boxed because a bucket handle beside a local store makes this variant far
    // larger than the other, and every Store in the process would pay for it.
    Bucket {
        bucket: Box<S3Store>,
        staging: LocalStore,
    },
}

impl Store {
    pub fn local(store: LocalStore) -> Self {
        Self::over(Backend::Local(store))
    }

    // Compression and encryption used to be stripped here, because a framed
    // object was only readable through the file the codec opened and a bucket key
    // is not one. The codec now reads from a bucket too, so the frames go up as
    // they are and come back decoded: the header and the index are three ranged
    // GETs, which is what the format was shaped for.
    pub fn bucket(bucket: S3Store, staging: LocalStore) -> Self {
        Self::over(Backend::Bucket {
            bucket: Box::new(bucket),
            staging,
        })
    }

    fn over(backend: Backend) -> Self {
        Self {
            backend,
            usage: super::usage::Usage::default(),
        }
    }

    fn staging(&self) -> &LocalStore {
        match &self.backend {
            Backend::Local(store) => store,
            Backend::Bucket { staging, .. } => staging,
        }
    }

    // Everything an interrupted upload can leave behind, wherever it left it. A
    // bucket deployment still stages locally, so both are swept and the figures
    // add up to one answer.
    pub async fn reclaim(&self, older_than: std::time::Duration) -> super::Reclaimed {
        let mut reclaimed = self.staging().reclaim_staging(older_than).await;

        if let Backend::Bucket { bucket, .. } = &self.backend {
            match bucket.reclaim_incoming(older_than).await {
                Ok(theirs) => {
                    reclaimed.files += theirs.files;
                    reclaimed.bytes += theirs.bytes;
                }
                Err(error) => {
                    tracing::warn!(%error, "abandoned uploads in the bucket could not be reclaimed");
                }
            }
        }

        reclaimed
    }

    // Readiness has to ask the backend that actually serves. Once the objects
    // live in a bucket the volume is a write buffer, and an instance whose
    // credentials were rotated or whose bucket is gone passes a probe that only
    // proves its scratch disk works, then fails every transfer it is handed.
    //
    // So both are asked, either failing takes the instance out, and they are
    // named apart: a full disk and a rotated key are not the same afternoon.
    pub async fn writable(&self) -> Result<(), Error> {
        self.staging().writable().await.map_err(|error| {
            Error::Storage(std::io::Error::other(format!(
                "the staging volume is not writable: {error}"
            )))
        })?;

        if let Backend::Bucket { bucket, .. } = &self.backend {
            bucket.reachable().await?;
        }

        Ok(())
    }

    pub fn scans(&self) -> u64 {
        self.staging().scans()
    }

    #[tracing::instrument(skip_all, fields(namespace = %ns, oid = %oid))]
    pub async fn exists(&self, ns: &Namespace, oid: &Oid) -> bool {
        match &self.backend {
            Backend::Local(store) => store.exists(ns, oid).await,
            Backend::Bucket { bucket, .. } => bucket.exists(ns, oid).await,
        }
    }

    // Where the client should fetch this object from, when that is somewhere
    // other than this server. None for a local store, and for a bucket the
    // operator has not asked to redirect, which is the default, because the
    // streamed path is the one that counts the bytes and holds the ceiling.
    //
    // The caller is responsible for having established that this repository
    // holds the object. This hands out a signature, not a permission.
    pub fn redirect(&self, oid: &Oid) -> Option<String> {
        match &self.backend {
            Backend::Local(_) => None,
            // A pre-signed URL hands over whatever sits under that key, and with
            // a codec in the path that is a frame rather than the object. The
            // client would hash what arrived, get a digest that is not the one it
            // asked for, and reject it. So the redirect is given up and the
            // download streams, which is the only path that can decode.
            //
            // Compression is enough on its own, even though it still lets a
            // client upload straight to the bucket. That asymmetry is the right
            // way round: an unframed object is a perfectly good entry, so a
            // direct upload stays safe, while one framed object anywhere in the
            // store makes every redirect a guess.
            Backend::Bucket { staging, .. } if staging.frames() => None,
            Backend::Bucket { bucket, .. } => bucket.presigned_download(oid),
        }
    }

    // Where the client should PUT the object, when that is the bucket rather than
    // this server. None for a local store and for a bucket the operator has not
    // asked to redirect.
    pub fn presigned_upload(
        &self,
        ns: &Namespace,
        oid: &Oid,
        size: u64,
    ) -> Option<super::s3::Presigned> {
        match &self.backend {
            Backend::Local(_) => None,
            // A client uploading straight to the bucket writes the object as it
            // is, so a configured key would never touch it and the bucket would
            // hold plaintext while an operator believed otherwise. Encryption is
            // a promise about what the storage provider can read; a faster upload
            // is not worth quietly breaking it. Those transfers keep coming
            // through the server, which seals them.
            Backend::Bucket { staging, .. } if staging.encrypts() => None,
            Backend::Bucket { bucket, .. } => bucket.presigned_upload(ns, oid, size),
        }
    }

    // How big an object waiting under this repository's own upload key is. None
    // when there is nothing waiting, which is every local deployment and every
    // client that has not used its URL.
    #[tracing::instrument(skip_all, fields(namespace = %ns, oid = %oid))]
    pub async fn uploaded_size(&self, ns: &Namespace, oid: &Oid) -> Result<Option<u64>, Error> {
        match &self.backend {
            Backend::Local(_) => Ok(None),
            Backend::Bucket { bucket, .. } => Ok(bucket.uploaded_size(ns, oid).await.ok()),
        }
    }

    // Take an upload this repository made into the shared keyspace. Only reachable
    // for a bucket, because only there does a client write anywhere this server
    // did not.
    #[tracing::instrument(skip_all, fields(namespace = %ns, oid = %oid))]
    pub async fn adopt(&self, ns: &Namespace, oid: &Oid, arrived: u64) -> Result<(), Error> {
        let outcome = match &self.backend {
            Backend::Local(_) => Err(Error::Unsupported(
                "objects are written through this server, so there is nothing to adopt",
            )),
            Backend::Bucket { bucket, .. } => bucket.adopt(ns, oid, arrived).await,
        };

        // Same reason as a write: verify is called once per object, so dropping
        // what is remembered here would make every one of them re-measure.
        if outcome.is_ok() {
            self.usage.stored(ns, arrived).await;
        }

        outcome
    }

    #[tracing::instrument(skip_all, fields(namespace = %ns, oid = %oid))]
    pub async fn open(&self, ns: &Namespace, oid: &Oid) -> Result<Object, Error> {
        match &self.backend {
            Backend::Local(store) => store.open(ns, oid).await,
            Backend::Bucket { bucket, staging } => {
                // The marker is the proof of possession and is checked before
                // anything is read, exactly as a local open checks the link.
                if !bucket.exists(ns, oid).await {
                    return Err(Error::NotFound);
                }

                let size = bucket.size_of(oid).await?;
                let reader = super::codec::Reader::Bucket {
                    bucket: (**bucket).clone(),
                    oid: oid.to_owned(),
                };

                match super::codec::Framed::open(
                    reader,
                    size,
                    staging.keyring().map(AsRef::as_ref),
                    oid,
                )
                .await?
                {
                    Some(framed) => Ok(Object::Framed(framed)),
                    // Not one of ours: the object is the bytes, and streaming
                    // them straight through costs no extra round trip.
                    None => Ok(Object::Remote {
                        bucket: (**bucket).clone(),
                        oid: oid.to_owned(),
                        size,
                    }),
                }
            }
        }
    }

    #[tracing::instrument(skip_all, fields(namespace = %ns, oid = %oid))]
    pub async fn write<S, E>(
        &self,
        ns: &Namespace,
        oid: &Oid,
        expected_size: Option<u64>,
        budget: Option<Budget>,
        chunks: S,
    ) -> Result<u64, Error>
    where
        S: Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let written = match &self.backend {
            Backend::Local(store) => store.write(ns, oid, expected_size, budget, chunks).await?,
            Backend::Bucket { bucket, staging } => {
                // Asked of the bucket, because the staging store answers about a
                // local layout a bucket deployment never fills in: it would call
                // every upload fresh, and re-pushing an object the repository
                // already holds would grow what is remembered without anything
                // being stored.
                let fresh = !bucket.exists(ns, oid).await;

                let staged = staging
                    .stage(ns, oid, expected_size, budget, chunks)
                    .await?;
                let outcome = bucket.store(ns, oid, &staged.path).await;

                // The staging file has served its purpose either way. Leaving it
                // would be a leak the reclaimer only notices a day later.
                let _ = tokio::fs::remove_file(&staged.path).await;
                outcome?;

                super::Written {
                    bytes: staged.written,
                    fresh,
                }
            }
        };

        // Added to what is remembered rather than dropping it: a client pushing
        // a hundred objects would otherwise make the next negotiation measure
        // the repository again, which on a bucket is what this cache exists to
        // avoid.
        if written.fresh {
            self.usage.stored(ns, written.bytes).await;
        }

        Ok(written.bytes)
    }

    // None rather than zero: a bucket has no cheap answer for what the whole
    // store holds, and building one from a full listing would cost a request per
    // object on every scrape. Zero would be read as an empty bucket by every
    // dashboard that averages it, which is the one lie this seam otherwise
    // refuses to tell: everything else it cannot do answers 501.
    pub async fn capacity(&self) -> Option<(u64, u64)> {
        match &self.backend {
            Backend::Local(store) => Some(store.usage().await),
            Backend::Bucket { .. } => None,
        }
    }

    // Measured at most once a minute per repository, whichever backend is
    // behind it. A bucket answers this by listing the repository's markers and
    // asking the size of each, so one uncached call per object in a batch made
    // a hundred-object push cost a hundred listings: the product, not the sum.
    pub async fn usage_of(&self, ns: &Namespace) -> (u64, u64) {
        if let Some(cached) = self.usage.cached(ns).await {
            return cached;
        }

        let measured = match &self.backend {
            Backend::Local(store) => store.measure_of(ns).await,
            Backend::Bucket { bucket, .. } => bucket.usage_of(ns).await,
        };

        self.usage.remember(ns, measured.0, measured.1).await;

        measured
    }

    #[tracing::instrument(skip_all, fields(namespace = %ns, dry_run))]
    pub async fn sweep(
        &self,
        ns: &Namespace,
        retained: &std::collections::HashSet<String>,
        grace: std::time::Duration,
        dry_run: bool,
    ) -> Result<SweepReport, Error> {
        match &self.backend {
            Backend::Local(store) => {
                let report = store.sweep(ns, retained, grace, dry_run).await;

                // Freeing gigabytes and then answering the next quota check from
                // the figure measured before is how a client is refused space it
                // has just been told it reclaimed.
                self.usage.forget(ns).await;

                report
            }
            Backend::Bucket { bucket, .. } => {
                let report = bucket.sweep(ns, retained, grace, dry_run).await;

                if report.is_ok() && !dry_run {
                    self.usage.forget(ns).await;
                }

                report
            }
        }
    }

    #[tracing::instrument(skip_all, fields(namespace = %ns, dry_run))]
    pub async fn dedupe(&self, ns: &Namespace, dry_run: bool) -> Result<DedupeReport, Error> {
        match &self.backend {
            Backend::Local(store) => {
                let report = store.dedupe(ns, dry_run).await;

                // Freeing gigabytes and then answering the next quota check from
                // the figure measured before is how a client is refused space it
                // has just been told it reclaimed.
                self.usage.forget(ns).await;

                report
            }
            // Content addressing already gives this: two repositories pushing the
            // same object write the same key, and each holds a marker beside it.
            // There is nothing left to fold in.
            Backend::Bucket { .. } => Err(Error::Unsupported(
                "a bucket stores each object once already, so there is nothing to deduplicate",
            )),
        }
    }

    #[tracing::instrument(skip_all, fields(namespace = %ns, dry_run))]
    pub async fn compress(&self, ns: &Namespace, dry_run: bool) -> Result<CompressReport, Error> {
        match &self.backend {
            Backend::Local(store) => {
                let report = store.compress(ns, dry_run).await;

                // Freeing gigabytes and then answering the next quota check from
                // the figure measured before is how a client is refused space it
                // has just been told it reclaimed.
                self.usage.forget(ns).await;

                report
            }
            // Objects arriving now are compressed if the server is configured to;
            // rewriting the ones already in the bucket means walking it and
            // reuploading, which is a different piece of work.
            Backend::Bucket { .. } => Err(Error::Unsupported(
                "rewriting objects already in a bucket is not implemented",
            )),
        }
    }

    #[tracing::instrument(skip_all, fields(namespace = %ns))]
    pub async fn verify(&self, ns: &Namespace) -> Result<VerifyReport, Error> {
        match &self.backend {
            Backend::Local(store) => store.verify(ns).await,
            Backend::Bucket { .. } => Err(Error::Unsupported(
                "verification is not implemented for a bucket yet",
            )),
        }
    }
}

#[cfg(test)]
mod tests;
