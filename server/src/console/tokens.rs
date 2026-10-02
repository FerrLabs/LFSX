use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    match store.read_meta(FILE).await? {
        Some(bytes) => Ok(serde_json::from_slice(&bytes)?),
        None => Ok(Vec::new()),
    }
}

async fn keep(store: &Store, issued: &[Issued]) -> Result<(), Error> {
    store
        .write_meta(FILE, serde_json::to_vec_pretty(issued)?)
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

    Ok(token)
}

pub async fn revoke(store: &Store, name: &str) -> Result<(), Refusal> {
    let mut issued = issued(store).await?;
    let before = issued.len();
    issued.retain(|token| token.name != name);
    if issued.len() == before {
        return Err(Refusal::Unknown(name.to_owned()));
    }

    Ok(keep(store, &issued).await?)
}

pub async fn holder(store: &Store, token: &str) -> Result<Option<String>, Error> {
    if !token.starts_with(PREFIX) {
        return Ok(None);
    }
    let hash = digest(token);

    Ok(issued(store)
        .await?
        .into_iter()
        .find(|issued| issued.hash == hash)
        .map(|issued| issued.name))
}

pub async fn still_issued(store: &Store, name: &str) -> Result<bool, Error> {
    Ok(issued(store)
        .await?
        .iter()
        .any(|issued| issued.name == name))
}
