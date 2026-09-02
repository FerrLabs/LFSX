use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::error::Error;
use crate::namespace::Namespace;

// The server's own GitHub identity, used for the one call it makes with no
// client token behind it: the anonymous public-repository lookup. An App JWT
// can only speak to App endpoints, so it is exchanged for an installation
// token, which carries the installation's own budget (5,000 an hour, scaling
// with repositories) instead of the 60-an-hour unauthenticated ceiling the
// anonymous path spends today.
pub struct App {
    id: String,
    key: jsonwebtoken::EncodingKey,
    grants: Mutex<HashMap<String, Grant>>,
}

#[derive(Clone)]
struct Grant {
    token: String,
    until: SystemTime,
}

#[derive(Deserialize)]
struct Installation {
    id: u64,
}

#[derive(Deserialize)]
struct Minted {
    token: String,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: time::OffsetDateTime,
}

#[derive(serde::Serialize)]
struct Claims {
    iss: String,
    iat: u64,
    exp: u64,
}

impl App {
    // At boot, and loudly: an operator who configured an App meant to have its
    // quota, so a key that does not load is a server that does not start.
    pub fn load(app_id: &str, key_file: &Path) -> Self {
        let pem = std::fs::read(key_file).unwrap_or_else(|error| {
            panic!("LFSX_GITHUB_APP_KEY_FILE could not be read from {key_file:?}: {error}")
        });
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(&pem).unwrap_or_else(|error| {
            panic!("LFSX_GITHUB_APP_KEY_FILE is not an RSA private key in PEM form: {error}")
        });

        Self {
            id: app_id.to_owned(),
            key,
            grants: Mutex::new(HashMap::new()),
        }
    }

    // Backdated a minute and expiring well under GitHub's ten-minute ceiling,
    // so clock drift on either side does not turn into a 401.
    fn jwt(&self) -> Result<String, Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is past 1970")
            .as_secs();
        let claims = Claims {
            iss: self.id.clone(),
            iat: now - 60,
            exp: now + 540,
        };

        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &self.key,
        )
        .map_err(|error| {
            tracing::warn!(%error, "the App JWT could not be signed");
            Error::Forge
        })
    }

    // An installation token that covers this repository, or None when the App
    // is not installed there, which sends the caller back to the plain
    // anonymous path rather than refusing a repository the App simply never
    // met. Cached per organization until shortly before expiry, so a busy
    // server exchanges once an hour, not once a request.
    pub async fn token(
        &self,
        client: &reqwest::Client,
        api_url: &str,
        ns: &Namespace,
    ) -> Result<Option<String>, Error> {
        let org = ns.org().to_owned();

        if let Some(grant) = self.grants.lock().unwrap().get(&org)
            && SystemTime::now() < grant.until
        {
            return Ok(Some(grant.token.clone()));
        }

        let jwt = self.jwt()?;

        let installed = crate::telemetry::propagated(
            client
                .get(format!("{api_url}/repos/{ns}/installation"))
                .bearer_auth(&jwt)
                .header("accept", "application/vnd.github+json"),
        )
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, "the forge could not be asked where the App is installed");
            Error::Forge
        })?;

        if installed.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let installation: Installation = installed
            .error_for_status()
            .map_err(|error| {
                tracing::warn!(%error, "the forge refused the App's installation lookup");
                Error::Forge
            })?
            .json()
            .await
            .map_err(|error| {
                tracing::warn!(%error, "the forge's installation answer could not be parsed");
                Error::Forge
            })?;

        let minted: Minted = crate::telemetry::propagated(
            client
                .post(format!(
                    "{api_url}/app/installations/{}/access_tokens",
                    installation.id
                ))
                .bearer_auth(&jwt)
                .header("accept", "application/vnd.github+json"),
        )
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, "the forge could not be asked for an installation token");
            Error::Forge
        })?
        .error_for_status()
        .map_err(|error| {
            tracing::warn!(%error, "the forge refused to mint an installation token");
            Error::Forge
        })?
        .json()
        .await
        .map_err(|error| {
            tracing::warn!(%error, "the forge's token answer could not be parsed");
            Error::Forge
        })?;

        let until = SystemTime::from(minted.expires_at) - Duration::from_secs(60);
        self.grants.lock().unwrap().insert(
            org,
            Grant {
                token: minted.token.clone(),
                until,
            },
        );

        Ok(Some(minted.token))
    }
}

#[cfg(test)]
mod tests;
