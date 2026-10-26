use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::access::EVERY;
use super::cached::Cached;
use crate::audit::audit_log;
use crate::error::Error;
use crate::storage::Store;

const FILE: &str = "dashboard-tokens.json";
const PREFIX: &str = "lfsx_";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issued {
    pub name: String,
    pub created: u64,
    hash: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Refusal {
    #[error("a token name is 1 to 64 letters, digits, dots, dashes or underscores")]
    Unreadable,
    #[error("a token named {0} already exists")]
    Taken(String),
    #[error("no token is named {0}")]
    Unknown(String),
    #[error(transparent)]
    Store(#[from] Error),
}

fn digest(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn readable(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

pub async fn issued(store: &Store) -> Result<Vec<Issued>, Error> {
    match store.read_signed(FILE).await? {
        Some(bytes) => Ok(serde_json::from_slice(&bytes)?),
        None => Ok(Vec::new()),
    }
}

async fn keep(store: &Store, issued: &[Issued]) -> Result<(), Error> {
    store
        .write_signed(FILE, serde_json::to_vec_pretty(issued)?)
        .await
}

pub async fn create(store: &Store, name: &str) -> Result<String, Refusal> {
    if !readable(name) {
        return Err(Refusal::Unreadable);
    }
    let mut issued = issued(store).await?;
    if issued.iter().any(|token| token.name == name) {
        return Err(Refusal::Taken(name.to_owned()));
    }

    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).expect("the operating system has a random number generator");
    let token = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(secret));

    issued.push(Issued {
        name: name.to_owned(),
        created: time::OffsetDateTime::now_utc()
            .unix_timestamp()
            .unsigned_abs(),
        hash: digest(&token),
    });
    keep(store, &issued).await?;
    audit_log!(name, "a dashboard token was created");

    Ok(token)
}

pub async fn revoke(store: &Store, name: &str) -> Result<(), Refusal> {
    let mut issued = issued(store).await?;
    let before = issued.len();
    issued.retain(|token| token.name != name);
    if issued.len() == before {
        return Err(Refusal::Unknown(name.to_owned()));
    }

    keep(store, &issued).await?;
    audit_log!(name, "a dashboard token was revoked");

    Ok(())
}

impl Issued {
    pub fn hash(&self) -> &str {
        &self.hash
    }
}

pub struct Registry {
    cached: Cached<Vec<Issued>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            cached: Cached::new(EVERY),
        }
    }
}

impl Registry {
    async fn all(&self, store: &Store) -> Result<Arc<Vec<Issued>>, Error> {
        self.cached.get(|| issued(store)).await
    }

    pub async fn holder(&self, store: &Store, token: &str) -> Result<Option<Issued>, Error> {
        if !token.starts_with(PREFIX) {
            return Ok(None);
        }
        let hash = digest(token);

        Ok(self
            .all(store)
            .await?
            .iter()
            .find(|issued| issued.hash == hash)
            .cloned())
    }

    pub async fn still_issued(&self, store: &Store, hash: &str) -> Result<bool, Error> {
        Ok(self
            .all(store)
            .await?
            .iter()
            .any(|issued| issued.hash == hash))
    }

    pub async fn create(&self, store: &Store, name: &str) -> Result<String, Refusal> {
        let created = create(store, name).await;
        self.cached.invalidate().await;
        created
    }

    pub async fn revoke(&self, store: &Store, name: &str) -> Result<(), Refusal> {
        let revoked = revoke(store, name).await;
        self.cached.invalidate().await;
        revoked
    }
}

#[cfg(test)]
mod tests;
