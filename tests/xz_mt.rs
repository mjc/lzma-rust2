use std::{
    io::{Cursor, Error, ErrorKind, Read, Write},
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use lzma_rust2::{
    Action, CheckType, Status, XzOptions, XzReader, XzReaderMt, XzStream, XzWriter, XzWriterMt,
};

static EXECUTABLE: &str = "tests/data/executable.exe";
static PG100: &str = "tests/data/pg100.txt";
static PG6800: &str = "tests/data/pg6800.txt";

/// A sink that can be read while the writer still holds it.
#[derive(Clone, Default)]
struct SharedSink(Arc<Mutex<Vec<u8>>>);

impl Write for SharedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Decodes the blocks of a stream that has no index and footer yet.
fn decode_blocks(compressed: &[u8]) -> Vec<u8> {
    let mut reader = XzReader::new(Cursor::new(compressed), false);
    let mut decoded = Vec::new();
    let mut buffer = [0; 8192];

    loop {
        match reader.read(&mut buffer) {
            // The stream ends in the middle of a record, so an error is expected here.
            Ok(0) | Err(_) => break,
            Ok(count) => decoded.extend_from_slice(&buffer[..count]),
        }
    }

    decoded
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn repair_block_header_crc(stream: &mut [u8]) {
    let block_start = 12;
    let header_size = (usize::from(stream[block_start]) + 1) * 4;
    let crc_start = block_start + header_size - 4;
    let crc = crc32(&stream[block_start..crc_start]).to_le_bytes();
    stream[crc_start..crc_start + 4].copy_from_slice(&crc);
}

fn read_vli(data: &[u8], offset: &mut usize) -> u64 {
    let mut value = 0;
    let mut shift = 0;
    loop {
        let byte = data[*offset];
        *offset += 1;
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

fn block_with_correct_declared_sizes() -> Vec<u8> {
    let mut options = XzOptions::with_preset(1);
    options.check_type = CheckType::Crc32;
    let mut writer = XzWriter::new(Vec::new(), options).unwrap();
    writer.write_all(b"x").unwrap();
    let mut stream = writer.finish().unwrap();

    let footer_start = stream.len() - 12;
    let index_size = (u32::from_le_bytes(
        stream[footer_start + 4..footer_start + 8]
            .try_into()
            .unwrap(),
    ) as usize
        + 1)
        * 4;
    let index_start = footer_start - index_size;
    let mut index_offset = index_start + 2;
    let unpadded_size = read_vli(&stream, &mut index_offset);
    let block_start = 12;
    let header_size = (usize::from(stream[block_start]) + 1) * 4;
    let compressed_size = unpadded_size - header_size as u64 - 4;
    assert!(compressed_size < 0x80);

    stream[block_start + 1] |= 0xC0;
    stream.copy_within(block_start + 2..block_start + 5, block_start + 4);
    stream[block_start + 2] = compressed_size as u8;
    stream[block_start + 3] = 1;
    repair_block_header_crc(&mut stream);
    stream
}

fn stream_error(data: &[u8]) -> Error {
    let mut decoder = XzStream::new(false);
    let mut output = [0u8; 4096];
    let mut in_pos = 0;

    for _ in 0..64 {
        match decoder.process(&data[in_pos..], &mut output, Action::Finish) {
            Ok(result) => {
                in_pos += result.bytes_consumed;
                assert_ne!(result.status, Status::StreamEnd, "the stream was accepted");
            }
            Err(error) => return error,
        }
    }
    panic!("the stream neither ended nor failed");
}

fn decode_stream(data: &[u8]) -> Vec<u8> {
    let mut decoder = XzStream::new(false);
    let mut output = [0u8; 4096];
    let mut in_pos = 0;
    let mut decoded = Vec::new();

    loop {
        let result = decoder
            .process(&data[in_pos..], &mut output, Action::Finish)
            .unwrap();
        in_pos += result.bytes_consumed;
        decoded.extend_from_slice(&output[..result.bytes_produced]);
        if result.status == Status::StreamEnd {
            return decoded;
        }
        assert!(result.bytes_consumed > 0 || result.bytes_produced > 0);
    }
}

fn assert_all_readers_reject(stream: &[u8]) {
    let mut output = Vec::new();
    assert_eq!(
        XzReader::new(stream, false)
            .read_to_end(&mut output)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidData
    );

    let mt_error = match XzReaderMt::new(Cursor::new(stream), false, 2) {
        Ok(mut reader) => reader.read_to_end(&mut Vec::new()).unwrap_err(),
        Err(error) => error,
    };
    assert_eq!(mt_error.kind(), ErrorKind::InvalidData);
    assert_eq!(stream_error(stream).kind(), ErrorKind::InvalidData);
}

fn block_with_declared_size(flag: u8) -> Vec<u8> {
    let mut writer = XzWriter::new(Vec::new(), XzOptions::with_preset(1)).unwrap();
    writer.write_all(b"x").unwrap();
    let mut stream = writer.finish().unwrap();
    let block_start = 12;
    let header_size = (usize::from(stream[block_start]) + 1) * 4;
    assert_eq!(header_size, 12);

    stream[block_start + 1] |= flag;
    stream.copy_within(block_start + 2..block_start + 5, block_start + 3);
    stream[block_start + 2] = 0;
    repair_block_header_crc(&mut stream);
    stream
}

#[test]
fn flush_writes_out_every_block() {
    let data = std::fs::read(PG100).unwrap();
    let block_size = 128 * 1024;

    for num_workers in [1, 4] {
        let mut options = XzOptions::with_preset(6);
        options.lzma_options.dict_size = block_size as u32;
        options.set_block_size(NonZeroU64::new(block_size));

        let sink = SharedSink::default();
        let mut writer = XzWriterMt::new(sink.clone(), options, num_workers).unwrap();

        let mut written = 0;

        for chunk in data[..900 * 1024].chunks(300 * 1024) {
            writer.write_all(chunk).unwrap();
            writer.flush().unwrap();
            written += chunk.len();

            let compressed = sink.0.lock().unwrap().clone();

            // We don't use assert_eq since the debug output would be too big.
            assert!(decode_blocks(&compressed) == data[..written]);
        }

        // Writing has to continue to work after the flushes.
        writer.finish().unwrap();

        let compressed = sink.0.lock().unwrap().clone();
        let mut uncompressed = Vec::new();
        XzReader::new(Cursor::new(compressed.as_slice()), false)
            .read_to_end(&mut uncompressed)
            .unwrap();

        assert!(uncompressed == data[..written]);
    }
}

fn test_round_trip(path: &str, level: u32) {
    let data = std::fs::read(path).unwrap();

    let mut options = XzOptions::with_preset(level);
    let dict_size = options.lzma_options.dict_size as u64;
    options.set_block_size(NonZeroU64::new(dict_size));

    let mut compressed = Vec::new();

    {
        let mut writer = XzWriterMt::new(&mut compressed, options, 2).unwrap();
        writer.write_all(&data).unwrap();
        writer.finish().unwrap();
    }

    let mut uncompressed = Vec::new();
    {
        let mut reader = XzReaderMt::new(Cursor::new(compressed.as_slice()), false, 2).unwrap();
        let data_len = reader.read_to_end(&mut uncompressed).unwrap();

        if dict_size < data_len as u64 {
            assert!(reader.block_count() > 1);
        }
    }

    // We don't use assert_eq since the debug output would be too big.
    assert!(uncompressed.as_slice() == data);

    // Also test decompression with liblzma to ensure compatibility
    let mut liblzma_uncompressed = Vec::new();
    {
        use liblzma::read::XzDecoder;
        let mut decoder = XzDecoder::new(compressed.as_slice());
        decoder.read_to_end(&mut liblzma_uncompressed).unwrap();
    }

    assert!(liblzma_uncompressed.as_slice() == data);
}

#[test]
fn empty_input_is_valid_empty_stream() {
    let mut options = XzOptions::with_preset(6);
    let dict_size = options.lzma_options.dict_size as u64;
    options.set_block_size(NonZeroU64::new(dict_size));

    let encoder = XzWriterMt::new(Vec::new(), options, 2).unwrap();
    let compressed = encoder.finish().unwrap();

    let reference = {
        use liblzma::write::XzEncoder;

        let encoder = XzEncoder::new(Vec::new(), 6);
        encoder.finish().unwrap()
    };

    assert_eq!(compressed, reference);

    let mut uncompressed = Vec::new();
    let mut reader = XzReaderMt::new(Cursor::new(compressed.as_slice()), false, 2).unwrap();
    reader.read_to_end(&mut uncompressed).unwrap();
    assert_eq!(reader.block_count(), 0);
    assert!(uncompressed.is_empty());

    let mut liblzma_uncompressed = Vec::new();
    {
        use liblzma::read::XzDecoder;
        let mut decoder = XzDecoder::new(compressed.as_slice());
        decoder.read_to_end(&mut liblzma_uncompressed).unwrap();
    }
    assert!(liblzma_uncompressed.is_empty());
}

#[test]
fn concatenated_streams_with_different_checks() {
    fn encode(data: &[u8], check_type: CheckType) -> Vec<u8> {
        let mut options = XzOptions::with_preset(1);
        options.check_type = check_type;
        let mut writer = XzWriter::new(Vec::new(), options).unwrap();
        writer.write_all(data).unwrap();
        writer.finish().unwrap()
    }

    let first = b"first stream";
    let second = b"second stream";
    let mut compressed = encode(first, CheckType::Crc32);
    compressed.extend_from_slice(&[0; 4]);
    let second_start = compressed.len() as u64;
    compressed.extend_from_slice(&encode(second, CheckType::Sha256));

    assert!(XzReaderMt::new(Cursor::new(&compressed), false, 2).is_err());

    let mut decoded = Vec::new();
    XzReaderMt::new(Cursor::new(&compressed), true, 2)
        .unwrap()
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, [first.as_slice(), second.as_slice()].concat());

    for allow_multiple_streams in [false, true] {
        let mut cursor = Cursor::new(&compressed);
        cursor.set_position(second_start);
        decoded.clear();
        XzReaderMt::new(cursor, allow_multiple_streams, 2)
            .unwrap()
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, second);
    }
}

#[test]
fn trailing_padding_must_be_a_multiple_of_four() {
    let mut writer = XzWriter::new(Vec::new(), XzOptions::with_preset(1)).unwrap();
    writer.write_all(b"payload").unwrap();
    let mut compressed = writer.finish().unwrap();
    compressed.push(0);

    let mut output = Vec::new();
    assert_eq!(
        XzReader::new(compressed.as_slice(), true)
            .read_to_end(&mut output)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
    assert_eq!(
        XzReaderMt::new(Cursor::new(&compressed), true, 2)
            .err()
            .unwrap()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn corrupt_block_header_crc_is_rejected_by_both_readers() {
    let mut writer = XzWriter::new(Vec::new(), XzOptions::with_preset(1)).unwrap();
    writer.write_all(b"payload").unwrap();
    let mut compressed = writer.finish().unwrap();
    let block_start = 12;
    let header_size = (usize::from(compressed[block_start]) + 1) * 4;
    compressed[block_start + header_size - 1] ^= 1;

    let mut output = Vec::new();
    assert_eq!(
        XzReader::new(compressed.as_slice(), false)
            .read_to_end(&mut output)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
    assert_eq!(
        XzReaderMt::new(Cursor::new(&compressed), false, 2)
            .unwrap()
            .read_to_end(&mut Vec::new())
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn block_header_crc_bytes_cannot_supply_missing_filter_properties() {
    let stream = [
        0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, 0x00, 0x01, 0x69, 0x22, 0xDE, 0x36, 0x01, 0x00, 0x21,
        0x01, 0x0C, 0x9D, 0x60, 0x62, 0x01, 0x00, 0x00, 0x78, 0x00, 0x00, 0x00, 0x00, 0x83, 0x16,
        0xDC, 0x8C, 0x00, 0x01, 0x11, 0x01, 0xAD, 0xA6, 0x58, 0x04, 0x90, 0x42, 0x99, 0x0D, 0x01,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x59, 0x5A,
    ];
    assert_all_readers_reject(&stream);
}

#[test]
fn nonzero_block_header_padding_is_rejected_with_a_valid_crc() {
    let mut writer = XzWriter::new(Vec::new(), XzOptions::with_preset(1)).unwrap();
    writer.write_all(b"x").unwrap();
    let mut stream = writer.finish().unwrap();
    stream[17] = 1;
    repair_block_header_crc(&mut stream);

    assert_all_readers_reject(&stream);
}

#[test]
fn optional_block_sizes_must_match_the_block_contents() {
    for flag in [0x40, 0x80] {
        assert_all_readers_reject(&block_with_declared_size(flag));
    }
}

#[test]
fn matching_optional_block_sizes_are_accepted_by_all_readers() {
    let stream = block_with_correct_declared_sizes();

    let mut output = Vec::new();
    XzReader::new(stream.as_slice(), false)
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, b"x");

    output.clear();
    XzReaderMt::new(Cursor::new(&stream), false, 2)
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, b"x");
    assert_eq!(decode_stream(&stream), b"x");
}

#[test]
fn bounded_parallel_decode_handles_large_uncompressed_block() {
    let mut state = 0x1234_5678u32;
    let input: Vec<u8> = (0..1 << 20)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let mut options = liblzma::stream::LzmaOptions::new_preset(1).unwrap();
    options.dict_size(4096);
    let mut filters = liblzma::stream::Filters::new();
    filters.lzma2(&options);
    let stream =
        liblzma::stream::Stream::new_stream_encoder(&filters, liblzma::stream::Check::None)
            .unwrap();
    let mut writer = liblzma::write::XzEncoder::new_stream(Vec::new(), stream);
    writer.write_all(&input).unwrap();
    let compressed = writer.finish().unwrap();

    let mut decoded = Vec::new();
    XzReaderMt::new_mem_limit(Cursor::new(&compressed), false, 2200, 1)
        .unwrap()
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, input);
}

#[test]
fn prefilters_are_applied_before_parallel_block_encoding() {
    let input = b"\xe8\x01\x00\x00\x00".repeat(1800);
    let mut options = XzOptions::with_preset(1);
    options.lzma_options.dict_size = 4096;
    options.set_block_size(NonZeroU64::new(4096));
    options.prepend_pre_filter(lzma_rust2::filter::FilterType::Delta, 1);
    options.prepend_pre_filter(lzma_rust2::filter::FilterType::BcjX86, 0);

    let mut writer = XzWriterMt::new(Vec::new(), options, 2).unwrap();
    writer.write_all(&input).unwrap();
    let compressed = writer.finish().unwrap();

    let mut decoded = Vec::new();
    XzReader::new(compressed.as_slice(), false)
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, input);

    decoded.clear();
    XzReaderMt::new(Cursor::new(compressed.as_slice()), false, 2)
        .unwrap()
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, input);

    decoded.clear();
    liblzma::read::XzDecoder::new(compressed.as_slice())
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, input);
}

#[test]
fn round_trip_executable_0() {
    test_round_trip(EXECUTABLE, 0);
}

#[test]
fn round_trip_executable_1() {
    test_round_trip(EXECUTABLE, 1);
}

#[test]
fn round_trip_executable_2() {
    test_round_trip(EXECUTABLE, 2);
}

#[test]
fn round_trip_executable_3() {
    test_round_trip(EXECUTABLE, 3);
}

#[test]
fn round_trip_executable_4() {
    test_round_trip(EXECUTABLE, 4);
}

#[test]
fn round_trip_executable_5() {
    test_round_trip(EXECUTABLE, 5);
}

#[test]
fn round_trip_executable_6() {
    test_round_trip(EXECUTABLE, 6);
}

#[test]
fn round_trip_executable_7() {
    test_round_trip(EXECUTABLE, 7);
}

#[test]
fn round_trip_executable_8() {
    test_round_trip(EXECUTABLE, 8);
}

#[test]
fn round_trip_executable_9() {
    test_round_trip(EXECUTABLE, 9);
}

#[test]
fn round_trip_pg100_0() {
    test_round_trip(PG100, 0);
}

#[test]
fn round_trip_pg100_1() {
    test_round_trip(PG100, 1);
}

#[test]
fn round_trip_pg100_2() {
    test_round_trip(PG100, 2);
}

#[test]
fn round_trip_pg100_3() {
    test_round_trip(PG100, 3);
}

#[test]
fn round_trip_pg100_4() {
    test_round_trip(PG100, 4);
}

#[test]
fn round_trip_pg100_5() {
    test_round_trip(PG100, 5);
}

#[test]
fn round_trip_pg100_6() {
    test_round_trip(PG100, 6);
}

#[test]
fn round_trip_pg100_7() {
    test_round_trip(PG100, 7);
}

#[test]
fn round_trip_pg100_8() {
    test_round_trip(PG100, 8);
}

#[test]
fn round_trip_pg100_9() {
    test_round_trip(PG100, 9);
}

#[test]
fn round_trip_pg6800_0() {
    test_round_trip(PG6800, 0);
}

#[test]
fn round_trip_pg6800_1() {
    test_round_trip(PG6800, 1);
}

#[test]
fn round_trip_pg6800_2() {
    test_round_trip(PG6800, 2);
}

#[test]
fn round_trip_pg6800_3() {
    test_round_trip(PG6800, 3);
}

#[test]
fn round_trip_pg6800_4() {
    test_round_trip(PG6800, 4);
}

#[test]
fn round_trip_pg6800_5() {
    test_round_trip(PG6800, 5);
}

#[test]
fn round_trip_pg6800_6() {
    test_round_trip(PG6800, 6);
}

#[test]
fn round_trip_pg6800_7() {
    test_round_trip(PG6800, 7);
}

#[test]
fn round_trip_pg6800_8() {
    test_round_trip(PG6800, 8);
}

#[test]
fn round_trip_pg6800_9() {
    test_round_trip(PG6800, 9);
}
