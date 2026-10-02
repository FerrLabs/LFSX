use super::{Backend, Store};
use crate::error::Error;

const DIRECTORY: &str = ".lfsx";

impl Store {
    pub async fn read_meta(&self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        match &self.backend {
            Backend::Local(store) => {
                match tokio::fs::read(store.root.join(DIRECTORY).join(name)).await {
                    Ok(bytes) => Ok(Some(bytes)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(error.into()),
                }
            }
            Backend::Bucket { bucket, .. } => bucket.read_meta(&key(name)).await,
        }
    }

    pub async fn write_meta(&self, name: &str, bytes: Vec<u8>) -> Result<(), Error> {
        match &self.backend {
            Backend::Local(store) => {
                let directory = store.root.join(DIRECTORY);
                tokio::fs::create_dir_all(&directory).await?;
                let staged = directory.join(format!("{name}.part"));
                tokio::fs::write(&staged, bytes).await?;
                tokio::fs::rename(&staged, directory.join(name)).await?;
                Ok(())
            }
            Backend::Bucket { bucket, .. } => bucket.write_meta(&key(name), bytes).await,
        }
    }

    pub async fn delete_meta(&self, name: &str) -> Result<(), Error> {
        match &self.backend {
            Backend::Local(store) => {
                match tokio::fs::remove_file(store.root.join(DIRECTORY).join(name)).await {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                    _ => Ok(()),
                }
            }
            Backend::Bucket { bucket, .. } => bucket.delete_meta(&key(name)).await,
        }
    }
}

fn key(name: &str) -> String {
    format!("{DIRECTORY}/{name}")
}
