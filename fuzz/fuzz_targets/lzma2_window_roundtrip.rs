#![no_main]

#[path = "lzma2_window.rs"]
mod lzma2_window;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    lzma2_window::roundtrip(data);
});
