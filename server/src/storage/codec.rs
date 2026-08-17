use std::io::SeekFrom;

use axum::body::Bytes;
use futures_util::Stream;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::crypt::{self, Keyring, ObjectKey};
use crate::error::Error;

// An object compressed or encrypted at rest is still named after the digest of
// its plaintext, because that name is what every other part of the server
// addresses it by — collection, deduplication, the shared store, the client's
// own verification. So the file says what it is in its first bytes instead of in
// its name, and a store from before either feature reads back unchanged: no
// magic, no header, just the object.
const MAGIC: &[u8; 4] = b"LFZ1";

// Version 1 frames are plaintext, compressed or stored. Version 2 frames are
// sealed, and the header carries what a reader needs to rebuild the key. Both
// are read; only the second is ever written once a key is configured. That is
// what makes turning encryption on something other than a flag day: what is
// already on disk keeps being served while it is still there.
const V_PLAIN: u8 = 1;
const V_SEALED: u8 = 2;

const HEADER: u64 = 32;
const SEALED_HEADER: u64 = 64;

fn header_len(version: u8) -> u64 {
    match version {
        V_SEALED => SEALED_HEADER,
        _ => HEADER,
    }
}

// Top bit of an index entry: this frame is not compressed. A frame is at most
// sixteen megabytes, so the bit is free.
const STORED: u32 = 1 << 31;

// Plaintext per frame. Each is compressed and sealed on its own, so serving a
// range means touching the frames it covers rather than everything before it —
// which is what keeps resuming a three-gigabyte download from costing three
// gigabytes of work. Larger frames compress better and seek worse; four
// megabytes is about where a home upstream stops noticing either.
pub const FRAME: u64 = 4 * 1024 * 1024;

// Where a framed object is read from. Everything the format needs is three
// positional reads before the first frame and one per frame after, which a file
// answers with a seek and a bucket answers with a ranged GET. That is the whole
// reason the format was built with a header and an index rather than a stream.
pub enum Reader {
    File(fs::File),
    Bucket {
        bucket: super::s3::S3Store,
        oid: String,
    },
}

impl Reader {
    async fn read_at(&mut self, at: u64, length: u64) -> Result<Vec<u8>, Error> {
        match self {
            Self::File(file) => {
                file.seek(SeekFrom::Start(at)).await?;
                let mut out = vec![0u8; length as usize];
                file.read_exact(&mut out).await?;

                Ok(out)
            }
            Self::Bucket { bucket, oid } => {
                use futures_util::StreamExt;

                let mut chunks = Box::pin(bucket.read(oid, at, length).await?);
                let mut out = Vec::with_capacity(length as usize);
                while let Some(chunk) = chunks.next().await {
                    let chunk =
                        chunk.map_err(|error| Error::Storage(std::io::Error::other(error)))?;
                    out.extend_from_slice(&chunk);
                }

                Ok(out)
            }
        }
    }
}

pub struct Framed {
    reader: Reader,
    plaintext: u64,
    frame: u64,
    frames: u32,
    offsets: Vec<u64>,
    stored: Vec<bool>,
    key: Option<ObjectKey>,
    oid: String,
}

pub struct Writer {
    file: fs::File,
    level: Option<i32>,
    key: Option<ObjectKey>,
    oid: String,
    lengths: Vec<u32>,
    plaintext: u64,
    pending: Vec<u8>,
}

impl Writer {
    // `level` and `key` are independent: an operator can compress, encrypt, or
    // both. Compression runs first, because sealed bytes are indistinguishable
    // from random and give up no ground at any level.
    pub async fn open(
        mut file: fs::File,
        level: Option<i32>,
        key: Option<ObjectKey>,
        oid: &str,
    ) -> Result<Self, Error> {
        let version = if key.is_some() { V_SEALED } else { V_PLAIN };
        file.write_all(&vec![0u8; header_len(version) as usize])
            .await?;

        Ok(Self {
            file,
            level,
            key,
            oid: oid.to_owned(),
            lengths: Vec::new(),
            plaintext: 0,
            pending: Vec::with_capacity(FRAME as usize),
        })
    }

    pub async fn push(&mut self, chunk: &[u8]) -> Result<(), Error> {
        self.plaintext += chunk.len() as u64;
        self.pending.extend_from_slice(chunk);

        // A full frame is only written once something follows it, because
        // whether a frame is the last one is part of what seals it and that is
        // not known until the stream ends.
        while self.pending.len() > FRAME as usize {
            let rest = self.pending.split_off(FRAME as usize);
            let frame = std::mem::replace(&mut self.pending, rest);
            self.flush(&frame, false).await?;
        }

        Ok(())
    }

    pub async fn finish(mut self) -> Result<(), Error> {
        let frame = std::mem::take(&mut self.pending);
        if !frame.is_empty() {
            self.flush(&frame, true).await?;
        }

        for length in &self.lengths {
            self.file.write_all(&length.to_le_bytes()).await?;
        }

        let header = header_len(self.version());
        let index = header
            + self
                .lengths
                .iter()
                .map(|length| (*length & !STORED) as u64)
                .sum::<u64>();
        self.file.seek(SeekFrom::Start(0)).await?;
        self.file.write_all(&self.header(index)).await?;
        self.file.flush().await?;
        self.file.sync_all().await?;

        Ok(())
    }

    fn version(&self) -> u8 {
        if self.key.is_some() {
            V_SEALED
        } else {
            V_PLAIN
        }
    }

    fn header(&self, index: u64) -> Vec<u8> {
        let version = self.version();
        let mut header = vec![0u8; header_len(version) as usize];
        header[0..4].copy_from_slice(MAGIC);
        header[4] = version;
        header[8..16].copy_from_slice(&self.plaintext.to_le_bytes());
        header[16..20].copy_from_slice(&(FRAME as u32).to_le_bytes());
        header[20..24].copy_from_slice(&(self.lengths.len() as u32).to_le_bytes());
        header[24..32].copy_from_slice(&index.to_le_bytes());

        if let Some(key) = &self.key {
            header[32..36].copy_from_slice(&key.id());
            header[36..52].copy_from_slice(&key.salt());
        }

        header
    }

    // Half an LFS store is PNG, MP3 and other formats that are already
    // compressed, and spending CPU to make those frames marginally larger is
    // worse than not trying. A frame that does not give up ground is stored as
    // it arrived, flagged in the index, and read straight back.
    async fn flush(&mut self, frame: &[u8], last: bool) -> Result<(), Error> {
        let (body, stored) = match self.level {
            Some(level) => {
                let compressed =
                    zstd::bulk::compress(frame, level).map_err(std::io::Error::other)?;

                if compressed.len() >= frame.len() - frame.len() / 20 {
                    (frame.to_vec(), true)
                } else {
                    (compressed, false)
                }
            }
            None => (frame.to_vec(), true),
        };

        let index = self.lengths.len() as u32;
        let body = match &self.key {
            Some(key) => key.seal(index, last, &self.oid, &body)?,
            None => body,
        };

        self.file.write_all(&body).await?;
        self.lengths
            .push(body.len() as u32 | if stored { STORED } else { 0 });

        Ok(())
    }
}

// The header is written by this server and read back from a file whose contents
// a client chose: an object is whatever bytes hash to the digest it was pushed
// under, so anyone with push rights can store a file that opens with a header of
// their own making. Every field is therefore treated as hostile.
//
// The frame size is the dangerous one, because it is the size of the buffer each
// frame is decompressed into: unbounded, it is a request that asks the server to
// allocate four gigabytes, repeatable on every download of that object. Bounding
// it rather than pinning it to today's value keeps the frame size a thing this
// format can change without orphaning what is already stored.
const FRAME_MIN: u64 = 64 * 1024;
const FRAME_MAX: u64 = 16 * 1024 * 1024;

fn plausible(plaintext: u64, frame: u64, frames: u64) -> bool {
    if !(FRAME_MIN..=FRAME_MAX).contains(&frame) {
        return false;
    }

    // Exactly the frames the plaintext needs — no more, so a header cannot claim
    // an object far larger than the file that carries it, and no fewer.
    frames == plaintext.div_ceil(frame)
}

impl Framed {
    // Sniffing the header is what makes a raw store, a compressed one and an
    // encrypted one the same store. A file that only looks like a header is
    // rejected on its own arithmetic: the frame size has to be one this format
    // uses, the frame count has to be the one the plaintext needs, the index has
    // to sit inside the file, and the frames have to account for exactly the
    // bytes between them.
    pub async fn open(
        mut reader: Reader,
        on_disk: u64,
        keys: Option<&Keyring>,
        oid: &str,
    ) -> Result<Option<Self>, Error> {
        if on_disk < header_len(V_PLAIN) {
            return Ok(None);
        }

        // The widest header this format has, in one read: a bucket charges a
        // round trip for each, and sniffing must not cost two.
        let prefix = reader
            .read_at(0, SEALED_HEADER.min(on_disk))
            .await
            .unwrap_or_default();

        if prefix.len() < HEADER as usize
            || &prefix[0..4] != MAGIC
            || !matches!(prefix[4], V_PLAIN | V_SEALED)
        {
            return Ok(None);
        }

        let version = prefix[4];
        let header = header_len(version);
        let plaintext = u64::from_le_bytes(prefix[8..16].try_into().expect("eight bytes"));
        let frame = u32::from_le_bytes(prefix[16..20].try_into().expect("four bytes")) as u64;
        let frames = u32::from_le_bytes(prefix[20..24].try_into().expect("four bytes")) as u64;
        let index = u64::from_le_bytes(prefix[24..32].try_into().expect("eight bytes"));

        let indexed = frames.saturating_mul(4);
        if !plausible(plaintext, frame, frames)
            || index < header
            || index.saturating_add(indexed) != on_disk
            || prefix.len() < header as usize
        {
            return Ok(None);
        }

        // Only now, once the file is known to be one of ours, does a missing key
        // become an error rather than a reason to read it as a plain object.
        // Answering `None` here would serve the ciphertext as the object.
        let key = match version {
            V_SEALED => Some(sealed_key(&prefix, keys)?),
            _ => None,
        };

        let lengths = reader.read_at(index, indexed).await?;

        let mut offsets = Vec::with_capacity(frames as usize + 1);
        let mut stored = Vec::with_capacity(frames as usize);
        let mut at = header;
        offsets.push(at);
        for length in lengths.chunks_exact(4) {
            let entry = u32::from_le_bytes(length.try_into().expect("four bytes"));
            at += (entry & !STORED) as u64;
            stored.push(entry & STORED != 0);
            offsets.push(at);
        }

        if at != index {
            return Ok(None);
        }

        Ok(Some(Self {
            reader,
            plaintext,
            frame,
            frames: frames as u32,
            offsets,
            stored,
            key,
            oid: oid.to_owned(),
        }))
    }

    pub fn plaintext(&self) -> u64 {
        self.plaintext
    }

    // One frame in flight, whatever the object weighs. A three-gigabyte asset
    // is served from four megabytes of memory, which is the property the whole
    // storage layer is built around.
    pub fn stream(self, start: u64, length: u64) -> impl Stream<Item = Result<Bytes, Error>> {
        futures_util::stream::try_unfold(
            (self, start, length),
            |(mut framed, at, wanted)| async move {
                if wanted == 0 {
                    return Ok(None);
                }

                let index = (at / framed.frame) as usize;
                let Some(bounds) = framed.offsets.get(index..index + 2).map(<[u64]>::to_vec) else {
                    return Ok(None);
                };

                let body = framed
                    .reader
                    .read_at(bounds[0], bounds[1] - bounds[0])
                    .await?;

                let plain = framed.decode(index, body)?;

                let from = (at % framed.frame) as usize;
                let take = wanted.min(plain.len().saturating_sub(from) as u64) as usize;
                let bytes = Bytes::copy_from_slice(&plain[from..from + take]);

                Ok(Some((
                    bytes,
                    (framed, at + take as u64, wanted - take as u64),
                )))
            },
        )
    }

    fn decode(&self, index: usize, body: Vec<u8>) -> Result<Vec<u8>, Error> {
        let body = match &self.key {
            Some(key) => key.open(
                index as u32,
                index as u32 + 1 == self.frames,
                &self.oid,
                &body,
            )?,
            None => body,
        };

        if self.stored.get(index).copied().unwrap_or_default() {
            return Ok(body);
        }

        zstd::bulk::decompress(&body, self.frame as usize)
            .map_err(|error| Error::Storage(std::io::Error::other(error)))
    }
}

// The header field is unauthenticated, so a wrong key id sends the reader at the
// wrong key and the frames refuse to open. That is the right failure: it is
// reported as tampering rather than as a bad key, which is what it is.
fn sealed_key(header: &[u8], keys: Option<&Keyring>) -> Result<ObjectKey, Error> {
    let keys = keys.ok_or(Error::NotDecryptable)?;

    let mut id = [0u8; crypt::ID];
    id.copy_from_slice(&header[32..32 + crypt::ID]);
    let mut salt = [0u8; crypt::SALT];
    salt.copy_from_slice(&header[36..36 + crypt::SALT]);

    keys.reading(id, salt)
}

#[cfg(test)]
mod tests;
