#[path = "../fuzz_targets/lzma2_window.rs"]
mod lzma2_window;

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
