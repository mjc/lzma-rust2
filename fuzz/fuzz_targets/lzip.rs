#![no_main]

use std::io::{self, Read};

use libfuzzer_sys::fuzz_target;
use lzma_rust2::LzipReader;

fuzz_target!(|data: &[u8]| {
    const MAX_OUTPUT: u64 = 1 << 18;

    let reader = LzipReader::new(data);
    let _ = io::copy(&mut reader.take(MAX_OUTPUT), &mut io::empty());
});
