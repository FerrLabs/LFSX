use futures_util::StreamExt;

use super::*;

async fn framed(payload: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("object");
    let file = fs::File::create(&path).await.unwrap();
    let mut writer = Writer::open(file, 3).await.unwrap();
    for chunk in payload.chunks(1024 * 1024) {
        writer.push(chunk).await.unwrap();
    }
    writer.finish().await.unwrap();

    (root, path)
}

async fn read(path: &std::path::Path, start: u64, length: u64) -> Vec<u8> {
    let file = fs::File::open(path).await.unwrap();
    let on_disk = file.metadata().await.unwrap().len();
    let framed = Framed::open(file, on_disk).await.unwrap().unwrap();

    let mut out = Vec::new();
    let mut chunks = Box::pin(framed.stream(start, length));
    while let Some(chunk) = chunks.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }

    out
}

fn compressible(len: usize) -> Vec<u8> {
    b"a mesh is float arrays and float arrays repeat themselves "
        .iter()
        .cycle()
        .take(len)
        .copied()
        .collect()
}

#[tokio::test]
async fn what_goes_in_comes_back_out() {
    let payload = compressible(9 * 1024 * 1024);
    let (root, path) = framed(&payload).await;

    assert_eq!(read(&path, 0, payload.len() as u64).await, payload);
    assert!(
        std::fs::metadata(&path).unwrap().len() < payload.len() as u64 / 4,
        "the whole point is that it takes less room"
    );
    drop(root);
}

#[tokio::test]
async fn a_range_reads_only_the_frames_it_touches() {
    let payload = compressible(10 * 1024 * 1024);
    let (_root, path) = framed(&payload).await;

    let start = FRAME * 2 + 1234;
    let length = 4096;

    assert_eq!(
        read(&path, start, length).await,
        payload[start as usize..(start + length) as usize],
        "a client resuming at 90% must be handed the bytes at 90%, not the ones at the frame \
         boundary before it"
    );
}

#[tokio::test]
async fn a_range_that_spans_a_frame_boundary_is_still_contiguous() {
    let payload = compressible(6 * 1024 * 1024);
    let (_root, path) = framed(&payload).await;

    let start = FRAME - 100;

    assert_eq!(
        read(&path, start, 200).await,
        payload[start as usize..start as usize + 200]
    );
}

#[tokio::test]
async fn an_empty_object_round_trips() {
    let (_root, path) = framed(b"").await;

    assert_eq!(read(&path, 0, 0).await, Vec::<u8>::new());
}

#[tokio::test]
async fn a_file_written_before_this_existed_is_read_as_itself() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("object");
    std::fs::write(
        &path,
        b"an object from a server that never compressed anything",
    )
    .unwrap();

    let file = fs::File::open(&path).await.unwrap();
    let on_disk = file.metadata().await.unwrap().len();

    assert!(
        Framed::open(file, on_disk).await.unwrap().is_none(),
        "a store written before compression has to keep reading back, or upgrading is a migration"
    );
}

#[tokio::test]
async fn a_file_that_merely_starts_like_a_header_is_not_mistaken_for_one() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("object");
    let mut impostor = MAGIC.to_vec();
    impostor.push(1);
    impostor.extend_from_slice(&[0u8; 64]);
    std::fs::write(&path, &impostor).unwrap();

    let file = fs::File::open(&path).await.unwrap();
    let on_disk = file.metadata().await.unwrap().len();

    assert!(
        Framed::open(file, on_disk).await.unwrap().is_none(),
        "the header has to be rejected on its own arithmetic, or an object whose first bytes \
         happen to collide is served as garbage"
    );
}

// A header the server would never write, in a file a client is free to push:
// an object is whatever bytes hash to its name, so this is a request, not a
// corruption.
fn forged(frame: u32, plaintext: u64, frames: u32) -> Vec<u8> {
    let payload = vec![0u8; 64];
    let index = HEADER + payload.len() as u64;

    let mut file = [0u8; HEADER as usize];
    file[0..4].copy_from_slice(MAGIC);
    file[4] = 1;
    file[8..16].copy_from_slice(&plaintext.to_le_bytes());
    file[16..20].copy_from_slice(&frame.to_le_bytes());
    file[20..24].copy_from_slice(&frames.to_le_bytes());
    file[24..32].copy_from_slice(&index.to_le_bytes());

    let mut out = file.to_vec();
    out.extend_from_slice(&payload);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out
}

async fn sniffs(bytes: &[u8]) -> bool {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("object");
    std::fs::write(&path, bytes).unwrap();

    let file = fs::File::open(&path).await.unwrap();
    let on_disk = file.metadata().await.unwrap().len();

    Framed::open(file, on_disk).await.unwrap().is_some()
}

#[tokio::test]
async fn a_frame_size_nobody_could_have_written_is_refused() {
    assert!(
        !sniffs(&forged(0xFFFF_FFFE, 64, 1)).await,
        "the frame size is the size of the buffer each frame is decompressed into, so an object \
         that names its own is asking the server to allocate four gigabytes per read"
    );
    assert!(
        !sniffs(&forged(1024, 64, 1)).await,
        "and one below the range this format uses is just as much a claim about allocation"
    );
}

#[tokio::test]
async fn a_plaintext_size_the_frames_could_not_hold_is_refused() {
    assert!(
        !sniffs(&forged(FRAME as u32, 100 * 1024 * 1024, 1)).await,
        "one frame cannot carry a hundred megabytes, and a header that says otherwise is \
         describing a file that does not exist"
    );
}

#[tokio::test]
async fn the_frame_size_this_server_writes_is_still_accepted() {
    assert!(sniffs(&forged(FRAME as u32, 64, 1)).await);
}

// Bytes that give a compressor nothing to work with, which is what a PNG or an
// OGG already is.
fn incompressible(len: usize) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

#[tokio::test]
async fn an_object_that_will_not_compress_is_stored_as_it_arrived() {
    let payload = incompressible(9 * 1024 * 1024);
    let (root, path) = framed(&payload).await;

    assert_eq!(
        read(&path, 0, payload.len() as u64).await,
        payload,
        "storing a frame raw has to read back as the frame, or every already-compressed asset in \
         the store comes out as noise"
    );

    let on_disk = std::fs::metadata(&path).unwrap().len();
    assert!(
        on_disk < payload.len() as u64 + 4096,
        "and it must not cost more than not trying: {on_disk} for {}",
        payload.len()
    );
    drop(root);
}

#[tokio::test]
async fn a_range_inside_an_uncompressed_frame_still_lands() {
    let payload = incompressible(6 * 1024 * 1024);
    let (_root, path) = framed(&payload).await;
    let start = FRAME + 5000;

    assert_eq!(
        read(&path, start, 1024).await,
        payload[start as usize..start as usize + 1024]
    );
}
