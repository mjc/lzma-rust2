use std::io::{self, Read};

pub const MAX_OUTPUT: u64 = 1 << 18;

pub fn read_bounded(reader: impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut output = Vec::new();
    reader.take(MAX_OUTPUT + 1).read_to_end(&mut output)?;
    Ok((output.len() as u64 <= MAX_OUTPUT).then_some(output))
}

pub fn compare(
    implementation: io::Result<Option<Vec<u8>>>,
    reference: io::Result<Option<Vec<u8>>>,
) {
    if let (Ok(Some(implementation)), Ok(Some(reference))) = (implementation, reference) {
        assert_eq!(implementation, reference);
    }
}
