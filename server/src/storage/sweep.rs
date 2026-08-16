use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use tokio::fs;

use super::LocalStore;
use crate::error::Error;
use crate::namespace::Namespace;

#[derive(Debug, Default, Serialize)]
pub struct SweepReport {
    pub swept: usize,
    pub bytes: u64,
    pub within_grace: usize,
    pub incomplete: bool,
    pub dry_run: bool,
}

impl LocalStore {
    // Objects go, the fanout directories they lived in stay. Removing an
    // emptied directory raced every upload: a push creates its fanout, and for
    // the moment between that and the staging file appearing the directory is
    // empty, so a collection running alongside took it and the push failed on a
    // directory that had just been made for it. Nothing here can hold a lock the
    // filesystem would honour, so the fix is to stop competing — the shared
    // .content tree has never pruned its directories either. What is left is an
    // inode and a block per prefix, reused by the next object that hashes into
    // it, against a push failing for a reason no operator could act on.
    pub async fn sweep(
        &self,
        ns: &Namespace,
        retained: &HashSet<String>,
        grace: Duration,
        dry_run: bool,
    ) -> Result<SweepReport, Error> {
        let walk = self.objects_of(ns).await;
        let mut report = SweepReport {
            dry_run,
            incomplete: !walk.complete,
            ..SweepReport::default()
        };

        for found in walk.objects {
            if retained.contains(&found.oid) {
                continue;
            }

            let metadata = fs::metadata(&found.path).await?;
            if age(&metadata) < grace {
                report.within_grace += 1;
                continue;
            }

            self.collect(&found.path, &found.oid, metadata.len(), ns, &mut report)
                .await?;
        }

        if !dry_run {
            self.forget(ns).await;
        }

        Ok(report)
    }

    // The bytes live once under .content, linked from each repository that holds
    // them. Dropping this repository's link frees nothing until the last one
    // goes, so only then is it counted as freed — and a dry run that counted
    // otherwise would promise space it cannot deliver.
    async fn collect(
        &self,
        path: &Path,
        oid: &str,
        held: u64,
        ns: &Namespace,
        report: &mut SweepReport,
    ) -> Result<(), Error> {
        report.swept += 1;

        if report.dry_run {
            if !self.referenced_elsewhere(oid, ns).await {
                report.bytes += held;
            }

            return Ok(());
        }

        fs::remove_file(path).await?;

        if self.referenced_elsewhere(oid, ns).await {
            return Ok(());
        }

        let content = self.content_path(oid);
        let size = fs::metadata(&content)
            .await
            .map(|shared| shared.len())
            .unwrap_or(held);

        // Count the bytes only if this call is the one that removed them. Two
        // repositories dropping their last reference at the same time would
        // otherwise each claim the same space, and two reports would add up to
        // more than the disk ever held.
        if fs::remove_file(&content).await.is_ok() {
            report.bytes += size;
        }

        Ok(())
    }

    // Is this object still linked from a repository other than the one being
    // swept? Reads the tree rather than the link count, because nlink is not
    // portable and the number of repositories is small.
    async fn referenced_elsewhere(&self, oid: &str, sweeping: &Namespace) -> bool {
        let Ok(mut orgs) = fs::read_dir(&self.root).await else {
            return false;
        };

        while let Ok(Some(org)) = orgs.next_entry().await {
            let org_name = org.file_name().to_string_lossy().into_owned();
            if org_name.starts_with('.') {
                continue;
            }

            let Ok(mut repos) = fs::read_dir(org.path()).await else {
                continue;
            };

            while let Ok(Some(repo)) = repos.next_entry().await {
                let repo_name = repo.file_name().to_string_lossy().into_owned();
                if org_name == sweeping.org() && repo_name == sweeping.repo() {
                    continue;
                }

                let candidate = repo.path().join(&oid[0..2]).join(&oid[2..4]).join(oid);
                if fs::metadata(candidate).await.is_ok() {
                    return true;
                }
            }
        }

        false
    }
}

pub(super) fn age(metadata: &std::fs::Metadata) -> Duration {
    metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .unwrap_or_default()
}

const USAGE_TTL: Duration = Duration::from_secs(60);

impl LocalStore {
    pub async fn usage(&self) -> (u64, u64) {
        let mut cached = self.usage.lock().await;

        if let Some((measured_at, objects, bytes)) = *cached
            && measured_at.elapsed() < USAGE_TTL
        {
            return (objects, bytes);
        }

        let measured = self.measure().await;
        *cached = Some((Instant::now(), measured.0, measured.1));

        measured
    }

    pub async fn usage_of(&self, ns: &Namespace) -> (u64, u64) {
        let key = ns.to_string();
        let mut cached = self.per_namespace.lock().await;

        if let Some((measured_at, objects, bytes)) = cached.get(&key)
            && measured_at.elapsed() < USAGE_TTL
        {
            return (*objects, *bytes);
        }

        let measured = self.walk(self.root.join(ns.org()).join(ns.repo())).await;
        cached.insert(key, (Instant::now(), measured.0, measured.1));

        measured
    }

    // A quota is checked on every negotiation, so the figure behind it can
    // afford neither a walk each time nor a minute of staleness: stale in one
    // direction lets a repository push past its budget, and in the other it
    // refuses space the client has just freed. A stored object adds to the
    // cached figure, and a collection drops it so the next reader measures what
    // is really left rather than trusting arithmetic across hard links.
    pub async fn stored(&self, ns: &Namespace, bytes: u64) {
        if let Some((_, objects, held)) = self.per_namespace.lock().await.get_mut(&ns.to_string()) {
            *objects += 1;
            *held += bytes;
        }
    }

    // Both figures, because both just became wrong. Freeing gigabytes and then
    // reporting the old total for another minute is how an operator concludes
    // the collection — or the migration — did nothing.
    pub async fn forget(&self, ns: &Namespace) {
        self.per_namespace.lock().await.remove(&ns.to_string());
        *self.usage.lock().await = None;
    }

    // What the disk actually holds, which is not the sum of what the
    // repositories logically hold: an object shared by three projects is three
    // links to one set of bytes. Counting per-repository paths would report the
    // pre-deduplication total and grow every time another project links the
    // same pack — the opposite of what the number is for.
    async fn measure(&self) -> (u64, u64) {
        #[cfg(unix)]
        {
            self.walk_unique(self.root.clone()).await
        }
        #[cfg(not(unix))]
        {
            let (shared_objects, shared_bytes) = self.walk(self.root.join(".content")).await;
            let (loose_objects, loose_bytes) = self.walk_unshared(self.root.clone()).await;

            (shared_objects + loose_objects, shared_bytes + loose_bytes)
        }
    }

    // Every hard link to one object reports the same inode, so counting each
    // inode once measures bytes on disk exactly — including the copy fallback,
    // which really does duplicate them and really should be counted twice.
    #[cfg(unix)]
    async fn walk_unique(&self, from: PathBuf) -> (u64, u64) {
        use std::collections::HashSet;
        use std::os::unix::fs::MetadataExt;

        self.scan(from, |metadata, seen: &mut HashSet<(u64, u64)>| {
            seen.insert((metadata.dev(), metadata.ino()))
        })
        .await
    }

    // Without inode numbers, count the shared store plus anything a repository
    // holds that has no counterpart there. A copy made by the fallback path is
    // undercounted, which needs a filesystem with no hard links to happen at all.
    #[cfg(not(unix))]
    async fn walk_unshared(&self, from: PathBuf) -> (u64, u64) {
        let mut objects = 0;
        let mut bytes = 0;
        let mut directories = vec![from];

        while let Some(directory) = directories.pop() {
            let Ok(mut entries) = fs::read_dir(&directory).await else {
                continue;
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                // Only the root carries dot-directories worth skipping: .content
                // is counted through the links that point into it, and .locks
                // holds no objects at all.
                if name.starts_with('.') && directory == self.root {
                    continue;
                }

                match entry.metadata().await {
                    Ok(metadata) if metadata.is_dir() => directories.push(entry.path()),
                    Ok(metadata)
                        if LocalStore::validate_oid(&name).is_ok()
                            && fs::metadata(self.content_path(&name)).await.is_err() =>
                    {
                        objects += 1;
                        bytes += metadata.len();
                    }
                    _ => {}
                }
            }
        }

        (objects, bytes)
    }

    async fn walk(&self, from: PathBuf) -> (u64, u64) {
        self.scan(from, |_, _: &mut ()| true).await
    }

    async fn scan<S: Default>(
        &self,
        from: PathBuf,
        mut counts: impl FnMut(&std::fs::Metadata, &mut S) -> bool,
    ) -> (u64, u64) {
        self.scans.fetch_add(1, Ordering::Relaxed);

        let mut objects = 0;
        let mut bytes = 0;
        let mut state = S::default();

        let mut directories = vec![from];
        while let Some(directory) = directories.pop() {
            let Ok(mut entries) = fs::read_dir(&directory).await else {
                continue;
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }

                match entry.metadata().await {
                    Ok(metadata) if metadata.is_dir() => directories.push(entry.path()),
                    Ok(metadata)
                        if LocalStore::validate_oid(&name).is_ok()
                            && counts(&metadata, &mut state) =>
                    {
                        objects += 1;
                        bytes += metadata.len();
                    }
                    _ => {}
                }
            }
        }

        (objects, bytes)
    }
}
