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
