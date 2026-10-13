#![no_main]

#[path = "decoder_mt.rs"]
mod decoder_mt;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    decoder_mt::lzip_mt_decode(data);
});
