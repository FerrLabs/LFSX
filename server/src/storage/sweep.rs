use std::collections::HashSet;
use std::path::Path;
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
                self.sweep_directory(&fanout.path(), retained, grace, &mut report)
                    .await?;
            }

            if !dry_run {
                let _ = fs::remove_dir(prefix.path()).await;
            }
        }

        Ok(report)
    }

    async fn sweep_directory(
        &self,
        directory: &Path,
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
            report.bytes += metadata.len();

            if !report.dry_run {
                fs::remove_file(entry.path()).await?;
            }
        }

        if !report.dry_run {
            let _ = fs::remove_dir(directory).await;
        }

        Ok(())
    }
}

fn age(metadata: &std::fs::Metadata) -> Duration {
    metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .unwrap_or_default()
}

const USAGE_TTL: Duration = Duration::from_secs(60);

impl LocalStore {
    pub async fn usage(&self) -> (u64, u64) {
        if let Some((measured_at, objects, bytes)) = *self.usage.lock().expect("usage cache")
            && measured_at.elapsed() < USAGE_TTL
        {
            return (objects, bytes);
        }

        let measured = self.measure().await;
        *self.usage.lock().expect("usage cache") = Some((Instant::now(), measured.0, measured.1));

        measured
    }

    async fn measure(&self) -> (u64, u64) {
        let mut objects = 0;
        let mut bytes = 0;

        let mut directories = vec![self.root.clone()];
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
                    Ok(metadata) if LocalStore::validate_oid(&name).is_ok() => {
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
