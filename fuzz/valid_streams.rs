use std::{io::Write, num::NonZeroU64};

use lzma_rust2::{
    CheckType, LzipOptions, LzipWriter, Lzma2Options, Lzma2Writer, LzmaWriter, XzOptions, XzWriter,
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

pub fn bcj2_payload(size: usize) -> Vec<u8> {
    const BRANCHES: [u8; 18] = [
        0x90, 0xE8, 0x04, 0, 0, 0, 0x90, 0xE9, 0x08, 0, 0, 0, 0x0F, 0x85, 0x02, 0, 0, 0,
    ];
    BRANCHES.into_iter().cycle().take(size).collect()
}

pub fn bcj2(payload: &[u8]) -> [Vec<u8>; 4] {
    let options = lzma_rust2::filter::bcj2::Bcj2Options {
        uncompressed_size: Some(payload.len() as u64),
        ..Default::default()
    };
    let outputs = core::array::from_fn(|_| Vec::new());
    let mut writer = lzma_rust2::filter::bcj2::Bcj2Writer::new(outputs, &options).unwrap();
    for chunk in payload.chunks(7) {
        writer.write_all(chunk).unwrap();
        writer.flush().unwrap();
    }
    writer.finish().unwrap()
}

pub fn lzma2(payload: &[u8]) -> Vec<u8> {
    let mut writer = Lzma2Writer::new(Vec::new(), lzma2_options());
    writer.write_all(payload).unwrap();
    writer.finish().unwrap()
}

#[allow(dead_code)]
pub fn lzma(payload: &[u8]) -> Vec<u8> {
    let options = lzma2_options().lzma_options;
    let mut writer =
        LzmaWriter::new_use_header(Vec::new(), &options, Some(payload.len() as u64)).unwrap();
    writer.write_all(payload).unwrap();
    writer.finish().unwrap()
}

#[allow(dead_code)]
pub fn lzma_raw(payload: &[u8], use_end_marker: bool) -> (Vec<u8>, u8, u32) {
    let options = lzma2_options().lzma_options;
    let mut writer = LzmaWriter::new_no_header(Vec::new(), &options, use_end_marker).unwrap();
    let props = writer.props();
    writer.write_all(payload).unwrap();
    (writer.finish().unwrap(), props, options.dict_size)
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
