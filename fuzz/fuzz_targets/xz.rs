#![no_main]

#[path = "reference_decode.rs"]
mod reference_decode;

use libfuzzer_sys::fuzz_target;
use liblzma::stream::{Stream, CONCATENATED};
use lzma_rust2::XzReader;

fuzz_target!(|data: &[u8]| {
    const MEM_LIMIT_KB: u32 = 8 * 1024;
    const REFERENCE_MEM_LIMIT: u64 = 16 * 1024 * 1024;

    let implementation =
        reference_decode::read_bounded(XzReader::new_mem_limit(data, true, MEM_LIMIT_KB));
    let reference = reference_decode::read_bounded(liblzma::read::XzDecoder::new_stream(
        data,
        Stream::new_stream_decoder(REFERENCE_MEM_LIMIT, CONCATENATED).unwrap(),
    ));
    reference_decode::compare(implementation, reference);
});
