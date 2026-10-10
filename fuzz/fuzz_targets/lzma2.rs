#![no_main]

use std::io::{self, Read};

use libfuzzer_sys::fuzz_target;
use lzma_rust2::{Lzma2Reader, LzmaOptions};

fuzz_target!(|data: &[u8]| {
    const MAX_OUTPUT: u64 = 1 << 18;
    const MEM_LIMIT_KB: u32 = 8 * 1024;

    let Ok(reader) =
        Lzma2Reader::new_mem_limit(data, LzmaOptions::DICT_SIZE_DEFAULT, MEM_LIMIT_KB, None)
    else {
        return;
    };
    let _ = io::copy(&mut reader.take(MAX_OUTPUT), &mut io::empty());
});
