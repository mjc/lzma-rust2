//! Generate reproducible inputs for the encoder round-trip targets.

use std::{fs, path::Path};

#[path = "../regression_input.rs"]
mod regression_input;
#[path = "../valid_streams.rs"]
#[allow(dead_code)]
mod valid_streams;

fn main() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");

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

    for target in [
        "lzma_roundtrip",
        "lzma2_roundtrip",
        "lzma2_mt_roundtrip",
        "xz_roundtrip",
        "xz_mt_roundtrip",
        "lzip_roundtrip",
        "lzip_mt_roundtrip",
    ] {
        let directory = root.join(target);
        fs::create_dir_all(&directory)?;
        for (name, data) in &seeds {
            fs::write(directory.join(name), data)?;
        }
        if target.starts_with("xz_") {
            for filter in 1..=10 {
                let mut settings = fragmented;
                settings[7] = filter << 4 | 2;
                settings[10] = 1;
                fs::write(
                    directory.join(format!("filter-{filter}")),
                    [
                        settings.as_slice(),
                        b"\xe8\x01\x00\x00\x00".repeat(1200).as_slice(),
                    ]
                    .concat(),
                )?;
            }
        }
        if target.starts_with("lzma2_") {
            fs::write(
                directory.join("reset-after-uncompressed"),
                regression_input::reset_after_uncompressed(),
            )?;
        }
    }

    let directory = root.join("lzma2_window_roundtrip");
    fs::create_dir_all(&directory)?;
    for (name, settings) in [
        ("raw-small-dict", [0, 0, 0, 1]),
        ("raw-fragmented", [64, 1, 2, 2]),
        ("xz-small-dict", [128, 2, 4, 3]),
        ("xz-patterned", [255, 3, 15, 4]),
        ("raw-threaded", [0, 0, 16, 1]),
        ("xz-threaded", [128, 2, 20, 3]),
    ] {
        fs::write(directory.join(name), settings)?;
    }

    let directory = root.join("bcj2_roundtrip");
    fs::create_dir_all(&directory)?;
    let payload = valid_streams::bcj2_payload(16_384);
    for (name, header) in [
        ("one-byte-chunks", [0, 0, 0]),
        ("known-size-flushed", [5, 1, 1]),
        ("large-chunks", [17, 3, 0]),
    ] {
        fs::write(
            directory.join(name),
            [header.as_slice(), payload.as_slice()].concat(),
        )?;
    }
    Ok(())
}
