use std::io::{self, Cursor, Read};

use lzma_rust2::{LzipReader, LzipReaderMt, Lzma2Reader, Lzma2ReaderMt, XzReader, XzReaderMt};

pub const HEADER_SIZE: usize = 2;
pub const MAX_INPUT_SIZE: usize = 64 * 1024;

const MAX_OUTPUT_SIZE: usize = 256 * 1024;
const MEM_LIMIT_KB: u32 = 8 * 1024;
const READ_SIZES: [usize; 4] = [1, 7, 64, 4096];
const WORKERS: [u32; 2] = [1, 2];

fn split_input(data: &[u8]) -> Option<(&[u8], &[u8])> {
    if !(HEADER_SIZE..=HEADER_SIZE + MAX_INPUT_SIZE).contains(&data.len()) {
        return None;
    }
    Some(data.split_at(HEADER_SIZE))
}

fn read_bounded(mut reader: impl Read, selector: u8) -> io::Result<Option<Vec<u8>>> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 4096];
    let read_size = READ_SIZES[usize::from(selector) % READ_SIZES.len()];

    loop {
        let count = reader.read(&mut buffer[..read_size])?;
        if count == 0 {
            assert_eq!(reader.read(&mut buffer[..read_size])?, 0);
            return Ok(Some(output));
        }
        if output.len().saturating_add(count) > MAX_OUTPUT_SIZE {
            return Ok(None);
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

fn compare_successes(
    serial: io::Result<Option<Vec<u8>>>,
    parallel: io::Result<Option<Vec<u8>>>,
    strict: bool,
) {
    match (serial, parallel) {
        (Ok(Some(serial)), Ok(Some(parallel))) => assert_eq!(parallel, serial),
        (Ok(None), _) | (_, Ok(None)) | (Err(_), Err(_)) => {}
        (Ok(Some(_)), Err(error)) | (Err(error), Ok(Some(_)))
            if error.kind() == io::ErrorKind::OutOfMemory => {}
        (serial, parallel) if strict => {
            panic!("decoder acceptance differs: {serial:?} vs {parallel:?}")
        }
        _ => {}
    }
}

#[allow(dead_code)]
pub fn lzma2_mt_decode(data: &[u8]) -> Option<()> {
    lzma2_mt_decode_with_mode(data, true)
}

#[allow(dead_code)]
pub fn lzma2_mt_decode_tolerant(data: &[u8]) -> Option<()> {
    lzma2_mt_decode_with_mode(data, false)
}

fn lzma2_mt_decode_with_mode(data: &[u8], strict: bool) -> Option<()> {
    let (header, stream) = split_input(data)?;
    let dict_size = [4 * 1024, 8 * 1024, 16 * 1024, 64 * 1024][usize::from(header[0] >> 1) % 4];
    let workers = WORKERS[usize::from(header[0]) % WORKERS.len()];

    let serial = Lzma2Reader::new_mem_limit(stream, dict_size, MEM_LIMIT_KB, None)
        .and_then(|reader| read_bounded(reader, header[1]));
    let parallel = read_bounded(
        Lzma2ReaderMt::new(Cursor::new(stream), dict_size, None, workers),
        header[1],
    );
    compare_successes(serial, parallel, strict);
    Some(())
}

#[allow(dead_code)]
pub fn xz_mt_decode(data: &[u8]) -> Option<()> {
    xz_mt_decode_with_mode(data, true)
}

#[allow(dead_code)]
pub fn xz_mt_decode_tolerant(data: &[u8]) -> Option<()> {
    xz_mt_decode_with_mode(data, false)
}

fn xz_mt_decode_with_mode(data: &[u8], strict: bool) -> Option<()> {
    let (header, stream) = split_input(data)?;
    let workers = WORKERS[usize::from(header[0]) % WORKERS.len()];
    let allow_multiple_streams = header[0] & 2 != 0;

    let serial = read_bounded(
        XzReader::new_mem_limit(stream, allow_multiple_streams, MEM_LIMIT_KB),
        header[1],
    );
    let parallel = XzReaderMt::new_mem_limit(
        Cursor::new(stream),
        allow_multiple_streams,
        MEM_LIMIT_KB,
        workers,
    )
    .and_then(|reader| read_bounded(reader, header[1]));
    compare_successes(serial, parallel, strict && allow_multiple_streams);
    Some(())
}

#[allow(dead_code)]
pub fn lzip_mt_decode(data: &[u8]) -> Option<()> {
    lzip_mt_decode_with_mode(data, true)
}

#[allow(dead_code)]
pub fn lzip_mt_decode_tolerant(data: &[u8]) -> Option<()> {
    lzip_mt_decode_with_mode(data, false)
}

fn lzip_mt_decode_with_mode(data: &[u8], strict: bool) -> Option<()> {
    let (header, stream) = split_input(data)?;
    let workers = WORKERS[usize::from(header[0]) % WORKERS.len()];

    let parallel = LzipReaderMt::new_mem_limit(Cursor::new(stream), MEM_LIMIT_KB, workers)
        .and_then(|reader| read_bounded(reader, header[1]));
    let serial = read_bounded(
        LzipReader::new_mem_limit(Cursor::new(stream), MEM_LIMIT_KB),
        header[1],
    );
    compare_successes(serial, parallel, strict);
    Some(())
}
