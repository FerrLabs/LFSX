mod s3;

use std::path::Path;
use std::time::Duration;

use axum::body::Bytes;
use futures_util::Stream;

use crate::error::Error;

pub(crate) use s3::S3Keys;

pub(crate) struct Listing {
    pub(crate) entries: Vec<Entry>,
    pub(crate) complete: bool,
}

pub(crate) struct Entry {
    pub(crate) key: String,
    pub(crate) modified: Option<time::OffsetDateTime>,
    pub(crate) size: u64,
}

impl Entry {
    // None when the store's timestamp cannot be read, which is treated as "too
    // young to touch": deleting somebody's upload on the strength of a date this
    // server could not parse is the wrong way to be wrong.
    pub(crate) fn age(&self) -> Option<Duration> {
        Duration::try_from(time::OffsetDateTime::now_utc() - self.modified?).ok()
    }
}

// An href a client uses directly, and the headers it has to send with it. The
// headers are part of the signature, so they are not advice.
pub struct Presigned {
    pub href: String,
    pub headers: Vec<(String, String)>,
}

// The bucket as a keyspace: whole values written, read, deleted and listed by
// key, with the signing and the HTTP client in one place. It knows nothing about
// objects, oids or repositories: what a key means is decided a layer up, which
// is what lets the lock store share the bucket with the object store without
// either of them reaching into the other.
#[derive(Clone)]
pub enum Keyspace {
    S3(S3Keys),
}

impl Keyspace {
    pub(crate) async fn reachable(&self) -> Result<(), Error> {
        match self {
            Self::S3(keys) => keys.reachable().await,
        }
    }

    pub(crate) fn signed_download(&self, key: &str) -> Option<String> {
        match self {
            Self::S3(keys) => Some(keys.signed_download(key)),
        }
    }

    pub(crate) fn signed_upload(&self, key: &str, digest: &str) -> Option<Presigned> {
        match self {
            Self::S3(keys) => Some(keys.signed_upload(key, digest)),
        }
    }

    pub(crate) async fn get_range(
        &self,
        key: &str,
        start: u64,
        length: u64,
    ) -> Result<impl Stream<Item = Result<Bytes, reqwest::Error>> + use<>, Error> {
        let response = match self {
            Self::S3(keys) => keys.get_range(key, start, length).await?,
        };

        Ok(response.bytes_stream())
    }

    pub(crate) async fn head(&self, key: &str) -> Result<u64, Error> {
        match self {
            Self::S3(keys) => keys.head(key).await,
        }
    }

    pub(crate) async fn put(&self, key: &str, body: Vec<u8>) -> Result<(), Error> {
        let length = body.len() as u64;

        match self {
            Self::S3(keys) => keys.put(key, reqwest::Body::from(body), length).await,
        }
    }

    pub(crate) async fn put_file(&self, key: &str, staged: &Path) -> Result<(), Error> {
        match self {
            Self::S3(keys) => keys.put_file(key, staged).await,
        }
    }

    pub(crate) async fn copy(&self, from: &str, to: &str) -> Result<(), Error> {
        match self {
            Self::S3(keys) => keys.copy(from, to).await,
        }
    }

    pub(crate) async fn put_if_absent(&self, key: &str, body: Vec<u8>) -> Result<bool, Error> {
        match self {
            Self::S3(keys) => keys.put_if_absent(key, body).await,
        }
    }

    pub(crate) async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, Error> {
        match self {
            Self::S3(keys) => keys.get_bytes(key).await,
        }
    }

    pub(crate) async fn delete(&self, key: &str) -> Result<bool, Error> {
        match self {
            Self::S3(keys) => keys.delete(key).await,
        }
    }

    pub(crate) async fn entries(&self, prefix: &str) -> Result<Vec<Entry>, Error> {
        match self {
            Self::S3(keys) => keys.entries(prefix).await,
        }
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
}

// A conditional write that is refused makes the store answer and hang up, and
// the connection goes back into the pool looking usable. The next request on it
// fails at the transport layer with nothing to do with the store's health, which
// is how a losing `git lfs lock` came back as a 500 instead of a 409.
//
// Retried once, and only for requests that carry no body: a GET and a HEAD can be
// repeated with no consequence, so a dead connection costs a round trip rather
// than an error. A PUT is not retried here.
pub(crate) async fn read_retrying(
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, Error> {
    let retry = request.try_clone();

    match request.send().await {
        Ok(response) => Ok(response),
        Err(_) => match retry {
            Some(retry) => retry.send().await.map_err(|_| unreachable_store()),
            None => Err(unreachable_store()),
        },
    }
}

pub(crate) fn unreachable_store() -> Error {
    Error::Storage(std::io::Error::other("the object store is unreachable"))
}

pub(crate) async fn expect_success(
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

pub(crate) fn content_length(response: &reqwest::Response) -> Result<u64, Error> {
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
