use std::time::Duration;

use axum::body::Bytes;
use futures_util::Stream;
use rusty_s3::actions::{GetObject, ListObjectsV2, PutObject, S3Action};
use rusty_s3::{Bucket, Credentials, UrlStyle};

use crate::error::Error;
use crate::namespace::Namespace;

// The same layout as the local store, for the same reasons. The bytes live once
// under a key derived from their digest, and a repository that holds them owns
// an empty marker beside it — the object store's answer to a hard link. It is
// what keeps two projects sharing an asset pack from paying twice, and what
// stops a repository reading an object it never pushed: the marker is the proof
// of possession, and it is the only thing the permission check consults.
pub struct S3Store {
    bucket: Bucket,
    credentials: Credentials,
    client: reqwest::Client,
    presign: Duration,
}

pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub path_style: bool,
}

impl S3Store {
    pub fn new(config: &S3Config) -> Result<Self, Error> {
        let style = if config.path_style {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };

        let bucket = Bucket::new(
            config
                .endpoint
                .parse()
                .map_err(|_| Error::Misconfigured("LFSX_S3_ENDPOINT is not a URL"))?,
            style,
            config.bucket.clone(),
            config.region.clone(),
        )
        .map_err(|_| Error::Misconfigured("LFSX_S3_BUCKET is not a usable bucket name"))?;

        Ok(Self {
            bucket,
            credentials: Credentials::new(config.access_key.clone(), config.secret_key.clone()),
            client: reqwest::Client::new(),
            presign: Duration::from_secs(1800),
        })
    }

    fn content_key(oid: &str) -> String {
        format!(".content/{}/{}/{oid}", &oid[0..2], &oid[2..4])
    }

    fn marker_key(ns: &Namespace, oid: &str) -> String {
        format!(
            "{}/{}/{}/{}/{oid}",
            ns.org(),
            ns.repo(),
            &oid[0..2],
            &oid[2..4]
        )
    }

    pub async fn exists(&self, ns: &Namespace, oid: &str) -> bool {
        if crate::storage::LocalStore::validate_oid(oid).is_err() {
            return false;
        }

        self.head(&Self::marker_key(ns, oid)).await.is_ok()
    }

    async fn head(&self, key: &str) -> Result<u64, Error> {
        let action = GetObject::new(&self.bucket, Some(&self.credentials), key);
        let url = action.sign(self.presign);

        let response = self.client.head(url).send().await.map_err(|_| {
            Error::Storage(std::io::Error::other("the object store is unreachable"))
        })?;

        if !response.status().is_success() {
            return Err(Error::NotFound);
        }

        Ok(response.content_length().unwrap_or_default())
    }

    pub async fn size_of(&self, oid: &str) -> Result<u64, Error> {
        self.head(&Self::content_key(oid)).await
    }

    // A download is streamed through this server rather than redirected, so the
    // features that live in the byte path — the counters, the ranges, and the
    // compression that will follow — keep working. The pre-signed redirect is a
    // separate mode for operators who would rather spend the object store's
    // bandwidth than their own.
    pub async fn read(
        &self,
        oid: &str,
        start: u64,
        length: u64,
    ) -> Result<impl Stream<Item = Result<Bytes, reqwest::Error>> + use<>, Error> {
        let key = Self::content_key(oid);
        let action = GetObject::new(&self.bucket, Some(&self.credentials), &key);
        let url = action.sign(self.presign);

        let response = self
            .client
            .get(url)
            .header(
                reqwest::header::RANGE,
                format!("bytes={start}-{}", start + length.saturating_sub(1)),
            )
            .send()
            .await
            .map_err(|_| {
                Error::Storage(std::io::Error::other("the object store is unreachable"))
            })?;

        if !response.status().is_success() {
            return Err(Error::NotFound);
        }

        Ok(response.bytes_stream())
    }

    pub fn presigned_download(&self, oid: &str) -> String {
        let key = Self::content_key(oid);

        GetObject::new(&self.bucket, Some(&self.credentials), &key)
            .sign(self.presign)
            .to_string()
    }

    async fn put(&self, key: &str, body: reqwest::Body) -> Result<(), Error> {
        let action = PutObject::new(&self.bucket, Some(&self.credentials), key);
        let url = action.sign(self.presign);

        let response = self.client.put(url).body(body).send().await.map_err(|_| {
            Error::Storage(std::io::Error::other("the object store is unreachable"))
        })?;

        response
            .error_for_status()
            .map_err(|error| Error::Storage(std::io::Error::other(error)))?;

        Ok(())
    }

    // The upload has already been streamed to a staging file, hashed and checked
    // against everything the server enforces, so that file is what goes up —
    // streamed from disk rather than read into memory, because an object here is
    // measured in gigabytes and the whole storage layer is built on holding at
    // most a few megabytes of one at a time.
    //
    // The bytes go up once, keyed by their digest, and the marker records that
    // this repository holds them. Content that is already there is skipped: the
    // key would receive the same bytes it already has.
    pub async fn store(
        &self,
        ns: &Namespace,
        oid: &str,
        staged: &std::path::Path,
    ) -> Result<(), Error> {
        if self.head(&Self::content_key(oid)).await.is_err() {
            let file = tokio::fs::File::open(staged).await?;
            let stream = tokio_util::io::ReaderStream::new(file);

            self.put(&Self::content_key(oid), reqwest::Body::wrap_stream(stream))
                .await?;
        }

        self.put(&Self::marker_key(ns, oid), reqwest::Body::from(Vec::new()))
            .await
    }

    // What the bucket holds for this repository, counted from its markers and
    // the content they point at. The markers are empty, so their own size says
    // nothing — this is a listing plus one head per object, which is why the
    // figure is cached the same way the local one is.
    pub async fn usage_of(&self, ns: &Namespace) -> (u64, u64) {
        let prefix = format!("{}/{}/", ns.org(), ns.repo());
        let mut objects = 0;
        let mut bytes = 0;

        for oid in self.list(&prefix).await {
            objects += 1;
            bytes += self.size_of(&oid).await.unwrap_or_default();
        }

        (objects, bytes)
    }

    async fn list(&self, prefix: &str) -> Vec<String> {
        let mut action = ListObjectsV2::new(&self.bucket, Some(&self.credentials));
        action.with_prefix(prefix);

        // Every step logs what stopped it rather than returning an empty
        // listing: a capacity figure that silently reads zero is worse than one
        // that is missing, because it looks like an answer.
        let response = match self.client.get(action.sign(self.presign)).send().await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(%error, "the object store could not be listed");
                return Vec::new();
            }
        };

        let body = match response.text().await {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(%error, "the listing could not be read");
                return Vec::new();
            }
        };

        let listing = match ListObjectsV2::parse_response(&body) {
            Ok(listing) => listing,
            Err(error) => {
                tracing::warn!(%error, "the listing could not be parsed");
                return Vec::new();
            }
        };

        listing
            .contents
            .into_iter()
            .filter_map(|object| object.key.rsplit('/').next().map(str::to_owned))
            .filter(|oid| crate::storage::LocalStore::validate_oid(oid).is_ok())
            .collect()
    }
}

#[cfg(test)]
mod tests;
