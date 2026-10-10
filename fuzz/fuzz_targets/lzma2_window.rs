use std::{io::{self, Read, Write}, num::NonZeroU64};

use liblzma::stream::{Filters, Stream};
use lzma_rust2::{
    Lzma2Options, Lzma2Reader, Lzma2Writer, Lzma2WriterMt, XzOptions, XzReader, XzWriter,
    XzWriterMt,
};

pub const MAX_INPUT_SIZE: usize = 4096;

struct BoundedOutput {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::other("encoder output limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn decode(mut reader: impl Read, expected: &[u8]) {
    let mut output = Vec::new();
    reader
        .by_ref()
        .take(expected.len() as u64 + 1)
        .read_to_end(&mut output)
        .unwrap();
    assert_eq!(output, expected);
    assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
}

pub fn roundtrip(data: &[u8]) -> Option<()> {
    if !(4..=MAX_INPUT_SIZE).contains(&data.len()) {
        return None;
    }

    let len = 384 * 1024 + usize::from(data[0]) * 2048;
    let dict_size = 4096 << (data[1] % 4);
    let mut state = u32::from_le_bytes([data[0], data[1], data[2], data[3]]).max(1);
    let mut payload = Vec::with_capacity(len);
    for i in 0..len {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let mut byte = (state as u8) ^ data[i % data.len()];
        if data[2] & 8 != 0 && i % (64 * 1024) < 4096 {
            byte = data[i % data.len()];
        }
        payload.push(byte);
    }

    let mut options = Lzma2Options::with_preset(if data[2] & 1 == 0 { 1 } else { 6 });
    options.lzma_options.dict_size = dict_size;
    let output = BoundedOutput {
        bytes: Vec::new(),
        limit: len * 2 + 4096,
    };
    let chunk_size = if data[2] & 2 == 0 { len } else { 8192 };

    if data[2] & 4 == 0 {
        let compressed = if data[2] & 16 == 0 {
            let mut writer = Lzma2Writer::new(output, options);
            for chunk in payload.chunks(chunk_size) {
                writer.write_all(chunk).unwrap();
            }
            writer.finish().unwrap().bytes
        } else {
            options.chunk_size = NonZeroU64::new(len as u64);
            let mut writer = Lzma2WriterMt::new(output, options, 2).unwrap();
            for chunk in payload.chunks(chunk_size) {
                writer.write_all(chunk).unwrap();
            }
            writer.finish().unwrap().bytes
        };
        decode(
            Lzma2Reader::new(compressed.as_slice(), dict_size, None),
            &payload,
        );

        let mut reference_options = liblzma::stream::LzmaOptions::new_preset(1).unwrap();
        reference_options.dict_size(dict_size);
        let mut filters = Filters::new();
        filters.lzma2(&reference_options);
        let stream = Stream::new_raw_decoder(&filters).unwrap();
        decode(
            liblzma::read::XzDecoder::new_stream(compressed.as_slice(), stream),
            &payload,
        );
    } else {
        let mut xz_options = XzOptions::with_preset(1);
        xz_options.lzma_options = options.lzma_options;
        let compressed = if data[2] & 16 == 0 {
            let mut writer = XzWriter::new(output, xz_options).unwrap();
            for chunk in payload.chunks(chunk_size) {
                writer.write_all(chunk).unwrap();
            }
            writer.finish().unwrap().bytes
        } else {
            xz_options.block_size = NonZeroU64::new(len as u64);
            let mut writer = XzWriterMt::new(output, xz_options, 2).unwrap();
            for chunk in payload.chunks(chunk_size) {
                writer.write_all(chunk).unwrap();
            }
            writer.finish().unwrap().bytes
        };
        decode(XzReader::new(compressed.as_slice(), false), &payload);
        decode(liblzma::read::XzDecoder::new(compressed.as_slice()), &payload);
    }
    Some(())
}
