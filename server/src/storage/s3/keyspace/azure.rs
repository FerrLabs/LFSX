mod credential;
mod listing;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use reqwest::{Method, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::{Entry, content_length, expect_success, read_retrying, unreachable_store};
use crate::config::AzureCredential;
use crate::error::Error;

pub(crate) use credential::{Credential, Scope};

const SINGLE_PUT_CEILING: u64 = 5000 * 1024 * 1024;
const BLOCK: u64 = 64 * 1024 * 1024;
const LARGEST_BLOCK: u64 = 4000 * 1024 * 1024;
const MOST_BLOCKS: u64 = 50_000;
const SERVER_PERMISSIONS: &str = "racwdl";

pub struct AzureConfig {
    pub endpoint: String,
    pub account: String,
    pub container: String,
    pub credential: AzureCredential,
    pub lifetime: Duration,
}

#[derive(Clone)]
pub struct AzureKeys {
    container_url: reqwest::Url,
    container: String,
    credential: Arc<Credential>,
    client: reqwest::Client,
    lifetime: Duration,
}

fn block_size(length: u64) -> u64 {
    BLOCK.max(length.div_ceil(MOST_BLOCKS)).min(LARGEST_BLOCK)
}

fn block_id(index: u64) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("{index:08}"))
}

impl AzureKeys {
    pub fn new(config: &AzureConfig) -> Result<Self, Error> {
        crate::tls::install_crypto_provider();

        let credential = Credential::new(&config.account, &config.credential)?;
        Self::with_credential(config, credential)
    }

    pub(crate) fn with_credential(
        config: &AzureConfig,
        credential: Credential,
    ) -> Result<Self, Error> {
        let mut container_url = reqwest::Url::parse(&config.endpoint)
            .map_err(|_| Error::Misconfigured("LFSX_AZURE_ENDPOINT is not a URL"))?;
        container_url
            .path_segments_mut()
            .map_err(|()| Error::Misconfigured("LFSX_AZURE_ENDPOINT is not a URL"))?
            .pop_if_empty()
            .push(&config.container);

        Ok(Self {
            container_url,
            container: config.container.clone(),
            credential: Arc::new(credential),
            client: reqwest::Client::new(),
            lifetime: config.lifetime,
        })
    }

    fn url(&self, blob: Option<&str>, query: &[(&str, &str)]) -> reqwest::Url {
        let mut url = self.container_url.clone();
        if let Some(blob) = blob
            && let Ok(mut segments) = url.path_segments_mut()
        {
            segments.extend(blob.split('/'));
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url
    }

    fn with_query(mut url: reqwest::Url, extra: &str) -> reqwest::Url {
        let query = match url.query() {
            Some(existing) if !existing.is_empty() => format!("{existing}&{extra}"),
            _ => extra.to_owned(),
        };
        url.set_query(Some(&query));
        url
    }

    async fn request(
        &self,
        method: Method,
        blob: Option<&str>,
        query: &[(&str, &str)],
    ) -> Result<reqwest::RequestBuilder, Error> {
        let url = self.url(blob, query);

        let request = match self.credential.as_ref() {
            Credential::Key { .. } => {
                let sas = self
                    .credential
                    .sas(
                        &self.container,
                        Scope::Container,
                        SERVER_PERMISSIONS,
                        self.lifetime,
                    )
                    .ok_or(Error::Misconfigured("the Azure account key could not sign"))?;
                self.client.request(method, Self::with_query(url, &sas))
            }
            Credential::Sas(token) => self.client.request(method, Self::with_query(url, token)),
            Credential::Identity(identity) => self
                .client
                .request(method, url)
                .bearer_auth(identity.token(&self.client).await?),
        };

        Ok(request.header("x-ms-version", credential::VERSION))
    }

    pub(crate) async fn reachable(&self) -> Result<(), Error> {
        let request = self
            .request(
                Method::GET,
                None,
                &[
                    ("restype", "container"),
                    ("comp", "list"),
                    ("maxresults", "1"),
                ],
            )
            .await?;
        let response = read_retrying(request).await?;

        if !response.status().is_success() {
            return Err(Error::Storage(std::io::Error::other(format!(
                "the object store answered {} for the container",
                response.status()
            ))));
        }

        Ok(())
    }

    pub(crate) fn signed_download(&self, key: &str) -> Option<String> {
        let sas = self
            .credential
            .sas(&self.container, Scope::Blob(key), "r", self.lifetime)?;

        Some(Self::with_query(self.url(Some(key), &[]), &sas).to_string())
    }

    pub(crate) async fn get_range(
        &self,
        key: &str,
        start: u64,
        length: u64,
    ) -> Result<reqwest::Response, Error> {
        let response = self
            .request(Method::GET, Some(key), &[])
            .await?
            .header(
                "x-ms-range",
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
        let response = read_retrying(self.request(Method::HEAD, Some(key), &[]).await?).await?;

        if !response.status().is_success() {
            return Err(Error::NotFound);
        }

        content_length(&response)
    }

    async fn put_blob(
        &self,
        key: &str,
        body: reqwest::Body,
        length: u64,
        only_if_absent: bool,
    ) -> Result<reqwest::Response, Error> {
        let mut request = self
            .request(Method::PUT, Some(key), &[])
            .await?
            .header("x-ms-blob-type", "BlockBlob")
            .header(reqwest::header::CONTENT_LENGTH, length);
        if only_if_absent {
            request = request.header(reqwest::header::IF_NONE_MATCH, "*");
        }

        request
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
        let response = self.put_blob(key, body, length, false).await?;
        expect_success(response, "write").await?;

        Ok(())
    }

    pub(crate) async fn put_file(&self, key: &str, staged: &Path) -> Result<(), Error> {
        let file = tokio::fs::File::open(staged).await?;
        let length = file.metadata().await?.len();

        if length > SINGLE_PUT_CEILING {
            drop(file);
            return self
                .put_in_blocks(key, staged, length, block_size(length))
                .await;
        }

        let stream = tokio_util::io::ReaderStream::new(file);
        self.put(key, reqwest::Body::wrap_stream(stream), length)
            .await
    }

    pub(crate) async fn put_in_blocks(
        &self,
        key: &str,
        staged: &Path,
        length: u64,
        size: u64,
    ) -> Result<(), Error> {
        let count = length.div_ceil(size);
        if count > MOST_BLOCKS {
            return Err(Error::Storage(std::io::Error::other(
                "this object needs more blocks than one blob may have",
            )));
        }

        tracing::info!(
            key,
            length,
            block_size = size,
            "an object over the single-request ceiling is going up in blocks"
        );

        let mut ids = Vec::new();
        for index in 0..count {
            let offset = index * size;
            let this = size.min(length - offset);

            let mut file = tokio::fs::File::open(staged).await?;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            let stream = tokio_util::io::ReaderStream::new(file.take(this));

            let id = block_id(index);
            let response = self
                .request(
                    Method::PUT,
                    Some(key),
                    &[("comp", "block"), ("blockid", &id)],
                )
                .await?
                .header(reqwest::header::CONTENT_LENGTH, this)
                .body(reqwest::Body::wrap_stream(stream))
                .send()
                .await
                .map_err(|_| unreachable_store())?;
            expect_success(response, "write a block").await?;

            ids.push(id);
        }

        let list = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>{}</BlockList>",
            ids.iter()
                .map(|id| format!("<Latest>{id}</Latest>"))
                .collect::<String>()
        );

        let response = self
            .request(Method::PUT, Some(key), &[("comp", "blocklist")])
            .await?
            .header(reqwest::header::CONTENT_LENGTH, list.len())
            .body(list)
            .send()
            .await
            .map_err(|_| unreachable_store())?;
        expect_success(response, "assemble a blob from its blocks").await?;

        Ok(())
    }

    pub(crate) async fn put_if_absent(&self, key: &str, body: Vec<u8>) -> Result<bool, Error> {
        let length = body.len() as u64;
        let response = self
            .put_blob(key, reqwest::Body::from(body), length, true)
            .await?;

        if matches!(
            response.status(),
            StatusCode::CONFLICT | StatusCode::PRECONDITION_FAILED
        ) {
            return Ok(false);
        }

        expect_success(response, "write").await?;

        Ok(true)
    }

    pub(crate) async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, Error> {
        let response = read_retrying(self.request(Method::GET, Some(key), &[]).await?).await?;

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
        let response = self
            .request(Method::DELETE, Some(key), &[])
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
        let mut marker: Option<String> = None;

        loop {
            let mut query = vec![
                ("restype", "container"),
                ("comp", "list"),
                ("prefix", prefix),
            ];
            if let Some(marker) = &marker {
                query.push(("marker", marker));
            }

            let response = read_retrying(self.request(Method::GET, None, &query).await?).await?;
            let body = expect_success(response, "list")
                .await?
                .text()
                .await
                .map_err(|_| unreachable_store())?;

            let page = listing::parse(&body)?;
            out.extend(page.entries);

            match page.next {
                Some(next) => marker = Some(next),
                None => break,
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests;
