use std::time::Duration;

use axum::body::Bytes;
use futures_util::Stream;
use rusty_s3::actions::{DeleteObject, GetObject, HeadObject, ListObjectsV2, PutObject, S3Action};
use rusty_s3::{Bucket, Credentials, UrlStyle};

use base64::Engine;

use crate::error::Error;
use crate::namespace::Namespace;
use crate::storage::Reclaimed;

const CHECKSUM: &str = "x-amz-checksum-sha256";
const COPY_SOURCE: &str = "x-amz-copy-source";

// One key as the store describes it.
struct Entry {
    key: String,
    last_modified: String,
    size: u64,
}

impl Entry {
    // None when the store's timestamp cannot be read, which is treated as "too
    // young to touch": deleting somebody's upload on the strength of a date this
    // server could not parse is the wrong way to be wrong.
    fn age(&self) -> Option<Duration> {
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
// an empty marker beside it — the object store's answer to a hard link. It is
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

#[derive(Clone)]
pub struct S3Store {
    bucket: Bucket,
    credentials: Credentials,
    client: reqwest::Client,
    lifetime: Duration,
    redirect: bool,
}

pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub path_style: bool,
    pub redirect: bool,
    // How long a signature is good for. It is the same number the batch
    // response advertises as `expires_in`, because a client told it has half an
    // hour and handed a URL that dies in five minutes will fail a resume it had
    // every reason to expect to work.
    pub lifetime: Duration,
}

impl S3Store {
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
            redirect: config.redirect,
        })
    }

    fn content_key(oid: &str) -> String {
        format!(".content/{}/{}/{oid}", &oid[0..2], &oid[2..4])
    }

    // Where a client uploads to when the bytes never pass through this server.
    // Per repository on purpose: the shared content key would take bytes from
    // anyone allowed to write, and then nothing distinguishes a repository that
    // uploaded an object from one that merely knew its digest. A key only this
    // repository was handed a signature for is the proof of possession that the
    // marker stands for everywhere else.
    fn incoming_key(ns: &Namespace, oid: &str) -> String {
        format!(
            ".incoming/{}/{}/{}/{}/{oid}",
            ns.org(),
            ns.repo(),
            &oid[0..2],
            &oid[2..4]
        )
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

    // Signed as a HEAD rather than reusing a GET signature: SigV4 covers the
    // method, and an implementation that checks it — which is the point of
    // testing against MinIO and Garage rather than only AWS — is entitled to
    // refuse the mismatch.
    async fn head(&self, key: &str) -> Result<u64, Error> {
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

    pub async fn size_of(&self, oid: &str) -> Result<u64, Error> {
        // Every entry point validates before slicing an oid into a key: the
        // fanout takes the first four characters, so a short one is a panic
        // rather than a refusal, and a panic is a 500 for something that should
        // have been a 422.
        crate::storage::LocalStore::validate_oid(oid)?;

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
        crate::storage::LocalStore::validate_oid(oid)?;

        let key = Self::content_key(oid);
        let action = GetObject::new(&self.bucket, Some(&self.credentials), &key);
        let url = action.sign(self.lifetime);

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

    // A URL the client fetches from the bucket directly, so the bytes never
    // cross this server. Whether the caller is entitled to them has already been
    // settled by the marker before this is called: the signature is scoped to
    // one content key and expires, and it grants nothing the batch response was
    // not about to grant anyway.
    pub fn presigned_download(&self, oid: &str) -> Option<String> {
        if !self.redirect || crate::storage::LocalStore::validate_oid(oid).is_err() {
            return None;
        }

        let key = Self::content_key(oid);

        Some(
            GetObject::new(&self.bucket, Some(&self.credentials), &key)
                .sign(self.lifetime)
                .to_string(),
        )
    }

    // A URL the client PUTs the object to, and the headers it has to send with
    // it. The digest is bound into the signature, so the store refuses anything
    // that does not hash to the object it was signed for: a client with this URL
    // cannot put arbitrary bytes anywhere, which is what makes handing one out
    // safe at all.
    pub fn presigned_upload(&self, ns: &Namespace, oid: &str) -> Option<Presigned> {
        if !self.redirect || crate::storage::LocalStore::validate_oid(oid).is_err() {
            return None;
        }

        let digest = base64::engine::general_purpose::STANDARD.encode(hex::decode(oid).ok()?);
        let key = Self::incoming_key(ns, oid);
        let mut action = PutObject::new(&self.bucket, Some(&self.credentials), &key);
        action
            .headers_mut()
            .insert(CHECKSUM, std::borrow::Cow::Owned(digest.clone()));

        Some(Presigned {
            href: action.sign(self.lifetime).to_string(),
            headers: vec![(CHECKSUM.to_owned(), digest)],
        })
    }

    // How big the object a client uploaded actually is, which is the first thing
    // this server learns about it: nothing measured the bytes on the way past.
    pub async fn uploaded_size(&self, ns: &Namespace, oid: &str) -> Result<u64, Error> {
        crate::storage::LocalStore::validate_oid(oid)?;

        self.head(&Self::incoming_key(ns, oid)).await
    }

    // Take an upload that landed under this repository's own key into the shared
    // keyspace. The bytes are already known to hash to the oid, because the store
    // refused everything else.
    pub async fn adopt(&self, ns: &Namespace, oid: &str) -> Result<(), Error> {
        crate::storage::LocalStore::validate_oid(oid)?;

        let incoming = Self::incoming_key(ns, oid);
        let content = Self::content_key(oid);

        // Already there means another repository pushed the same object, and the
        // bytes are identical by construction.
        if self.head(&content).await.is_err() {
            self.copy(&incoming, &content).await?;
        }

        self.put(
            &Self::marker_key(ns, oid),
            reqwest::Body::from(Vec::new()),
            0,
        )
        .await?;

        // Leaving it would pay for the object twice. A failure here is not worth
        // failing the push over: the object is adopted, and what is left is a key
        // the operator can see.
        if let Err(error) = self.delete(&incoming).await {
            tracing::warn!(%error, key = incoming, "an adopted upload could not be cleaned up");
        }

        Ok(())
    }

    // A copy is a PUT to the destination carrying `x-amz-copy-source`, so this is
    // a signed PutObject with that header bound rather than a separate action.
    // The bytes move inside the store: nothing crosses this server.
    async fn copy(&self, from: &str, to: &str) -> Result<(), Error> {
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

    async fn put(&self, key: &str, body: reqwest::Body, length: u64) -> Result<(), Error> {
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
        crate::storage::LocalStore::validate_oid(oid)?;

        if self.head(&Self::content_key(oid)).await.is_err() {
            let file = tokio::fs::File::open(staged).await?;
            let length = file.metadata().await?.len();
            let stream = tokio_util::io::ReaderStream::new(file);

            self.put(
                &Self::content_key(oid),
                reqwest::Body::wrap_stream(stream),
                length,
            )
            .await?;
        }

        self.put(
            &Self::marker_key(ns, oid),
            reqwest::Body::from(Vec::new()),
            0,
        )
        .await
    }

    // Everything below is the bucket as a keyspace rather than as an object
    // store: whole small values, written, read, deleted and listed by key. The
    // lock store is built on it, and it is kept here so the signing and the
    // client stay in one place.

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

    // What an interrupted upload leaves behind. A client can negotiate, PUT the
    // object, and never report it: the bytes sit under its own upload key and
    // nothing else will ever look at them. The local path has had a reclaimer for
    // this since the beginning, and a bucket had none, so the cost was unbounded
    // over time and invisible.
    pub async fn reclaim_incoming(&self, older_than: Duration) -> Result<Reclaimed, Error> {
        let mut reclaimed = Reclaimed::default();

        for entry in self.entries(".incoming/").await? {
            // A slow client on a bad connection is not an abandoned one.
            if entry.age().is_none_or(|age| age < older_than) {
                continue;
            }

            if self.delete(&entry.key).await.is_ok() {
                reclaimed.files += 1;
                reclaimed.bytes += entry.size;
            }
        }

        Ok(reclaimed)
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

    async fn entries(&self, prefix: &str) -> Result<Vec<Entry>, Error> {
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

    async fn expect_success(
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
        let response = match self.client.get(action.sign(self.lifetime)).send().await {
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
pub(crate) mod tests;
