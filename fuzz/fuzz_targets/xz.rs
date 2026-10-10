#![no_main]

use std::io::{self, Read};

use libfuzzer_sys::fuzz_target;
use lzma_rust2::XzReader;

fuzz_target!(|data: &[u8]| {
    const MAX_OUTPUT: u64 = 1 << 18;
    const MEM_LIMIT_KB: u32 = 8 * 1024;

    let reader = XzReader::new_mem_limit(data, true, MEM_LIMIT_KB);
    let _ = io::copy(&mut reader.take(MAX_OUTPUT), &mut io::empty());
});
