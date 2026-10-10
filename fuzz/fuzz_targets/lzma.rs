#![no_main]

#[path = "reference_decode.rs"]
mod reference_decode;

use libfuzzer_sys::fuzz_target;
use liblzma::stream::Stream;
use lzma_rust2::LzmaReader;

fuzz_target!(|data: &[u8]| {
    const MEM_LIMIT_KB: u32 = 8 * 1024;
    const REFERENCE_MEM_LIMIT: u64 = 16 * 1024 * 1024;

    let implementation = LzmaReader::new_mem_limit(data, MEM_LIMIT_KB, None)
        .and_then(reference_decode::read_bounded);
    let reference = reference_decode::read_bounded(liblzma::read::XzDecoder::new_stream(
        data,
        Stream::new_lzma_decoder(REFERENCE_MEM_LIMIT).unwrap(),
    ));
    reference_decode::compare(implementation, reference);
});
