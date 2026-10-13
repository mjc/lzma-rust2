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

fn raw_head(mode: u8, props: u8, dict_size: u32, size: Option<usize>) -> [u8; 8] {
    let pb = props / (9 * 5);
    let remainder = props % (9 * 5);
    let lp = remainder / 9;
    let lc = remainder % 9;
    let mut head = [0; 8];
    head[0] = mode << 2 | u8::from(size.is_some());
    head[1] = if mode == 1 {
        props
    } else {
        (0..=u8::MAX)
            .find(|candidate| *candidate % 9 == lc && (*candidate >> 4) % 5 == lp)
            .expect("valid LZMA properties")
    };
    head[2] = pb;
    head[4..6].copy_from_slice(&(dict_size as u16).to_le_bytes());
    if let Some(size) = size {
        head[6..8].copy_from_slice(&(size as u16).to_le_bytes());
    }
    head
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
        let (lzma_raw_eopm, raw_props, raw_dict_size) = valid_streams::lzma_raw(&payload, true);
        let (lzma_raw_known, _, _) = valid_streams::lzma_raw(&payload, false);
        let lzma2 = valid_streams::lzma2(&payload);
        let lzip = valid_streams::lzip(&payload);
        let bcj2_payload = valid_streams::bcj2_payload(size);
        let bcj2_streams = valid_streams::bcj2(&bcj2_payload);

        write_direct_seed(&root, &["lzma"], name, &lzma)?;
        write_direct_seed(&root, &["lzma2"], name, &lzma2)?;
        write_direct_seed(&root, &["lzip"], name, &lzip)?;

        let mut bcj2_seed = Vec::new();
        bcj2_seed.extend_from_slice(&(bcj2_payload.len() as u16).to_le_bytes());
        for stream in bcj2_streams.iter().take(3) {
            bcj2_seed.extend_from_slice(&(stream.len() as u16).to_le_bytes());
        }
        for stream in &bcj2_streams {
            bcj2_seed.extend_from_slice(stream);
        }
        write_direct_seed(&root, &["bcj2_decode"], name, &bcj2_seed)?;

        let plan = [0x00, 0x11, 0x22, 0x33, 0x66, 0x99, 0xCC, 0xFF];
        let lzma_plan = [[0; 8].as_slice(), plan.as_slice()].concat();
        let lzma2_plan = [[0].as_slice(), plan.as_slice()].concat();
        write_planned_seed(&root, "lzma_stream", name, &lzma_plan, &lzma)?;
        for (suffix, stream, size) in [
            ("raw-props-eopm", &lzma_raw_eopm, None),
            ("raw-fields-eopm", &lzma_raw_eopm, None),
            ("raw-props-known", &lzma_raw_known, Some(size)),
            ("raw-fields-known", &lzma_raw_known, Some(size)),
        ] {
            let mode = if suffix.contains("fields") { 2 } else { 1 };
            let head = raw_head(mode, raw_props, raw_dict_size, size);
            let raw_name = format!("{name}-{suffix}");
            let raw_plan = [head.as_slice(), plan.as_slice()].concat();
            write_planned_seed(&root, "lzma_stream", &raw_name, &raw_plan, stream)?;
        }
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
