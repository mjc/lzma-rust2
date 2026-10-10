#![no_main]

#[path = "reference_decode.rs"]
mod reference_decode;

use libfuzzer_sys::fuzz_target;
use liblzma::stream::{Filters, Stream};
use lzma_rust2::{Action, Lzma2Stream, LzmaOptions, Status};

const CHUNK_SIZES: [usize; 16] = [
    1, 2, 3, 5, 7, 19, 20, 21, 32, 39, 40, 41, 64, 512, 2048, 4096,
];
const OUTPUT_SIZES: [usize; 8] = [1, 2, 7, 19, 64, 512, 2048, 4096];

const MAX_OUTPUT: usize = 1 << 18;
const MAX_STEPS: usize = 4096;
const MEM_LIMIT_KB: u32 = 16 * 1024;
const PLAN_LEN: usize = 8;

fuzz_target!(|data: &[u8]| {
    if data.len() < 1 + PLAN_LEN {
        return;
    }
    let dict_size = [
        4 * 1024,
        64 * 1024,
        1024 * 1024,
        LzmaOptions::DICT_SIZE_DEFAULT,
    ][usize::from(data[0]) % 4];
    let (plan, stream) = data[1..].split_at(PLAN_LEN);

    let mut decoder = Lzma2Stream::new_mem_limit(dict_size, MEM_LIMIT_KB);
    let mut output_buf = [0u8; 4096];
    let mut output = Vec::new();
    let mut in_pos = 0usize;

    for step in 0..MAX_STEPS {
        let choice = plan[step % PLAN_LEN];
        let chunk = CHUNK_SIZES[usize::from(choice & 0x0F)];
        let out_len = OUTPUT_SIZES[usize::from(choice >> 4) % OUTPUT_SIZES.len()];
        let end = in_pos.saturating_add(chunk).min(stream.len());
        let action = if end >= stream.len() {
            Action::Finish
        } else {
            Action::Run
        };
        let result = match decoder.process(&stream[in_pos..end], &mut output_buf[..out_len], action)
        {
            Ok(r) => r,
            Err(_) => return,
        };
        in_pos += result.bytes_consumed;
        output.extend_from_slice(&output_buf[..result.bytes_produced]);
        if result.status == Status::StreamEnd {
            let mut options = liblzma::stream::LzmaOptions::new_preset(6).unwrap();
            options.dict_size(dict_size);
            let mut filters = Filters::new();
            filters.lzma2(&options);
            let reference = reference_decode::read_bounded(liblzma::read::XzDecoder::new_stream(
                stream,
                Stream::new_raw_decoder(&filters).unwrap(),
            ));
            reference_decode::compare(Ok(Some(output)), reference);
            return;
        }
        if output.len() > MAX_OUTPUT {
            return;
        }
        if action == Action::Finish && result.bytes_consumed == 0 && result.bytes_produced == 0 {
            return;
        }
    }
});
