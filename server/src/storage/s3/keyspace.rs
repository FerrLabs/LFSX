use std::time::Duration;

use axum::body::Bytes;
use futures_util::Stream;
use rusty_s3::actions::{
    DeleteObject, GetObject, HeadBucket, HeadObject, ListObjectsV2, PutObject, S3Action,
};
use rusty_s3::{Bucket, Credentials, UrlStyle};

use crate::error::Error;
use crate::storage::s3::S3Config;

const COPY_SOURCE: &str = "x-amz-copy-source";

// One key as the store describes it.
// Everything a listing found, and whether it ran out before the end.
pub(crate) struct Listing {
    pub(crate) entries: Vec<Entry>,
    pub(crate) complete: bool,
}

pub(crate) struct Entry {
    pub(crate) key: String,
    last_modified: String,
    pub(crate) size: u64,
}

impl Entry {
    // None when the store's timestamp cannot be read, which is treated as "too
    // young to touch": deleting somebody's upload on the strength of a date this
    // server could not parse is the wrong way to be wrong.
    pub(crate) fn age(&self) -> Option<Duration> {
        let written = time::OffsetDateTime::parse(
            &self.last_modified,
            &time::format_description::well_known::Rfc3339,
        )
        .ok()?;

        Duration::try_from(time::OffsetDateTime::now_utc() - written).ok()
    }
}

// An href a client uses directly, and the headers it has to send with it. The
// headers are part of the signature, so they are not advice.
pub struct Presigned {
    pub href: String,
    pub headers: Vec<(String, String)>,
}

// The same layout as the local store, for the same reasons. The bytes live once
// under a key derived from their digest, and a repository that holds them owns
// an empty marker beside it, the object store's answer to a hard link. It is
// what keeps two projects sharing an asset pack from paying twice, and what
// stops a repository reading an object it never pushed: the marker is the proof
// of possession, and it is the only thing the permission check consults.
// A conditional write that is refused makes the store answer and hang up, and
// the connection goes back into the pool looking usable. The next request on it
// fails at the transport layer with nothing to do with the store's health, which
// is how a losing `git lfs lock` came back as a 500 instead of a 409.
//
// Retried once, and only for requests that carry no body: a GET and a HEAD can be
// repeated with no consequence, so a dead connection costs a round trip rather
// than an error. A PUT is not retried here.
async fn read_retrying(request: reqwest::RequestBuilder) -> Result<reqwest::Response, Error> {
    let retry = request.try_clone();

    match request.send().await {
        Ok(response) => Ok(response),
        Err(_) => match retry {
            Some(retry) => retry.send().await.map_err(|_| unreachable_store()),
            None => Err(unreachable_store()),
        },
    }
}

fn unreachable_store() -> Error {
    Error::Storage(std::io::Error::other("the object store is unreachable"))
}

// The bucket as a keyspace: whole values written, read, deleted and listed by
// key, with the signing and the HTTP client in one place. It knows nothing about
// objects, oids or repositories: what a key means is decided a layer up, which
// is what lets the lock store share the bucket with the object store without
// either of them reaching into the other.
#[derive(Clone)]
pub struct Keyspace {
    bucket: Bucket,
    credentials: Credentials,
    client: reqwest::Client,
    lifetime: Duration,
}

impl Keyspace {
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

    // A signature handed to a client so it reads the key straight from the store.
    // For the one caller that has to send a request the way a client would,
    // rather than the way this server does: the checksum probe puts a body
    // against a URL it was handed, and it has to do it over the configured
    // client so that the TLS, the proxy and the timeouts are the real ones.
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

    // The same for a write, with headers bound into the signature rather than
    // merely suggested: a conforming store refuses a body that does not match
    // them, which is what makes handing out a write URL safe at all.
    //
    // Conforming is the load-bearing word, and it is not assumed. `probe` asks
    // the store at startup whether it really does refuse, because a store that
    // accepts the header and ignores it turns this from a guarantee into a hope.
    pub(crate) fn signed_upload(&self, key: &str, headers: Vec<(String, String)>) -> Presigned {
        let mut action = PutObject::new(&self.bucket, Some(&self.credentials), key);

        for (name, value) in &headers {
            action
                .headers_mut()
                .insert(name.clone(), std::borrow::Cow::Owned(value.clone()));
        }

        Presigned {
            href: action.sign(self.lifetime).to_string(),
            headers,
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
    ) -> Result<impl Stream<Item = Result<Bytes, reqwest::Error>> + use<>, Error> {
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

        Ok(response.bytes_stream())
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
        response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| {
                Error::Storage(std::io::Error::other(
                    "the object store gave no object size",
                ))
            })
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
            .map_err(|_| {
                Error::Storage(std::io::Error::other("the object store is unreachable"))
            })?;

        let status = response.status();
        if !status.is_success() {
            // The store says why in the body, and an operator staring at a
            // failing push has nothing else to go on: a bucket that does not
            // exist, a key that is denied and a clock that has drifted are three
            // different afternoons.
            let detail = response.text().await.unwrap_or_default();

            return Err(Error::Storage(std::io::Error::other(format!(
                "the object store refused a write with {status}: {}",
                detail.trim()
            ))));
        }

        Ok(())
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

        self.expect_success(response, "copy").await?;

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

        self.expect_success(response, "write").await?;

        Ok(true)
    }

    pub(crate) async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, Error> {
        let action = GetObject::new(&self.bucket, Some(&self.credentials), key);
        let response = read_retrying(self.client.get(action.sign(self.lifetime))).await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = self.expect_success(response, "read").await?;

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

        self.expect_success(response, "delete").await?;

        Ok(existed)
    }

    // Every key under a prefix, following the continuation token to the end.
    // Stopping at the first page would report a repository holding a thousand
    // locks as holding a thousand and none of the rest, and a lock nobody can
    // see is a lock nobody respects.
    pub(crate) async fn keys(&self, prefix: &str) -> Result<Vec<String>, Error> {
        Ok(self
            .entries(prefix)
            .await?
            .into_iter()
            .map(|entry| entry.key)
            .collect())
    }

    // A listing that says whether it finished. Collection needs the difference:
    // concluding "no marker anywhere references this object" from a listing that
    // stopped halfway is how a sweep deletes bytes another repository still
    // holds. Everything else wants the strict form and gets `entries`.
    pub(crate) async fn listing(&self, prefix: &str) -> Listing {
        match self.entries(prefix).await {
            Ok(entries) => Listing {
                entries,
                complete: true,
            },
            Err(error) => {
                tracing::warn!(%error, prefix, "the listing could not be finished");
                Listing {
                    entries: Vec::new(),
                    complete: false,
                }
            }
        }
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
            let body = self
                .expect_success(response, "list")
                .await?
                .text()
                .await
                .map_err(|_| unreachable_store())?;

            let listing = ListObjectsV2::parse_response(&body).map_err(|error| {
                Error::Storage(std::io::Error::other(format!(
                    "the object store sent a listing this server could not read: {error}"
                )))
            })?;

            out.extend(listing.contents.into_iter().map(|object| Entry {
                key: object.key,
                last_modified: object.last_modified,
                size: object.size,
            }));

            match listing.next_continuation_token {
                Some(next) => token = Some(next),
                None => break,
            }
        }

        Ok(out)
    }

    pub(crate) async fn expect_success(
        &self,
        response: reqwest::Response,
        what: &str,
    ) -> Result<reqwest::Response, Error> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let detail = response.text().await.unwrap_or_default();

        Err(Error::Storage(std::io::Error::other(format!(
            "the object store refused a {what} with {status}: {}",
            detail.trim()
        ))))
    }
}
