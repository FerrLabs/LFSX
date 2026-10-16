use std::time::Duration;

use crate::error::Error;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const KEEPALIVE: Duration = Duration::from_secs(30);

pub(super) fn bucket_client() -> Result<reqwest::Client, Error> {
    reads(READ_TIMEOUT)
}

fn reads(read: Duration) -> Result<reqwest::Client, Error> {
    build(reqwest::Client::builder().read_timeout(read))
}

pub(super) fn upload_client() -> Result<reqwest::Client, Error> {
    build(reqwest::Client::builder().tcp_keepalive(KEEPALIVE))
}

pub(super) fn for_body<'a>(
    body: &reqwest::Body,
    reads: &'a reqwest::Client,
    uploads: &'a reqwest::Client,
) -> &'a reqwest::Client {
    if body.as_bytes().is_some() {
        reads
    } else {
        uploads
    }
}

fn build(builder: reqwest::ClientBuilder) -> Result<reqwest::Client, Error> {
    builder
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|error| Error::Storage(std::io::Error::other(error)))
}

#[cfg(test)]
mod tests;
