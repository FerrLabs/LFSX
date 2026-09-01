use std::sync::OnceLock;

use crate::oid::Oid;
use crate::storage::codec;
use crate::storage::crypt::Keyring;
use crate::storage::s3::{refs, sizes};

// The façade the fuzz targets drive, compiled only under the `fuzzing`
// feature. Each wrapper hands a parser its natural input and promises one
// thing: whatever the bytes, the answer is a value or an `Err`, never an
// abort. The release profile turns any panic into a crash, which is what
// makes a reachable panic a finding rather than a style complaint.

pub fn parse_oid(raw: &str) {
    let _ = Oid::parse(raw);
}

pub fn parse_size_key(key: &str) {
    let _ = sizes::read(key);
}

pub fn parse_marker_key(key: &str) {
    let _ = refs::from_marker(key);
}

pub fn parse_range(header: Option<&str>, size: u64) {
    let range = crate::range::Range::parse(header, size);
    let _ = range.length(size);
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime for the fuzz harness")
    })
}

fn keyring() -> &'static Keyring {
    static KEYS: OnceLock<Keyring> = OnceLock::new();
    KEYS.get_or_init(|| {
        let dir = std::env::temp_dir().join("lfsx-fuzz");
        std::fs::create_dir_all(&dir).expect("a scratch directory for the fuzz keyring");
        let path = dir.join("key");
        std::fs::write(&path, format!("{}\n", "ab".repeat(32)))
            .expect("the fuzz keyring fits on disk");
        Keyring::load(&path).expect("a fixed hex key always loads")
    })
}

// The bytes are the store: whatever is on disk, `Framed::open` has to answer
// with a framed object, `None` for a raw one, or an error, and streaming any
// range of what it accepted has to terminate. Half the inputs are read with a
// keyring so the sealed-header paths run too.
pub fn feed_codec(data: &[u8]) {
    let Some((selector, bytes)) = data.split_first() else {
        return;
    };
    let keys = (selector & 1 == 1).then(keyring);

    let oid = Oid::parse(&"ab".repeat(32)).expect("the harness oid is a digest");
    let dir = std::env::temp_dir().join("lfsx-fuzz");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("object-{}", std::process::id()));
    if std::fs::write(&path, bytes).is_err() {
        return;
    }

    runtime().block_on(async {
        let Ok(file) = tokio::fs::File::open(&path).await else {
            return;
        };
        let on_disk = bytes.len() as u64;

        if let Ok(Some(framed)) =
            codec::Framed::open(codec::Reader::File(file), on_disk, keys, &oid).await
        {
            use futures_util::StreamExt;

            let plaintext = framed.plaintext();
            let length = plaintext.min(1 << 20);
            let start = plaintext.saturating_sub(length) / 2;
            let mut chunks = Box::pin(framed.stream(start, length));
            while let Some(chunk) = chunks.next().await {
                if chunk.is_err() {
                    break;
                }
            }
        }
    });
}

// A well-formed framed object for the seed corpus: libFuzzer mutates its way
// into the format far faster from a valid header than from noise.
pub fn sample_object(compressed: bool, sealed: bool) -> Vec<u8> {
    let oid = Oid::parse(&"ab".repeat(32)).expect("the harness oid is a digest");
    let dir = std::env::temp_dir().join("lfsx-fuzz");
    std::fs::create_dir_all(&dir).expect("a scratch directory for the sample");
    let path = dir.join(format!("sample-{}", std::process::id()));

    runtime().block_on(async {
        let file = tokio::fs::File::create(&path)
            .await
            .expect("the sample fits on disk");
        let level = compressed.then_some(3);
        let key = sealed.then(|| keyring().writing());
        let mut writer = codec::Writer::open(file, level, key, &oid)
            .await
            .expect("the sample writer opens");
        writer
            .push(&b"a mesh that compresses ".repeat(512))
            .await
            .expect("the sample bytes go in");
        writer.finish().await.expect("the sample finishes");
    });

    std::fs::read(&path).expect("the sample reads back")
}
