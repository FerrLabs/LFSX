use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::{S3Store, refs, sizes};
use crate::error::Error;
use crate::namespace::Namespace;
use crate::storage::SweepReport;
use crate::storage::s3::keyspace;

// Collection, and only collection. It left `s3.rs` because it had become the
// longest thing in it and the least like the rest: everything else there is one
// request about one object, and this is a policy about the whole keyspace, with
// two paths through it and three indexes to keep straight.

impl S3Store {
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
        retained: &HashSet<String>,
        grace: Duration,
        dry_run: bool,
    ) -> Result<SweepReport, Error> {
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
        retained: &HashSet<String>,
        grace: Duration,
        report: &mut SweepReport,
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
        retained: &HashSet<String>,
        grace: Duration,
        dry_run: bool,
    ) -> Result<SweepReport, Error> {
        let listing = self.keys.listing(&Self::own_prefix(ns)).await;
        let mut report = SweepReport {
            dry_run,
            incomplete: !listing.complete,
            ..Default::default()
        };

        // The size index shares this prefix, so it arrives in the same listing.
        // Kept rather than discarded, because dropping a marker should take its
        // entry with it and the key carries a number this sweep has no other way
        // of knowing.
        let mut sized = HashMap::new();
        let mut mine = Vec::new();

        for entry in listing.entries {
            if sizes::is_one(&entry.key) {
                if let Some((oid, _)) = sizes::read(&entry.key) {
                    sized.insert(oid, entry.key);
                }
                continue;
            }

            let Some(oid) = entry.key.rsplit('/').next().map(str::to_owned) else {
                continue;
            };
            if crate::storage::LocalStore::validate_oid(&oid).is_ok() {
                mine.push((entry, oid));
            }
        }

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

            // Tidiness rather than correctness: a size whose marker has gone is
            // counted by nobody, because only a marker says this repository holds
            // anything.
            if let Some(key) = sized.get(&oid)
                && let Err(error) = self.keys.delete(key).await
            {
                tracing::warn!(%error, oid, "a dropped marker left its size behind");
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
        retained: &HashSet<String>,
        grace: Duration,
        dry_run: bool,
    ) -> Result<SweepReport, Error> {
        let listing = self.keys.listing("").await;
        let mut report = SweepReport {
            dry_run,
            incomplete: !listing.complete,
            ..Default::default()
        };

        let ours = Self::own_prefix(ns);
        let mut markers = Vec::new();
        let mut sized = HashMap::new();
        let mut mine = Vec::new();
        let mut claimed_elsewhere = HashSet::new();
        let mut content_sizes = HashMap::new();

        for entry in listing.entries {
            if let Some(rest) = entry.key.strip_prefix(".content/") {
                if let Some(oid) = rest.rsplit('/').next() {
                    content_sizes.insert(oid.to_owned(), entry.size);
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

            // The size index shares a repository's prefix, so it arrives here
            // among the markers. Read as one, an entry of it is a claim on an
            // object whose name ends in a number. Kept rather than dropped,
            // because a marker this sweep removes should take its size along and
            // the key is the only place that number is written down.
            if sizes::is_one(&entry.key) {
                if entry.key.starts_with(&ours)
                    && let Some((oid, _)) = sizes::read(&entry.key)
                {
                    sized.insert(oid, entry.key);
                }

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
            let size = content_sizes.get(&oid).copied().unwrap_or_default();

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

            if let Some(key) = sized.get(&oid)
                && let Err(error) = self.keys.delete(key).await
            {
                tracing::warn!(%error, oid, "a dropped marker left its size behind");
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
}
