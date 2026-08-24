pub(crate) mod keyspace;
pub(crate) mod multipart;
pub(crate) mod probe;
pub(crate) mod refs;

use std::time::Duration;

use axum::body::Bytes;
use futures_util::{Stream, StreamExt};

use base64::Engine;

use crate::error::Error;
use crate::namespace::Namespace;
use crate::storage::Reclaimed;

pub use keyspace::{Keyspace, Presigned};

const CHECKSUM: &str = "x-amz-checksum-sha256";

// Enough to hide the round trips a bucket charges for without becoming a burst
// the store answers with 503, and the same figure the batch endpoint settled on
// for the same reason.
const SIZES_AT_ONCE: usize = 16;

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

    fn own_prefix(ns: &Namespace) -> String {
        format!("{}/{}/", ns.org(), ns.repo())
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
    //
    // None above the single-request ceiling, and that is not a refusal: the
    // object falls back to coming through this server, which sends it in parts.
    // A client cannot do the same, because the `basic` transfer adapter every
    // git-lfs speaks does one PUT to one href and has nowhere to put a second.
    // So the ceiling multipart removes for the streamed path is real and
    // permanent for this one, and the only question is whether the client learns
    // it now or after uploading five gigabytes.
    //
    // It also keeps `adopt` honest: `CopyObject` stops at the same 5 GiB, and
    // nothing can reach `.incoming/` above it while this holds.
    pub fn presigned_upload(&self, ns: &Namespace, oid: &str, size: u64) -> Option<Presigned> {
        if !self.redirect
            || size > multipart::SINGLE_PUT_CEILING
            || crate::storage::LocalStore::validate_oid(oid).is_err()
        {
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

        // First, before anything here so much as looks at the content.
        //
        // The marker is the claim and the ref is the index of it, so a crash
        // between the two has to leave a ref nobody claims rather than a claim
        // nothing indexes: the first leaks an object, the second lets a later
        // sweep free bytes this repository holds.
        //
        // Writing it up here rather than beside the marker costs nothing and buys
        // the race below. A sweep asks the index one last time before deleting
        // bytes, so a claim recorded before this repository even checked whether
        // the content exists is a claim that sweep will see.
        refs::write(&self.keys, ns, oid).await?;

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

        // Before the content is even looked at, for the reason `adopt` gives:
        // this is what a sweep re-reads before deleting bytes, so a claim
        // recorded here cannot be missed by one that is already deciding.
        refs::write(&self.keys, ns, oid).await?;

        if self.keys.head(&Self::content_key(oid)).await.is_err() {
            let file = tokio::fs::File::open(staged).await?;
            let length = file.metadata().await?.len();

            // One request while one request will carry it, which is every
            // object a store normally sees, and parts when it will not. The
            // split is here rather than always going in parts because the
            // single write is one round trip and needs no cleanup if it fails.
            if length > multipart::SINGLE_PUT_CEILING {
                drop(file);
                multipart::put(&self.keys, &Self::content_key(oid), staged, length).await?;
            } else {
                let stream = tokio_util::io::ReaderStream::new(file);

                self.keys
                    .put(
                        &Self::content_key(oid),
                        reqwest::Body::wrap_stream(stream),
                        length,
                    )
                    .await?;
            }
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

        // `.probe/` too. A startup probe draws a key nothing else uses so that no
        // run can read another's leftovers, which means a run that dies before
        // cleaning up leaves one behind rather than overwriting it. They are
        // empty or nearly so, and this is already the sweep for writes nobody
        // will ever come back for.
        for prefix in [".incoming/", ".probe/"] {
            for entry in self.keys.entries(prefix).await? {
                // A slow client on a bad connection is not an abandoned one.
                if entry.age().is_none_or(|age| age < older_than) {
                    continue;
                }

                if self.keys.delete(&entry.key).await.is_ok() {
                    reclaimed.files += 1;
                    reclaimed.bytes += entry.size;
                }
            }
        }

        Ok(reclaimed)
    }

    // Collection, with the marker keyspace standing in for the link count a
    // filesystem keeps. A repository's marker is its claim on the bytes, and the
    // bytes go when the last claim does.
    //
    // Everything hard here is one question: does any *other* repository still
    // claim this object? A marker is `{org}/{repo}/.../{oid}`, so the oid is the
    // suffix and the org and repo that would make a prefix are exactly what is
    // unknown. The claim index turns that into one prefix listing per object. A
    // bucket that predates the index has to be read whole instead, and that pass
    // builds the index as it goes, so it is paid once rather than every sweep.
    pub async fn sweep(
        &self,
        ns: &Namespace,
        retained: &std::collections::HashSet<String>,
        grace: Duration,
        dry_run: bool,
    ) -> Result<crate::storage::SweepReport, Error> {
        if refs::ready(&self.keys).await {
            self.sweep_indexed(ns, retained, grace, dry_run).await
        } else {
            self.sweep_whole_bucket(ns, retained, grace, dry_run).await
        }
    }

    // The last question asked before bytes go, and the reason the index is read
    // twice for one object.
    //
    // Between deciding an object is unclaimed and deleting it, another repository
    // can push the same digest. It finds the content already there, skips the
    // upload, and writes a claim, so deleting now leaves it holding a marker
    // pointing at nothing, which its client meets as a missing object on the next
    // pull.
    //
    // A push writes its ref before it so much as looks at the content, so a claim
    // that landed at any moment before this question is one this sees. What is
    // left is the width of a single request, between reading this answer and the
    // delete that follows it. Closing that needs a lease the deleting side takes
    // and every push waits on, which is a round trip on the hot path bought
    // against a window this narrow, and it is not obviously the right trade.
    async fn claimed_since(&self, ns: &Namespace, oid: &str) -> bool {
        if refs::claimed_by_another(&self.keys, ns, oid).await {
            tracing::info!(
                oid,
                "another repository claimed this object while it was being collected, so its bytes \
                 stay"
            );

            return true;
        }

        false
    }

    // The markers this repository is allowed to drop. Retained is what the client
    // says it still needs; the grace window is what keeps a push still in flight
    // from being read as an abandoned object.
    fn droppable(
        mine: Vec<(keyspace::Entry, String)>,
        retained: &std::collections::HashSet<String>,
        grace: Duration,
        report: &mut crate::storage::SweepReport,
    ) -> Vec<(keyspace::Entry, String)> {
        mine.into_iter()
            .filter(|(entry, oid)| {
                if retained.contains(oid) {
                    return false;
                }

                if entry.age().is_none_or(|age| age < grace) {
                    report.within_grace += 1;
                    return false;
                }

                report.swept += 1;
                true
            })
            .collect()
    }

    // The cost this exists to avoid: one listing of this repository's own prefix,
    // then one listing of a short index prefix per object actually being dropped.
    // Nothing here is proportional to the size of the bucket.
    async fn sweep_indexed(
        &self,
        ns: &Namespace,
        retained: &std::collections::HashSet<String>,
        grace: Duration,
        dry_run: bool,
    ) -> Result<crate::storage::SweepReport, Error> {
        let listing = self.keys.listing(&Self::own_prefix(ns)).await;
        let mut report = crate::storage::SweepReport {
            dry_run,
            incomplete: !listing.complete,
            ..Default::default()
        };

        let mine = listing
            .entries
            .into_iter()
            .filter_map(|entry| {
                let oid = entry.key.rsplit('/').next()?.to_owned();
                crate::storage::LocalStore::validate_oid(&oid).ok()?;
                Some((entry, oid))
            })
            .collect();

        for (entry, oid) in Self::droppable(mine, retained, grace, &mut report) {
            let frees = !refs::claimed_by_another(&self.keys, ns, &oid).await;

            if dry_run {
                if frees {
                    report.bytes += self.size_of(&oid).await.unwrap_or_default();
                }
                continue;
            }

            self.keys.delete(&entry.key).await?;

            // After the marker, never before. A failure between the two has to
            // leave a ref with no claim behind it, which costs an object nobody
            // reads, rather than a claim with no ref, which would let the next
            // sweep free bytes this repository still holds.
            if let Err(error) = self.keys.delete(&refs::key(ns, &oid)).await {
                tracing::warn!(%error, oid, "a dropped marker left its index entry behind");
            }

            if frees && !self.claimed_since(ns, &oid).await {
                // Asked before the delete, because afterwards there is nothing
                // left to ask.
                let size = self.size_of(&oid).await.unwrap_or_default();

                if self.keys.delete(&Self::content_key(&oid)).await? {
                    report.bytes += size;
                }
            }
        }

        Ok(report)
    }

    // What a bucket with no index costs, and what builds one.
    //
    // One listing of the whole bucket answers all three questions at once: which
    // markers this repository holds, which oids any other repository still
    // claims, and how big each content object is. Asked separately they would
    // cost a request per object, which on a bucket is the difference between a
    // collection an operator runs and one they read about.
    //
    // A listing that did not finish is the dangerous case. It cannot be used to
    // conclude that nothing references an object, because the reference may sit
    // in the pages that never arrived. So an incomplete listing still drops this
    // repository's markers, which the retained set alone decides, and leaves
    // every content key exactly where it is.
    async fn sweep_whole_bucket(
        &self,
        ns: &Namespace,
        retained: &std::collections::HashSet<String>,
        grace: Duration,
        dry_run: bool,
    ) -> Result<crate::storage::SweepReport, Error> {
        let listing = self.keys.listing("").await;
        let mut report = crate::storage::SweepReport {
            dry_run,
            incomplete: !listing.complete,
            ..Default::default()
        };

        let ours = Self::own_prefix(ns);
        let mut markers = Vec::new();
        let mut mine = Vec::new();
        let mut claimed_elsewhere = std::collections::HashSet::new();
        let mut sizes = std::collections::HashMap::new();

        for entry in listing.entries {
            if let Some(rest) = entry.key.strip_prefix(".content/") {
                if let Some(oid) = rest.rsplit('/').next() {
                    sizes.insert(oid.to_owned(), entry.size);
                }
                continue;
            }

            // Locks live at `.locks/{org}/{repo}/{id}`, so they never match the
            // marker prefix and are never swept. Skipped explicitly all the same:
            // falling through would file every lock id in the claimed set, and an
            // object whose digest happened to equal a lock id would then never be
            // collected. The odds are absurd today and the line costs nothing,
            // but the code should not depend on ids and digests never colliding.
            //
            // The index is skipped for a sharper reason than caution:
            // `.refs/{oid}/{org}/{repo}` ends in a repository name, so reading one
            // as a marker would file that name as an oid somebody claims.
            if entry.key.starts_with(".incoming/")
                || entry.key.starts_with(".locks/")
                || entry.key.starts_with(".refs/")
                || entry.key.starts_with(".probe/")
            {
                continue;
            }

            let Some(oid) = entry.key.rsplit('/').next().map(str::to_owned) else {
                continue;
            };

            markers.push(entry.key.clone());

            if entry.key.starts_with(&ours) {
                mine.push((entry, oid));
            } else {
                claimed_elsewhere.insert(oid);
            }
        }

        // Before anything is deleted, so the index never gains a ref for a marker
        // this sweep is about to drop. Built from the listing already paid for,
        // and only when that listing finished: an index built from half a bucket
        // would be missing holders, which is the one direction it must never
        // drift in.
        //
        // A failure is not fatal. The listing above has already answered the
        // question correctly on its own, so collection proceeds and the next
        // sweep reads the bucket again.
        if !dry_run
            && listing.complete
            && let Err(error) = refs::backfill(&self.keys, &markers).await
        {
            tracing::warn!(
                %error,
                "the claim index could not be built, so the next sweep reads the bucket again"
            );
        }

        for (entry, oid) in Self::droppable(mine, retained, grace, &mut report) {
            // Only what this call actually frees is counted. Another repository
            // holding the same bytes means dropping this marker frees nothing,
            // and a dry run that said otherwise would promise space it cannot
            // deliver.
            let frees = listing.complete && !claimed_elsewhere.contains(&oid);
            let size = sizes.get(&oid).copied().unwrap_or_default();

            if dry_run {
                if frees {
                    report.bytes += size;
                }
                continue;
            }

            self.keys.delete(&entry.key).await?;

            if let Err(error) = self.keys.delete(&refs::key(ns, &oid)).await {
                tracing::warn!(%error, oid, "a dropped marker left its index entry behind");
            }

            // Counted only when this call is the one that removed them, so two
            // repositories letting go at once cannot each claim the same space.
            // The listing that decided `frees` was taken before any of these
            // deletes, so it is the stalest answer there is and the index gets
            // the last word.
            if frees
                && !self.claimed_since(ns, &oid).await
                && self.keys.delete(&Self::content_key(&oid)).await?
            {
                report.bytes += size;
            }
        }

        Ok(report)
    }

    // What the bucket holds for this repository, counted from its markers and
    // the content they point at. The markers are empty, so their own size says
    // nothing — this is a listing plus one head per object, which is why the
    // figure is cached the same way the local one is.
    pub async fn usage_of(&self, ns: &Namespace) -> (u64, u64) {
        let oids = self.list(&Self::own_prefix(ns)).await;
        let objects = oids.len() as u64;

        // Asked a few at a time rather than one after another. The number of
        // requests is the same, and it is the cost this cannot avoid without
        // changing the layout, but in series a repository holding fifty thousand
        // objects is fifty thousand round trips end to end: minutes of a client
        // waiting on a quota check that the cache was meant to make invisible.
        //
        // What would remove the requests rather than overlap them is still open
        // in #174, because both answers there cost something else.
        let bytes = futures_util::stream::iter(oids)
            .map(|oid| {
                let store = &self;
                async move { store.size_of(&oid).await.unwrap_or_default() }
            })
            .buffer_unordered(SIZES_AT_ONCE)
            .fold(0, |held, size| async move { held + size })
            .await;

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

#[cfg(test)]
pub(crate) mod tests;
