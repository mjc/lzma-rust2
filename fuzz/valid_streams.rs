use std::{io::Write, num::NonZeroU64};

use lzma_rust2::{
    CheckType, LzipOptions, LzipWriter, Lzma2Options, Lzma2Writer, XzOptions, XzWriter,
};

const BLOCK_SIZE: u32 = 4 * 1024;

fn lzma2_options() -> Lzma2Options {
    let mut options = Lzma2Options::with_preset(1);
    options.lzma_options.dict_size = BLOCK_SIZE;
    options.set_chunk_size(NonZeroU64::new(u64::from(BLOCK_SIZE)));
    options
}

pub fn payload(size: usize) -> Vec<u8> {
    let mut state = 0x1234_5678_u32;
    (0..size)
        .map(|index| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()[0] ^ (index / 251) as u8
        })
        .collect()
}

pub fn lzma2(payload: &[u8]) -> Vec<u8> {
    let mut writer = Lzma2Writer::new(Vec::new(), lzma2_options());
    writer.write_all(payload).unwrap();
    writer.finish().unwrap()
}

pub fn xz(payload: &[u8], check_type: CheckType) -> Vec<u8> {
    let lzma_options = lzma2_options().lzma_options;
    let mut writer = XzWriter::new(
        Vec::new(),
        XzOptions {
            lzma_options,
            check_type,
            block_size: NonZeroU64::new(u64::from(BLOCK_SIZE)),
            filters: Vec::new(),
        },
    )
    .unwrap();
    writer.write_all(payload).unwrap();
    writer.finish().unwrap()
}

pub fn lzip(payload: &[u8]) -> Vec<u8> {
    let mut writer = LzipWriter::new(
        Vec::new(),
        LzipOptions {
            lzma_options: lzma2_options().lzma_options,
            member_size: NonZeroU64::new(u64::from(BLOCK_SIZE)),
        },
    );
    writer.write_all(payload).unwrap();
    writer.finish().unwrap()
}
