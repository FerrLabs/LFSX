use std::path::Path;
use std::time::Duration;

use rusty_s3::actions::{
    DeleteObject, GetObject, HeadBucket, HeadObject, ListObjectsV2, PutObject, S3Action,
};
use rusty_s3::{Bucket, Credentials, UrlStyle};

use super::{Entry, Presigned, content_length, expect_success, read_retrying, unreachable_store};
use crate::error::Error;
use crate::storage::s3::{S3Config, multipart};

const COPY_SOURCE: &str = "x-amz-copy-source";
const CHECKSUM: &str = "x-amz-checksum-sha256";

#[derive(Clone)]
pub struct S3Keys {
    bucket: Bucket,
    credentials: Credentials,
    client: reqwest::Client,
    lifetime: Duration,
}

impl S3Keys {
    pub fn new(config: &S3Config) -> Result<Self, Error> {
        crate::tls::install_crypto_provider();

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
            lifetime: config.lifetime,
        })
    }

    // Whether this server can reach the store at all, which is one HEAD on the
    // bucket. Only the status is reported: the store says why in a body that
    // names the bucket, and readiness is answered to whoever asks.
    pub(crate) async fn reachable(&self) -> Result<(), Error> {
        let action = HeadBucket::new(&self.bucket, Some(&self.credentials));
        let response = read_retrying(self.client.head(action.sign(self.lifetime))).await?;

        if !response.status().is_success() {
            return Err(Error::Storage(std::io::Error::other(format!(
                "the object store answered {} for the bucket",
                response.status()
            ))));
        }

        Ok(())
    }

    // What it takes to sign an action this module does not itself perform. A
    // multipart upload is four different actions sharing one upload id, which is
    // a sequence rather than a key operation, so it lives beside this rather than
    // inside it and borrows the signing material.
    pub(crate) fn bucket(&self) -> &Bucket {
        &self.bucket
    }

    pub(crate) fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    pub(crate) fn lifetime(&self) -> Duration {
        self.lifetime
    }

    pub(crate) fn client(&self) -> &reqwest::Client {
        &self.client
    }

    // Whether the caller is entitled to the bytes is settled before this is
    // called: the signature is scoped to one key and it expires.
    pub(crate) fn signed_download(&self, key: &str) -> String {
        GetObject::new(&self.bucket, Some(&self.credentials), key)
            .sign(self.lifetime)
            .to_string()
    }

    // The same for a write, with the digest bound into the signature rather than
    // merely suggested: a conforming store refuses a body that does not match
    // it, which is what makes handing out a write URL safe at all.
    //
    // Conforming is the load-bearing word, and it is not assumed. `probe` asks
    // the store at startup whether it really does refuse, because a store that
    // accepts the header and ignores it turns this from a guarantee into a hope.
    pub(crate) fn signed_upload(&self, key: &str, digest: &str) -> Presigned {
        let mut action = PutObject::new(&self.bucket, Some(&self.credentials), key);
        action
            .headers_mut()
            .insert(CHECKSUM, std::borrow::Cow::Owned(digest.to_owned()));

        Presigned {
            href: action.sign(self.lifetime).to_string(),
            headers: vec![(CHECKSUM.to_owned(), digest.to_owned())],
        }
    }

    // A ranged read streamed rather than buffered: a value here can be measured
    // in gigabytes, and the whole storage layer is built on holding at most a few
    // megabytes of one at a time.
    pub(crate) async fn get_range(
        &self,
        key: &str,
        start: u64,
        length: u64,
    ) -> Result<reqwest::Response, Error> {
        let action = GetObject::new(&self.bucket, Some(&self.credentials), key);

        let response = self
            .client
            .get(action.sign(self.lifetime))
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

    // Signed as a HEAD rather than reusing a GET signature: SigV4 covers the
    // method, and an implementation that checks it (which is the point of
    // testing against MinIO and Garage rather than only AWS) is entitled to
    // refuse the mismatch.
    pub(crate) async fn head(&self, key: &str) -> Result<u64, Error> {
        let action = HeadObject::new(&self.bucket, Some(&self.credentials), key);
        let url = action.sign(self.lifetime);

        let response = read_retrying(self.client.head(url)).await?;

        if !response.status().is_success() {
            return Err(Error::NotFound);
        }

        // Read the header rather than the body length: a HEAD has no body, and
        // asking the response how long it is answers about what was received
        // rather than what is there.
        content_length(&response)
    }

    pub(crate) async fn put(
        &self,
        key: &str,
        body: reqwest::Body,
        length: u64,
    ) -> Result<(), Error> {
        let action = PutObject::new(&self.bucket, Some(&self.credentials), key);
        let url = action.sign(self.lifetime);

        let response = self
            .client
            .put(url)
            // S3 has no use for a chunked body and answers 501 rather than
            // starting the upload. reqwest cannot infer a length from a stream,
            // so it comes from the staging file being sent.
            .header(reqwest::header::CONTENT_LENGTH, length)
            .body(body)
            .send()
            .await
            .map_err(|_| unreachable_store())?;

        // The store says why in the body, and an operator staring at a failing
        // push has nothing else to go on: a bucket that does not exist, a key
        // that is denied and a clock that has drifted are three different
        // afternoons.
        expect_success(response, "write").await?;

        Ok(())
    }

    // One request while one request will carry it, which is every object a
    // store normally sees, and parts when it will not. The split is here rather
    // than always going in parts because the single write is one round trip and
    // needs no cleanup if it fails.
    pub(crate) async fn put_file(&self, key: &str, staged: &Path) -> Result<(), Error> {
        let file = tokio::fs::File::open(staged).await?;
        let length = file.metadata().await?.len();

        if length > multipart::SINGLE_PUT_CEILING {
            drop(file);
            return multipart::put(self, key, staged, length).await;
        }

        let stream = tokio_util::io::ReaderStream::new(file);
        self.put(key, reqwest::Body::wrap_stream(stream), length)
            .await
    }

    // A copy is a PUT to the destination carrying `x-amz-copy-source`, so this is
    // a signed PutObject with that header bound rather than a separate action.
    // The bytes move inside the store: nothing crosses this server.
    pub(crate) async fn copy(&self, from: &str, to: &str) -> Result<(), Error> {
        let source = format!("/{}/{from}", self.bucket.name());
        let mut action = PutObject::new(&self.bucket, Some(&self.credentials), to);
        action
            .headers_mut()
            .insert(COPY_SOURCE, std::borrow::Cow::Owned(source.clone()));

        let response = self
            .client
            .put(action.sign(self.lifetime))
            .header(COPY_SOURCE, source)
            .header(reqwest::header::CONTENT_LENGTH, 0)
            .send()
            .await
            .map_err(|_| unreachable_store())?;

        expect_success(response, "copy").await?;

        Ok(())
    }

    // The mutual exclusion `create_new` gives on a filesystem, asked of S3.
    // `If-None-Match: *` is a conditional write: the store itself decides who
    // arrived first, and answers 412 to everyone after. Without it two replicas
    // sharing a bucket would each believe they took the lock.
    //
    // The header is bound into the signature and sent alongside, so a store that
    // ignores conditional writes cannot silently accept both.
    pub(crate) async fn put_if_absent(&self, key: &str, body: Vec<u8>) -> Result<bool, Error> {
        let mut action = PutObject::new(&self.bucket, Some(&self.credentials), key);
        action.headers_mut().insert("if-none-match", "*");
        let url = action.sign(self.lifetime);

        let length = body.len();
        let response = self
            .client
            .put(url)
            .header("if-none-match", "*")
            .header(reqwest::header::CONTENT_LENGTH, length)
            .body(body)
            .send()
            .await
            .map_err(|_| unreachable_store())?;

        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Ok(false);
        }

        expect_success(response, "write").await?;

        Ok(true)
    }

    pub(crate) async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, Error> {
        let action = GetObject::new(&self.bucket, Some(&self.credentials), key);
        let response = read_retrying(self.client.get(action.sign(self.lifetime))).await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = expect_success(response, "read").await?;

        response
            .bytes()
            .await
            .map(|bytes| Some(bytes.to_vec()))
            .map_err(|_| unreachable_store())
    }

    pub(crate) async fn delete(&self, key: &str) -> Result<bool, Error> {
        // S3 answers 204 whether or not the key was there, so whether this
        // removed anything is settled before asking.
        let existed = self.head(key).await.is_ok();

        let action = DeleteObject::new(&self.bucket, Some(&self.credentials), key);
        let response = self
            .client
            .delete(action.sign(self.lifetime))
            .send()
            .await
            .map_err(|_| unreachable_store())?;

        expect_success(response, "delete").await?;

        Ok(existed)
    }

    pub(crate) async fn entries(&self, prefix: &str) -> Result<Vec<Entry>, Error> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;

        loop {
            let mut action = ListObjectsV2::new(&self.bucket, Some(&self.credentials));
            action.with_prefix(prefix);
            if let Some(token) = &token {
                action.with_continuation_token(token);
            }

            let response = read_retrying(self.client.get(action.sign(self.lifetime))).await?;
            let body = expect_success(response, "list")
                .await?
                .text()
                .await
                .map_err(|_| unreachable_store())?;

            let listing = ListObjectsV2::parse_response(&body).map_err(|error| {
                Error::Storage(std::io::Error::other(format!(
                    "the object store sent a listing this server could not read: {error}"
                )))
            })?;

            out.extend(listing.contents.into_iter().map(|object| {
                Entry {
                    key: object.key,
                    modified: time::OffsetDateTime::parse(
                        &object.last_modified,
                        &time::format_description::well_known::Rfc3339,
                    )
                    .ok(),
                    size: object.size,
                }
            }));

            match listing.next_continuation_token {
                Some(next) => token = Some(next),
                None => break,
            }
        }

        Ok(out)
    }
}
