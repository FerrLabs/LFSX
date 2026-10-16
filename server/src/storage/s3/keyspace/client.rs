use std::time::Duration;

use crate::error::Error;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) fn bucket_client() -> Result<reqwest::Client, Error> {
    build(CONNECT_TIMEOUT, READ_TIMEOUT)
}

fn build(connect: Duration, read: Duration) -> Result<reqwest::Client, Error> {
    reqwest::Client::builder()
        .connect_timeout(connect)
        .read_timeout(read)
        .build()
        .map_err(|error| Error::Storage(std::io::Error::other(error)))
}

#[cfg(test)]
mod tests;
