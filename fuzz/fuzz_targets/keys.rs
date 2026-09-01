#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(key) = std::str::from_utf8(data) {
        lfsx_server::fuzzing::parse_size_key(key);
        lfsx_server::fuzzing::parse_marker_key(key);
        lfsx_server::fuzzing::parse_oid(key);
    }
});
