use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::error::Error;

const REFRESH_BEFORE_EXPIRY: Duration = Duration::from_secs(300);

#[derive(serde::Deserialize)]
pub(crate) struct Issued {
    access_token: String,
    expires_in: serde_json::Value,
}

#[derive(Default)]
pub(crate) struct Cached {
    slot: Mutex<Option<(String, Instant)>>,
}

impl Cached {
    pub(crate) async fn get<F>(&self, fetch: F) -> Result<String, Error>
    where
        F: Future<Output = Result<Issued, Error>>,
    {
        if let Some((token, until)) = self.slot.lock().unwrap().as_ref()
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }

        let issued = fetch.await?;
        let lifetime = match &issued.expires_in {
            serde_json::Value::Number(seconds) => seconds.as_u64(),
            serde_json::Value::String(seconds) => seconds.parse().ok(),
            _ => None,
        }
        .map(Duration::from_secs)
        .unwrap_or_default();

        *self.slot.lock().unwrap() = Some((
            issued.access_token.clone(),
            Instant::now() + lifetime.saturating_sub(REFRESH_BEFORE_EXPIRY),
        ));

        Ok(issued.access_token)
    }
}

pub(crate) async fn issued(request: reqwest::RequestBuilder) -> Result<Issued, Error> {
    let response = request
        .send()
        .await
        .map_err(|_| refused("the identity endpoint is unreachable"))?;

    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "the identity endpoint refused a token");
        return Err(refused("the identity endpoint refused a token"));
    }

    response
        .json()
        .await
        .map_err(|_| refused("the identity endpoint sent no usable token"))
}

fn refused(what: &'static str) -> Error {
    Error::Storage(std::io::Error::other(what))
}
