#[path = "../fuzz_targets/lzma2_window.rs"]
mod lzma2_window;

use std::{
    io::{Cursor, Read, Write},
    num::NonZeroU64,
};

use lzma_rust2::{Lzma2Options, Lzma2ReaderMt, Lzma2WriterMt, XzOptions, XzReaderMt, XzWriterMt};

#[test]
fn long_incompressible_input_round_trips_with_small_dictionaries() {
    for settings in [
        [0, 0, 0, 1],
        [64, 1, 2, 2],
        [128, 2, 4, 3],
        [255, 3, 7, 4],
        [0, 0, 16, 1],
        [128, 2, 20, 3],
    ] {
        assert!(lzma2_window::roundtrip(&settings).is_some());
    }
}

#[test]
fn rejects_inputs_outside_the_target_limits() {
    assert!(lzma2_window::roundtrip(&[0; 3]).is_none());
    assert!(lzma2_window::roundtrip(&vec![0; lzma2_window::MAX_INPUT_SIZE + 1]).is_none());
}

#[test]
fn threaded_window_writers_emit_multiple_independent_units() {
    let dict_size = 4 * 1024;
    let payload: Vec<_> = (0..3 * dict_size + 1).map(|i| (i * 73) as u8).collect();

    let mut lzma2_options = Lzma2Options::with_preset(1);
    lzma2_options.lzma_options.dict_size = dict_size as u32;
    lzma2_options.chunk_size = NonZeroU64::new(dict_size as u64);
    let mut lzma2_writer = Lzma2WriterMt::new(Vec::new(), lzma2_options, 2).unwrap();
    lzma2_writer.write_all(&payload).unwrap();
    let lzma2 = lzma2_writer.finish().unwrap();
    let mut lzma2_reader = Lzma2ReaderMt::new(Cursor::new(lzma2), dict_size as u32, None, 2);
    let mut lzma2_decoded = Vec::new();
    lzma2_reader.read_to_end(&mut lzma2_decoded).unwrap();
    assert_eq!(lzma2_decoded, payload);
    assert!(lzma2_reader.chunk_count() >= 2);

    let mut xz_options = XzOptions::with_preset(1);
    xz_options.lzma_options.dict_size = dict_size as u32;
    xz_options.block_size = NonZeroU64::new(dict_size as u64);
    let mut xz_writer = XzWriterMt::new(Vec::new(), xz_options, 2).unwrap();
    xz_writer.write_all(&payload).unwrap();
    let xz = xz_writer.finish().unwrap();
    let mut xz_reader = XzReaderMt::new(Cursor::new(xz), false, 2).unwrap();
    let mut xz_decoded = Vec::new();
    xz_reader.read_to_end(&mut xz_decoded).unwrap();
    assert_eq!(xz_decoded, payload);
    assert!(xz_reader.block_count() >= 2);
}
