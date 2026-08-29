use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::fs;

use super::LocalStore;
use super::sweep::age;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reclaimed {
    pub files: u64,
    pub bytes: u64,
}

impl LocalStore {
    // An upload streams into `<oid>.<n>.part` and renames on success. Every error
    // path the handler controls removes it, but a kill or a host crash mid-transfer
    // leaves one behind for good, inside the object fanout where it is easy to
    // mistake for an object.
    //
    // `older_than` has to exceed the longest transfer the server could plausibly
    // still be serving: a .part file younger than that is not litter, it is an
    // upload in flight, and removing it would break a client doing nothing wrong.
    pub async fn reclaim_staging(&self, older_than: Duration) -> Reclaimed {
        let mut reclaimed = Reclaimed::default();
        let mut directories = vec![self.root.clone()];

        while let Some(directory) = directories.pop() {
            let Ok(mut entries) = fs::read_dir(&directory).await else {
                continue;
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let Ok(metadata) = entry.metadata().await else {
                    continue;
                };

                if metadata.is_dir() {
                    // Staging files only ever appear under org/repo/xx/yy, so the
                    // shared .content store and .locks hold none, walking them
                    // every hour would cost I/O that grows with the whole store
                    // instead of with the litter. Only the root carries those:
                    // deeper down, a repository really can be named .github.
                    if directory != self.root || !is_dotted(&path) {
                        directories.push(path);
                    }
                    continue;
                }

                let is_staging = path
                    .extension()
                    .is_some_and(|extension| extension == "part");
                if !is_staging || age(&metadata) < older_than {
                    continue;
                }

                if fs::remove_file(&path).await.is_ok() {
                    reclaimed.files += 1;
                    reclaimed.bytes += metadata.len();
                }
            }
        }

        reclaimed
    }
}

fn is_dotted(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
}

pub async fn reclaim(root: PathBuf, older_than: Duration) {
    let reclaimed = LocalStore::new(root).reclaim_staging(older_than).await;

    if reclaimed.files > 0 {
        tracing::info!(
            files = reclaimed.files,
            bytes = reclaimed.bytes,
            "reclaimed staging files left by interrupted uploads"
        );
    }
}

#[cfg(test)]
mod tests;
