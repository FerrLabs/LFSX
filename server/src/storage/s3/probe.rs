use base64::Engine;

use super::CHECKSUM;
use super::keyspace::Keyspace;
use crate::error::Error;

// Does this store actually refuse a body that does not match the checksum its
// upload URL was signed for?
//
// Everything a pre-signed upload rests on is that one behaviour. The URL carries
// `x-amz-checksum-sha256` inside the signature, so a client cannot change it, and
// a conforming store rejects a body that does not hash to it. That is the only
// thing standing between a client with push rights and arbitrary bytes under a
// digest of its choosing.
//
// The cost of being wrong is not one bad object. Bytes live once at
// `.content/{oid}`, and every repository that later pushes that digest gets a
// marker pointing at them without uploading anything, because the content is
// already there. So a store that shrugs at the header lets whoever pushes first
// decide what an object is, for everybody, and the marker is the only thing the
// read path consults.
//
// `x-amz-checksum-*` is a late addition to the S3 API, and accepting a header
// while ignoring it is exactly the shape of an incomplete implementation. So it
// is asked rather than assumed.

const KEY: &str = ".probe/checksum";
const BODY: &[u8] = b"lfsx checksum probe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Checksums {
    Enforced,
    Ignored,
    // The store could not be asked. Treated as the store cannot be trusted with
    // pre-signing, because the alternative is trusting it on the strength of a
    // question that was never answered.
    Unknown,
}

// A digest that is not this body's, so a store that checks has to refuse the
// write. Thirty-two zero bytes: well formed, and not the SHA-256 of anything
// anybody holds.
fn wrong_digest() -> String {
    base64::engine::general_purpose::STANDARD.encode([0u8; 32])
}

pub(crate) async fn checksums(keys: &Keyspace) -> Checksums {
    let signed = keys.signed_upload(KEY, vec![(CHECKSUM.to_owned(), wrong_digest())]);

    let mut request = keys.client().put(&signed.href).body(BODY.to_vec());
    for (name, value) in &signed.headers {
        request = request.header(name, value);
    }

    if let Err(error) = request.send().await {
        tracing::warn!(%error, "the object store could not be reached to check it verifies uploads");
        return Checksums::Unknown;
    }

    // What the store answered is not the question, and stores disagree about
    // which status says no: a mismatch is a 400 in one and a 403 in another.
    // Whether the bytes landed is the question, because that is precisely the
    // property a pre-signed upload depends on.
    match keys.head(KEY).await {
        Err(Error::NotFound) => Checksums::Enforced,
        Err(error) => {
            tracing::warn!(%error, "the object store could not say whether it kept the probe");
            Checksums::Unknown
        }
        Ok(_) => {
            // They landed. This store took a body that does not hash to the
            // digest its own signature named, so nothing stops a client doing the
            // same with a digest somebody else's repository will later claim.
            if let Err(error) = keys.delete(KEY).await {
                tracing::warn!(%error, key = KEY, "the probe object could not be cleaned up");
            }

            Checksums::Ignored
        }
    }
}

#[cfg(test)]
mod tests;
