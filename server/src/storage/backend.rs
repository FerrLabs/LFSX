use futures_util::Stream;

use super::s3::S3Store;
use super::{Budget, CompressReport, DedupeReport, LocalStore, Object, SweepReport, VerifyReport};
use crate::error::Error;
use crate::namespace::Namespace;
#[cfg(test)]
use sha2::Digest;
#[cfg(test)]
use std::time::Duration;

// Where the objects live. A bucket decouples capacity from the machine, at the
// price of the things a filesystem gave for nothing — hard links, a directory
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
    // proves its scratch disk works — then fails every transfer it is handed.
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

    pub async fn exists(&self, ns: &Namespace, oid: &str) -> bool {
        match &self.backend {
            Backend::Local(store) => store.exists(ns, oid).await,
            Backend::Bucket { bucket, .. } => bucket.exists(ns, oid).await,
        }
    }

    // Where the client should fetch this object from, when that is somewhere
    // other than this server. None for a local store, and for a bucket the
    // operator has not asked to redirect — which is the default, because the
    // streamed path is the one that counts the bytes and holds the ceiling.
    //
    // The caller is responsible for having established that this repository
    // holds the object. This hands out a signature, not a permission.
    pub fn redirect(&self, oid: &str) -> Option<String> {
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
        oid: &str,
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
    pub async fn uploaded_size(&self, ns: &Namespace, oid: &str) -> Result<Option<u64>, Error> {
        match &self.backend {
            Backend::Local(_) => Ok(None),
            Backend::Bucket { bucket, .. } => Ok(bucket.uploaded_size(ns, oid).await.ok()),
        }
    }

    // Take an upload this repository made into the shared keyspace. Only reachable
    // for a bucket, because only there does a client write anywhere this server
    // did not.
    pub async fn adopt(&self, ns: &Namespace, oid: &str, arrived: u64) -> Result<(), Error> {
        let outcome = match &self.backend {
            Backend::Local(_) => Err(Error::Unsupported(
                "objects are written through this server, so there is nothing to adopt",
            )),
            Backend::Bucket { bucket, .. } => bucket.adopt(ns, oid).await,
        };

        // Same reason as a write: verify is called once per object, so dropping
        // what is remembered here would make every one of them re-measure.
        if outcome.is_ok() {
            self.usage.stored(ns, arrived).await;
        }

        outcome
    }

    pub async fn open(&self, ns: &Namespace, oid: &str) -> Result<Object, Error> {
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
    // refuses to tell — everything else it cannot do answers 501.
    pub async fn capacity(&self) -> Option<(u64, u64)> {
        match &self.backend {
            Backend::Local(store) => Some(store.usage().await),
            Backend::Bucket { .. } => None,
        }
    }

    // Measured at most once a minute per repository, whichever backend is
    // behind it. A bucket answers this by listing the repository's markers and
    // asking the size of each, so one uncached call per object in a batch made
    // a hundred-object push cost a hundred listings — the product, not the sum.
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
mod tests {
    use futures_util::StreamExt;

    use super::*;
    use crate::storage::crypt::Keyring;
    use crate::storage::s3::tests::{bucket, redirecting, store};

    fn keyring() -> std::sync::Arc<Keyring> {
        std::sync::Arc::new(
            Keyring::parse(&hex::encode([7u8; crate::storage::crypt::KEY])).unwrap(),
        )
    }

    fn namespace() -> Namespace {
        Namespace::new("FerrLabs", "Blastlands").unwrap()
    }

    fn bucket_store(root: &tempfile::TempDir, endpoint: &str) -> Store {
        Store::bucket(store(endpoint), LocalStore::new(root.path()))
    }

    async fn read_back(store: &Store, ns: &Namespace, oid: &str) -> Vec<u8> {
        let object = store.open(ns, oid).await.unwrap();
        let size = object.size();
        let mut chunks = object.stream(0, size).await.unwrap();
        let mut out = Vec::new();

        while let Some(chunk) = chunks.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }

        out
    }

    #[tokio::test]
    async fn an_upload_lands_in_the_bucket_and_reads_back_through_the_same_seam() {
        let root = tempfile::tempdir().unwrap();
        let (endpoint, _objects) = bucket().await;
        let store = bucket_store(&root, &endpoint);
        let payload = b"an asset that never touches this disk for long".repeat(32);
        let oid = hex::encode(sha2::Sha256::digest(&payload));

        let written = store
            .write(
                &namespace(),
                &oid,
                Some(payload.len() as u64),
                None,
                futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                    payload.clone(),
                ))]),
            )
            .await
            .unwrap();

        assert_eq!(written, payload.len() as u64);
        assert!(store.exists(&namespace(), &oid).await);

        assert_eq!(read_back(&store, &namespace(), &oid).await, payload);
    }

    // Compression and a bucket are configured independently, and a studio that
    // turns both on gets no warning from either. What lands under the digest
    // has to be the object, because the only thing that will ever read it back
    // is a client that asked for those bytes by that name.
    #[tokio::test]
    async fn a_bucket_holds_the_object_even_when_the_server_was_told_to_compress() {
        let root = tempfile::tempdir().unwrap();
        let (endpoint, _objects) = bucket().await;
        let store = Store::bucket(
            store(&endpoint),
            LocalStore::new(root.path()).with_compression(Some(3)),
        );
        let payload = b"a mesh that gives up most of its ground to zstd ".repeat(4096);
        let oid = hex::encode(sha2::Sha256::digest(&payload));

        store
            .write(
                &namespace(),
                &oid,
                Some(payload.len() as u64),
                None,
                futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                    payload.clone(),
                ))]),
            )
            .await
            .unwrap();

        let restored = read_back(&store, &namespace(), &oid).await;

        assert_eq!(
            hex::encode(sha2::Sha256::digest(&restored)),
            oid,
            "the client asked for the object named by this digest and has no way to know the              server framed it on the way past: {} bytes came back",
            restored.len()
        );
        assert_eq!(restored, payload);
    }

    #[tokio::test]
    async fn the_staging_file_does_not_outlive_the_upload() {
        let root = tempfile::tempdir().unwrap();
        let (endpoint, _objects) = bucket().await;
        let store = bucket_store(&root, &endpoint);
        let payload = b"an asset passing through".to_vec();
        let oid = hex::encode(sha2::Sha256::digest(&payload));

        store
            .write(
                &namespace(),
                &oid,
                None,
                None,
                futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                    payload,
                ))]),
            )
            .await
            .unwrap();

        let leftovers = crate::storage::tests::staging_files(root.path());
        assert!(
            leftovers.is_empty(),
            "local disk is a write buffer here, and one that is never emptied is a disk that \
             fills: {leftovers:?}"
        );
    }

    #[tokio::test]
    async fn a_bucket_reports_no_capacity_rather_than_an_empty_one() {
        let root = tempfile::tempdir().unwrap();
        let (endpoint, _objects) = bucket().await;

        assert!(
            bucket_store(&root, &endpoint).capacity().await.is_none(),
            "zero would be read as an empty store by every dashboard that averages it"
        );
        assert!(
            Store::local(LocalStore::new(root.path()))
                .capacity()
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn the_maintenance_commands_say_they_do_not_apply_rather_than_lying() {
        let root = tempfile::tempdir().unwrap();
        let (endpoint, _objects) = bucket().await;
        let store = bucket_store(&root, &endpoint);
        let ns = namespace();

        for outcome in [
            store.dedupe(&ns, true).await.err(),
            store.compress(&ns, true).await.err(),
            store.verify(&ns).await.err(),
        ] {
            assert!(
                matches!(outcome, Some(Error::Unsupported(_))),
                "an operator running one of these against a bucket has to be told it did nothing,                  not handed an empty report that reads like success: {outcome:?}"
            );
        }

        // Collection is the one that no longer belongs in that list.
        assert!(
            store
                .sweep(&ns, &std::collections::HashSet::new(), Duration::ZERO, true)
                .await
                .is_ok(),
            "collection is implemented for a bucket and must not answer Unsupported"
        );
    }

    // The bug this guards. A pre-signed download hands the client the bucket key
    // itself, which is only the object while nothing framed it on the way in.
    // `presigned_upload` has refused to sign an upload under a key since
    // encryption landed, for exactly this reason; the download side had no such
    // guard and handed out frames.
    //
    // Asserted against what is actually in the bucket rather than against the
    // flag, so this fails if framing ever stops happening and the guard becomes
    // theatre.
    #[tokio::test]
    async fn a_download_is_never_redirected_to_a_frame() {
        for label in ["compressed", "encrypted"] {
            let root = tempfile::tempdir().unwrap();
            let (endpoint, objects) = bucket().await;
            let staging = match label {
                "compressed" => LocalStore::new(root.path()).with_compression(Some(3)),
                _ => LocalStore::new(root.path()).with_encryption(Some(keyring())),
            };
            let store = Store::bucket(redirecting(&endpoint), staging);

            let payload =
                b"a scene file that compresses and must still come back whole ".repeat(512);
            let oid = hex::encode(sha2::Sha256::digest(&payload));

            store
                .write(
                    &namespace(),
                    &oid,
                    Some(payload.len() as u64),
                    None,
                    futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                        payload.clone(),
                    ))]),
                )
                .await
                .unwrap();

            let stored = objects
                .lock()
                .unwrap()
                .values()
                .find(|object| !object.is_empty())
                .cloned()
                .unwrap();

            assert_ne!(
                stored, payload,
                "{label}: the bucket holds a frame, which is the premise of the rest of this test"
            );
            assert_eq!(
                store.redirect(&oid),
                None,
                "{label}: a client sent to the bucket would hash {} bytes of frame and reject the \
                 object it asked for",
                stored.len()
            );
            assert_eq!(
                read_back(&store, &namespace(), &oid).await,
                payload,
                "{label}: giving up the redirect is only correct because the streamed path decodes"
            );
        }
    }

    // And the guard does not quietly disable the feature it protects. With no
    // codec configured the bucket holds the object itself, so the redirect is
    // exactly what the operator asked for.
    #[tokio::test]
    async fn a_bucket_holding_the_object_itself_still_redirects() {
        let root = tempfile::tempdir().unwrap();
        let (endpoint, _objects) = bucket().await;
        let store = Store::bucket(redirecting(&endpoint), LocalStore::new(root.path()));

        let payload = b"an object stored as it arrived".repeat(32);
        let oid = hex::encode(sha2::Sha256::digest(&payload));

        store
            .write(
                &namespace(),
                &oid,
                Some(payload.len() as u64),
                None,
                futures_util::stream::iter([Ok::<_, std::io::Error>(axum::body::Bytes::from(
                    payload.clone(),
                ))]),
            )
            .await
            .unwrap();

        assert!(store.redirect(&oid).is_some());
    }
}
