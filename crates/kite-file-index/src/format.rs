use std::io::{self, Read};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const SNAP_MAGIC: &[u8; 8] = b"KFSNAP02";

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn snapshot_header(reader: &mut impl Read) -> Result<u64> {
    let mut header = [0; 16];
    reader.read_exact(&mut header)?;
    if &header[..8] != SNAP_MAGIC {
        return Err(invalid("invalid snapshot magic").into());
    }
    Ok(u64_at(&header, 8))
}

fn path_record(reader: &mut impl Read, remaining: u64, path: &mut Vec<u8>) -> Result<u64> {
    let mut header = [0; 5];
    reader.read_exact(&mut header)?;
    let length = u32_at(&header, 1) as u64;
    if header[0] > 1 || remaining < 5 || length > remaining - 5 || length % 2 != 0 {
        return Err(invalid("invalid snapshot record").into());
    }
    path.resize(length as usize, 0);
    reader.read_exact(path)?;
    Ok(length + 5)
}

fn utf16_units(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

fn decode_path(units: &[u16]) -> (String, bool) {
    match String::from_utf16(units) {
        Ok(text) => (text, false),
        Err(_) => (String::from_utf16_lossy(units), true),
    }
}
