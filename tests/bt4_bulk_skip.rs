#![cfg(all(feature = "std", feature = "encoder", feature = "xz"))]

use std::io::{Read, Write};

use lzma_rust2::{EncodeMode, Lzma2Options, Lzma2Reader, Lzma2Writer, MfType};
use sha2::{Digest, Sha256};

fn sha256_hex(input: &[u8]) -> String {
    Sha256::digest(input)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn lifecycle_payload() -> Vec<u8> {
    let mut input = Vec::new();
    for (segment, period) in [5usize, 31, 127, 3].into_iter().enumerate() {
        for i in 0..72_000 {
            input.push((i % period) as u8 + (segment as u8 * 37));
        }
    }
    input
}

#[test]
fn bt4_continuation_preserves_stream_across_flushes_and_preset_dictionary() {
    let input = lifecycle_payload();
    let preset = input[..4096].to_vec();
    let mut options = Lzma2Options::with_preset(5);
    options.lzma_options.dict_size = 4096;
    options.lzma_options.nice_len = 32;
    options.lzma_options.preset_dict = Some(preset.clone());
    assert_eq!(options.lzma_options.mode, EncodeMode::Normal);
    assert_eq!(options.lzma_options.mf, MfType::Bt4);
    let mut writer = Lzma2Writer::new(Vec::new(), options);
    let mut offset = 0;
    for (index, chunk_size) in [13, 997, 79, 1024, 37, 311].into_iter().cycle().enumerate() {
        if offset == input.len() {
            break;
        }
        let end = (offset + chunk_size).min(input.len());
        let part = &input[offset..end];
        writer.write_all(part).unwrap();
        if index % 4 == 2 {
            writer.flush().unwrap();
        }
        offset = end;
    }
    let compressed = writer.finish().unwrap();
    assert_eq!(
        sha256_hex(&compressed),
        "bbb06f06b5ea9fd195b3bf5617798ef4a482a9ca907dc0e1f089d3896cfb147c"
    );
    let mut decoded = Vec::new();
    Lzma2Reader::new(compressed.as_slice(), 4096, Some(&preset))
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, input);
}
