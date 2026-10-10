//! Generate valid inputs for the parallel decoder fuzz targets.

use std::{fs, path::Path};

use lzma_rust2::CheckType;

#[path = "../valid_streams.rs"]
mod valid_streams;

fn write_seed(
    root: &Path,
    target: &str,
    name: &str,
    selector: u8,
    stream: &[u8],
) -> std::io::Result<()> {
    let directory = root.join(target);
    fs::create_dir_all(&directory)?;
    fs::write(
        directory.join(name),
        [[0, selector].as_slice(), stream].concat(),
    )
}

fn main() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");

    for (name, size) in [
        ("empty", 0),
        ("one-byte", 1),
        ("partial-block", 4095),
        ("one-block", 4096),
        ("many-blocks", 3 * 4096 + 137),
    ] {
        let payload = valid_streams::payload(size);
        write_seed(
            &root,
            "lzma2_mt_decode",
            name,
            size as u8,
            &valid_streams::lzma2(&payload),
        )?;
        write_seed(
            &root,
            "lzip_mt_decode",
            name,
            size as u8,
            &valid_streams::lzip(&payload),
        )?;
        for (check_name, check_type) in [
            ("none", CheckType::None),
            ("crc32", CheckType::Crc32),
            ("crc64", CheckType::Crc64),
            ("sha256", CheckType::Sha256),
        ] {
            write_seed(
                &root,
                "xz_mt_decode",
                &format!("{name}-{check_name}"),
                size as u8,
                &valid_streams::xz(&payload, check_type),
            )?;
        }
    }
    Ok(())
}
