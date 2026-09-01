#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((split, rest)) = data.split_first() else {
        return;
    };
    let size = u64::from_le_bytes([
        *split,
        rest.first().copied().unwrap_or(0),
        rest.get(1).copied().unwrap_or(0),
        rest.get(2).copied().unwrap_or(0),
        0,
        0,
        0,
        0,
    ]);
    let header = rest.get(3..).and_then(|bytes| std::str::from_utf8(bytes).ok());
    lfsx_server::fuzzing::parse_range(header, size);
});
