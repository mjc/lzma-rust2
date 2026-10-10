#[path = "../fuzz_targets/decoder_mt.rs"]
mod decoder_mt;

#[path = "../valid_streams.rs"]
mod valid_streams;

use decoder_mt::{
    lzip_mt_decode, lzip_mt_decode_tolerant, lzma2_mt_decode, lzma2_mt_decode_tolerant,
    xz_mt_decode, xz_mt_decode_tolerant, HEADER_SIZE, MAX_INPUT_SIZE,
};
use lzma_rust2::{
    CheckType, LzipReader, LzipReaderMt, Lzma2Reader, Lzma2ReaderMt, XzReader, XzReaderMt,
};
use std::io::{Cursor, Read, Write};

fn input(selector: u8, stream: &[u8]) -> Vec<u8> {
    [[selector, selector % 4].as_slice(), stream].concat()
}

fn read_with_size(mut reader: impl Read, read_size: usize) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = reader.read(&mut buffer[..read_size]).unwrap();
        if count == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..count]);
    }
    output
}

#[test]
fn parallel_readers_decode_valid_multiblock_streams() {
    let payload = valid_streams::payload(3 * 4096 + 137);
    let lzma2 = valid_streams::lzma2(&payload);
    let lzip = valid_streams::lzip(&payload);

    for selector in 0..8 {
        let read_size = [1, 7, 64, 4096][selector as usize % 4];
        let lzma2_serial =
            read_with_size(Lzma2Reader::new(lzma2.as_slice(), 4 * 1024, None), read_size);
        let lzma2_parallel = read_with_size(
            Lzma2ReaderMt::new(Cursor::new(lzma2.as_slice()), 4 * 1024, None, 2),
            read_size,
        );
        assert_eq!(lzma2_serial, payload);
        assert_eq!(lzma2_parallel, payload);

        let lzip_serial =
            read_with_size(LzipReader::new_mem_limit(lzip.as_slice(), 16 * 1024), read_size);
        let lzip_parallel = read_with_size(
            LzipReaderMt::new_mem_limit(Cursor::new(lzip.as_slice()), 16 * 1024, 2).unwrap(),
            read_size,
        );
        assert_eq!(lzip_serial, payload);
        assert_eq!(lzip_parallel, payload);
        for check_type in [
            CheckType::None,
            CheckType::Crc32,
            CheckType::Crc64,
            CheckType::Sha256,
        ] {
            let xz = valid_streams::xz(&payload, check_type);
            let xz_serial = read_with_size(XzReader::new(xz.as_slice(), false), read_size);
            let xz_parallel = read_with_size(
                XzReaderMt::new(Cursor::new(xz.as_slice()), false, 2).unwrap(),
                read_size,
            );
            assert_eq!(xz_serial, payload);
            assert_eq!(xz_parallel, payload);
        }
    }
}

#[test]
fn malformed_and_truncated_streams_return_without_panicking() {
    let payload = valid_streams::payload(2 * 4096 + 31);
    for mut stream in [
        valid_streams::lzma2(&payload),
        valid_streams::xz(&payload, CheckType::Crc64),
        valid_streams::lzip(&payload),
    ] {
        for length in [0, 1, 5, 12, stream.len() / 2, stream.len() - 1] {
            let candidate = input(length as u8, &stream[..length]);
            lzma2_mt_decode_tolerant(&candidate);
            xz_mt_decode_tolerant(&candidate);
            lzip_mt_decode_tolerant(&candidate);
        }
        for offset in [0, stream.len() / 3, stream.len() / 2, stream.len() - 1] {
            stream[offset] ^= 0x80;
            let candidate = input(offset as u8, &stream);
            lzma2_mt_decode_tolerant(&candidate);
            xz_mt_decode_tolerant(&candidate);
            lzip_mt_decode_tolerant(&candidate);
            stream[offset] ^= 0x80;
        }
    }
}

#[test]
fn input_bounds_are_enforced() {
    assert!(lzma2_mt_decode(&[0; HEADER_SIZE - 1]).is_none());
    assert!(xz_mt_decode(&[0; HEADER_SIZE - 1]).is_none());
    assert!(lzip_mt_decode(&[0; HEADER_SIZE - 1]).is_none());

    let oversized = vec![0; HEADER_SIZE + MAX_INPUT_SIZE + 1];
    assert!(lzma2_mt_decode(&oversized).is_none());
    assert!(xz_mt_decode(&oversized).is_none());
    assert!(lzip_mt_decode(&oversized).is_none());
}

#[test]
fn output_cap_does_not_compare_incomplete_xz_decodes() {
    let mut options = lzma_rust2::XzOptions::with_preset(1);
    options.check_type = CheckType::Crc32;
    let mut writer = lzma_rust2::XzWriter::new(Vec::new(), options).unwrap();
    writer.write_all(&vec![0; 300_000]).unwrap();
    let mut compressed = writer.finish().unwrap();
    let footer = &compressed[compressed.len() - 12..];
    let index_size = (u32::from_le_bytes(footer[4..8].try_into().unwrap()) as usize + 1) * 4;
    let index_start = compressed.len() - 12 - index_size;
    compressed[index_start - 1] ^= 1;

    let mut serial = XzReader::new(compressed.as_slice(), true);
    assert!(serial.read_to_end(&mut Vec::new()).is_err());
    assert!(xz_mt_decode(&[[3, 2].as_slice(), compressed.as_slice()].concat()).is_some());
}

#[test]
fn single_stream_mode_does_not_compare_different_consumption_contracts() {
    let first = valid_streams::xz(b"first", CheckType::Crc32);
    let second = valid_streams::xz(b"second", CheckType::Crc32);
    let concatenated = [first, second].concat();
    assert!(xz_mt_decode(&[[1, 2].as_slice(), concatenated.as_slice()].concat()).is_some());
}

#[test]
fn empty_lzma2_input_is_rejected_by_both_readers() {
    assert!(lzma2_mt_decode(&[0, 0]).is_some());
}
