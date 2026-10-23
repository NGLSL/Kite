//! Merge completed volume snapshots and their opt-in identity streams once.
use crate::{invalid, snapshot_header, u32_at, Result, SNAP_MAGIC};
use serde_json::json;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Instant;

pub(crate) fn run(snapshot: &Path, identities: &Path, sources: &[OsString]) -> Result<()> {
    if sources.is_empty() || sources.len() % 2 != 0 {
        return Err(invalid("merge needs snapshot/identity pairs").into());
    }
    let started = Instant::now();
    let mut out = BufWriter::with_capacity(
        1024 * 1024,
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(snapshot)?,
    );
    let mut ids = BufWriter::with_capacity(
        1024 * 1024,
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(identities)?,
    );
    out.write_all(SNAP_MAGIC)?;
    out.write_all(&0u64.to_le_bytes())?;
    let mut offset = 16u64;
    let mut ordinal = 0u64;
    for pair in sources.chunks_exact(2) {
        let mut source = File::open(&pair[0])?;
        let bytes = source.metadata()?.len();
        let entries = snapshot_header(&mut source)?;
        if bytes < 16 || entries > (bytes - 16) / 5 {
            return Err(invalid("invalid source snapshot").into());
        }
        let identity = File::open(&pair[1])?;
        if identity.metadata()?.len()
            != entries
                .checked_mul(24)
                .ok_or_else(|| invalid("identity count overflow"))?
        {
            return Err(invalid("source identities count mismatch").into());
        }
        let copied = std::io::copy(&mut source, &mut out)?;
        if copied != bytes - 16 {
            return Err(invalid("source snapshot size changed during merge").into());
        }
        let mut source_ids = BufReader::with_capacity(1024 * 1024, identity);
        for local in 0..entries {
            if local & 0x3fff == 0 {
                crate::cancel::check()?;
            }
            let mut row = [0u8; 24];
            source_ids.read_exact(&mut row)?;
            if u32_at(&row, 20) as u64 != local || u32_at(&row, 16) < 16 {
                return Err(invalid("raw identity source order invalid").into());
            }
            let merged_offset = u32::try_from(offset - 16 + u32_at(&row, 16) as u64)?;
            let merged_ordinal = u32::try_from(ordinal + local)?;
            row[16..20].copy_from_slice(&merged_offset.to_le_bytes());
            row[20..24].copy_from_slice(&merged_ordinal.to_le_bytes());
            ids.write_all(&row)?;
        }
        offset += bytes - 16;
        ordinal += entries;
    }
    out.flush()?;
    out.seek(SeekFrom::Start(8))?;
    out.write_all(&ordinal.to_le_bytes())?;
    out.flush()?;
    out.get_ref().sync_all()?;
    ids.flush()?;
    ids.get_ref().sync_all()?;
    crate::diagnostic!(
        "{}",
        json!({"entries":ordinal,"snapshot_bytes":offset,
        "identity_bytes":ordinal*24,"wall_ms":started.elapsed().as_secs_f64()*1000.0})
    );
    Ok(())
}
