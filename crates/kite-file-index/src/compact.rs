//! KFINDEX3: entry/name indirection, unique lowercase basenames, reusable query bitset.
use super::{
    decode_path, invalid, path_record, snapshot_header, u32_at, u64_at, utf16_units, Result,
};
use memchr::memmem::Finder;
use memmap2::{Mmap, MmapOptions};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Instant;
const MAGIC: &[u8; 8] = b"KFINDEX3";
const HEADER: u64 = 48;
fn layout(entries: u64, names: u64, pool: u64) -> Result<(u64, u64, u64)> {
    let offsets = HEADER
        .checked_add(
            entries
                .checked_mul(8)
                .ok_or_else(|| invalid("entry table overflow"))?,
        )
        .ok_or_else(|| invalid("entry table overflow"))?;
    let start = offsets
        .checked_add(
            names
                .checked_add(1)
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| invalid("dictionary table overflow"))?,
        )
        .ok_or_else(|| invalid("dictionary table overflow"))?;
    let length = start
        .checked_add(pool)
        .ok_or_else(|| invalid("index length overflow"))?;
    Ok((offsets, start, length))
}
fn snapshot_size(bytes: u64) -> Result<()> {
    if !(16..(1u64 << 32)).contains(&bytes) {
        return Err(
            invalid("KFINDEX3 requires snapshot size below 4 GiB and at least 16 bytes").into(),
        );
    }
    Ok(())
}
pub(super) fn build(snapshot: &Path, index: &Path) -> Result<()> {
    let started = Instant::now();
    let source = File::open(snapshot)?;
    let snapshot_bytes = source.metadata()?.len();
    snapshot_size(snapshot_bytes)?;
    let mut reader = BufReader::with_capacity(1024 * 1024, source);
    let count = snapshot_header(&mut reader)?;
    if count > (snapshot_bytes - 16) / 5 {
        return Err(invalid("snapshot count exceeds file size").into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(index)?;
    file.write_all(&[0u8; 48])?;
    let mut output = BufWriter::with_capacity(1024 * 1024, file);
    let mut dictionary: HashMap<String, u32> = HashMap::new();
    let mut pool = Vec::<u8>::new();
    let mut offsets = vec![0u32];
    let mut path = Vec::new();
    let mut snapshot_offset = 16u64;
    let mut invalid_utf16_paths = 0u64;
    for record_number in 0..count {
        if record_number & 0x3fff == 0 {
            crate::cancel::check()?;
        }
        let consumed = path_record(&mut reader, snapshot_bytes - snapshot_offset, &mut path)?;
        let units = utf16_units(&path);
        let (text, bad) = decode_path(&units);
        invalid_utf16_paths += u64::from(bad);
        let name = text
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&text)
            .to_lowercase();
        let id = if let Some(id) = dictionary.get(&name) {
            *id
        } else {
            let id = u32::try_from(dictionary.len())?;
            let next = pool
                .len()
                .checked_add(name.len())
                .ok_or_else(|| invalid("pool overflow"))?;
            let next = u32::try_from(next)?;
            pool.extend_from_slice(name.as_bytes());
            offsets.push(next);
            dictionary.insert(name, id);
            id
        };
        output.write_all(&id.to_le_bytes())?;
        output.write_all(&u32::try_from(snapshot_offset)?.to_le_bytes())?;
        snapshot_offset = snapshot_offset
            .checked_add(consumed)
            .ok_or_else(|| invalid("snapshot offset overflow"))?;
    }
    if snapshot_offset != snapshot_bytes {
        return Err(invalid("snapshot has trailing bytes").into());
    }
    let unique = dictionary.len() as u64;
    let pool_bytes = pool.len() as u64;
    let (offset_start, pool_start, index_bytes) = layout(count, unique, pool_bytes)?;
    // Dictionary allocation is build-only; the search process maps the resulting flat data.
    drop(dictionary);
    for offset in offsets {
        output.write_all(&offset.to_le_bytes())?;
    }
    output.write_all(&pool)?;
    output.flush()?;
    let mut file = output.into_inner()?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(MAGIC)?;
    for value in [
        count,
        snapshot_bytes,
        pool_bytes,
        invalid_utf16_paths,
        unique,
    ] {
        file.write_all(&value.to_le_bytes())?;
    }
    file.set_len(index_bytes)?;
    file.sync_all()?;
    crate::diagnostic!(
        "{}",
        json!({"format":"KFINDEX3","entries":count,"distinct_names":unique,"build_ms":started.elapsed().as_secs_f64()*1000.0,"index_bytes":index_bytes,"snapshot_bytes":snapshot_bytes,"invalid_utf16_paths":invalid_utf16_paths,"entry_table_bytes":offset_start-HEADER,"dictionary_offset_bytes":pool_start-offset_start,"pool_bytes":pool_bytes,"header_bytes":HEADER})
    );
    Ok(())
}
/// Build-only flat parts; discarded before the separate low-resident search starts.
pub(super) struct BuildParts {
    pub entries: Vec<u8>,
    pub offsets: Vec<u32>,
    pub pool: Vec<u8>,
    pub count: u64,
    pub snapshot_bytes: u64,
    pub invalid_utf16_paths: u64,
}
pub(super) fn build_parts(snapshot: &Path) -> Result<BuildParts> {
    let file = File::open(snapshot)?;
    let snapshot_bytes = file.metadata()?.len();
    snapshot_size(snapshot_bytes)?;
    let map = unsafe { MmapOptions::new().map(&file)? };
    if &map[..8] != super::SNAP_MAGIC {
        return Err(invalid("invalid snapshot magic").into());
    }
    let count = u64_at(&map, 8);
    if count > (snapshot_bytes - 16) / 5 {
        return Err(invalid("snapshot count exceeds file size").into());
    }
    let capacity = usize::try_from(
        count
            .checked_mul(8)
            .ok_or_else(|| invalid("entry bytes overflow"))?,
    )?;
    let mut parts = BuildParts {
        entries: Vec::with_capacity(capacity),
        offsets: vec![0],
        pool: Vec::new(),
        count,
        snapshot_bytes,
        invalid_utf16_paths: 0,
    };
    let mut dictionary: HashMap<String, u32> = HashMap::new();
    let mut units = Vec::new();
    let mut position = 16usize;
    for record_number in 0..count {
        if record_number & 0x3fff == 0 {
            crate::cancel::check()?;
        }
        let head = position
            .checked_add(5)
            .filter(|&end| end <= map.len())
            .ok_or_else(|| invalid("truncated snapshot record"))?;
        let length = u32_at(&map, position + 1) as usize;
        let end = head
            .checked_add(length)
            .filter(|&end| end <= map.len())
            .ok_or_else(|| invalid("snapshot record outside file"))?;
        if map[position] > 1 || length % 2 != 0 {
            return Err(invalid("invalid snapshot record").into());
        }
        // Copy UTF16 into a reusable vector; ordinary paths need no decoder pass.
        units.clear();
        units.extend(
            map[head..end]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]])),
        );
        let contains_surrogate = units.iter().any(|&unit| unit & 0xf800 == 0xd800);
        parts.invalid_utf16_paths += u64::from(
            contains_surrogate
                && std::char::decode_utf16(units.iter().copied()).any(|unit| unit.is_err()),
        );
        let basename = units
            .iter()
            .rposition(|&unit| unit == 47 || unit == 92)
            .map_or(0, |i| i + 1);
        // Lowercase the complete basename string, preserving contextual sigma behavior.
        let name = String::from_utf16_lossy(&units[basename..]).to_lowercase();
        let id = if let Some(id) = dictionary.get(&name) {
            *id
        } else {
            let id = u32::try_from(dictionary.len())?;
            let next = parts
                .pool
                .len()
                .checked_add(name.len())
                .ok_or_else(|| invalid("pool overflow"))?;
            let next = u32::try_from(next)?;
            parts.pool.extend_from_slice(name.as_bytes());
            parts.offsets.push(next);
            dictionary.insert(name, id);
            id
        };
        parts.entries.extend_from_slice(&id.to_le_bytes());
        parts
            .entries
            .extend_from_slice(&u32::try_from(position)?.to_le_bytes());
        position = end;
    }
    if position != map.len() {
        return Err(invalid("snapshot has trailing bytes").into());
    }
    Ok(parts)
}
struct Search {
    map: Mmap,
    snapshot: File,
    snapshot_bytes: u64,
    entries: usize,
    unique: usize,
    offset_start: usize,
    pool_start: usize,
    invalid_utf16_paths: u64,
    bits: Vec<u8>,
}
impl Search {
    fn open(snapshot: &Path, index: &Path) -> Result<Self> {
        let mut snapshot = File::open(snapshot)?;
        let bytes = snapshot.metadata()?.len();
        snapshot_size(bytes)?;
        let count = snapshot_header(&mut snapshot)?;
        let file = File::open(index)?;
        let map = unsafe { MmapOptions::new().map(&file)? };
        if map.len() < HEADER as usize || &map[..8] != MAGIC {
            return Err(invalid("invalid KFINDEX3 header").into());
        }
        let n = u64_at(&map, 8);
        let pool = u64_at(&map, 24);
        let bad = u64_at(&map, 32);
        let unique = u64_at(&map, 40);
        if count != n
            || u64_at(&map, 16) != bytes
            || n > (bytes - 16) / 5
            || bad > n
            || unique > n
            || unique > u32::MAX as u64
            || pool > u32::MAX as u64
            || (n == 0 && unique != 0)
            || (n > 0 && unique == 0)
        {
            return Err(invalid("invalid counts/snapshot association").into());
        }
        let (offset_start, pool_start, length) = layout(n, unique, pool)?;
        if length != map.len() as u64 {
            return Err(invalid("index size mismatch").into());
        }
        let n = usize::try_from(n)?;
        let unique = usize::try_from(unique)?;
        let offset_start = usize::try_from(offset_start)?;
        let pool_start = usize::try_from(pool_start)?;
        if u32_at(&map, offset_start) != 0 {
            return Err(invalid("dictionary must start at zero").into());
        }
        let mut previous = 0usize;
        for id in 0..unique {
            let end = u32_at(&map, offset_start + (id + 1) * 4) as usize;
            if end < previous || end > pool as usize {
                return Err(invalid("dictionary offsets not monotonic/in bounds").into());
            }
            std::str::from_utf8(&map[pool_start + previous..pool_start + end])?;
            previous = end;
        }
        if previous != pool as usize {
            return Err(invalid("dictionary does not cover name pool").into());
        }
        let mut last = 0u64;
        for i in 0..n {
            let pos = HEADER as usize + i * 8;
            let id = u32_at(&map, pos) as usize;
            let offset = u32_at(&map, pos + 4) as u64;
            if id >= unique
                || offset < 16
                || offset.checked_add(5).is_none_or(|end| end > bytes)
                || (i == 0 && offset != 16)
                || (i > 0 && offset < last + 5)
            {
                return Err(invalid("entry name ID/snapshot offset out of bounds/order").into());
            }
            last = offset;
        }
        let bit_bytes = unique
            .checked_add(7)
            .ok_or_else(|| invalid("bitset overflow"))?
            / 8;
        Ok(Self {
            map,
            snapshot,
            snapshot_bytes: bytes,
            entries: n,
            unique,
            offset_start,
            pool_start,
            invalid_utf16_paths: bad,
            bits: vec![0; bit_bytes],
        })
    }
    fn query(&mut self, input: &Value) -> Result<Value> {
        let started = Instant::now();
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("query must be a string"))?;
        let limit = match input.get("limit") {
            None => 20,
            Some(v) => v
                .as_u64()
                .filter(|n| *n <= 1000)
                .ok_or_else(|| invalid("limit must be an integer from 0 to 1000"))?
                as usize,
        };
        let lowercase = query.to_lowercase();
        let terms: Vec<_> = lowercase
            .split_whitespace()
            .map(|t| Finder::new(t.as_bytes()))
            .collect();
        self.bits.fill(0);
        let pool = &self.map[self.pool_start..];
        for id in 0..self.unique {
            let a = u32_at(&self.map, self.offset_start + id * 4) as usize;
            let b = u32_at(&self.map, self.offset_start + (id + 1) * 4) as usize;
            if terms.iter().all(|term| term.find(&pool[a..b]).is_some()) {
                self.bits[id / 8] |= 1 << (id % 8);
            }
        }
        let mut total = 0u64;
        let mut selected = Vec::with_capacity(limit);
        for i in 0..self.entries {
            let pos = HEADER as usize + i * 8;
            let id = u32_at(&self.map, pos) as usize;
            if self.bits[id / 8] & (1 << (id % 8)) != 0 {
                total += 1;
                if selected.len() < limit {
                    selected.push(u32_at(&self.map, pos + 4) as u64);
                }
            }
        }
        let mut paths = Vec::with_capacity(selected.len());
        let mut paths_utf16 = Vec::with_capacity(selected.len());
        let mut buffer = Vec::new();
        for offset in selected {
            self.snapshot.seek(SeekFrom::Start(offset))?;
            path_record(
                &mut self.snapshot,
                self.snapshot_bytes - offset,
                &mut buffer,
            )?;
            let units = utf16_units(&buffer);
            let (path, bad) = decode_path(&units);
            paths.push(path);
            paths_utf16.push(if bad { Some(units) } else { None });
        }
        Ok(
            json!({"query":query,"total":total,"paths":paths,"paths_utf16":paths_utf16,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0}),
        )
    }
}
pub(super) fn serve(snapshot: &Path, index: &Path) -> Result<()> {
    let started = Instant::now();
    let mut search = Search::open(snapshot, index)?;
    let mut output = io::stdout().lock();
    writeln!(
        output,
        "{}",
        json!({"ready":true,"format":"KFINDEX3","entries":search.entries,"distinct_names":search.unique,"index_bytes":search.map.len(),"invalid_utf16_paths":search.invalid_utf16_paths,"load_ms":started.elapsed().as_secs_f64()*1000.0,"query_bitset_bytes":search.bits.len()})
    )?;
    output.flush()?;
    for line in io::stdin().lock().lines() {
        let response: Result<Value> = serde_json::from_str(&line?)
            .map_err(Into::into)
            .and_then(|value| search.query(&value));
        let response = match response {
            Ok(value) => value,
            Err(error) => json!({"error":error.to_string()}),
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}
