use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::Store;
use crate::error::Error;

#[derive(Serialize, Deserialize)]
struct Envelope {
    payload: serde_json::Value,
    mac: String,
}

fn mac(key: &[u8], name: &str, payload: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(name.as_bytes());
    mac.update(&[0]);
    mac.update(payload);
    mac
}

impl Store {
    pub fn with_signing(mut self, key: Option<[u8; 32]>) -> Self {
        self.signing = key;
        self
    }

    pub async fn read_signed(&self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        let Some(bytes) = self.read_meta(name).await? else {
            return Ok(None);
        };
        let Some(key) = &self.signing else {
            return Ok(Some(bytes));
        };

        let verified = serde_json::from_slice::<Envelope>(&bytes)
            .ok()
            .and_then(|envelope| {
                let payload = serde_json::to_vec(&envelope.payload).ok()?;
                let signature = hex::decode(&envelope.mac).ok()?;
                mac(key, name, &payload).verify_slice(&signature).ok()?;
                Some(payload)
            });
        if verified.is_none() {
            tracing::warn!(
                file = name,
                "a file kept in the store is not signed with LFSX_SESSION_KEY_FILE: it was \
                 written without that key or has been replaced, so it is ignored"
            );
        }

        Ok(verified)
    }

    pub async fn write_signed(&self, name: &str, bytes: Vec<u8>) -> Result<(), Error> {
        let Some(key) = &self.signing else {
            return self.write_meta(name, bytes).await;
        };

        let payload: serde_json::Value = serde_json::from_slice(&bytes)?;
        let signature = mac(key, name, &serde_json::to_vec(&payload)?)
            .finalize()
            .into_bytes();
        let envelope = Envelope {
            payload,
            mac: hex::encode(signature),
        };

        self.write_meta(name, serde_json::to_vec_pretty(&envelope)?)
            .await
    }
}

#[cfg(test)]
mod tests;
