use std::{
    io::{self, Cursor, Read, Write},
    num::NonZeroU64,
};

use liblzma::stream::{Filters, Stream};
use lzma_rust2::{
    filter::FilterType, CheckType, EncodeMode, LzipOptions, LzipReader, LzipReaderMt, LzipWriter,
    LzipWriterMt, Lzma2Options, Lzma2Reader, Lzma2Writer, Lzma2WriterMt, LzmaOptions, LzmaReader,
    LzmaWriter, MfType, XzOptions, XzReader, XzWriter, XzWriterMt,
};

pub const HEADER_SIZE: usize = 12;
pub const MAX_INPUT_SIZE: usize = 64 * 1024;

fn options(data: &[u8], lzma2: bool) -> Option<(LzmaOptions, &[u8])> {
    if !(HEADER_SIZE..=HEADER_SIZE + MAX_INPUT_SIZE).contains(&data.len()) {
        return None;
    }
    let payload = &data[HEADER_SIZE..];
    let lc = u32::from(data[1]) % if lzma2 { 5 } else { 9 };
    let lp = u32::from(data[2]) % if lzma2 { 5 - lc } else { 5 };
    let mut options = LzmaOptions::new(
        4096 << (data[0] % 5),
        lc,
        lp,
        u32::from(data[3]) % 5,
        if data[4] & 1 == 0 {
            EncodeMode::Fast
        } else {
            EncodeMode::Normal
        },
        8 + u32::from(u16::from_le_bytes([data[5], data[6]])) % 266,
        if data[4] & 2 == 0 {
            MfType::Hc4
        } else {
            MfType::Bt4
        },
        [0, 4, 16, 64][usize::from(data[7] % 4)],
    );
    if data[4] & 4 != 0 {
        options.preset_dict = Some(payload[..payload.len().min(4096)].to_vec());
    }
    Some((options, payload))
}

// An unexpected encoder expansion fails without accumulating unbounded output.
struct BoundedOutput {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedOutput {
    fn new(input_size: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit: input_size * 8 + 1024,
        }
    }
}

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::other("fuzz encoder output limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FragmentedInput<'a> {
    bytes: &'a [u8],
    chunk_size: usize,
}

impl Read for FragmentedInput<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(self.chunk_size).min(self.bytes.len());
        output[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        Ok(count)
    }
}

fn write_input(writer: &mut impl Write, payload: &[u8], data: &[u8], max_flushes: usize) {
    let chunk_size = [1, 7, 273, 4096, MAX_INPUT_SIZE][usize::from(data[8] % 5)];
    let flush_interval = usize::from(data[9]);
    let mut flushes = 0;
    writer.write_all(&[]).unwrap();
    for (index, chunk) in payload.chunks(chunk_size).enumerate() {
        writer.write_all(chunk).unwrap();
        if flush_interval != 0 && (index + 1) % flush_interval == 0 && flushes < max_flushes {
            writer.flush().unwrap();
            flushes += 1;
        }
    }
    writer.flush().unwrap();
}

fn check_output(mut reader: impl Read, expected: &[u8], data: &[u8]) {
    let size = [1, 7, 273, 4096][usize::from(data[11] % 4)];
    let mut buffer = [0; 4096];
    let mut offset = 0;
    loop {
        let count = reader.read(&mut buffer[..size]).unwrap_or_else(|error| {
            panic!(
                "decoder failed after {offset}/{} bytes: {error}",
                expected.len()
            )
        });
        if count == 0 {
            break;
        }
        assert!(
            count <= expected.len() - offset,
            "decoder exceeded expected output"
        );
        assert_eq!(&buffer[..count], &expected[offset..offset + count]);
        offset += count;
    }
    assert_eq!(offset, expected.len(), "decoder ended before the payload");
}

fn input<'a>(compressed: &'a [u8], data: &[u8]) -> FragmentedInput<'a> {
    FragmentedInput {
        bytes: compressed,
        chunk_size: [1, 7, 273, 4096][usize::from(data[10] % 4)],
    }
}

fn reference_stream(options: &LzmaOptions, lzma2: bool) -> Stream {
    let mut reference = liblzma::stream::LzmaOptions::new_preset(0).unwrap();
    reference
        .dict_size(options.dict_size)
        .literal_context_bits(options.lc)
        .literal_position_bits(options.lp)
        .position_bits(options.pb);
    let mut filters = Filters::new();
    if lzma2 {
        filters.lzma2(&reference);
    } else {
        filters.lzma1(&reference);
    }
    Stream::new_raw_decoder(&filters).unwrap()
}

// Each fuzz binary uses one of these entry points; the integration test uses both.
#[allow(dead_code)]
pub fn lzma_roundtrip(data: &[u8]) -> Option<()> {
    let (options, payload) = options(data, false)?;
    let header = data[4] & 8 != 0 && options.preset_dict.is_none();
    let known_size = data[4] & 16 != 0;
    let end_marker = !known_size || data[4] & 32 != 0;
    let mut writer = LzmaWriter::new(
        BoundedOutput::new(payload.len()),
        &options,
        header,
        end_marker,
        known_size.then_some(payload.len() as u64),
    )
    .unwrap();
    write_input(&mut writer, payload, data, usize::MAX);
    let compressed = writer.finish().unwrap().bytes;
    let reader = if header {
        LzmaReader::new_mem_limit(input(&compressed, data), 16 * 1024, None).unwrap()
    } else {
        LzmaReader::new_with_props(
            input(&compressed, data),
            if known_size {
                payload.len() as u64
            } else {
                u64::MAX
            },
            options.get_props(),
            options.dict_size,
            options.preset_dict.as_deref(),
        )
        .unwrap()
    };
    check_output(reader, payload, data);

    // liblzma supports lc + lp <= 4 and this wrapper has no preset-dictionary setter.
    // Its raw LZMA1 decoder also needs an end marker. Library-only cases still
    // exercise larger valid properties, preset dictionaries, and size-delimited data.
    if options.lc + options.lp <= 4 && options.preset_dict.is_none() && (header || end_marker) {
        let stream = if header {
            Stream::new_lzma_decoder(16 * 1024 * 1024).unwrap()
        } else {
            reference_stream(&options, false)
        };
        check_output(
            liblzma::read::XzDecoder::new_stream(compressed.as_slice(), stream),
            payload,
            data,
        );
    }
    Some(())
}

#[allow(dead_code)]
pub fn lzma2_roundtrip(data: &[u8]) -> Option<()> {
    let (options, payload) = options(data, true)?;
    let chunk_size = match data[10] % 3 {
        0 => None,
        1 => NonZeroU64::new(u64::from(options.dict_size)),
        _ => NonZeroU64::new(u64::from(options.dict_size) * 2),
    };
    let mut writer = Lzma2Writer::new(
        BoundedOutput::new(payload.len()),
        Lzma2Options {
            lzma_options: options.clone(),
            chunk_size,
        },
    );
    write_input(&mut writer, payload, data, usize::MAX);
    let compressed = writer.finish().unwrap().bytes;
    if options.preset_dict.is_none() {
        check_output(
            liblzma::read::XzDecoder::new_stream(
                compressed.as_slice(),
                reference_stream(&options, true),
            ),
            payload,
            data,
        );
    }
    let reader = Lzma2Reader::new_mem_limit(
        input(&compressed, data),
        options.dict_size,
        16 * 1024,
        options.preset_dict.as_deref(),
    )
    .unwrap();
    check_output(reader, payload, data);
    Some(())
}

#[allow(dead_code)]
pub fn lzma2_mt_roundtrip(data: &[u8]) -> Option<()> {
    let (mut options, payload) = options(data, true)?;
    // Each worker starts a fresh block, so preset dictionaries are not used.
    options.preset_dict = None;
    let chunk_size = u64::from(options.dict_size) * if data[10] & 4 == 0 { 1 } else { 2 };
    let workers = [1, 2, 4][usize::from(data[11] % 3)];
    let mut writer = Lzma2WriterMt::new(
        BoundedOutput::new(payload.len()),
        Lzma2Options {
            lzma_options: options.clone(),
            chunk_size: NonZeroU64::new(chunk_size),
        },
        workers,
    )
    .unwrap();
    // A flush dispatches a block; cap them so one-byte writes stay cheap to fuzz.
    write_input(&mut writer, payload, data, 16);
    let compressed = writer.finish().unwrap().bytes;
    check_output(
        liblzma::read::XzDecoder::new_stream(
            compressed.as_slice(),
            reference_stream(&options, true),
        ),
        payload,
        data,
    );
    let reader =
        Lzma2Reader::new_mem_limit(input(&compressed, data), options.dict_size, 16 * 1024, None)
            .unwrap();
    check_output(reader, payload, data);
    Some(())
}

fn container_options(data: &[u8]) -> Option<(LzmaOptions, &[u8])> {
    let (mut options, payload) = options(data, true)?;
    // Container formats do not carry an external preset dictionary.
    options.preset_dict = None;
    Some((options, payload))
}

fn check_xz(compressed: &[u8], payload: &[u8], data: &[u8]) {
    check_output(XzReader::new(input(compressed, data), false), payload, data);
    check_output(liblzma::read::XzDecoder::new(compressed), payload, data);
}

fn set_xz_filters(options: &mut XzOptions, data: &[u8]) {
    match data[7] >> 4 {
        1 => options.prepend_pre_filter(FilterType::Delta, 1 + u32::from(data[6] % 32)),
        2 => options.prepend_pre_filter(FilterType::BcjX86, 0),
        3 => {
            options.prepend_pre_filter(FilterType::Delta, 1 + u32::from(data[6] % 32));
            options.prepend_pre_filter(FilterType::BcjX86, 0);
        }
        4 => options.prepend_pre_filter(FilterType::BcjPpc, 0),
        5 => options.prepend_pre_filter(FilterType::BcjIa64, 0),
        6 => options.prepend_pre_filter(FilterType::BcjArm, 0),
        7 => options.prepend_pre_filter(FilterType::BcjArmThumb, 0),
        8 => options.prepend_pre_filter(FilterType::BcjSparc, 0),
        9 => options.prepend_pre_filter(FilterType::BcjArm64, 0),
        10 => options.prepend_pre_filter(FilterType::BcjRiscv, 0),
        _ => {}
    }
}

#[allow(dead_code)]
pub fn xz_roundtrip(data: &[u8]) -> Option<()> {
    let (lzma_options, payload) = container_options(data)?;
    let dict_size = u64::from(lzma_options.dict_size);
    let mut options = XzOptions {
        lzma_options,
        check_type: [
            CheckType::None,
            CheckType::Crc32,
            CheckType::Crc64,
            CheckType::Sha256,
        ][usize::from(data[7] % 4)],
        block_size: match data[10] % 3 {
            0 => None,
            1 => NonZeroU64::new(dict_size),
            _ => NonZeroU64::new(dict_size * 2),
        },
        filters: Vec::new(),
    };
    set_xz_filters(&mut options, data);
    let mut writer = XzWriter::new(BoundedOutput::new(payload.len()), options).unwrap();
    write_input(&mut writer, payload, data, 16);
    let compressed = writer.finish().unwrap().bytes;
    check_xz(&compressed, payload, data);
    Some(())
}

#[allow(dead_code)]
pub fn xz_mt_roundtrip(data: &[u8]) -> Option<()> {
    let (lzma_options, payload) = container_options(data)?;
    let dict_size = u64::from(lzma_options.dict_size);
    let mut options = XzOptions {
        lzma_options,
        check_type: [
            CheckType::None,
            CheckType::Crc32,
            CheckType::Crc64,
            CheckType::Sha256,
        ][usize::from(data[7] % 4)],
        block_size: NonZeroU64::new(dict_size),
        filters: Vec::new(),
    };
    set_xz_filters(&mut options, data);
    let workers = [1, 2, 4][usize::from(data[11] % 3)];
    let mut writer = XzWriterMt::new(BoundedOutput::new(payload.len()), options, workers).unwrap();
    write_input(&mut writer, payload, data, 16);
    let compressed = writer.finish().unwrap().bytes;
    check_xz(&compressed, payload, data);
    Some(())
}

fn check_lzip(compressed: &[u8], payload: &[u8], data: &[u8]) {
    check_output(LzipReader::new(input(compressed, data)), payload, data);
    let reader = LzipReaderMt::new_mem_limit(Cursor::new(compressed), 16 * 1024 * 1024, 2).unwrap();
    check_output(reader, payload, data);
}

#[allow(dead_code)]
pub fn lzip_roundtrip(data: &[u8]) -> Option<()> {
    let (lzma_options, payload) = container_options(data)?;
    let member_size = match data[10] % 3 {
        0 => None,
        1 => NonZeroU64::new(u64::from(lzma_options.dict_size)),
        _ => NonZeroU64::new(u64::from(lzma_options.dict_size) * 2),
    };
    let mut writer = LzipWriter::new(
        BoundedOutput::new(payload.len()),
        LzipOptions {
            lzma_options,
            member_size,
        },
    );
    write_input(&mut writer, payload, data, 16);
    let compressed = writer.finish().unwrap().bytes;
    check_lzip(&compressed, payload, data);
    Some(())
}

#[allow(dead_code)]
pub fn lzip_mt_roundtrip(data: &[u8]) -> Option<()> {
    let (lzma_options, payload) = container_options(data)?;
    let member_size =
        NonZeroU64::new(u64::from(lzma_options.dict_size) * if data[10] & 4 == 0 { 1 } else { 2 });
    let workers = [1, 2, 4][usize::from(data[11] % 3)];
    let mut writer = LzipWriterMt::new(
        BoundedOutput::new(payload.len()),
        LzipOptions {
            lzma_options,
            member_size,
        },
        workers,
    )
    .unwrap();
    write_input(&mut writer, payload, data, 16);
    let compressed = writer.finish().unwrap().bytes;
    check_lzip(&compressed, payload, data);
    Some(())
}
