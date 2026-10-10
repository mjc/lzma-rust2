//! Generate valid inputs for the decoder fuzz targets.

use std::{fs, path::Path};

use lzma_rust2::CheckType;

#[path = "../valid_streams.rs"]
mod valid_streams;

fn write_seed(
    root: &Path,
    target: &str,
    name: &str,
    config: u8,
    selector: u8,
    stream: &[u8],
) -> std::io::Result<()> {
    let directory = root.join(target);
    fs::create_dir_all(&directory)?;
    fs::write(
        directory.join(name),
        [[config, selector].as_slice(), stream].concat(),
    )
}

fn write_direct_seed(
    root: &Path,
    targets: &[&str],
    name: &str,
    stream: &[u8],
) -> std::io::Result<()> {
    for target in targets {
        let directory = root.join(target);
        fs::create_dir_all(&directory)?;
        fs::write(directory.join(name), stream)?;
    }
    Ok(())
}

fn write_planned_seed(
    root: &Path,
    target: &str,
    name: &str,
    head: &[u8],
    stream: &[u8],
) -> std::io::Result<()> {
    let directory = root.join(target);
    fs::create_dir_all(&directory)?;
    fs::write(directory.join(name), [head, stream].concat())
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
        let lzma = valid_streams::lzma(&payload);
        let lzma2 = valid_streams::lzma2(&payload);
        let lzip = valid_streams::lzip(&payload);

        write_direct_seed(&root, &["lzma"], name, &lzma)?;
        write_direct_seed(&root, &["lzma2"], name, &lzma2)?;
        write_direct_seed(&root, &["lzip"], name, &lzip)?;

        let plan = [0x00, 0x11, 0x22, 0x33, 0x66, 0x99, 0xCC, 0xFF];
        let lzma_plan = [[0; 8].as_slice(), plan.as_slice()].concat();
        let lzma2_plan = [[0].as_slice(), plan.as_slice()].concat();
        write_planned_seed(&root, "lzma_stream", name, &lzma_plan, &lzma)?;
        write_planned_seed(&root, "lzma2_stream", name, &lzma2_plan, &lzma2)?;
        write_planned_seed(&root, "lzip_stream", name, &plan, &lzip)?;

        write_seed(
            &root,
            "lzma2_mt_decode",
            name,
            size as u8,
            size as u8,
            &lzma2,
        )?;
        write_seed(&root, "lzip_mt_decode", name, size as u8, size as u8, &lzip)?;
        for (check_name, check_type) in [
            ("none", CheckType::None),
            ("crc32", CheckType::Crc32),
            ("crc64", CheckType::Crc64),
            ("sha256", CheckType::Sha256),
        ] {
            let xz = valid_streams::xz(&payload, check_type);
            let seed_name = format!("{name}-{check_name}");
            write_direct_seed(&root, &["xz"], &seed_name, &xz)?;
            let xz_plan = [[0].as_slice(), plan.as_slice()].concat();
            write_planned_seed(&root, "xz_stream", &seed_name, &xz_plan, &xz)?;
            write_seed(
                &root,
                "xz_mt_decode",
                &seed_name,
                size as u8,
                size as u8,
                &xz,
            )?;

            let concatenated = [xz.as_slice(), xz.as_slice()].concat();
            let concatenated_name = format!("{seed_name}-concatenated");
            write_direct_seed(&root, &["xz"], &concatenated_name, &concatenated)?;
            let xz_multi_plan = [[1].as_slice(), plan.as_slice()].concat();
            write_planned_seed(
                &root,
                "xz_stream",
                &concatenated_name,
                &xz_multi_plan,
                &concatenated,
            )?;
            write_seed(
                &root,
                "xz_mt_decode",
                &concatenated_name,
                3,
                2,
                &concatenated,
            )?;
        }
    }
    Ok(())
}
