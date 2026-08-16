use std::path::PathBuf;

use tokio::fs;

use super::LocalStore;
use crate::namespace::Namespace;

pub(super) struct Found {
    pub path: PathBuf,
    pub oid: String,
}

// Everything a repository holds, and whether that is everything. The second half
// is not decoration: every command built on this walk reports by silence, and an
// operator reads the report rather than the log. A collection that could not list
// a prefix, or a migration that skipped a fanout, must not come back looking like
// it finished.
pub(super) struct Walk {
    pub objects: Vec<Found>,
    pub complete: bool,
}

impl LocalStore {
    pub(super) async fn objects_of(&self, ns: &Namespace) -> Walk {
        let mut walk = Walk {
            objects: Vec::new(),
            complete: true,
        };

        let root = self.root.join(ns.org()).join(ns.repo());
        let Some(prefixes) = self.entries(&root, &mut walk).await else {
            // A repository with nothing in it has no directory, and that is not
            // an incomplete walk — it is an empty one.
            return Walk {
                objects: Vec::new(),
                complete: true,
            };
        };

        for prefix in prefixes {
            let Some(fanouts) = self.entries(&prefix, &mut walk).await else {
                continue;
            };

            for fanout in fanouts {
                let Some(objects) = self.entries(&fanout, &mut walk).await else {
                    continue;
                };

                for path in objects {
                    let oid = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();

                    if Self::validate_oid(&oid).is_ok() {
                        walk.objects.push(Found { path, oid });
                    }
                }
            }
        }

        walk
    }

    // None means this directory could not be read at all, which is different from
    // an empty one and is recorded as such.
    async fn entries(&self, directory: &std::path::Path, walk: &mut Walk) -> Option<Vec<PathBuf>> {
        let mut entries = match fs::read_dir(directory).await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(?directory, %error, "could not be listed");
                walk.complete = false;
                return None;
            }
        };

        let mut found = Vec::new();
        loop {
            match entries.next_entry().await {
                Ok(Some(entry)) => found.push(entry.path()),
                Ok(None) => return Some(found),
                Err(error) => {
                    tracing::warn!(?directory, %error, "listing stopped early");
                    walk.complete = false;
                    return Some(found);
                }
            }
        }
    }
}
