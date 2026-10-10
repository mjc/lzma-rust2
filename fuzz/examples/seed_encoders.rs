//! Generate reproducible inputs for both encoder round-trip targets.

use std::{fs, path::Path};

#[path = "../regression_input.rs"]
mod regression_input;

fn main() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");

    // Twelve settings bytes precede each payload; see README.md for the mapping.
    let fragmented = [0, 3, 0, 2, 3, 24, 0, 0, 1, 3, 1, 1];
    let preset = [0, 3, 0, 2, 7, 24, 0, 0, 2, 3, 1, 2];
    let known_size = [0, 3, 0, 2, 27, 24, 0, 0, 2, 0, 0, 3];
    let maximum = [4, 8, 4, 4, 1, 9, 1, 3, 4, 0, 2, 3];

    let mut noise = Vec::with_capacity(16_385);
    let mut state = 0x1234_5678_u32;
    for _ in 0..16_385 {
        state ^= state.wrapping_shl(13);
        state ^= state >> 17;
        state ^= state.wrapping_shl(5);
        noise.push(state as u8);
    }

    let seeds = [
        ("empty", fragmented.to_vec()),
        ("one-byte", [fragmented.as_slice(), b"x"].concat()),
        (
            "repeated",
            [fragmented.as_slice(), b"abcabcabcd".repeat(900).as_slice()].concat(),
        ),
        (
            "preset",
            [preset.as_slice(), b"dictionary".repeat(900).as_slice()].concat(),
        ),
        (
            "known-size",
            [
                known_size.as_slice(),
                b"size-delimited".repeat(600).as_slice(),
            ]
            .concat(),
        ),
        (
            "maximum",
            [maximum.as_slice(), vec![0x90; 65_536].as_slice()].concat(),
        ),
        (
            "incompressible",
            [fragmented.as_slice(), noise.as_slice()].concat(),
        ),
    ];

    for target in ["lzma_roundtrip", "lzma2_roundtrip", "lzma2_mt_roundtrip"] {
        let directory = root.join(target);
        fs::create_dir_all(&directory)?;
        for (name, data) in &seeds {
            fs::write(directory.join(name), data)?;
        }
        if target != "lzma_roundtrip" {
            fs::write(
                directory.join("reset-after-uncompressed"),
                regression_input::reset_after_uncompressed(),
            )?;
        }
    }
    Ok(())
}
