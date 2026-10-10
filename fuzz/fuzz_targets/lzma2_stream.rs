#![no_main]

use libfuzzer_sys::fuzz_target;
use lzma_rust2::{Action, Lzma2Stream, LzmaOptions, Status};

const MAX_OUTPUT: usize = 1 << 18;
const MAX_STEPS: usize = 4096;
const MEM_LIMIT_KB: u32 = 16 * 1024;

fuzz_target!(|data: &[u8]| {
    let mut decoder = Lzma2Stream::new_mem_limit(LzmaOptions::DICT_SIZE_DEFAULT, MEM_LIMIT_KB);
    let mut output_buf = [0u8; 4096];
    let mut in_pos = 0;
    let mut total_out = 0;

    for _ in 0..MAX_STEPS {
        let action = if in_pos >= data.len() {
            Action::Finish
        } else {
            Action::Run
        };
        let result = match decoder.process(&data[in_pos..], &mut output_buf, action) {
            Ok(r) => r,
            Err(_) => return,
        };
        in_pos += result.bytes_consumed;
        total_out += result.bytes_produced;
        if result.status == Status::StreamEnd {
            return;
        }
        if total_out > MAX_OUTPUT {
            return;
        }
        if result.bytes_consumed == 0 && result.bytes_produced == 0 {
            return;
        }
    }
});
