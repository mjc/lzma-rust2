use std::{
    env,
    error::Error,
    io::{Read, Write},
    time::Instant,
};

use lzma_rust2::{Lzma2Options, Lzma2Reader, Lzma2Writer};
use sha2::{Digest, Sha256};

fn input_for(name: &str) -> Vec<u8> {
    match name {
        "zeros-1g" => vec![0; 1 << 30],
        "text-16m" => {
            const WORDS: &[&[u8]] = &[
                b"the",
                b"compression",
                b"dictionary",
                b"stream",
                b"match",
                b"length",
                b"binary",
                b"tree",
                b"encoder",
                b"distance",
                b"repeated",
                b"pattern",
                b"data",
                b"block",
                b"symbol",
                b"window",
                b"search",
                b"byte",
            ];
            let mut data = Vec::with_capacity(16 << 20);
            let mut index = 0usize;
            while data.len() < 16 << 20 {
                data.extend_from_slice(WORDS[index % WORDS.len()]);
                data.push(b' ');
                index += 1;
            }
            data.truncate(16 << 20);
            data
        }
        "random-16m" => {
            let mut state = 0x1234_5678_u32;
            let mut data = Vec::with_capacity(16 << 20);
            for _ in 0..(16 << 20) {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                data.push(state as u8);
            }
            data
        }
        _ => panic!("expected zeros-1g, text-16m, or random-16m"),
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest_hex(&Sha256::digest(bytes))
}

fn options() -> Lzma2Options {
    let mut options = Lzma2Options::with_preset(5);
    options.lzma_options.dict_size = 16 << 20;
    options.lzma_options.nice_len = 32;
    options.lzma_options.depth_limit = 32;
    options
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let (name, input) = match args.as_slice() {
        [flag, path] if flag == "--file" => {
            let input = std::fs::read(path)?;
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned();
            (name, input)
        }
        [name] => (name.clone(), input_for(name)),
        _ => {
            return Err(
                "usage: library_benchmark <zeros-1g|text-16m|random-16m> | --file PATH".into(),
            );
        }
    };
    let options = options();
    let estimated_memory_kib = options.lzma_options.get_memory_usage();

    let start = Instant::now();
    let mut writer = Lzma2Writer::new(Vec::new(), options.clone());
    for chunk in input.chunks(64 << 10) {
        writer.write_all(chunk)?;
    }
    let compressed = writer.finish()?;
    let elapsed = start.elapsed();

    let input_hash = sha256_hex(&input);
    let compressed_hash = sha256_hex(&compressed);
    let mut reader = Lzma2Reader::new(compressed.as_slice(), options.lzma_options.dict_size, None);
    let mut buffer = [0; 64 << 10];
    let mut decoded_hash = Sha256::new();
    let mut offset = 0;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        assert_eq!(&buffer[..count], &input[offset..offset + count]);
        decoded_hash.update(&buffer[..count]);
        offset += count;
    }
    assert_eq!(offset, input.len());
    assert_eq!(digest_hex(&decoded_hash.finalize()), input_hash);

    println!(
        "workload={name} input_bytes={} estimated_memory_kib={estimated_memory_kib} elapsed_s={:.6} compressed_bytes={} input_sha256={input_hash} compressed_sha256={compressed_hash}",
        input.len(),
        elapsed.as_secs_f64(),
        compressed.len(),
    );
    Ok(())
}
