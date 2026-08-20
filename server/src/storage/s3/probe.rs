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

const BODY: &[u8] = b"lfsx probe";

// A key nothing else will ever use, drawn fresh for each probe.
//
// A fixed one is a trap in both directions. A leftover, from a run that died or
// from a store that has since been fixed, is found by the `HEAD` below and read
// as this run's own write: a compliant store then reports as one that keeps
// whatever it is sent, permanently and with nothing to say why. And two replicas
// booting against the same bucket at the same moment tread on each other's key,
// which is the deployment a bucket exists to make possible.
//
// What this leaves behind instead is swept with everything else under `.probe/`,
// on the same schedule as an abandoned upload.
fn probe_key(what: &str) -> String {
    let mut suffix = [0u8; 8];
    getrandom::fill(&mut suffix).expect("the operating system has a random number generator");

    format!(".probe/{what}-{}", hex::encode(suffix))
}

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
    let key = probe_key("checksum");
    let signed = keys.signed_upload(&key, vec![(CHECKSUM.to_owned(), wrong_digest())]);

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
    match keys.head(&key).await {
        // Nothing is under a key nothing else has written, so the store refused
        // it, and there is nothing to clean up.
        Err(Error::NotFound) => Checksums::Enforced,
        Err(error) => {
            tracing::warn!(%error, "the object store could not say whether it kept the probe");
            discard(keys, &key).await;
            Checksums::Unknown
        }
        Ok(_) => {
            // They landed. This store took a body that does not hash to the
            // digest its own signature named, so nothing stops a client doing the
            // same with a digest somebody else's repository will later claim.
            discard(keys, &key).await;
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
    let key = probe_key("conditional");

    match keys.put_if_absent(&key, BODY.to_vec()).await {
        Ok(true) => {}
        // Refused on a key nothing has ever written. Whatever that is, it is not
        // the answer this asked for.
        Ok(false) => {
            tracing::warn!(
                key,
                "the object store refused the first write to a key it had never seen, so whether \
                 it refuses a conditional write was not established"
            );
            return Conditional::Unknown;
        }
        Err(error) => {
            tracing::warn!(%error, "the object store could not be reached to check it refuses a conditional write");
            return Conditional::Unknown;
        }
    }

    let verdict = match keys.put_if_absent(&key, BODY.to_vec()).await {
        // The key is already there and the store said so, which is the whole
        // contract a lock is built on.
        Ok(false) => Conditional::Enforced,
        Ok(true) => Conditional::Ignored,
        Err(error) => {
            tracing::warn!(%error, "the object store gave no usable answer to a conditional write");
            Conditional::Unknown
        }
    };

    discard(keys, &key).await;

    verdict
}

async fn discard(keys: &Keyspace, key: &str) {
    if let Err(error) = keys.delete(key).await {
        tracing::warn!(
            %error,
            key,
            "the probe object could not be cleaned up, and is left for the reclaimer"
        );
    }
}

#[cfg(test)]
mod tests;
