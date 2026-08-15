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
    pub dry_run: bool,
}

impl LocalStore {
    pub async fn sweep(
        &self,
        ns: &Namespace,
        retained: &HashSet<String>,
        grace: Duration,
        dry_run: bool,
    ) -> Result<SweepReport, Error> {
        let mut report = SweepReport {
            dry_run,
            ..SweepReport::default()
        };

        let Ok(mut prefixes) = fs::read_dir(self.root.join(ns.org()).join(ns.repo())).await else {
            return Ok(report);
        };

        while let Some(prefix) = prefixes.next_entry().await? {
            let Ok(mut fanouts) = fs::read_dir(prefix.path()).await else {
                continue;
            };

            while let Some(fanout) = fanouts.next_entry().await? {
                self.sweep_directory(&fanout.path(), ns, retained, grace, &mut report)
                    .await?;
            }

            if !dry_run {
                let _ = fs::remove_dir(prefix.path()).await;
            }
        }

        Ok(report)
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

    async fn sweep_directory(
        &self,
        directory: &Path,
        ns: &Namespace,
        retained: &HashSet<String>,
        grace: Duration,
        report: &mut SweepReport,
    ) -> Result<(), Error> {
        let Ok(mut entries) = fs::read_dir(directory).await else {
            return Ok(());
        };

        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if Self::validate_oid(&name).is_err() || retained.contains(&name) {
                continue;
            }

            let metadata = entry.metadata().await?;
            if age(&metadata) < grace {
                report.within_grace += 1;
                continue;
            }

            report.swept += 1;

            if report.dry_run {
                // A dry run must not count bytes another repository still holds,
                // or it promises space it cannot free.
                if !self.referenced_elsewhere(&name, ns).await {
                    report.bytes += metadata.len();
                }
                continue;
            }

            fs::remove_file(entry.path()).await?;

            // The bytes live once under .content, linked from each repository
            // that holds them. Dropping this repository's link frees nothing
            // until the last one goes, so only then is it counted as freed.
            if !self.referenced_elsewhere(&name, ns).await {
                let content = self.content_path(&name);
                let size = fs::metadata(&content)
                    .await
                    .map(|content_metadata| content_metadata.len())
                    .unwrap_or_else(|_| metadata.len());

                // Count the bytes only if this call is the one that removed
                // them. Two repositories dropping their last reference at the
                // same time would otherwise each claim the same space, and two
                // reports would add up to more than the disk ever held.
                if fs::remove_file(&content).await.is_ok() {
                    report.bytes += size;
                }
            }
        }

        if !report.dry_run {
            let _ = fs::remove_dir(directory).await;
        }

        Ok(())
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
