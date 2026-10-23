//! Small, bounded live overlay for the KFPAGED1 snapshot.
//!
//! The immutable snapshot/index remains the search base.  The overlay only
//! keeps changed objects, a bit per shadowed baseline path, and lazy directory
//! prefix rewrites.  It is intentionally a prototype: when its bounded state
//! is full it reports `rebuild_required` instead of growing without limit.

use super::{decode_path, invalid, paged, path_record, snapshot_header, utf16_units, Result};
use memchr::memmem::Finder;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

const ID_MAGIC: &[u8; 8] = b"KFLIVEI1";
const ID_HEADER: usize = 64;
const ID_RECORD: usize = 24;
const MAX_OBJECTS: usize = 65_536;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
struct Key {
    volume: u8,
    frn: u64,
}

#[derive(Clone, Copy, Debug)]
struct IdentityRecord {
    volume: u8,
    is_dir: bool,
    frn: u64,
    offset: u32,
    ordinal: u32,
}

impl IdentityRecord {
    fn parse(row: &[u8]) -> Result<Self> {
        if row.len() != ID_RECORD
            || !(b'A'..=b'Z').contains(&row[0])
            || row[1] > 1
            || row[2..8].iter().any(|b| *b != 0)
        {
            return Err(invalid("invalid live identity record").into());
        }
        Ok(Self {
            volume: row[0],
            is_dir: row[1] != 0,
            frn: u64::from_le_bytes(row[8..16].try_into().unwrap()),
            offset: u32::from_le_bytes(row[16..20].try_into().unwrap()),
            ordinal: u32::from_le_bytes(row[20..24].try_into().unwrap()),
        })
    }

    fn write(self, output: &mut impl Write) -> Result<()> {
        let mut row = [0u8; ID_RECORD];
        row[0] = self.volume;
        row[1] = u8::from(self.is_dir);
        row[8..16].copy_from_slice(&self.frn.to_le_bytes());
        row[16..20].copy_from_slice(&self.offset.to_le_bytes());
        row[20..24].copy_from_slice(&self.ordinal.to_le_bytes());
        output.write_all(&row)?;
        Ok(())
    }

    fn key(self) -> (u8, u64, u32) {
        (self.volume, self.frn, self.ordinal)
    }
}

fn put_u64(header: &mut [u8], offset: usize, value: u64) {
    header[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u64(header: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap())
}

fn scan_snapshot_meta(snapshot: &Path) -> Result<(u64, u64, Vec<u32>, Vec<bool>)> {
    let file = File::open(snapshot)?;
    let bytes = file.metadata()?.len();
    let mut file = BufReader::with_capacity(1024 * 1024, file);
    let entries = snapshot_header(&mut file)?;
    if !(16..(1u64 << 32)).contains(&bytes) || entries > (bytes - 16) / 5 {
        return Err(invalid("invalid snapshot size/count for live identities").into());
    }
    let mut offsets = Vec::with_capacity(usize::try_from(entries)?);
    let mut dirs = Vec::with_capacity(usize::try_from(entries)?);
    let mut offset = 16u64;
    let mut path = Vec::new();
    for _ in 0..entries {
        let (consumed, is_dir) =
            snapshot_meta_record(&mut file, bytes.saturating_sub(offset), &mut path)?;
        offsets.push(u32::try_from(offset)?);
        dirs.push(is_dir);
        offset = offset
            .checked_add(consumed)
            .ok_or_else(|| invalid("snapshot offset overflow"))?;
    }
    if offset != bytes {
        return Err(invalid("snapshot has trailing bytes").into());
    }
    Ok((bytes, entries, offsets, dirs))
}

fn snapshot_meta_record(
    reader: &mut impl Read,
    remaining: u64,
    path: &mut Vec<u8>,
) -> Result<(u64, bool)> {
    if remaining < 5 {
        return Err(invalid("truncated snapshot record").into());
    }
    let mut head = [0u8; 5];
    reader.read_exact(&mut head)?;
    let length = u32::from_le_bytes(head[1..5].try_into().unwrap()) as u64;
    if head[0] > 1 || length % 2 != 0 || length > remaining - 5 {
        return Err(invalid("invalid snapshot record while building identities").into());
    }
    path.resize(length as usize, 0);
    reader.read_exact(path)?;
    Ok((5 + length, head[0] != 0))
}

/// Convert the raw 24-byte identity side table emitted beside a snapshot to
/// a disk-sorted table.  The raw table remains a simple snapshot-order stream;
/// this build step is the only point where all records are resident.
pub(crate) fn build_identities(snapshot: &Path, raw: &Path, sorted: &Path) -> Result<()> {
    let started = Instant::now();
    let (snapshot_bytes, entries, offsets, dirs) = scan_snapshot_meta(snapshot)?;
    let raw_file = File::open(raw)?;
    let raw_bytes = raw_file.metadata()?.len();
    if raw_bytes % ID_RECORD as u64 != 0 || raw_bytes / ID_RECORD as u64 != entries {
        return Err(invalid("identity raw count does not match snapshot").into());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(raw_bytes)?);
    BufReader::new(raw_file).read_to_end(&mut bytes)?;
    let mut seen = vec![false; usize::try_from(entries)?];
    let mut records = Vec::with_capacity(usize::try_from(entries)?);
    for row in bytes.chunks_exact(ID_RECORD) {
        let record = IdentityRecord::parse(row)?;
        if record.frn == 0 || record.ordinal as usize >= seen.len() || seen[record.ordinal as usize]
        {
            return Err(invalid("identity ordinal is duplicated/out of range").into());
        }
        let ordinal = record.ordinal as usize;
        if record.offset != offsets[ordinal] || record.is_dir != dirs[ordinal] {
            return Err(invalid("identity record does not match snapshot ordinal").into());
        }
        seen[ordinal] = true;
        records.push(record);
    }
    if seen.iter().any(|value| !value) {
        return Err(invalid("identity ordinals do not cover the snapshot").into());
    }
    // The raw stream and coverage bitmap are only validation work.  Release
    // them before the high-memory sort and disk write so the sort resident
    // set is limited to the records that actually need ordering.
    drop(seen);
    drop(bytes);
    records.sort_unstable_by_key(|record| record.key());

    let mut header = [0u8; ID_HEADER];
    header[..8].copy_from_slice(ID_MAGIC);
    put_u64(&mut header, 8, 1);
    put_u64(&mut header, 16, entries);
    put_u64(&mut header, 24, snapshot_bytes);
    put_u64(&mut header, 32, raw_bytes);
    put_u64(&mut header, 40, ID_HEADER as u64);
    put_u64(&mut header, 48, ID_RECORD as u64);
    put_u64(&mut header, 56, 0);

    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(sorted)?;
    output.write_all(&header)?;
    {
        let mut buffered = io::BufWriter::with_capacity(1024 * 1024, &mut output);
        for record in records {
            record.write(&mut buffered)?;
        }
        buffered.flush()?;
    }
    output.sync_all()?;
    crate::diagnostic!(
        "{}",
        json!({
            "format": "KFLIVEI1",
            "entries": entries,
            "snapshot_bytes": snapshot_bytes,
            "raw_bytes": raw_bytes,
            "sorted_bytes": ID_HEADER as u64 + raw_bytes,
            "build_ms": started.elapsed().as_secs_f64() * 1000.0,
            "resident_sort_records": entries,
        })
    );
    Ok(())
}

struct IdentityTable {
    file: File,
    entries: u64,
    snapshot_bytes: u64,
    startup_records: Option<Vec<u8>>,
    startup_cache_enabled: bool,
}

impl IdentityTable {
    fn open(path: &Path, snapshot_bytes: u64, entries: u64) -> Result<Self> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        if length < ID_HEADER as u64 || &read_fixed::<8>(&mut file)? != ID_MAGIC {
            return Err(invalid("invalid live identity header").into());
        }
        file.seek(SeekFrom::Start(8))?;
        let mut header = [0u8; ID_HEADER - 8];
        file.read_exact(&mut header)?;
        let version = get_u64(&header, 0);
        let count = get_u64(&header, 8);
        let stored_snapshot = get_u64(&header, 16);
        let raw_bytes = get_u64(&header, 24);
        let header_bytes = get_u64(&header, 32);
        let record_bytes = get_u64(&header, 40);
        if version != 1
            || count != entries
            || stored_snapshot != snapshot_bytes
            || header_bytes != ID_HEADER as u64
            || record_bytes != ID_RECORD as u64
            || raw_bytes != count.saturating_mul(ID_RECORD as u64)
            || length != ID_HEADER as u64 + raw_bytes
        {
            return Err(invalid("live identity header/count/snapshot mismatch").into());
        }
        Ok(Self {
            file,
            entries,
            snapshot_bytes,
            startup_records: None,
            startup_cache_enabled: true,
        })
    }

    /// Load the sorted records once for startup catch-up.  The table is about
    /// 74 MiB for the full C/D/E corpus; it is deliberately temporary and is
    /// released before the process announces ready.
    fn load_startup_cache(&mut self) -> Result<()> {
        if !self.startup_cache_enabled || self.startup_records.is_some() {
            return Ok(());
        }
        let bytes = self
            .entries
            .checked_mul(ID_RECORD as u64)
            .ok_or_else(|| invalid("startup identity cache size overflow"))?;
        let capacity = usize::try_from(bytes)?;
        self.file.seek(SeekFrom::Start(ID_HEADER as u64))?;
        let mut cache = Vec::with_capacity(capacity);
        let mut reader = BufReader::with_capacity(1024 * 1024, &mut self.file);
        reader.read_to_end(&mut cache)?;
        if cache.len() != capacity {
            return Err(invalid("startup identity cache length mismatch").into());
        }
        self.startup_records = Some(cache);
        Ok(())
    }

    fn release_startup_cache(&mut self) {
        self.startup_records.take();
        self.startup_cache_enabled = false;
    }

    fn record(&mut self, index: u64) -> Result<IdentityRecord> {
        if index >= self.entries {
            return Err(invalid("live identity record index out of bounds").into());
        }
        self.file
            .seek(SeekFrom::Start(ID_HEADER as u64 + index * ID_RECORD as u64))?;
        let mut row = [0u8; ID_RECORD];
        self.file.read_exact(&mut row)?;
        let record = IdentityRecord::parse(&row)?;
        if record.offset as u64 >= self.snapshot_bytes || record.ordinal as u64 >= self.entries {
            return Err(invalid("live identity record bounds invalid").into());
        }
        Ok(record)
    }

    fn cached_record(&self, cache: &[u8], index: u64) -> Result<IdentityRecord> {
        if index >= self.entries {
            return Err(invalid("live identity record index out of bounds").into());
        }
        let start = usize::try_from(index)?
            .checked_mul(ID_RECORD)
            .ok_or_else(|| invalid("live identity cache offset overflow"))?;
        let end = start
            .checked_add(ID_RECORD)
            .ok_or_else(|| invalid("live identity cache offset overflow"))?;
        let record = IdentityRecord::parse(
            cache
                .get(start..end)
                .ok_or_else(|| invalid("live identity cache record out of bounds"))?,
        )?;
        if record.offset as u64 >= self.snapshot_bytes || record.ordinal as u64 >= self.entries {
            return Err(invalid("live identity record bounds invalid").into());
        }
        Ok(record)
    }

    fn lower_bound(&mut self, key: (u8, u64)) -> Result<u64> {
        let mut low = 0u64;
        let mut high = self.entries;
        while low < high {
            let middle = low + (high - low) / 2;
            let record = self.record(middle)?;
            if (record.volume, record.frn) < key {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        Ok(low)
    }

    fn paths_for(&mut self, key: Key) -> Result<Vec<IdentityRecord>> {
        if self.startup_cache_enabled && self.startup_records.is_none() {
            self.load_startup_cache()?;
        }
        if let Some(cache) = self.startup_records.as_deref() {
            let mut low = 0u64;
            let mut high = self.entries;
            while low < high {
                let middle = low + (high - low) / 2;
                let record = self.cached_record(cache, middle)?;
                if (record.volume, record.frn) < (key.volume, key.frn) {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            let mut records = Vec::new();
            let mut index = low;
            while index < self.entries {
                let record = self.cached_record(cache, index)?;
                if record.volume != key.volume || record.frn != key.frn {
                    break;
                }
                records.push(record);
                index += 1;
            }
            return Ok(records);
        }
        let start = self.lower_bound((key.volume, key.frn))?;
        let mut records = Vec::new();
        let mut index = start;
        while index < self.entries {
            let record = self.record(index)?;
            if record.volume != key.volume || record.frn != key.frn {
                break;
            }
            records.push(record);
            index += 1;
        }
        Ok(records)
    }
}

fn read_fixed<const N: usize>(file: &mut File) -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_snapshot_path(file: &mut File, bytes: u64, offset: u32) -> Result<Vec<u16>> {
    let offset = offset as u64;
    if offset < 16 || offset + 5 > bytes {
        return Err(invalid("snapshot path offset out of bounds").into());
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut path = Vec::new();
    path_record(file, bytes - offset, &mut path)?;
    Ok(utf16_units(&path))
}

fn volume_byte(value: &Value) -> Result<u8> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid("live change volume must be a string"))?;
    let bytes = text.as_bytes();
    if bytes.len() == 2 && bytes[1] == b':' && (b'A'..=b'Z').contains(&bytes[0]) {
        Ok(bytes[0])
    } else if bytes.len() == 1 && (b'A'..=b'Z').contains(&bytes[0]) {
        Ok(bytes[0])
    } else {
        Err(invalid("live volume must be an uppercase drive letter").into())
    }
}

fn parse_units(value: &Value) -> Result<Vec<u16>> {
    let values = value
        .as_array()
        .ok_or_else(|| invalid("paths_utf16 item must be an array"))?;
    let mut result = Vec::with_capacity(values.len());
    for item in values {
        let unit = item
            .as_u64()
            .filter(|value| *value <= u16::MAX as u64)
            .ok_or_else(|| invalid("invalid UTF16 path unit"))?;
        result.push(unit as u16);
    }
    if result.len() > u32::MAX as usize / 2 {
        return Err(invalid("UTF16 path is too long").into());
    }
    if result.is_empty() {
        return Err(invalid("UTF16 path must not be empty").into());
    }
    Ok(result)
}

#[derive(Clone)]
struct Change {
    key: Key,
    is_dir: bool,
    paths: Vec<Vec<u16>>,
}

fn parse_change(value: &Value) -> Result<Change> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("live change must be an object"))?;
    let key = Key {
        volume: volume_byte(
            object
                .get("volume")
                .ok_or_else(|| invalid("change volume missing"))?,
        )?,
        frn: object
            .get("frn")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("change FRN must be a string"))?
            .parse()
            .map_err(|_| invalid("change FRN is not a u64"))?,
    };
    if key.frn == 0 {
        return Err(invalid("change FRN must be nonzero").into());
    }
    let is_dir = object
        .get("is_dir")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("change is_dir must be boolean"))?;
    let values = object
        .get("paths_utf16")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("change paths_utf16 must be an array"))?;
    let mut paths = Vec::with_capacity(values.len());
    for value in values {
        paths.push(parse_units(value)?);
    }
    Ok(Change { key, is_dir, paths })
}

fn path_under(path: &[u16], prefix: &[u16]) -> bool {
    path == prefix
        || (path.len() > prefix.len()
            && path.starts_with(prefix)
            && matches!(path[prefix.len()], 0x5c | 0x2f))
}

fn replace_prefix(path: &[u16], old: &[u16], new: &[u16]) -> Vec<u16> {
    let mut result = Vec::with_capacity(new.len() + path.len().saturating_sub(old.len()));
    result.extend_from_slice(new);
    result.extend_from_slice(&path[old.len()..]);
    result
}

#[derive(Clone)]
struct PrefixMap {
    volume: u8,
    base: Vec<u16>,
    current: Vec<u16>,
}

/// Snapshot matching context captured at the point where a directory delete
/// is applied.  Directory transforms still happen in feed order; only the
/// expensive immutable-snapshot membership checks are merged by prefix state.
struct SnapshotDeleteGroup {
    volume: u8,
    prefix_revision: u64,
    mappings: Vec<PrefixMap>,
    current_prefixes: Vec<Vec<u16>>,
}

#[derive(Default)]
struct SnapshotDeleteBatch {
    groups: Vec<SnapshotDeleteGroup>,
}

impl SnapshotDeleteBatch {
    fn add(
        &mut self,
        volume: u8,
        prefix_revision: u64,
        prefixes: &[PrefixMap],
        current_prefixes: &[Vec<u16>],
    ) {
        if let Some(group) = self
            .groups
            .iter_mut()
            .find(|group| group.volume == volume && group.prefix_revision == prefix_revision)
        {
            group
                .current_prefixes
                .extend(current_prefixes.iter().cloned());
            return;
        }
        self.groups.push(SnapshotDeleteGroup {
            volume,
            prefix_revision,
            mappings: prefixes
                .iter()
                .filter(|mapping| mapping.volume == volume)
                .cloned()
                .collect(),
            current_prefixes: current_prefixes.to_vec(),
        });
    }

    fn apply(self, overlay: &mut Overlay) -> Result<()> {
        if self.groups.is_empty() {
            return Ok(());
        }
        overlay.load_startup_snapshot_cache()?;
        for group in self.groups {
            let matcher = PrefixMatcher::new(&group.current_prefixes);
            let matched = if let Some(snapshot) = overlay.startup_snapshot.as_deref() {
                overlay.snapshot_matches_prefixes_bytes_with_mappings(
                    snapshot,
                    &matcher,
                    group.volume,
                    &group.mappings,
                )?
            } else {
                let file = File::open(&overlay.snapshot_path)?;
                let mut reader = BufReader::with_capacity(1024 * 1024, file);
                overlay.snapshot_matches_prefixes_with_mappings(
                    &mut reader,
                    &matcher,
                    group.volume,
                    &group.mappings,
                )?
            };
            for ordinal in matched {
                let ordinal = ordinal as usize;
                overlay.shadowed[ordinal / 8] |= 1 << (ordinal % 8);
            }
        }
        Ok(())
    }
}

fn raw_utf16_unit(path: &[u8], index: usize) -> u16 {
    u16::from_le_bytes([path[index * 2], path[index * 2 + 1]])
}

fn raw_starts_with(path: &[u8], prefix: &[u16]) -> bool {
    if path.len() < prefix.len() * 2 {
        return false;
    }
    prefix
        .iter()
        .enumerate()
        .all(|(index, unit)| raw_utf16_unit(path, index) == *unit)
}

fn raw_path_under(path: &[u8], prefix: &[u16]) -> bool {
    let units = path.len() / 2;
    if units == prefix.len() {
        return raw_starts_with(path, prefix);
    }
    if units <= prefix.len() || !raw_starts_with(path, prefix) {
        return false;
    }
    matches!(raw_utf16_unit(path, prefix.len()), 0x5c | 0x2f)
}

fn transformed_unit(path: &[u8], base_len: usize, replacement: &[u16], index: usize) -> u16 {
    if index < replacement.len() {
        replacement[index]
    } else {
        raw_utf16_unit(path, base_len + index - replacement.len())
    }
}

#[derive(Default)]
struct PrefixMatcherNode {
    children: Vec<(u16, usize)>,
    terminal: bool,
}

/// Temporary trie for one immutable-snapshot delete group.  Matching a path
/// against all delete prefixes used to scan every prefix linearly; this keeps
/// the same exact UTF-16 and separator-boundary semantics while sharing common
/// path components across the group.  The trie is dropped after that group.
struct PrefixMatcher {
    nodes: Vec<PrefixMatcherNode>,
}

impl PrefixMatcher {
    fn new(prefixes: &[Vec<u16>]) -> Self {
        let mut matcher = Self {
            nodes: vec![PrefixMatcherNode::default()],
        };
        for prefix in prefixes {
            let mut node = 0;
            for &unit in prefix {
                let next = matcher.nodes[node]
                    .children
                    .iter()
                    .find(|(child, _)| *child == unit)
                    .map(|(_, child)| *child);
                node = if let Some(next) = next {
                    next
                } else {
                    let next = matcher.nodes.len();
                    matcher.nodes.push(PrefixMatcherNode::default());
                    matcher.nodes[node].children.push((unit, next));
                    next
                };
            }
            matcher.nodes[node].terminal = true;
        }
        matcher
    }

    fn matches_with(&self, units: usize, mut unit_at: impl FnMut(usize) -> u16) -> bool {
        let mut node = 0;
        if self.nodes[node].terminal && (units == 0 || matches!(unit_at(0), 0x5c | 0x2f)) {
            return true;
        }
        for index in 0..units {
            let Some(next) = self.nodes[node]
                .children
                .iter()
                .find(|(unit, _)| *unit == unit_at(index))
                .map(|(_, child)| *child)
            else {
                return false;
            };
            node = next;
            if self.nodes[node].terminal
                && (index + 1 == units || matches!(unit_at(index + 1), 0x5c | 0x2f))
            {
                return true;
            }
        }
        false
    }

    fn matches_raw(&self, path: &[u8]) -> bool {
        self.matches_with(path.len() / 2, |index| raw_utf16_unit(path, index))
    }

    fn matches_transformed(&self, path: &[u8], base_len: usize, replacement: &[u16]) -> bool {
        let units = path.len() / 2;
        if units < base_len {
            return false;
        }
        let transformed_len = replacement.len() + units - base_len;
        self.matches_with(transformed_len, |index| {
            transformed_unit(path, base_len, replacement, index)
        })
    }
}

struct ObjectState {
    is_dir: bool,
    baseline_ordinals: Vec<u32>,
    baseline_paths: Vec<Vec<u16>>,
    paths: Vec<Vec<u16>>,
}

impl ObjectState {
    fn estimate_bytes(&self) -> usize {
        128 + self.baseline_ordinals.len() * 4
            + self
                .baseline_paths
                .iter()
                .chain(self.paths.iter())
                .map(|path| path.len() * 2 + 24)
                .sum::<usize>()
    }
}

struct Overlay {
    snapshot_path: PathBuf,
    snapshot: File,
    snapshot_bytes: u64,
    startup_snapshot: Option<Vec<u8>>,
    startup_cache_enabled: bool,
    identities: IdentityTable,
    shadowed: Vec<u8>,
    objects: HashMap<Key, ObjectState>,
    prefixes: Vec<PrefixMap>,
    prefix_revision: u64,
    last_seq: u64,
    batches: u64,
    changes: u64,
    stale: Option<String>,
    journal_stopped: bool,
    pending_objects: u64,
    end_seq: Option<u64>,
    last_operation: Option<String>,
    bytes_estimate: usize,
}

impl Overlay {
    fn new(snapshot: &Path, identities: &Path, entries: u64, snapshot_bytes: u64) -> Result<Self> {
        let file = File::open(snapshot)?;
        let shadow_bytes = usize::try_from(
            entries
                .checked_add(7)
                .ok_or_else(|| invalid("shadow bitset overflow"))?
                / 8,
        )?;
        Ok(Self {
            snapshot_path: snapshot.to_path_buf(),
            snapshot: file,
            snapshot_bytes,
            startup_snapshot: None,
            startup_cache_enabled: true,
            identities: IdentityTable::open(identities, snapshot_bytes, entries)?,
            shadowed: vec![0; shadow_bytes],
            objects: HashMap::new(),
            prefixes: Vec::new(),
            prefix_revision: 0,
            last_seq: 0,
            batches: 0,
            changes: 0,
            stale: None,
            journal_stopped: false,
            pending_objects: 0,
            end_seq: None,
            last_operation: None,
            bytes_estimate: shadow_bytes,
        })
    }

    /// Read the immutable snapshot once for bootstrap.  Directory-delete
    /// events may otherwise rescan the same 680 MiB file repeatedly.  This
    /// cache is dropped before ready; steady-state path reads stay on disk.
    fn load_startup_snapshot_cache(&mut self) -> Result<()> {
        if !self.startup_cache_enabled || self.startup_snapshot.is_some() {
            return Ok(());
        }
        let capacity = usize::try_from(self.snapshot_bytes)?;
        self.snapshot.seek(SeekFrom::Start(0))?;
        let mut cache = Vec::with_capacity(capacity);
        let mut reader = BufReader::with_capacity(1024 * 1024, &mut self.snapshot);
        reader.read_to_end(&mut cache)?;
        if cache.len() != capacity {
            return Err(invalid("startup snapshot cache length mismatch").into());
        }
        self.startup_snapshot = Some(cache);
        Ok(())
    }

    fn release_startup_caches(&mut self) {
        self.startup_snapshot.take();
        self.startup_cache_enabled = false;
        self.identities.release_startup_cache();
    }

    fn mark_stale(&mut self, reason: impl Into<String>) {
        if self.stale.is_none() {
            self.stale = Some(reason.into());
        }
    }

    fn shadow_range(&mut self, records: &[IdentityRecord]) {
        for record in records {
            let ordinal = record.ordinal as usize;
            self.shadowed[ordinal / 8] |= 1 << (ordinal % 8);
        }
    }

    fn baseline_paths(&mut self, records: &[IdentityRecord]) -> Result<Vec<Vec<u16>>> {
        records
            .iter()
            .map(|record| {
                read_snapshot_path(&mut self.snapshot, self.snapshot_bytes, record.offset)
            })
            .collect()
    }

    fn transform_path(&self, volume: u8, path: &[u16]) -> Vec<u16> {
        let best = self
            .prefixes
            .iter()
            .filter(|prefix| prefix.volume == volume && path_under(path, &prefix.base))
            .max_by_key(|prefix| prefix.base.len());
        best.map_or_else(
            || path.to_vec(),
            |prefix| replace_prefix(path, &prefix.base, &prefix.current),
        )
    }

    fn update_prefix(&mut self, volume: u8, base: &[u16], current: &[u16]) {
        if base.is_empty() {
            return;
        }
        let mut changed = false;
        self.prefixes.retain(|prefix| {
            let keep = !(prefix.volume == volume && prefix.base == base);
            changed |= !keep;
            keep
        });
        if base != current {
            self.prefixes.push(PrefixMap {
                volume,
                base: base.to_vec(),
                current: current.to_vec(),
            });
            changed = true;
        }
        if changed {
            self.prefix_revision = self.prefix_revision.wrapping_add(1);
        }
    }

    fn relocate_current_prefix(&mut self, volume: u8, old: &[u16], new: &[u16], except: Key) {
        if old == new || old.is_empty() {
            return;
        }
        let mut changed = false;
        for prefix in &mut self.prefixes {
            if prefix.volume == volume && path_under(&prefix.current, old) {
                prefix.current = replace_prefix(&prefix.current, old, new);
                changed = true;
            }
        }
        let keys: Vec<_> = self.objects.keys().copied().collect();
        for key in keys {
            if key == except || key.volume != volume {
                continue;
            }
            if let Some(state) = self.objects.get_mut(&key) {
                for path in &mut state.paths {
                    if path_under(path, old) {
                        *path = replace_prefix(path, old, new);
                    }
                }
            }
        }
        self.prefixes.retain(|prefix| {
            let keep = prefix.base != prefix.current;
            changed |= !keep;
            keep
        });
        if changed {
            self.prefix_revision = self.prefix_revision.wrapping_add(1);
        }
    }

    fn snapshot_path_matches_prefixes_with_mappings(
        &self,
        path: &[u8],
        matcher: &PrefixMatcher,
        volume: u8,
        mappings: &[PrefixMap],
    ) -> bool {
        let best = mappings
            .iter()
            .filter(|mapping| mapping.volume == volume && raw_path_under(path, &mapping.base))
            .max_by_key(|mapping| mapping.base.len());
        match best {
            Some(mapping) => {
                matcher.matches_transformed(path, mapping.base.len(), &mapping.current)
            }
            None => matcher.matches_raw(path),
        }
    }

    fn snapshot_matches_prefixes_with_mappings(
        &self,
        reader: &mut impl Read,
        matcher: &PrefixMatcher,
        volume: u8,
        mappings: &[PrefixMap],
    ) -> Result<Vec<u32>> {
        let entries = snapshot_header(reader)?;
        if entries != self.identities.entries {
            return Err(
                invalid("snapshot entry count changed while deleting directory subtree").into(),
            );
        }
        let mut offset = 16u64;
        let mut path = Vec::new();
        let mut matched = Vec::new();
        for ordinal in 0..usize::try_from(entries)? {
            if ordinal & 0x3fff == 0 {
                crate::cancel::check()?;
            }
            let consumed = path_record(reader, self.snapshot_bytes - offset, &mut path)?;
            if self.snapshot_path_matches_prefixes_with_mappings(&path, matcher, volume, mappings) {
                matched.push(u32::try_from(ordinal)?);
            }
            offset += consumed;
        }
        if offset != self.snapshot_bytes {
            return Err(invalid("snapshot trailing bytes while deleting directory subtree").into());
        }
        Ok(matched)
    }

    fn snapshot_matches_prefixes_bytes_with_mappings(
        &self,
        snapshot: &[u8],
        matcher: &PrefixMatcher,
        volume: u8,
        mappings: &[PrefixMap],
    ) -> Result<Vec<u32>> {
        let snapshot_bytes = usize::try_from(self.snapshot_bytes)?;
        if snapshot.len() != snapshot_bytes {
            return Err(invalid("startup snapshot cache length mismatch").into());
        }
        let mut header_reader = snapshot;
        let entries = snapshot_header(&mut header_reader)?;
        if entries != self.identities.entries {
            return Err(
                invalid("snapshot entry count changed while deleting directory subtree").into(),
            );
        }
        let mut offset = 16usize;
        let mut matched = Vec::new();
        for ordinal in 0..usize::try_from(entries)? {
            if ordinal & 0x3fff == 0 {
                crate::cancel::check()?;
            }
            if offset.checked_add(5).is_none_or(|end| end > snapshot.len()) {
                return Err(invalid("truncated snapshot record").into());
            }
            let flags = snapshot[offset];
            let length = u32::from_le_bytes(
                snapshot[offset + 1..offset + 5]
                    .try_into()
                    .expect("snapshot record header has four length bytes"),
            ) as usize;
            if flags > 1 || length % 2 != 0 {
                return Err(invalid("invalid snapshot record").into());
            }
            let path_start = offset + 5;
            let path_end = path_start
                .checked_add(length)
                .ok_or_else(|| invalid("snapshot record length overflow"))?;
            if path_end > snapshot.len() {
                return Err(invalid("truncated snapshot record").into());
            }
            if self.snapshot_path_matches_prefixes_with_mappings(
                &snapshot[path_start..path_end],
                matcher,
                volume,
                mappings,
            ) {
                matched.push(u32::try_from(ordinal)?);
            }
            offset = path_end;
        }
        if offset != snapshot.len() {
            return Err(invalid("snapshot trailing bytes while deleting directory subtree").into());
        }
        Ok(matched)
    }

    fn delete_directory_subtree(
        &mut self,
        current_prefixes: &[Vec<u16>],
        volume: u8,
        has_baseline: bool,
        snapshot_deletes: &mut SnapshotDeleteBatch,
    ) -> Result<()> {
        let moved_baseline_under_delete = self.prefixes.iter().any(|mapping| {
            mapping.volume == volume
                && current_prefixes
                    .iter()
                    .any(|prefix| path_under(&mapping.current, prefix))
        });
        if has_baseline || moved_baseline_under_delete {
            snapshot_deletes.add(
                volume,
                self.prefix_revision,
                &self.prefixes,
                current_prefixes,
            );
        }
        let keys: Vec<_> = self.objects.keys().copied().collect();
        for key in keys {
            if key.volume != volume {
                continue;
            }
            if let Some(state) = self.objects.get_mut(&key) {
                state.paths.retain(|path| {
                    !current_prefixes
                        .iter()
                        .any(|prefix| path_under(path, prefix))
                });
            }
            if self
                .objects
                .get(&key)
                .is_some_and(|state| state.paths.is_empty())
            {
                self.objects.remove(&key);
            }
        }
        let mut prefixes_changed = false;
        self.prefixes.retain(|mapping| {
            let keep = !current_prefixes
                .iter()
                .any(|prefix| path_under(&mapping.current, prefix));
            prefixes_changed |= !keep;
            keep
        });
        if prefixes_changed {
            self.prefix_revision = self.prefix_revision.wrapping_add(1);
        }
        self.last_operation = Some(
            if has_baseline || moved_baseline_under_delete {
                "directory_delete_scanned_snapshot"
            } else {
                "directory_delete_delta_only_no_baseline"
            }
            .to_owned(),
        );
        Ok(())
    }

    fn state_size(&self, key: Key) -> usize {
        self.objects
            .get(&key)
            .map_or(0, ObjectState::estimate_bytes)
    }

    fn estimate_bytes(&self) -> usize {
        self.shadowed
            .len()
            .saturating_add(
                self.prefixes
                    .iter()
                    .map(|prefix| 64 + (prefix.base.len() + prefix.current.len()) * 2)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.objects
                    .values()
                    .map(ObjectState::estimate_bytes)
                    .sum::<usize>(),
            )
    }

    /// Apply only the namespace movement caused by a directory event to the
    /// state that existed before this batch.  Fresh paths from this batch are
    /// committed in a second pass; otherwise a later directory event could
    /// rewrite an already-fresh absolute path and make the result depend on
    /// FRN ordering within the batch.
    fn apply_directory_transform(
        &mut self,
        change: &Change,
        snapshot_deletes: &mut SnapshotDeleteBatch,
    ) -> Result<()> {
        if !change.is_dir {
            return Ok(());
        }
        let records = self.identities.paths_for(change.key)?;
        if records.iter().any(|record| record.is_dir != change.is_dir) {
            return Err(invalid("live change directory type disagrees with identity table").into());
        }
        let baseline_paths = self.baseline_paths(&records)?;
        if change.paths.is_empty() {
            self.shadow_range(&records);
            let state_paths = self
                .objects
                .get(&change.key)
                .map(|state| state.paths.clone())
                .filter(|paths| !paths.is_empty())
                .unwrap_or_default();
            let current_prefixes = if !state_paths.is_empty() {
                state_paths
            } else {
                baseline_paths
                    .iter()
                    .map(|path| self.transform_path(change.key.volume, path))
                    .collect()
            };
            if !current_prefixes.is_empty() {
                self.delete_directory_subtree(
                    &current_prefixes,
                    change.key.volume,
                    !records.is_empty(),
                    snapshot_deletes,
                )?;
            }
            self.objects.remove(&change.key);
            return Ok(());
        }

        self.shadow_range(&records);
        let state_paths = self
            .objects
            .get(&change.key)
            .map(|state| state.paths.clone())
            .unwrap_or_default();
        let old_current_paths = if !state_paths.is_empty() {
            state_paths
        } else {
            baseline_paths
                .iter()
                .map(|path| self.transform_path(change.key.volume, path))
                .collect()
        };
        for (index, new) in change.paths.iter().enumerate() {
            let Some(old) = old_current_paths.get(index) else {
                break;
            };
            if old != new {
                self.relocate_current_prefix(change.key.volume, old, new, change.key);
            }
            if let Some(base) = baseline_paths.get(index) {
                self.update_prefix(change.key.volume, base, new);
            }
        }
        Ok(())
    }

    /// Commit the complete metadata paths reported for one object.  Directory
    /// prefix movement has already been applied to the pre-batch overlay, so
    /// this pass deliberately never relocates other fresh paths.
    fn commit_change(&mut self, change: Change) -> Result<()> {
        let records = self.identities.paths_for(change.key)?;
        if records.iter().any(|record| record.is_dir != change.is_dir) {
            return Err(invalid("live change directory type disagrees with identity table").into());
        }
        let baseline_paths = if change.is_dir {
            self.baseline_paths(&records)?
        } else {
            Vec::new()
        };
        self.shadow_range(&records);
        if change.paths.is_empty() {
            self.objects.remove(&change.key);
            return Ok(());
        }

        if change.is_dir {
            for (index, new) in change.paths.iter().enumerate() {
                if let Some(base) = baseline_paths.get(index) {
                    // Reassert the fresh directory target after phase one;
                    // this also restores the directory's own state when a
                    // parent delete/rename removed or moved it earlier in
                    // the same batch.
                    self.update_prefix(change.key.volume, base, new);
                }
            }
        }
        let new_paths = change.paths;
        let state = self
            .objects
            .entry(change.key)
            .or_insert_with(|| ObjectState {
                is_dir: change.is_dir,
                baseline_ordinals: records.iter().map(|record| record.ordinal).collect(),
                baseline_paths: baseline_paths.clone(),
                paths: Vec::new(),
            });
        if state.is_dir != change.is_dir {
            return Err(invalid("live object type changed").into());
        }
        if state.baseline_ordinals.is_empty() {
            state.baseline_ordinals = records.iter().map(|record| record.ordinal).collect();
        }
        if state.is_dir && state.baseline_paths.is_empty() {
            state.baseline_paths = baseline_paths;
        }
        state.paths = new_paths;
        Ok(())
    }

    fn apply_batch(&mut self, seq: u64, changes: Vec<Change>, pending_objects: u64) -> Result<()> {
        crate::cancel::check()?;
        if self.stale.is_some() {
            return Ok(());
        }
        if seq != self.last_seq.saturating_add(1) {
            return Err(invalid("live feed sequence gap").into());
        }
        if changes.len() > MAX_OBJECTS {
            return Err(invalid("live batch exceeds object limit").into());
        }
        let mut projected_objects = self.objects.len();
        let mut projected_sizes: HashMap<Key, usize> = HashMap::new();
        for change in &changes {
            if !projected_sizes.contains_key(&change.key) {
                projected_sizes.insert(change.key, self.state_size(change.key));
            }
            let old = projected_sizes[&change.key];
            if change.paths.is_empty() {
                if old != 0 {
                    projected_objects = projected_objects.saturating_sub(1);
                }
                projected_sizes.insert(change.key, 0);
            } else {
                if old == 0 {
                    projected_objects += 1;
                }
                let paths_bytes = change
                    .paths
                    .iter()
                    .map(|path| path.len() * 2 + 24)
                    .sum::<usize>();
                projected_sizes.insert(change.key, 128 + paths_bytes);
            }
        }
        let mut projected_bytes = self
            .shadowed
            .len()
            .saturating_add(
                self.prefixes
                    .iter()
                    .map(|p| 64 + (p.base.len() + p.current.len()) * 2)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.objects
                    .values()
                    .map(ObjectState::estimate_bytes)
                    .sum::<usize>(),
            );
        for (key, projected) in &projected_sizes {
            projected_bytes = projected_bytes
                .saturating_sub(self.state_size(*key))
                .saturating_add(*projected);
        }
        if projected_objects > MAX_OBJECTS || projected_bytes > MAX_BYTES {
            return Err(invalid("rebuild_required: live overlay bound exceeded").into());
        }
        let change_count = changes.len() as u64;
        // First move only the pre-existing overlay.  The journal refresh paths
        // are a same-round snapshot of the final namespace and are committed
        // untouched in the second pass, independent of FRN ordering.
        let mut snapshot_deletes = SnapshotDeleteBatch::default();
        for change in &changes {
            let prefix_revision = self.prefix_revision;
            self.apply_directory_transform(change, &mut snapshot_deletes)?;
            if self.prefix_revision != prefix_revision {
                // Do not retain older mapping snapshots while later renames
                // clone and extend the prefix table.  Deletes within one
                // unchanged revision remain merged.
                std::mem::take(&mut snapshot_deletes).apply(self)?;
            }
        }
        // Directory deletion effects above were applied in feed order.  Only
        // the immutable baseline membership checks are merged, grouped by the
        // exact prefix state captured at each delete.
        snapshot_deletes.apply(self)?;
        for change in changes {
            self.commit_change(change)?;
        }
        self.last_seq = seq;
        self.batches += 1;
        self.changes += change_count;
        self.pending_objects = pending_objects;
        self.bytes_estimate = self.estimate_bytes();
        if self.objects.len() > MAX_OBJECTS || self.bytes_estimate > MAX_BYTES {
            self.mark_stale("rebuild_required: live overlay bound exceeded after apply");
        }
        Ok(())
    }

    fn status(&self) -> Value {
        let shadowed = self
            .shadowed
            .iter()
            .map(|value| value.count_ones() as u64)
            .sum::<u64>();
        json!({
            "live": !self.journal_stopped,
            "stale": self.stale.is_some(),
            "error": self.stale,
            "last_seq": self.last_seq,
            "batches": self.batches,
            "changes": self.changes,
            "objects": self.objects.len(),
            "prefix_mappings": self.prefixes.len(),
            "shadowed_entries": shadowed,
            "overlay_bytes": self.bytes_estimate,
            "rebuild_required": self.stale.is_some(),
            "error_code": if self.stale.is_some(){Some("NeedsRebuild")}else{None},
            "journal_stopped": self.journal_stopped,
            "pending_objects": self.pending_objects,
            "end_seq": self.end_seq,
            "last_operation": self.last_operation,
        })
    }
}

fn parse_limit(value: &Value) -> Result<usize> {
    match value.get("limit") {
        None => Ok(20),
        Some(value) => value
            .as_u64()
            .filter(|limit| *limit <= 1000)
            .map(|limit| limit as usize)
            .ok_or_else(|| invalid("limit must be an integer from 0 to 1000").into()),
    }
}

fn matches_basename(path: &[u16], terms: &[Finder<'_>]) -> bool {
    let start = path
        .iter()
        .rposition(|unit| matches!(unit, 0x5c | 0x2f))
        .map_or(0, |index| index + 1);
    let name = String::from_utf16_lossy(&path[start..]).to_lowercase();
    terms
        .iter()
        .all(|term| term.find(name.as_bytes()).is_some())
}

fn live_query(
    search: &mut paged::Search,
    overlay: &Overlay,
    value: &Value,
    filter: &crate::filter::Filter,
) -> Result<Value> {
    live_query_inner(search, overlay, value, filter, true)
}

/// Limited interactive query.  It intentionally omits `total`: the base
/// table walk stops once the pre-dedup result limit is filled, so any count
/// would only be a lower bound.  The path arrays retain the full query's
/// ordering and base-before-overlay merge semantics.
fn live_query_limited(
    search: &mut paged::Search,
    overlay: &Overlay,
    value: &Value,
    filter: &crate::filter::Filter,
) -> Result<Value> {
    live_query_inner(search, overlay, value, filter, false)
}

fn live_query_inner(
    search: &mut paged::Search,
    overlay: &Overlay,
    value: &Value,
    filter: &crate::filter::Filter,
    exact_total: bool,
) -> Result<Value> {
    let started = Instant::now();
    let query = value
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("query must be a string"))?;
    let limit = parse_limit(value)?;
    if let Some(reason) = &overlay.stale {
        return Ok(
            json!({"error":"live feed stale","error_code":"NeedsRebuild","stale":true,"reason":reason}),
        );
    }
    if !exact_total {
        crate::cancel::check()?;
    }
    let lowercase = query.to_lowercase();
    let terms: Vec<_> = lowercase
        .split_whitespace()
        .map(|term| Finder::new(term.as_bytes()))
        .collect();
    let (base_total, hits) = if !exact_total && limit == 0 {
        // Filter validation and overlay health have already happened above;
        // a limited zero-result query need not touch the immutable index.
        (0, Vec::new())
    } else if filter.active() {
        let mut accept = |path: &[u16], is_dir: bool| {
            let volume = path.first().copied().map_or(0, |v| v as u8);
            filter.accepts(&overlay.transform_path(volume, path), is_dir)
        };
        if exact_total {
            search.query_base_filtered_with_shadow(query, &overlay.shadowed, limit, &mut accept)?
        } else {
            (
                0,
                search.query_base_filtered_limited_with_shadow(
                    query,
                    &overlay.shadowed,
                    limit,
                    &mut accept,
                )?,
            )
        }
    } else if exact_total {
        search.query_base_with_shadow(query, &overlay.shadowed, limit)?
    } else {
        (
            0,
            search.query_base_limited_with_shadow(query, &overlay.shadowed, limit)?,
        )
    };
    let mut total = base_total;
    let mut paths = Vec::with_capacity(limit);
    let mut paths_utf16 = Vec::with_capacity(limit);
    let mut is_dirs = Vec::with_capacity(limit);
    let mut seen = HashSet::<Vec<u16>>::new();
    for hit in hits {
        let volume = hit.path.first().copied().map_or(0, |value| value as u8);
        let path = overlay.transform_path(volume, &hit.path);
        if seen.insert(path.clone()) && paths.len() < limit {
            let (text, bad) = decode_path(&path);
            paths.push(text);
            is_dirs.push(hit.is_dir);
            paths_utf16.push(if bad { Some(path) } else { None });
        }
    }
    if exact_total || paths.len() < limit {
        let mut overlay_steps = 0usize;
        for state in overlay.objects.values() {
            for current in &state.paths {
                if !exact_total {
                    overlay_steps += 1;
                    if overlay_steps % 64 == 0 {
                        crate::cancel::check()?;
                    }
                }
                // Delta paths are already the current full metadata paths.
                // Prefix relocation is applied when a directory event arrives;
                // applying the baseline rewrite again here would move a
                // newly-created sibling under an old directory rename.
                let path = current;
                if !matches_basename(path, &terms) || !filter.accepts(path, state.is_dir) {
                    continue;
                }
                if exact_total {
                    total += 1;
                }
                if paths.len() < limit && seen.insert(path.clone()) {
                    let (text, bad) = decode_path(path);
                    paths.push(text);
                    is_dirs.push(state.is_dir);
                    paths_utf16.push(if bad { Some(path.clone()) } else { None });
                }
                if !exact_total && paths.len() >= limit {
                    break;
                }
            }
            if !exact_total && paths.len() >= limit {
                break;
            }
        }
    }

    if !exact_total {
        crate::cancel::check()?;
        return Ok(json!({
            "query": query,
            "paths": paths,
            "paths_utf16": paths_utf16,
            "is_dirs": is_dirs,
            "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
            "live": !overlay.journal_stopped,
            "last_seq": overlay.last_seq,
        }));
    }
    Ok(json!({
        "query": query,
        "total": total,
        "paths": paths,
        "paths_utf16": paths_utf16,
        "is_dirs": is_dirs,
        "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        "live": !overlay.journal_stopped,
        "last_seq": overlay.last_seq,
    }))
}

fn begin_fields(value: &Value) -> Result<&Value> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("live feed begin must be an object"))?;
    if object.get("kind").and_then(Value::as_str) != Some("begin") {
        return Err(invalid("live feed must start with kind=begin").into());
    }
    if object.get("scope").and_then(Value::as_str) != Some(super::mft::FILE_SEARCH_SCOPE) {
        return Err(invalid("live feed scope does not match the snapshot").into());
    }
    if object
        .get("generation")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(invalid("live feed generation is missing").into());
    }
    if object.get("volumes").and_then(Value::as_array).is_none() {
        return Err(invalid("live feed volumes are missing").into());
    }
    Ok(value)
}

fn provenance_begin(value: &Value) -> &Value {
    value
        .get("begin")
        .or_else(|| value.get("feed_begin"))
        .unwrap_or(value)
}

fn provenance_number(value: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(number) = value.get(*key).and_then(Value::as_u64) {
            return Some(number);
        }
    }
    value
        .get("snapshot")
        .and_then(|snapshot| provenance_number(snapshot, keys))
}

fn validate_provenance(
    path: &Path,
    begin: &Value,
    snapshot_bytes: u64,
    entries: u64,
) -> Result<()> {
    let sidecar: Value = serde_json::from_reader(BufReader::new(File::open(path)?))?;
    let side_begin = provenance_begin(&sidecar);
    for field in ["scope", "generation", "volumes"] {
        if side_begin.get(field) != begin.get(field) {
            return Err(invalid("identity provenance does not match feed begin").into());
        }
    }
    let side_entries = provenance_number(&sidecar, &["snapshot_entries", "entries"])
        .ok_or_else(|| invalid("identity provenance has no snapshot entry count"))?;
    let side_bytes = provenance_number(&sidecar, &["snapshot_bytes", "bytes", "size"])
        .ok_or_else(|| invalid("identity provenance has no snapshot byte count"))?;
    if side_entries != entries || side_bytes != snapshot_bytes {
        return Err(invalid("identity provenance snapshot count/size mismatch").into());
    }
    Ok(())
}

fn provenance_candidates(identities: &Path) -> [PathBuf; 2] {
    [
        PathBuf::from(format!("{}.provenance.json", identities.display())),
        identities.with_extension("provenance.json"),
    ]
}

fn first_complete_line(reader: &mut BufReader<File>) -> Result<Value> {
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 || !line.ends_with('\n') {
        return Err(invalid("live feed begin is missing or incomplete").into());
    }
    Ok(serde_json::from_str(line.trim_end())?)
}

fn feed_batch(value: &Value) -> Result<(u64, Vec<Change>, u64)> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("live batch must be an object"))?;
    if object.get("kind").and_then(Value::as_str) != Some("batch") {
        return Err(invalid("expected live batch").into());
    }
    let seq = object
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("live batch sequence is missing"))?;
    let changes = object
        .get("changes")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("live batch changes are missing"))?;
    if object.get("cursors").and_then(Value::as_array).is_none() {
        return Err(invalid("live batch cursors are missing").into());
    }
    let pending_objects = object
        .get("pending_objects")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("live batch pending_objects is missing"))?;
    let mut parsed = Vec::with_capacity(changes.len());
    for change in changes {
        parsed.push(parse_change(change)?);
    }
    Ok((seq, parsed, pending_objects))
}

fn validate_catchup(value: &Value, begin: &Value, state: &Overlay) -> Result<()> {
    if value["generation"] != begin["generation"]
        || value["scope"] != begin["scope"]
        || value["seq"].as_u64() != Some(state.last_seq)
        || value["pending_objects"] != 0
        || state.pending_objects != 0
    {
        return Err(invalid("catch-up marker generation/sequence/pending mismatch").into());
    }
    let volumes = begin["volumes"]
        .as_array()
        .ok_or_else(|| invalid("begin volumes missing"))?;
    let targets = value["targets"]
        .as_array()
        .ok_or_else(|| invalid("catch-up targets missing"))?;
    let cursors = value["cursors"]
        .as_array()
        .ok_or_else(|| invalid("catch-up cursors missing"))?;
    if targets.len() != volumes.len() || cursors.len() != volumes.len() {
        return Err(invalid("catch-up volume inventory differs").into());
    }
    let mut seen = HashSet::new();
    for target in targets {
        let volume = target["volume"]
            .as_str()
            .ok_or_else(|| invalid("catch-up volume missing"))?;
        if !seen.insert(volume) {
            return Err(invalid("duplicate catch-up target").into());
        }
        let expected = volumes
            .iter()
            .find(|v| v["volume"] == volume)
            .ok_or_else(|| invalid("unknown catch-up volume"))?;
        let cursor = cursors
            .iter()
            .find(|v| v["volume"] == volume)
            .ok_or_else(|| invalid("missing catch-up cursor"))?;
        let number = |v: &Value| -> Result<u64> {
            Ok(v.as_str()
                .ok_or_else(|| invalid("catch-up cursor must be decimal string"))?
                .parse()?)
        };
        if target["journal_id"] != expected["journal_id"]
            || target["volume_serial"] != expected["volume_serial"]
            || cursor["journal_id"] != expected["journal_id"]
            || number(&target["next_usn"])? < number(&expected["start_usn"])?
            || number(&cursor["next_usn"])? < number(&target["next_usn"])?
        {
            return Err(invalid("catch-up journal identity/cursor mismatch").into());
        }
    }
    Ok(())
}
fn tail_feed(
    mut reader: BufReader<File>,
    shared: Arc<Mutex<Overlay>>,
    stop: Arc<AtomicBool>,
    bootstrap_start: u64,
    bootstrap_cutoff: u64,
    bootstrap_done: Arc<AtomicBool>,
    begin: Value,
    allow_recorded_end: bool,
) {
    let mut partial = String::new();
    let mut last_activity = Instant::now();
    let mut read_offset = bootstrap_start;
    while !stop.load(Ordering::Acquire) {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                if let Ok(mut state) = shared.lock() {
                    if !state.journal_stopped && last_activity.elapsed() > HEARTBEAT_TIMEOUT {
                        state.mark_stale("live feed heartbeat timeout (>5s)");
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(100));
            }
            Ok(_) => {
                read_offset = read_offset.saturating_add(line.len() as u64);
                last_activity = Instant::now();
                partial.push_str(&line);
                if !line.ends_with('\n') {
                    continue;
                }
                let complete = std::mem::take(&mut partial);
                let value = match serde_json::from_str::<Value>(complete.trim_end()) {
                    Ok(value) => value,
                    Err(error) => {
                        if let Ok(mut state) = shared.lock() {
                            state.mark_stale(format!("invalid live feed JSON: {error}"));
                        }
                        break;
                    }
                };
                let kind = value.get("kind").and_then(Value::as_str);
                if kind == Some("batch") {
                    let parsed = feed_batch(&value).and_then(|(seq, changes, pending_objects)| {
                        let mut state = shared
                            .lock()
                            .map_err(|_| invalid("live overlay lock poisoned"))?;
                        state.apply_batch(seq, changes, pending_objects)
                    });
                    if let Err(error) = parsed {
                        if let Ok(mut state) = shared.lock() {
                            state.mark_stale(error.to_string());
                        }
                        break;
                    }
                } else if kind == Some("caught_up") {
                    if let Ok(mut state) = shared.lock() {
                        match validate_catchup(&value, &begin, &state) {
                            Ok(()) => bootstrap_done.store(true, Ordering::Release),
                            Err(error) => {
                                state.mark_stale(error.to_string());
                                break;
                            }
                        }
                    } else {
                        break;
                    }
                } else if kind == Some("error") {
                    if let Ok(mut state) = shared.lock() {
                        let reason = value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("live feed error");
                        state.mark_stale(reason.to_owned());
                    }
                    break;
                } else if kind == Some("end") {
                    let mut end_ok = false;
                    if let Ok(mut state) = shared.lock() {
                        let end_seq = value.get("seq").and_then(Value::as_u64);
                        if end_seq != Some(state.last_seq.saturating_add(1)) {
                            state.mark_stale(
                                "live feed end sequence does not follow the last batch",
                            );
                        } else {
                            state.end_seq = end_seq;
                            state.journal_stopped = true;
                            end_ok = true;
                        }
                    }
                    if end_ok
                        && allow_recorded_end
                        && partial.is_empty()
                        && read_offset >= bootstrap_cutoff
                    {
                        bootstrap_done.store(true, Ordering::Release);
                    } else if end_ok {
                        if let Ok(mut state) = shared.lock() {
                            state.mark_stale(
                                "metadata reader stopped; live search requires reconnection",
                            );
                        }
                    }
                    break;
                } else if let Ok(mut state) = shared.lock() {
                    state.mark_stale("unexpected live feed event kind");
                    break;
                } else {
                    break;
                }
            }
            Err(error) => {
                if let Ok(mut state) = shared.lock() {
                    state.mark_stale(format!("live feed read failed: {error}"));
                }
                break;
            }
        }
    }
}

/// Serve the existing paged index with a bounded USN/metadata overlay.
pub struct LiveSearch {
    search: paged::Search,
    shared: Arc<Mutex<Overlay>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl LiveSearch {
    pub fn open(generation: &crate::Generation, feed_jsonl: &Path) -> Result<Self> {
        Self::open_with_cancel(generation, feed_jsonl, Arc::new(AtomicBool::new(false)))
    }
    pub fn open_with_cancel(
        generation: &crate::Generation,
        feed_jsonl: &Path,
        cancel: crate::CancelToken,
    ) -> Result<Self> {
        Self::open_inner(generation, feed_jsonl, cancel, false)
    }
    /// Offline inspection of a complete recorded feed; status remains journal_stopped.
    pub fn open_recorded(
        generation: &crate::Generation,
        feed_jsonl: &Path,
        cancel: crate::CancelToken,
    ) -> Result<Self> {
        Self::open_inner(generation, feed_jsonl, cancel, true)
    }
    fn open_inner(
        generation: &crate::Generation,
        feed_jsonl: &Path,
        cancel: crate::CancelToken,
        allow_recorded_end: bool,
    ) -> Result<Self> {
        let _guard = crate::cancel::enter(cancel.clone());
        let snapshot = &generation.snapshot;
        let index = &generation.index;
        let identities = &generation.identities;

        let mut snapshot_file = File::open(snapshot)?;
        let snapshot_bytes = snapshot_file.metadata()?.len();
        let entries = snapshot_header(&mut snapshot_file)?;
        let mut feed_reader = BufReader::new(File::open(feed_jsonl)?);
        let begin = begin_fields(&first_complete_line(&mut feed_reader)?)?.clone();
        let bootstrap_cutoff = feed_reader.get_ref().metadata()?.len();
        let bootstrap_start = feed_reader.stream_position()?;
        if bootstrap_start > bootstrap_cutoff {
            return Err(invalid("live feed changed while reading its begin line").into());
        }
        let provenance = provenance_candidates(identities)
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| invalid("identity provenance sidecar is missing"))?;
        validate_provenance(&provenance, &begin, snapshot_bytes, entries)?;
        let search = paged::Search::open_for_live(snapshot, index)?;
        if search.entries() as u64 != entries {
            return Err(invalid("paged index entry count differs from snapshot").into());
        }
        let overlay = Overlay::new(snapshot, identities, entries, snapshot_bytes)?;
        let shared = Arc::new(Mutex::new(overlay));
        let stop = Arc::new(AtomicBool::new(false));
        let bootstrap_done = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stop = Arc::clone(&stop);
        let thread_bootstrap_done = Arc::clone(&bootstrap_done);
        let worker = thread::spawn(move || {
            let _guard = crate::cancel::enter(cancel);
            tail_feed(
                feed_reader,
                thread_shared,
                thread_stop,
                bootstrap_start,
                bootstrap_cutoff,
                thread_bootstrap_done,
                begin,
                allow_recorded_end,
            )
        });

        loop {
            if crate::cancel::cancelled() {
                stop.store(true, Ordering::Release);
                let _ = worker.join();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "live index opening cancelled",
                )
                .into());
            }
            let stale = match shared.lock() {
                Ok(state) => state.stale.clone(),
                Err(_) => {
                    stop.store(true, Ordering::Release);
                    let _ = worker.join();
                    return Err(invalid("live overlay lock poisoned during bootstrap").into());
                }
            };
            if let Some(reason) = stale {
                stop.store(true, Ordering::Release);
                let _ = worker.join();
                return Err(format!("live feed bootstrap failed: {reason}").into());
            }
            if bootstrap_done.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        let ready_error = shared
            .lock()
            .map(|state| {
                state.stale.clone().or_else(|| {
                    if state.journal_stopped && !allow_recorded_end {
                        Some("metadata reader stopped before live Ready".into())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(Some("overlay lock poisoned".into()));
        if let Some(error) = ready_error {
            stop.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(error.into());
        }
        match shared.lock() {
            Ok(mut state) => state.release_startup_caches(),
            Err(_) => {
                stop.store(true, Ordering::Release);
                let _ = worker.join();
                return Err(
                    invalid("live overlay lock poisoned while releasing startup cache").into(),
                );
            }
        }

        Ok(Self {
            search,
            shared,
            stop,
            worker: Some(worker),
        })
    }
    pub fn query(&mut self, query: &crate::Query) -> Result<Value> {
        let filter = crate::filter::Filter::new(query)?;
        let value = serde_json::json!({"query":query.query,"limit":query.limit});
        let state = self
            .shared
            .lock()
            .map_err(|_| invalid("live overlay lock poisoned"))?;
        live_query(&mut self.search, &state, &value, &filter)
    }
    /// Run the interactive limited query with cooperative cancellation.
    ///
    /// The response deliberately omits `total`; the base scan may stop once
    /// the pre-dedup limit is filled.  Existing `query` callers retain their
    /// exact-total behavior and response shape.
    pub fn query_limited_with_cancel(
        &mut self,
        query: &crate::Query,
        cancel: crate::CancelToken,
    ) -> Result<Value> {
        let filter = crate::filter::Filter::new(query)?;
        let value = serde_json::json!({"query":query.query,"limit":query.limit});
        let _guard = crate::cancel::enter(cancel);
        let state = self
            .shared
            .lock()
            .map_err(|_| invalid("live overlay lock poisoned"))?;
        live_query_limited(&mut self.search, &state, &value, &filter)
    }
    pub fn status(&self) -> Value {
        self.shared
            .lock()
            .map(|state| state.status())
            .unwrap_or_else(
                |_| serde_json::json!({"stale":true,"reason":"live overlay lock poisoned"}),
            )
    }
}
impl Drop for LiveSearch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
