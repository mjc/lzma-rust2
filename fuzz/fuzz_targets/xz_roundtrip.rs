#![no_main]

mod encoder_roundtrip;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    encoder_roundtrip::xz_roundtrip(data);
});
