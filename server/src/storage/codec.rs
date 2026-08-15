use std::io::SeekFrom;

use axum::body::Bytes;
use futures_util::Stream;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::error::Error;

// An object compressed at rest is still named after the digest of its plaintext,
// because that name is what every other part of the server addresses it by —
// collection, deduplication, the shared store, the client's own verification. So
// the file says what it is in its first bytes instead of in its name, and a
// store from before this feature reads back unchanged: no magic, no header, just
// the object.
const MAGIC: &[u8; 4] = b"LFZ1";
const HEADER: u64 = 32;

// Plaintext per frame. Each is compressed on its own, so serving a range means
// decompressing the frames it touches rather than everything before it — which
// is what keeps resuming a three-gigabyte download from costing three gigabytes
// of CPU. Larger frames compress better and seek worse; four megabytes is about
// where a home upstream stops noticing either.
pub const FRAME: u64 = 4 * 1024 * 1024;

pub struct Framed {
    file: fs::File,
    plaintext: u64,
    frame: u64,
    offsets: Vec<u64>,
}

pub struct Writer {
    file: fs::File,
    level: i32,
    lengths: Vec<u32>,
    plaintext: u64,
    pending: Vec<u8>,
}

impl Writer {
    pub async fn open(mut file: fs::File, level: i32) -> Result<Self, Error> {
        file.write_all(&[0u8; HEADER as usize]).await?;

        Ok(Self {
            file,
            level,
            lengths: Vec::new(),
            plaintext: 0,
            pending: Vec::with_capacity(FRAME as usize),
        })
    }

    pub async fn push(&mut self, chunk: &[u8]) -> Result<(), Error> {
        self.plaintext += chunk.len() as u64;
        self.pending.extend_from_slice(chunk);

        while self.pending.len() >= FRAME as usize {
            let rest = self.pending.split_off(FRAME as usize);
            let frame = std::mem::replace(&mut self.pending, rest);
            self.flush(&frame).await?;
        }

        Ok(())
    }

    pub async fn finish(mut self) -> Result<(), Error> {
        if !self.pending.is_empty() {
            let frame = std::mem::take(&mut self.pending);
            self.flush(&frame).await?;
        }

        for length in &self.lengths {
            self.file.write_all(&length.to_le_bytes()).await?;
        }

        let index = HEADER
            + self
                .lengths
                .iter()
                .map(|length| *length as u64)
                .sum::<u64>();
        self.file.seek(SeekFrom::Start(0)).await?;
        self.file
            .write_all(&header(self.plaintext, self.lengths.len() as u32, index))
            .await?;
        self.file.flush().await?;
        self.file.sync_all().await?;

        Ok(())
    }

    async fn flush(&mut self, frame: &[u8]) -> Result<(), Error> {
        let compressed = zstd::bulk::compress(frame, self.level).map_err(std::io::Error::other)?;
        self.file.write_all(&compressed).await?;
        self.lengths.push(compressed.len() as u32);

        Ok(())
    }
}

fn header(plaintext: u64, frames: u32, index: u64) -> [u8; HEADER as usize] {
    let mut header = [0u8; HEADER as usize];
    header[0..4].copy_from_slice(MAGIC);
    header[4] = 1;
    header[8..16].copy_from_slice(&plaintext.to_le_bytes());
    header[16..20].copy_from_slice(&(FRAME as u32).to_le_bytes());
    header[20..24].copy_from_slice(&frames.to_le_bytes());
    header[24..32].copy_from_slice(&index.to_le_bytes());
    header
}

impl Framed {
    // Sniffing the header is what makes a raw store and a compressed one the
    // same store. A file that only looks like a header is rejected on its own
    // arithmetic: the index has to sit inside the file and the frames have to
    // account for exactly the bytes between them.
    pub async fn open(mut file: fs::File, on_disk: u64) -> Result<Option<Self>, Error> {
        if on_disk < HEADER {
            return Ok(None);
        }

        let mut header = [0u8; HEADER as usize];
        file.read_exact(&mut header).await?;

        if &header[0..4] != MAGIC || header[4] != 1 {
            file.seek(SeekFrom::Start(0)).await?;
            return Ok(None);
        }

        let plaintext = u64::from_le_bytes(header[8..16].try_into().expect("eight bytes"));
        let frame = u32::from_le_bytes(header[16..20].try_into().expect("four bytes")) as u64;
        let frames = u32::from_le_bytes(header[20..24].try_into().expect("four bytes")) as u64;
        let index = u64::from_le_bytes(header[24..32].try_into().expect("eight bytes"));

        let indexed = frames.saturating_mul(4);
        if frame == 0 || index < HEADER || index.saturating_add(indexed) != on_disk {
            file.seek(SeekFrom::Start(0)).await?;
            return Ok(None);
        }

        file.seek(SeekFrom::Start(index)).await?;
        let mut lengths = vec![0u8; indexed as usize];
        file.read_exact(&mut lengths).await?;

        let mut offsets = Vec::with_capacity(frames as usize + 1);
        let mut at = HEADER;
        offsets.push(at);
        for length in lengths.chunks_exact(4) {
            at += u32::from_le_bytes(length.try_into().expect("four bytes")) as u64;
            offsets.push(at);
        }

        if at != index {
            file.seek(SeekFrom::Start(0)).await?;
            return Ok(None);
        }

        Ok(Some(Self {
            file,
            plaintext,
            frame,
            offsets,
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

                framed.file.seek(SeekFrom::Start(bounds[0])).await?;
                let mut compressed = vec![0u8; (bounds[1] - bounds[0]) as usize];
                framed.file.read_exact(&mut compressed).await?;

                let plain = zstd::bulk::decompress(&compressed, framed.frame as usize)
                    .map_err(std::io::Error::other)?;

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
}

#[cfg(test)]
mod tests;
