#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    lfsx_server::fuzzing::feed_codec(data);
});
