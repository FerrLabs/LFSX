// Regenerates the framed-object seeds in `corpus/codec/` through the codec's
// own writer, so the corpus can always be rebuilt from the current format
// rather than trusted as opaque bytes:
//
//   cargo run --bin seed
//
// The leading selector byte mirrors what the `codec` target reads: even means
// the harness opens the file without a keyring, odd means with one.
fn main() {
    for (name, selector, compressed, sealed) in [
        ("plain-selector0", 0u8, true, false),
        ("sealed-selector1", 1u8, true, true),
    ] {
        let bytes = lfsx_server::fuzzing::sample_object(compressed, sealed);
        let mut entry = vec![selector];
        entry.extend_from_slice(&bytes);
        let path = format!("corpus/codec/{name}");
        std::fs::write(&path, entry).expect("the corpus directory is writable");
        println!("wrote {path}");
    }
}
