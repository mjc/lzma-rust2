#[path = "../fuzz_targets/encoder_roundtrip.rs"]
mod encoder_roundtrip;

#[path = "../regression_input.rs"]
mod regression_input;

use encoder_roundtrip::{
    lzma2_mt_roundtrip, lzma2_roundtrip, lzma_roundtrip, HEADER_SIZE, MAX_INPUT_SIZE,
};

fn case(settings: [u8; HEADER_SIZE], payload: &[u8]) -> Vec<u8> {
    [settings.as_slice(), payload].concat()
}

#[test]
fn fragmented_encodes_round_trip_across_modes_and_match_finders() {
    let payloads = [
        Vec::new(),
        vec![0],
        b"abcabcabcd".repeat(900),
        (0..8193).map(|i| (i * 71) as u8).collect(),
    ];
    for mode_and_finder in 0..4 {
        for flags in [0, 4, 8, 24] {
            for payload in &payloads {
                let input = case(
                    [0, 3, 0, 2, mode_and_finder | flags, 8, 0, 0, 1, 1, 1, 0],
                    payload,
                );
                assert!(lzma_roundtrip(&input).is_some());
                assert!(lzma2_roundtrip(&input).is_some());
            }
        }
    }
}

#[test]
fn extreme_properties_and_full_payload_round_trip() {
    let payload = vec![0x90; MAX_INPUT_SIZE];
    let input = case([4, 8, 4, 4, 1, 9, 1, 3, 4, 0, 2, 4], &payload);
    assert!(lzma_roundtrip(&input).is_some());
    assert!(lzma2_roundtrip(&input).is_some());
}

#[test]
fn tiny_writes_flushes_and_independent_chunks_round_trip() {
    let payload: Vec<_> = (0..16385).map(|i| (i * 17 + i / 251) as u8).collect();
    let input = case([0, 0, 0, 0, 0, 255, 0, 1, 0, 1, 1, 0], &payload);
    assert!(lzma_roundtrip(&input).is_some());
    assert!(lzma2_roundtrip(&input).is_some());
}

#[test]
fn rejects_inputs_outside_the_harness_limits() {
    assert!(lzma_roundtrip(&[0; HEADER_SIZE - 1]).is_none());
    assert!(lzma2_roundtrip(&[0; HEADER_SIZE - 1]).is_none());
    assert!(lzma2_mt_roundtrip(&[0; HEADER_SIZE - 1]).is_none());
    let oversized = vec![0; HEADER_SIZE + MAX_INPUT_SIZE + 1];
    assert!(lzma_roundtrip(&oversized).is_none());
    assert!(lzma2_roundtrip(&oversized).is_none());
    assert!(lzma2_mt_roundtrip(&oversized).is_none());
}

#[test]
fn known_size_header_with_end_marker_round_trips() {
    let input = [0, 3, 0, 63, 57, 0, 0, 0, 0, 0, 0, 254, 145];
    assert!(lzma_roundtrip(&input).is_some());
}

#[test]
fn independent_lzma2_chunk_with_uncompressed_prefix_keeps_dictionary_in_sync() {
    let input = regression_input::reset_after_uncompressed();
    assert!(lzma2_roundtrip(&input).is_some());
    assert!(lzma2_mt_roundtrip(input).is_some());
}

#[test]
fn threaded_lzma2_round_trips_across_block_boundaries() {
    let payload: Vec<_> = (0..3 * 4096 + 137)
        .map(|i| (i * 17 + i / 251) as u8)
        .collect();
    for workers in 0..3 {
        let input = case([0, 3, 0, 2, 1, 24, 0, 0, 2, 2, 1, workers], &payload);
        assert!(lzma2_mt_roundtrip(&input).is_some());
    }
}
