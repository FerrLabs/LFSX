use std::path::Path;

use rusty_s3::actions::{
    AbortMultipartUpload, CompleteMultipartUpload, CreateMultipartUpload, S3Action, UploadPart,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::keyspace::Keyspace;
use crate::error::Error;

// S3 caps one `PutObject` at 5 GiB. For a store built to hold packaged game
// assets and captured footage that is a ceiling reached rather than a
// theoretical one, and it used to be reached the worst way: the client spent an
// hour uploading and the store refused at the end, with an error from the bucket
// rather than from this server.
//
// Above it the object goes up in parts and the store assembles them, so the key
// appears whole or not at all. What is left is S3's own object limit rather than
// one this server invented.
pub(crate) const SINGLE_PUT_CEILING: u64 = 5 * 1024 * 1024 * 1024;

// S3's floor for every part except the last, and its cap on how many parts one
// upload may have. Between them they decide the part size.
const SMALLEST_PART: u64 = 5 * 1024 * 1024;
const MOST_PARTS: u64 = 10_000;
const PART: u64 = 64 * 1024 * 1024;

// Large enough that a big object does not become ten thousand round trips, and
// grown when it would have to be. A fixed part size is a second ceiling wearing
// a different number, which is the thing this module exists to remove.
fn part_size(length: u64) -> u64 {
    PART.max(length.div_ceil(MOST_PARTS)).max(SMALLEST_PART)
}

pub(crate) async fn put(
    keys: &Keyspace,
    key: &str,
    staged: &Path,
    length: u64,
) -> Result<(), Error> {
    put_in_parts(keys, key, staged, length, part_size(length)).await
}

async fn put_in_parts(
    keys: &Keyspace,
    key: &str,
    staged: &Path,
    length: u64,
    size: u64,
) -> Result<(), Error> {
    let upload = begin(keys, key).await?;

    tracing::info!(
        key,
        length,
        part_size = size,
        "an object over the single-request ceiling is going up in parts"
    );

    match parts(keys, key, &upload, staged, length, size).await {
        Ok(etags) => finish(keys, key, &upload, etags).await,
        Err(error) => {
            // The parts already sent are charged for until something removes
            // them, and nothing else will: an upload that was never completed
            // does not appear in a listing, so neither collection nor an
            // operator reading the bucket would ever find it.
            if let Err(abandoned) = abort(keys, key, &upload).await {
                tracing::warn!(
                    %abandoned,
                    key,
                    "an interrupted multipart upload could not be aborted, so its parts stay until \
                     a lifecycle rule removes them"
                );
            }

            Err(error)
        }
    }
}

async fn begin(keys: &Keyspace, key: &str) -> Result<String, Error> {
    let action = CreateMultipartUpload::new(keys.bucket(), Some(keys.credentials()), key);
    let url = action.sign(keys.lifetime());

    let response = keys
        .client()
        .post(url)
        .header(reqwest::header::CONTENT_LENGTH, 0)
        .send()
        .await
        .map_err(|_| unreachable())?;

    let body = keys.expect_success(response, "start a multipart upload").await?.text().await.map_err(|error| {
        Error::Storage(std::io::Error::other(format!(
            "the object store gave an unreadable answer when starting a multipart upload: {error}"
        )))
    })?;

    CreateMultipartUpload::parse_response(&body)
        .map(|parsed| parsed.upload_id().to_owned())
        .map_err(|error| {
            Error::Storage(std::io::Error::other(format!(
                "the object store named no upload id: {error}"
            )))
        })
}

// Each part is streamed out of the staging file rather than read into memory.
// An object here is measured in gigabytes and the whole storage layer is built
// on holding at most a few megabytes of one at a time, which a part size does
// not get to change.
async fn parts(
    keys: &Keyspace,
    key: &str,
    upload: &str,
    staged: &Path,
    length: u64,
    size: u64,
) -> Result<Vec<String>, Error> {
    let count = u16::try_from(length.div_ceil(size)).map_err(|_| {
        Error::Storage(std::io::Error::other(
            "this object needs more parts than one upload may have",
        ))
    })?;

    let mut etags = Vec::with_capacity(count.into());

    for index in 0..count {
        let offset = u64::from(index) * size;
        let this = size.min(length - offset);

        let mut file = tokio::fs::File::open(staged).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let stream = tokio_util::io::ReaderStream::new(file.take(this));

        let action = UploadPart::new(
            keys.bucket(),
            Some(keys.credentials()),
            key,
            // S3 numbers parts from one.
            index + 1,
            upload,
        );

        let response = keys
            .client()
            .put(action.sign(keys.lifetime()))
            .header(reqwest::header::CONTENT_LENGTH, this)
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await
            .map_err(|_| unreachable())?;

        let response = keys.expect_success(response, "write a part").await?;

        // The completion names every part by the tag the store gave it, and a
        // store that sent none has given nothing to assemble the object from.
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                Error::Storage(std::io::Error::other("the object store tagged no part"))
            })?;

        etags.push(etag.to_owned());
    }

    Ok(etags)
}

async fn finish(keys: &Keyspace, key: &str, upload: &str, etags: Vec<String>) -> Result<(), Error> {
    let action = CompleteMultipartUpload::new(
        keys.bucket(),
        Some(keys.credentials()),
        key,
        upload,
        etags.iter().map(String::as_str),
    );
    let url = action.sign(keys.lifetime());
    let body = action.body();

    let response = keys
        .client()
        .post(url)
        .header(reqwest::header::CONTENT_LENGTH, body.len())
        .body(body)
        .send()
        .await
        .map_err(|_| unreachable())?;

    keys.expect_success(response, "assemble a multipart upload")
        .await?;

    Ok(())
}

async fn abort(keys: &Keyspace, key: &str, upload: &str) -> Result<(), Error> {
    let action = AbortMultipartUpload::new(keys.bucket(), Some(keys.credentials()), key, upload);

    let response = keys
        .client()
        .delete(action.sign(keys.lifetime()))
        .send()
        .await
        .map_err(|_| unreachable())?;

    keys.expect_success(response, "abort a multipart upload")
        .await?;

    Ok(())
}

fn unreachable() -> Error {
    Error::Storage(std::io::Error::other("the object store is unreachable"))
}

#[cfg(test)]
mod tests;
