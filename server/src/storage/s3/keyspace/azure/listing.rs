use serde::Deserialize;

use crate::error::Error;
use crate::storage::s3::keyspace::Entry;

#[derive(Deserialize)]
struct EnumerationResults {
    #[serde(rename = "Blobs", default)]
    blobs: Blobs,
    #[serde(rename = "NextMarker", default)]
    next_marker: Option<String>,
}

#[derive(Deserialize, Default)]
struct Blobs {
    #[serde(rename = "Blob", default)]
    blob: Vec<Blob>,
}

#[derive(Deserialize)]
struct Blob {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Properties")]
    properties: Properties,
}

#[derive(Deserialize)]
struct Properties {
    #[serde(rename = "Last-Modified")]
    last_modified: String,
    #[serde(rename = "Content-Length")]
    content_length: u64,
}

pub(crate) struct Page {
    pub(crate) entries: Vec<Entry>,
    pub(crate) next: Option<String>,
}

pub(crate) fn parse(body: &str) -> Result<Page, Error> {
    let results: EnumerationResults = quick_xml::de::from_str(body).map_err(|error| {
        Error::Storage(std::io::Error::other(format!(
            "the object store sent a listing this server could not read: {error}"
        )))
    })?;

    Ok(Page {
        entries: results
            .blobs
            .blob
            .into_iter()
            .map(|blob| Entry {
                key: blob.name,
                modified: http_date(&blob.properties.last_modified),
                size: blob.properties.content_length,
            })
            .collect(),
        next: results.next_marker.filter(|marker| !marker.is_empty()),
    })
}

fn http_date(value: &str) -> Option<time::OffsetDateTime> {
    let format = time::format_description::parse_borrowed::<1>(
        "[weekday repr:short], [day] [month repr:short] [year] [hour]:[minute]:[second] GMT",
    )
    .ok()?;

    time::PrimitiveDateTime::parse(value, &format)
        .ok()
        .map(time::PrimitiveDateTime::assume_utc)
}

#[cfg(test)]
mod tests;
