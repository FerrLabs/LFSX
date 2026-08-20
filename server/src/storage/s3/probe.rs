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
const CONDITIONAL_KEY: &str = ".probe/conditional";
const BODY: &[u8] = b"lfsx probe";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Conditional {
    Enforced,
    Ignored,
    Unknown,
}

// Does this store actually refuse the second of two conditional writes?
//
// `If-None-Match: *` is the whole of lock uniqueness in a bucket. Two clients
// race for the same path, both PUT, and the store is the only thing that decides
// one of them arrived second. A store that accepts the header without
// implementing the condition performs both writes and answers success twice, so
// both callers are told they hold the lock. Nothing detects it and nothing logs
// it: the feature quietly becomes advisory.
//
// Which is precisely the failure locking exists to prevent. Two artists are told
// the scene is theirs, both edit it, and whoever pushes second loses the work.
//
// The local backend needs none of this. `create_new` is a filesystem primitive
// and it either creates the file or it does not.
pub(crate) async fn conditional_writes(keys: &Keyspace) -> Conditional {
    // A key left behind by a run that died before cleaning up would make the
    // first write below the second one, and a refusal then would look like
    // enforcement that was never actually tested.
    let _ = keys.delete(CONDITIONAL_KEY).await;

    match keys.put_if_absent(CONDITIONAL_KEY, BODY.to_vec()).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(
                key = CONDITIONAL_KEY,
                "the probe key could not be cleared, so whether this store refuses a conditional \
                 write was not established"
            );
            return Conditional::Unknown;
        }
        Err(error) => {
            tracing::warn!(%error, "the object store could not be reached to check it refuses a conditional write");
            return Conditional::Unknown;
        }
    }

    let verdict = match keys.put_if_absent(CONDITIONAL_KEY, BODY.to_vec()).await {
        // The key is already there and the store said so, which is the whole
        // contract a lock is built on.
        Ok(false) => Conditional::Enforced,
        Ok(true) => Conditional::Ignored,
        Err(error) => {
            tracing::warn!(%error, "the object store gave no usable answer to a conditional write");
            Conditional::Unknown
        }
    };

    if let Err(error) = keys.delete(CONDITIONAL_KEY).await {
        tracing::warn!(%error, key = CONDITIONAL_KEY, "the probe object could not be cleaned up");
    }

    verdict
}

#[cfg(test)]
mod tests;
