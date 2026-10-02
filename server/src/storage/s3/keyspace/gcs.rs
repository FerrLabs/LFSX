mod credential;
mod signing;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::{Entry, expect_success, read_retrying, unreachable_store};
use crate::config::GcsCredential;
use crate::error::Error;

pub(crate) use credential::Credential;

const SINGLE_PUT_CEILING: u64 = 256 * 1024 * 1024;
const CHUNK: u64 = 64 * 1024 * 1024;
const RESUME_INCOMPLETE: u16 = 308;

pub struct GcsConfig {
    pub endpoint: String,
    pub bucket: String,
    pub credential: GcsCredential,
    pub lifetime: Duration,
}

#[derive(Clone)]
pub struct GcsKeys {
    endpoint: reqwest::Url,
    bucket: String,
    credential: Arc<Credential>,
    client: reqwest::Client,
    lifetime: Duration,
}

#[derive(serde::Deserialize)]
struct Object {
    name: String,
    size: String,
    updated: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    #[serde(default)]
    items: Vec<Object>,
    next_page_token: Option<String>,
}

fn malformed(what: &'static str) -> Error {
    Error::Storage(std::io::Error::other(what))
}

impl GcsKeys {
    pub fn new(config: &GcsConfig) -> Result<Self, Error> {
        crate::tls::install_crypto_provider();

        Self::with_credential(config, Credential::new(&config.credential)?)
    }

    pub(crate) fn with_credential(
        config: &GcsConfig,
        credential: Credential,
    ) -> Result<Self, Error> {
        let mut endpoint = reqwest::Url::parse(&config.endpoint)
            .map_err(|_| Error::Misconfigured("LFSX_GCS_ENDPOINT is not a URL"))?;
        if endpoint.cannot_be_a_base() {
            return Err(Error::Misconfigured("LFSX_GCS_ENDPOINT is not a URL"));
        }
        if let Ok(mut segments) = endpoint.path_segments_mut() {
            segments.pop_if_empty();
        }

        Ok(Self {
            endpoint,
            bucket: config.bucket.clone(),
            credential: Arc::new(credential),
            client: reqwest::Client::new(),
            lifetime: config.lifetime,
        })
    }

    fn url(&self, upload: bool, object: Option<&str>, query: &[(&str, &str)]) -> reqwest::Url {
        let mut url = self.endpoint.clone();
        if let Ok(mut segments) = url.path_segments_mut() {
            if upload {
                segments.push("upload");
            }
            segments.extend(["storage", "v1", "b", &self.bucket, "o"]);
            if let Some(object) = object {
                segments.push(object);
            }
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url
    }

    async fn authorized(
        &self,
        method: Method,
        url: reqwest::Url,
    ) -> Result<reqwest::RequestBuilder, Error> {
        let request = self.client.request(method, url);

        Ok(match self.credential.bearer(&self.client).await? {
            Some(token) => request.bearer_auth(token),
            None => request,
        })
    }

    pub(crate) async fn reachable(&self) -> Result<(), Error> {
        let url = self.url(false, None, &[("maxResults", "1")]);
        let response = read_retrying(self.authorized(Method::GET, url).await?).await?;

        if !response.status().is_success() {
            return Err(Error::Storage(std::io::Error::other(format!(
                "the object store answered {} for the bucket",
                response.status()
            ))));
        }

        Ok(())
    }

    pub(crate) fn signed_download(&self, key: &str) -> Option<String> {
        signing::signed_download(
            &self.endpoint,
            &self.bucket,
            key,
            self.credential.service_account()?,
            self.lifetime,
        )
    }

    pub(crate) async fn get_range(
        &self,
        key: &str,
        start: u64,
        length: u64,
    ) -> Result<reqwest::Response, Error> {
        let url = self.url(false, Some(key), &[("alt", "media")]);
        let response = self
            .authorized(Method::GET, url)
            .await?
            .header(
                reqwest::header::RANGE,
                format!("bytes={start}-{}", start + length.saturating_sub(1)),
            )
            .send()
            .await
            .map_err(|_| unreachable_store())?;

        if !response.status().is_success() {
            return Err(Error::NotFound);
        }

        Ok(response)
    }

    pub(crate) async fn head(&self, key: &str) -> Result<u64, Error> {
        let url = self.url(false, Some(key), &[]);
        let response = read_retrying(self.authorized(Method::GET, url).await?).await?;

        if !response.status().is_success() {
            return Err(Error::NotFound);
        }

        let object: Object = response.json().await.map_err(|_| {
            malformed("the object store described an object this server could not read")
        })?;

        object
            .size
            .parse()
            .map_err(|_| malformed("the object store gave no object size"))
    }

    async fn insert(
        &self,
        key: &str,
        body: reqwest::Body,
        length: u64,
        only_if_absent: bool,
    ) -> Result<reqwest::Response, Error> {
        let mut query = vec![("uploadType", "media"), ("name", key)];
        if only_if_absent {
            query.push(("ifGenerationMatch", "0"));
        }

        self.authorized(Method::POST, self.url(true, None, &query))
            .await?
            .header(reqwest::header::CONTENT_LENGTH, length)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .send()
            .await
            .map_err(|_| unreachable_store())
    }

    pub(crate) async fn put(
        &self,
        key: &str,
        body: reqwest::Body,
        length: u64,
    ) -> Result<(), Error> {
        let response = self.insert(key, body, length, false).await?;
        expect_success(response, "write").await?;

        Ok(())
    }

    pub(crate) async fn put_file(&self, key: &str, staged: &Path) -> Result<(), Error> {
        let file = tokio::fs::File::open(staged).await?;
        let length = file.metadata().await?.len();

        if length > SINGLE_PUT_CEILING {
            drop(file);
            return self.put_resumable(key, staged, length, CHUNK).await;
        }

        let stream = tokio_util::io::ReaderStream::new(file);
        self.put(key, reqwest::Body::wrap_stream(stream), length)
            .await
    }

    pub(crate) async fn put_resumable(
        &self,
        key: &str,
        staged: &Path,
        length: u64,
        chunk: u64,
    ) -> Result<(), Error> {
        let url = self.url(true, None, &[("uploadType", "resumable"), ("name", key)]);
        let response = self
            .authorized(Method::POST, url)
            .await?
            .header(reqwest::header::CONTENT_LENGTH, 0)
            .header("x-upload-content-length", length)
            .send()
            .await
            .map_err(|_| unreachable_store())?;
        let session = expect_success(response, "start a resumable upload")
            .await?
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| malformed("the object store named no upload session"))?;

        tracing::info!(
            key,
            length,
            chunk,
            "an object over the single-request ceiling is going up in chunks"
        );

        let mut offset = 0;
        while offset < length {
            let this = chunk.min(length - offset);

            let mut file = tokio::fs::File::open(staged).await?;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            let stream = tokio_util::io::ReaderStream::new(file.take(this));

            let session = self
                .endpoint
                .join(&session)
                .map_err(|_| malformed("the object store named an unusable upload session"))?;
            let response = self
                .authorized(Method::PUT, session)
                .await?
                .header(reqwest::header::CONTENT_LENGTH, this)
                .header(
                    reqwest::header::CONTENT_RANGE,
                    format!("bytes {offset}-{}/{length}", offset + this - 1),
                )
                .body(reqwest::Body::wrap_stream(stream))
                .send()
                .await
                .map_err(|_| unreachable_store())?;

            offset += this;
            let finished = offset == length;
            match response.status().as_u16() {
                RESUME_INCOMPLETE if !finished => {}
                _ if finished => {
                    expect_success(response, "finish a resumable upload").await?;
                }
                _ => {
                    expect_success(response, "write a chunk").await?;
                    return Err(malformed(
                        "the object store finished an upload before it had every chunk",
                    ));
                }
            }
        }

        Ok(())
    }

    pub(crate) async fn put_if_absent(&self, key: &str, body: Vec<u8>) -> Result<bool, Error> {
        let length = body.len() as u64;
        let response = self
            .insert(key, reqwest::Body::from(body), length, true)
            .await?;

        if response.status() == StatusCode::PRECONDITION_FAILED {
            return Ok(false);
        }

        expect_success(response, "write").await?;

        Ok(true)
    }

    pub(crate) async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, Error> {
        let url = self.url(false, Some(key), &[("alt", "media")]);
        let response = read_retrying(self.authorized(Method::GET, url).await?).await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        expect_success(response, "read")
            .await?
            .bytes()
            .await
            .map(|bytes| Some(bytes.to_vec()))
            .map_err(|_| unreachable_store())
    }

    pub(crate) async fn delete(&self, key: &str) -> Result<bool, Error> {
        let url = self.url(false, Some(key), &[]);
        let response = self
            .authorized(Method::DELETE, url)
            .await?
            .send()
            .await
            .map_err(|_| unreachable_store())?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }

        expect_success(response, "delete").await?;

        Ok(true)
    }

    pub(crate) async fn entries(&self, prefix: &str) -> Result<Vec<Entry>, Error> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;

        loop {
            let mut query = vec![
                ("prefix", prefix),
                ("fields", "items(name,size,updated),nextPageToken"),
            ];
            if let Some(token) = &token {
                query.push(("pageToken", token));
            }

            let url = self.url(false, None, &query);
            let response = read_retrying(self.authorized(Method::GET, url).await?).await?;
            let page: Page = expect_success(response, "list")
                .await?
                .json()
                .await
                .map_err(|_| {
                    malformed("the object store sent a listing this server could not read")
                })?;

            for object in page.items {
                out.push(Entry {
                    size: object
                        .size
                        .parse()
                        .map_err(|_| malformed("the object store listed an object with no size"))?,
                    modified: object.updated.as_deref().and_then(|updated| {
                        time::OffsetDateTime::parse(
                            updated,
                            &time::format_description::well_known::Rfc3339,
                        )
                        .ok()
                    }),
                    key: object.name,
                });
            }

            match page.next_page_token.filter(|token| !token.is_empty()) {
                Some(next) => token = Some(next),
                None => break,
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests;
