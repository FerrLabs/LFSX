use jsonwebtoken::{Algorithm, EncodingKey, Header};

use crate::config::GcsCredential;
use crate::error::Error;
use crate::storage::s3::keyspace::token::{Cached, Issued, issued};

const SCOPE: &str = "https://www.googleapis.com/auth/devstorage.read_write";
const METADATA_HOST: &str = "metadata.google.internal";
const METADATA_TOKEN: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
const ASSERTION_LIFETIME: i64 = 3600;

pub(crate) struct ServiceAccount {
    pub(crate) email: String,
    pub(crate) key: EncodingKey,
    token_uri: String,
}

pub(crate) enum Source {
    ServiceAccount(ServiceAccount),
    Metadata { endpoint: String },
    Anonymous,
}

pub(crate) struct Credential {
    source: Source,
    cached: Cached,
}

#[derive(serde::Deserialize)]
struct KeyFile {
    client_email: String,
    private_key: String,
    token_uri: String,
}

#[derive(serde::Serialize)]
struct Claims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

impl ServiceAccount {
    pub(crate) fn from_json(json: &str) -> Result<Self, Error> {
        let file: KeyFile = serde_json::from_str(json).map_err(|_| {
            Error::Misconfigured("LFSX_GCS_CREDENTIALS is not a service account key file")
        })?;

        Ok(Self {
            email: file.client_email,
            key: EncodingKey::from_rsa_pem(file.private_key.as_bytes()).map_err(|_| {
                Error::Misconfigured("the service account key file holds no usable RSA key")
            })?,
            token_uri: file.token_uri,
        })
    }

    fn assertion(&self) -> Result<String, Error> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let claims = Claims {
            iss: &self.email,
            scope: SCOPE,
            aud: &self.token_uri,
            iat: now,
            exp: now + ASSERTION_LIFETIME,
        };

        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &self.key).map_err(|_| {
            Error::Storage(std::io::Error::other(
                "the service account key could not sign a token request",
            ))
        })
    }
}

impl Credential {
    pub(crate) fn new(credential: &GcsCredential) -> Result<Self, Error> {
        let source = match credential {
            GcsCredential::ServiceAccount(path) => {
                let json = std::fs::read_to_string(path).map_err(|_| {
                    Error::Misconfigured("LFSX_GCS_CREDENTIALS names a file that cannot be read")
                })?;
                Source::ServiceAccount(ServiceAccount::from_json(&json)?)
            }
            GcsCredential::Metadata => {
                let host = std::env::var("GCE_METADATA_HOST")
                    .ok()
                    .filter(|host| !host.is_empty())
                    .unwrap_or_else(|| METADATA_HOST.into());
                Source::Metadata {
                    endpoint: format!("http://{host}{METADATA_TOKEN}"),
                }
            }
            GcsCredential::Anonymous => Source::Anonymous,
        };

        Ok(Self::from_source(source))
    }

    pub(crate) fn from_source(source: Source) -> Self {
        Self {
            source,
            cached: Cached::default(),
        }
    }

    pub(crate) fn service_account(&self) -> Option<&ServiceAccount> {
        match &self.source {
            Source::ServiceAccount(account) => Some(account),
            _ => None,
        }
    }

    pub(crate) async fn bearer(&self, client: &reqwest::Client) -> Result<Option<String>, Error> {
        let request = match &self.source {
            Source::Anonymous => return Ok(None),
            Source::Metadata { endpoint } => {
                client.get(endpoint).header("Metadata-Flavor", "Google")
            }
            Source::ServiceAccount(account) => {
                let body = form_urlencoded::Serializer::new(String::new())
                    .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer")
                    .append_pair("assertion", &account.assertion()?)
                    .finish();

                client
                    .post(&account.token_uri)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(body)
            }
        };

        self.cached.get(fetch(request)).await.map(Some)
    }
}

async fn fetch(request: reqwest::RequestBuilder) -> Result<Issued, Error> {
    issued(request).await
}
