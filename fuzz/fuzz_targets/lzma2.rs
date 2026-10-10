#![no_main]

#[path = "reference_decode.rs"]
mod reference_decode;

use libfuzzer_sys::fuzz_target;
use liblzma::stream::{Filters, Stream};
use lzma_rust2::{Lzma2Reader, LzmaOptions};

fuzz_target!(|data: &[u8]| {
    const MEM_LIMIT_KB: u32 = 16 * 1024;

    let implementation =
        Lzma2Reader::new_mem_limit(data, LzmaOptions::DICT_SIZE_DEFAULT, MEM_LIMIT_KB, None)
            .and_then(reference_decode::read_bounded);

    let mut options = liblzma::stream::LzmaOptions::new_preset(6).unwrap();
    options.dict_size(LzmaOptions::DICT_SIZE_DEFAULT);
    let mut filters = Filters::new();
    filters.lzma2(&options);
    let reference = reference_decode::read_bounded(liblzma::read::XzDecoder::new_stream(
        data,
        Stream::new_raw_decoder(&filters).unwrap(),
    ));
    reference_decode::compare(implementation, reference);
});
