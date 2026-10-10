#[path = "../fuzz_targets/decoder_mt.rs"]
mod decoder_mt;

#[path = "../valid_streams.rs"]
mod valid_streams;

use decoder_mt::{lzip_mt_decode, lzma2_mt_decode, xz_mt_decode, HEADER_SIZE, MAX_INPUT_SIZE};
use lzma_rust2::CheckType;

fn input(selector: u8, stream: &[u8]) -> Vec<u8> {
    [[selector, selector.rotate_left(3)].as_slice(), stream].concat()
}

#[test]
fn parallel_readers_decode_valid_multiblock_streams() {
    let payload = valid_streams::payload(3 * 4096 + 137);
    let lzma2 = valid_streams::lzma2(&payload);
    let lzip = valid_streams::lzip(&payload);

    for selector in 0..8 {
        assert!(lzma2_mt_decode(&input(selector, &lzma2)).is_some());
        assert!(lzip_mt_decode(&input(selector, &lzip)).is_some());
        for check_type in [
            CheckType::None,
            CheckType::Crc32,
            CheckType::Crc64,
            CheckType::Sha256,
        ] {
            let xz = valid_streams::xz(&payload, check_type);
            assert!(xz_mt_decode(&input(selector, &xz)).is_some());
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
            lzma2_mt_decode(&candidate);
            xz_mt_decode(&candidate);
            lzip_mt_decode(&candidate);
        }
        for offset in [0, stream.len() / 3, stream.len() / 2, stream.len() - 1] {
            stream[offset] ^= 0x80;
            let candidate = input(offset as u8, &stream);
            lzma2_mt_decode(&candidate);
            xz_mt_decode(&candidate);
            lzip_mt_decode(&candidate);
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
