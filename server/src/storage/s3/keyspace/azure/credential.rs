use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::config::AzureCredential;
use crate::error::Error;
use crate::storage::s3::keyspace::token::{Cached, Issued, issued};

pub(crate) const VERSION: &str = "2022-11-02";

const STORAGE_SCOPE: &str = "https://storage.azure.com/.default";
const STORAGE_RESOURCE: &str = "https://storage.azure.com/";
const MANAGED_IDENTITY_ENDPOINT: &str = "http://169.254.169.254/metadata/identity/oauth2/token";
const DEFAULT_AUTHORITY: &str = "https://login.microsoftonline.com/";

pub(crate) enum Credential {
    Key { account: String, key: Vec<u8> },
    Sas(String),
    Identity(Identity),
}

pub(crate) enum Scope<'a> {
    Container,
    Blob(&'a str),
}

impl Credential {
    pub(crate) fn new(account: &str, credential: &AzureCredential) -> Result<Self, Error> {
        Ok(match credential {
            AzureCredential::AccountKey(key) => Self::Key {
                account: account.to_owned(),
                key: base64::engine::general_purpose::STANDARD
                    .decode(key.trim())
                    .map_err(|_| Error::Misconfigured("LFSX_AZURE_ACCOUNT_KEY is not base64"))?,
            },
            AzureCredential::Sas(token) => Self::Sas(token.trim_start_matches('?').to_owned()),
            AzureCredential::Identity => Self::Identity(Identity::from_env()),
        })
    }

    pub(crate) fn sas(
        &self,
        container: &str,
        scope: Scope<'_>,
        permissions: &str,
        lifetime: Duration,
    ) -> Option<String> {
        let Self::Key { account, key } = self else {
            return None;
        };

        let expiry = (time::OffsetDateTime::now_utc() + lifetime)
            .replace_nanosecond(0)
            .ok()?
            .format(&time::format_description::well_known::Rfc3339)
            .ok()?;

        let (resource, kind) = match scope {
            Scope::Container => (format!("/blob/{account}/{container}"), "c"),
            Scope::Blob(blob) => (format!("/blob/{account}/{container}/{blob}"), "b"),
        };

        let unsigned = [
            permissions,
            "",
            &expiry,
            &resource,
            "",
            "",
            "",
            VERSION,
            kind,
            "",
            "",
            "",
            "",
            "",
            "",
            "",
        ]
        .join("\n");

        let mut mac = Hmac::<Sha256>::new_from_slice(key).ok()?;
        mac.update(unsigned.as_bytes());
        let signature =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

        Some(
            form_urlencoded::Serializer::new(String::new())
                .append_pair("sv", VERSION)
                .append_pair("sp", permissions)
                .append_pair("se", &expiry)
                .append_pair("sr", kind)
                .append_pair("sig", &signature)
                .finish(),
        )
    }
}

pub(crate) struct Identity {
    source: Source,
    cached: Cached,
}

pub(crate) enum Source {
    Workload {
        authority: String,
        tenant: String,
        client_id: String,
        token_file: PathBuf,
    },
    Managed {
        endpoint: String,
        client_id: Option<String>,
    },
}

impl Identity {
    pub(crate) fn new(source: Source) -> Self {
        Self {
            source,
            cached: Cached::default(),
        }
    }

    fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

        let source = match (
            var("AZURE_FEDERATED_TOKEN_FILE"),
            var("AZURE_TENANT_ID"),
            var("AZURE_CLIENT_ID"),
        ) {
            (Some(token_file), Some(tenant), Some(client_id)) => Source::Workload {
                authority: var("AZURE_AUTHORITY_HOST").unwrap_or_else(|| DEFAULT_AUTHORITY.into()),
                tenant,
                client_id,
                token_file: token_file.into(),
            },
            (_, _, client_id) => Source::Managed {
                endpoint: MANAGED_IDENTITY_ENDPOINT.into(),
                client_id,
            },
        };

        Self::new(source)
    }

    pub(crate) async fn token(&self, client: &reqwest::Client) -> Result<String, Error> {
        self.cached.get(self.fetch(client)).await
    }

    async fn fetch(&self, client: &reqwest::Client) -> Result<Issued, Error> {
        let request = match &self.source {
            Source::Workload {
                authority,
                tenant,
                client_id,
                token_file,
            } => {
                let assertion = tokio::fs::read_to_string(token_file).await?;
                let body = form_urlencoded::Serializer::new(String::new())
                    .append_pair("grant_type", "client_credentials")
                    .append_pair("client_id", client_id)
                    .append_pair("scope", STORAGE_SCOPE)
                    .append_pair(
                        "client_assertion_type",
                        "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                    )
                    .append_pair("client_assertion", assertion.trim())
                    .finish();

                client
                    .post(format!(
                        "{}/{tenant}/oauth2/v2.0/token",
                        authority.trim_end_matches('/')
                    ))
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(body)
            }
            Source::Managed {
                endpoint,
                client_id,
            } => {
                let mut url = reqwest::Url::parse(endpoint).map_err(|_| {
                    Error::Misconfigured("the managed identity endpoint is not a URL")
                })?;
                url.query_pairs_mut()
                    .append_pair("api-version", "2018-02-01")
                    .append_pair("resource", STORAGE_RESOURCE);
                if let Some(client_id) = client_id {
                    url.query_pairs_mut().append_pair("client_id", client_id);
                }

                client.get(url).header("Metadata", "true")
            }
        };

        issued(request).await
    }
}
