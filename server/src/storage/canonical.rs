use std::io;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;

use super::s3::Keyspace;
use crate::error::Error;

const CONCURRENCY: usize = 16;
const DONE: &str = ".lfsx/namespace-case";

pub(crate) fn canonical_key(key: &str) -> Option<String> {
    let parts: Vec<&str> = key.split('/').collect();

    let (head, org, repo, tail): (&[&str], &str, &str, &[&str]) = match parts.as_slice() {
        [".locks", org, repo, tail @ ..] if !tail.is_empty() => (&[".locks"], org, repo, tail),
        [".refs", oid, org, repo] if *oid != ".complete" => (&[".refs", oid], org, repo, &[]),
        [first, ..] if first.starts_with('.') => return None,
        [org, repo, tail @ ..] if !tail.is_empty() => (&[], org, repo, tail),
        _ => return None,
    };

    let canonical: Vec<String> = head
        .iter()
        .map(|part| (*part).to_owned())
        .chain([org.to_ascii_lowercase(), repo.to_ascii_lowercase()])
        .chain(tail.iter().map(|part| (*part).to_owned()))
        .collect();
    let canonical = canonical.join("/");

    (canonical != key).then_some(canonical)
}

pub(crate) async fn fold_bucket(keys: &Keyspace) -> Result<u64, Error> {
    if keys.get_bytes(DONE).await?.is_some() {
        return Ok(0);
    }

    let moves: Vec<(String, String)> = keys
        .keys("")
        .await?
        .into_iter()
        .filter_map(|key| canonical_key(&key).map(|canonical| (key, canonical)))
        .collect();
    let count = moves.len() as u64;

    let mut running = futures_util::stream::iter(moves.into_iter().map(|(from, to)| {
        let keys = keys.clone();
        async move {
            let body = keys.get_bytes(&from).await?.unwrap_or_default();
            keys.put_if_absent(&to, body).await?;
            keys.delete(&from).await.map(|_| ())
        }
    }))
    .buffer_unordered(CONCURRENCY);

    while let Some(moved) = running.next().await {
        moved?;
    }

    keys.put(DONE, Vec::new()).await?;

    Ok(count)
}

pub(crate) async fn fold_local(root: &Path) -> Result<u64, Error> {
    let root = root.to_owned();

    tokio::task::spawn_blocking(move || {
        let mut folded = 0;
        for base in [root.clone(), root.join(".locks")] {
            folded += fold_level(&base)?;
            for org in subdirectories(&base)? {
                folded += fold_level(&org)?;
            }
        }
        Ok(folded)
    })
    .await
    .map_err(|error| Error::Storage(io::Error::other(error)))?
}

fn subdirectories(base: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries = match std::fs::read_dir(base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') && entry.file_type()?.is_dir() {
            found.push(entry.path());
        }
    }
    Ok(found)
}

const PARKED: &str = ".fold-";

fn fold_level(base: &Path) -> Result<u64, Error> {
    let mut folded = 0;

    for source in subdirectories(base)? {
        let name = source.file_name().unwrap_or_default().to_string_lossy();
        let lower = name.to_ascii_lowercase();
        if lower == name {
            continue;
        }

        let parked = base.join(format!("{PARKED}{lower}"));
        std::fs::rename(&source, &parked)?;
        settle(&parked, &base.join(&lower))?;
        folded += 1;
    }

    for parked in interrupted(base)? {
        let name = parked.file_name().unwrap_or_default().to_string_lossy();
        let target = base.join(name.strip_prefix(PARKED).unwrap_or(&name));
        settle(&parked, &target)?;
        folded += 1;
    }

    Ok(folded)
}

fn interrupted(base: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries = match std::fs::read_dir(base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(PARKED) {
            found.push(entry.path());
        }
    }
    Ok(found)
}

fn settle(parked: &Path, target: &Path) -> io::Result<()> {
    if target.exists() {
        merge(parked, target)
    } else {
        std::fs::rename(parked, target)
    }
}

fn merge(from: &Path, into: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let destination = into.join(entry.file_name());

        if !destination.exists() {
            std::fs::rename(&source, &destination)?;
        } else if entry.file_type()?.is_dir() {
            merge(&source, &destination)?;
        } else {
            std::fs::remove_file(&source)?;
        }
    }

    std::fs::remove_dir(from)
}

#[cfg(test)]
mod tests;
