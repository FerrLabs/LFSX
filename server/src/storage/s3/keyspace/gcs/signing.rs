use std::time::Duration;

use base64::Engine;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use sha2::{Digest, Sha256};

use super::credential::ServiceAccount;

const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
const PATH: &AsciiSet = &UNRESERVED.remove(b'/');

pub(crate) struct Canonical {
    pub(crate) request: String,
    datetime: String,
    scope: String,
    url: String,
}

impl Canonical {
    pub(crate) fn to_sign(&self) -> String {
        [
            "GOOG4-RSA-SHA256",
            &self.datetime,
            &self.scope,
            &hex::encode(Sha256::digest(self.request.as_bytes())),
        ]
        .join("\n")
    }
}

fn stamp(at: time::OffsetDateTime, pattern: &str) -> String {
    time::format_description::parse_borrowed::<1>(pattern)
        .ok()
        .and_then(|format| at.format(&format).ok())
        .unwrap_or_default()
}

pub(crate) fn canonical(
    endpoint: &reqwest::Url,
    bucket: &str,
    object: &str,
    email: &str,
    at: time::OffsetDateTime,
    expires: Duration,
) -> Canonical {
    let date = stamp(at, "[year][month][day]");
    let datetime = stamp(at, "[year][month][day]T[hour][minute][second]Z");
    let scope = format!("{date}/auto/storage/goog4_request");

    let host = match endpoint.port() {
        Some(port) => format!("{}:{port}", endpoint.host_str().unwrap_or_default()),
        None => endpoint.host_str().unwrap_or_default().to_owned(),
    };
    let path = format!("/{bucket}/{}", utf8_percent_encode(object, PATH));

    let query = [
        ("X-Goog-Algorithm", "GOOG4-RSA-SHA256".to_owned()),
        ("X-Goog-Credential", format!("{email}/{scope}")),
        ("X-Goog-Date", datetime.clone()),
        ("X-Goog-Expires", expires.as_secs().to_string()),
        ("X-Goog-SignedHeaders", "host".to_owned()),
    ]
    .iter()
    .map(|(name, value)| format!("{name}={}", utf8_percent_encode(value, UNRESERVED)))
    .collect::<Vec<_>>()
    .join("&");

    let request = [
        "GET",
        &path,
        &query,
        &format!("host:{host}\n"),
        "host",
        "UNSIGNED-PAYLOAD",
    ]
    .join("\n");

    Canonical {
        request,
        datetime,
        scope,
        url: format!("{}://{host}{path}?{query}", endpoint.scheme()),
    }
}

pub(crate) fn signed_download(
    endpoint: &reqwest::Url,
    bucket: &str,
    object: &str,
    account: &ServiceAccount,
    lifetime: Duration,
) -> Option<String> {
    signed(
        &canonical(
            endpoint,
            bucket,
            object,
            &account.email,
            time::OffsetDateTime::now_utc(),
            lifetime,
        ),
        account,
    )
}

fn signed(canonical: &Canonical, account: &ServiceAccount) -> Option<String> {
    let signature = jsonwebtoken::crypto::sign(
        canonical.to_sign().as_bytes(),
        &account.key,
        jsonwebtoken::Algorithm::RS256,
    )
    .ok()?;
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(signature)
        .ok()?;

    Some(format!(
        "{}&X-Goog-Signature={}",
        canonical.url,
        hex::encode(signature)
    ))
}

#[cfg(test)]
mod tests;
