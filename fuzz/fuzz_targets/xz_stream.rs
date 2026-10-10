#![no_main]

use libfuzzer_sys::fuzz_target;
use lzma_rust2::{Action, Status, XzStream};

const CHUNK_SIZES: [usize; 16] = [
    1, 2, 3, 5, 7, 19, 20, 21, 32, 39, 40, 41, 64, 512, 2048, 4096,
];
const OUTPUT_SIZES: [usize; 8] = [1, 2, 7, 19, 64, 512, 2048, 4096];

const MAX_OUTPUT: usize = 1 << 18;
const MAX_STEPS: usize = 4096;
const MEM_LIMIT_KB: u32 = 8 * 1024;
const PLAN_LEN: usize = 8;

fuzz_target!(|data: &[u8]| {
    if data.len() < 1 + PLAN_LEN {
        return;
    }
    let allow_multiple_streams = data[0] & 1 != 0;
    let (plan, stream) = data[1..].split_at(PLAN_LEN);

    let mut decoder = XzStream::new_mem_limit(allow_multiple_streams, MEM_LIMIT_KB);
    let mut output_buf = [0u8; 4096];
    let mut in_pos = 0usize;
    let mut total_out = 0usize;

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
        total_out += result.bytes_produced;
        if result.status == Status::StreamEnd {
            return;
        }
        if total_out > MAX_OUTPUT {
            return;
        }
        if action == Action::Finish && result.bytes_consumed == 0 && result.bytes_produced == 0 {
            return;
        }
    }
});
