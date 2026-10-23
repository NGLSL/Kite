//! Durable disk-backed live overlay for the KFPAGED1 snapshot.
//!
//! Changed identities, directory mappings and incomplete journal rounds live
//! in SQLite with a fixed page cache. Only the baseline shadow bitmap remains
//! resident. A complete round commits its overlay and recovery cursors together.

use super::{decode_path, invalid, paged, path_record, snapshot_header, utf16_units, Result};
use memchr::memmem::Finder;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
#[path = "live_store.rs"]
mod live_store;
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
// Normal producer frames target about 1 MiB. A single NTFS identity can
// legally carry 1,024 long hard-link paths, so this is a corruption bound,
// independent of the number of changed identities in a journal round.
const MAX_FRAME_BYTES: u64 = 256 * 1024 * 1024;
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

fn raw_utf16_unit(path: &[u8], index: usize) -> u16 {
    u16::from_le_bytes([path[index * 2], path[index * 2 + 1]])
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
}

#[derive(Serialize, Deserialize)]
struct ObjectState {
    is_dir: bool,
    baseline_ordinals: Vec<u32>,
    baseline_paths: Vec<Vec<u16>>,
    paths: Vec<Vec<u16>>,
}

struct Overlay {
    snapshot_path: PathBuf,
    snapshot: File,
    snapshot_bytes: u64,
    identities: IdentityTable,
    shadowed: Vec<u8>,
    store: live_store::Store,
    round: Option<u64>,
    work_stop: Option<Arc<AtomicBool>>,
    stage_ordinal: i64,
    pending_deletes: Vec<Vec<u16>>,
    pending_delete_bytes: usize,
    prefix_cache: RefCell<HashMap<(u8, Vec<u16>), Option<Vec<u16>>>>,
    has_prefixes: Cell<Option<bool>>,
    last_seq: u64,
    batches: u64,
    changes: u64,
    stale: Option<String>,
    journal_stopped: bool,
    pending_objects: u64,
    end_seq: Option<u64>,
    last_operation: Option<String>,
}

fn units_blob(path: &[u16]) -> Vec<u8> {
    path.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

impl Overlay {
    fn new(
        snapshot: &Path,
        identities: &Path,
        entries: u64,
        snapshot_bytes: u64,
        store: live_store::Store,
    ) -> Result<Self> {
        let shadow_bytes = usize::try_from(
            entries
                .checked_add(7)
                .ok_or_else(|| invalid("shadow bitset overflow"))?
                / 8,
        )?;
        let shadowed = store.load_shadow(shadow_bytes)?;
        let mut identities = IdentityTable::open(identities, snapshot_bytes, entries)?;
        identities.release_startup_cache();
        Ok(Self {
            snapshot_path: snapshot.to_owned(),
            snapshot: File::open(snapshot)?,
            snapshot_bytes,
            identities,
            shadowed,
            store,
            round: None,
            work_stop: None,
            stage_ordinal: 0,
            pending_deletes: Vec::new(),
            pending_delete_bytes: 0,
            prefix_cache: RefCell::new(HashMap::new()),
            has_prefixes: Cell::new(None),
            last_seq: 0,
            batches: 0,
            changes: 0,
            stale: None,
            journal_stopped: false,
            pending_objects: 0,
            end_seq: None,
            last_operation: None,
        })
    }
    fn check_cancel(&self) -> Result<()> {
        crate::cancel::check()?;
        if self
            .work_stop
            .as_ref()
            .is_some_and(|stop| stop.load(Ordering::Acquire))
        {
            return Err(
                io::Error::new(io::ErrorKind::Interrupted, "live overlay work stopped").into(),
            );
        }
        Ok(())
    }
    fn save_view_sequence(&self, seq: u64) -> Result<()> {
        self.store.conn.execute("INSERT INTO metadata(key,value) VALUES('view_seq',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[seq.to_string()])?;
        Ok(())
    }
    fn release_startup_caches(&mut self) {
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
    fn object(&self, key: Key) -> Result<Option<ObjectState>> {
        let value: Option<String> = self
            .store
            .conn
            .query_row(
                "SELECT state FROM objects WHERE volume=?1 AND frn=?2",
                params![key.volume, key.frn.to_be_bytes().as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| Ok(serde_json::from_str(&value)?))
            .transpose()
    }
    fn put_object(&self, key: Key, state: &ObjectState) -> Result<()> {
        self.store.conn.execute("INSERT INTO objects(volume,frn,is_dir,state) VALUES(?1,?2,?3,?4) ON CONFLICT(volume,frn) DO UPDATE SET is_dir=excluded.is_dir,state=excluded.state",params![key.volume,key.frn.to_be_bytes().as_slice(),state.is_dir,serde_json::to_string(state)?])?;
        Ok(())
    }
    fn remove_object(&self, key: Key) -> Result<()> {
        self.store.conn.execute(
            "DELETE FROM objects WHERE volume=?1 AND frn=?2",
            params![key.volume, key.frn.to_be_bytes().as_slice()],
        )?;
        Ok(())
    }
    fn next_object(&self, after: Option<Key>) -> Result<Option<(Key, ObjectState)>> {
        let (volume, frn) = after.map_or((-1, Vec::new()), |key| {
            (key.volume as i64, key.frn.to_be_bytes().to_vec())
        });
        let row: Option<(u8,Vec<u8>,String)> = self.store.conn.prepare_cached("SELECT volume,frn,state FROM objects WHERE (volume,frn)>(?1,?2) ORDER BY volume,frn LIMIT 1")?.query_row(params![volume,frn],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        row.map(|(volume, frn, state)| {
            let frn: [u8; 8] = frn.try_into().map_err(|_| invalid("invalid stored FRN"))?;
            Ok((
                Key {
                    volume,
                    frn: u64::from_be_bytes(frn),
                },
                serde_json::from_str(&state)?,
            ))
        })
        .transpose()
    }
    fn next_prefix(&self, after: Option<(u8, Vec<u8>)>) -> Result<Option<PrefixMap>> {
        let (volume, base) = after.map_or((-1, Vec::new()), |(v, b)| (v as i64, b));
        let row: Option<(u8,Vec<u8>,Vec<u8>)> = self.store.conn.prepare_cached("SELECT volume,base,current FROM prefixes WHERE (volume,base)>(?1,?2) ORDER BY volume,base LIMIT 1")?.query_row(params![volume,base],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        Ok(row.map(|(volume, base, current)| PrefixMap {
            volume,
            base: utf16_units(&base),
            current: utf16_units(&current),
        }))
    }
    fn transform_path(&self, volume: u8, path: &[u16]) -> Result<Vec<u16>> {
        let has_prefixes = match self.has_prefixes.get() {
            Some(value) => value,
            None => {
                let value: bool = self.store.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM prefixes)",
                    [],
                    |row| row.get(0),
                )?;
                self.has_prefixes.set(Some(value));
                value
            }
        };
        if !has_prefixes {
            return Ok(path.to_vec());
        }
        // Only ancestors can match; the number of probes is bounded by path depth.
        let mut length = path.len();
        loop {
            let key = (volume, path[..length].to_vec());
            let cached = self.prefix_cache.borrow().get(&key).cloned();
            let target = if let Some(cached) = cached {
                cached
            } else {
                let target: Option<Vec<u8>> = self
                    .store
                    .conn
                    .query_row(
                        "SELECT current FROM prefixes WHERE volume=?1 AND base=?2",
                        params![volume, units_blob(&path[..length])],
                        |row| row.get(0),
                    )
                    .optional()?;
                let target = target.map(|bytes| utf16_units(&bytes));
                // Cache memory is bounded by both entry count and path lengths.
                if key.1.len() <= 1024 && target.as_ref().is_none_or(|path| path.len() <= 1024) {
                    let mut cache = self.prefix_cache.borrow_mut();
                    if cache.len() >= 512 {
                        cache.clear();
                    }
                    cache.insert(key, target.clone());
                }
                target
            };
            if let Some(target) = target {
                return Ok(replace_prefix(path, &path[..length], &target));
            }
            let Some(parent) = path[..length]
                .iter()
                .rposition(|unit| matches!(unit, 0x5c | 0x2f))
            else {
                break;
            };
            if parent == length {
                break;
            }
            length = parent;
        }
        Ok(path.to_vec())
    }
    fn update_prefix(&mut self, volume: u8, base: &[u16], current: &[u16]) -> Result<()> {
        if base.is_empty() {
            return Ok(());
        }
        let old: Option<Vec<u8>> = self
            .store
            .conn
            .query_row(
                "SELECT current FROM prefixes WHERE volume=?1 AND base=?2",
                params![volume, units_blob(base)],
                |row| row.get(0),
            )
            .optional()?;
        if (base == current && old.is_none())
            || (base != current && old.as_deref() == Some(units_blob(current).as_slice()))
        {
            return Ok(());
        }
        // Pending deletes refer to the namespace before this mapping edit.
        self.flush_pending_deletes()?;
        self.prefix_cache.borrow_mut().clear();
        self.has_prefixes.set(None);
        if base == current {
            self.store.conn.execute(
                "DELETE FROM prefixes WHERE volume=?1 AND base=?2",
                params![volume, units_blob(base)],
            )?;
        } else {
            self.store.conn.execute("INSERT INTO prefixes(volume,base,current) VALUES(?1,?2,?3) ON CONFLICT(volume,base) DO UPDATE SET current=excluded.current",params![volume,units_blob(base),units_blob(current)])?;
        }
        Ok(())
    }
    fn relocate_current_prefix(
        &mut self,
        volume: u8,
        old: &[u16],
        new: &[u16],
        except: Key,
    ) -> Result<()> {
        if old == new || old.is_empty() {
            return Ok(());
        }
        let mut after = None;
        while let Some(prefix) = self.next_prefix(after)? {
            self.check_cancel()?;
            after = Some((prefix.volume, units_blob(&prefix.base)));
            if prefix.volume == volume && path_under(&prefix.current, old) {
                self.update_prefix(
                    volume,
                    &prefix.base,
                    &replace_prefix(&prefix.current, old, new),
                )?;
            }
        }
        let mut after = None;
        while let Some((key, mut state)) = self.next_object(after)? {
            self.check_cancel()?;
            after = Some(key);
            if key == except || key.volume != volume {
                continue;
            }
            let mut changed = false;
            for path in &mut state.paths {
                if path_under(path, old) {
                    *path = replace_prefix(path, old, new);
                    changed = true;
                }
            }
            if changed {
                self.put_object(key, &state)?;
            }
        }
        Ok(())
    }
    fn queue_snapshot_delete(&mut self, prefix: &[u16]) -> Result<()> {
        let bytes = prefix.len().saturating_mul(2).saturating_add(24);
        if bytes > 1024 * 1024 {
            return Err(invalid("directory delete prefix exceeds path bound").into());
        }
        if self
            .pending_deletes
            .iter()
            .any(|queued| path_under(prefix, queued))
        {
            return Ok(());
        }
        // Within one unchanged namespace, deleting a parent also covers its
        // queued children. Keep the bounded group small for directory trees.
        self.pending_deletes
            .retain(|queued| !path_under(queued, prefix));
        self.pending_delete_bytes = self
            .pending_deletes
            .iter()
            .map(|queued| queued.len() * 2 + 24)
            .sum();
        if self.pending_deletes.len() >= 128
            || self.pending_delete_bytes.saturating_add(bytes) > 1024 * 1024
        {
            self.flush_pending_deletes()?;
        }
        self.pending_deletes.push(prefix.to_vec());
        self.pending_delete_bytes += bytes;
        Ok(())
    }
    fn flush_pending_deletes(&mut self) -> Result<()> {
        if self.pending_deletes.is_empty() {
            return Ok(());
        }
        let prefixes = std::mem::take(&mut self.pending_deletes);
        self.pending_delete_bytes = 0;
        let matcher = PrefixMatcher::new(&prefixes);
        let has_mappings: bool =
            self.store
                .conn
                .query_row("SELECT EXISTS(SELECT 1 FROM prefixes)", [], |row| {
                    row.get(0)
                })?;
        let mut reader = BufReader::with_capacity(1024 * 1024, File::open(&self.snapshot_path)?);
        let entries = snapshot_header(&mut reader)?;
        if entries != self.identities.entries {
            return Err(invalid("snapshot count changed during directory delete").into());
        }
        let mut offset = 16u64;
        let mut raw = Vec::new();
        for ordinal in 0..usize::try_from(entries)? {
            if ordinal & 0x3fff == 0 {
                self.check_cancel()?;
            }
            offset += path_record(&mut reader, self.snapshot_bytes - offset, &mut raw)?;
            if self.shadowed[ordinal / 8] & (1 << (ordinal % 8)) != 0 {
                continue;
            }
            let matched = if !has_mappings {
                matcher.matches_raw(&raw)
            } else {
                let units = utf16_units(&raw);
                let volume = units.first().copied().unwrap_or(0) as u8;
                let current = self.transform_path(volume, &units)?;
                matcher.matches_with(current.len(), |index| current[index])
            };
            if matched {
                self.shadowed[ordinal / 8] |= 1 << (ordinal % 8);
            }
        }
        if offset != self.snapshot_bytes {
            return Err(invalid("snapshot trailing bytes during directory delete").into());
        }
        Ok(())
    }
    fn delete_directory_subtree(
        &mut self,
        prefixes: &[Vec<u16>],
        volume: u8,
        has_baseline: bool,
    ) -> Result<()> {
        // A delta-only directory cannot contain baseline paths unless a
        // baseline prefix was previously moved beneath its current location.
        let mut moved_baseline_under_delete = false;
        if !has_baseline {
            let mut after = None;
            while let Some(mapping) = self.next_prefix(after)? {
                self.check_cancel()?;
                after = Some((mapping.volume, units_blob(&mapping.base)));
                if mapping.volume == volume
                    && prefixes
                        .iter()
                        .any(|prefix| path_under(&mapping.current, prefix))
                {
                    moved_baseline_under_delete = true;
                    break;
                }
            }
        }
        let scan_snapshot = has_baseline || moved_baseline_under_delete;
        if scan_snapshot {
            for prefix in prefixes {
                self.queue_snapshot_delete(prefix)?;
            }
        }
        let mut after = None;
        while let Some((key, mut state)) = self.next_object(after)? {
            self.check_cancel()?;
            after = Some(key);
            if key.volume != volume {
                continue;
            }
            let old = state.paths.len();
            state
                .paths
                .retain(|path| !prefixes.iter().any(|prefix| path_under(path, prefix)));
            if state.paths.is_empty() {
                self.remove_object(key)?;
            } else if old != state.paths.len() {
                self.put_object(key, &state)?;
            }
        }
        let mut after = None;
        while let Some(prefix) = self.next_prefix(after)? {
            self.check_cancel()?;
            after = Some((prefix.volume, units_blob(&prefix.base)));
            if prefix.volume == volume
                && prefixes
                    .iter()
                    .any(|current| path_under(&prefix.current, current))
            {
                self.flush_pending_deletes()?;
                self.prefix_cache.borrow_mut().clear();
                self.has_prefixes.set(None);
                self.store.conn.execute(
                    "DELETE FROM prefixes WHERE volume=?1 AND base=?2",
                    params![prefix.volume, units_blob(&prefix.base)],
                )?;
            }
        }
        self.last_operation = Some(
            if scan_snapshot {
                "directory_delete_scanned_snapshot"
            } else {
                "directory_delete_delta_only_no_baseline"
            }
            .into(),
        );
        Ok(())
    }
    fn validate_change(
        &mut self,
        change: &Change,
        identity_only: bool,
    ) -> Result<Vec<IdentityRecord>> {
        if identity_only && !change.paths.is_empty() {
            return Err(invalid("identity-only live change must have no paths").into());
        }
        let records = self.identities.paths_for(change.key)?;
        if records.iter().any(|record| record.is_dir != change.is_dir) {
            return Err(invalid("live change directory type disagrees with identity table").into());
        }
        if self
            .object(change.key)?
            .is_some_and(|state| state.is_dir != change.is_dir)
        {
            return Err(invalid("live object type changed").into());
        }
        Ok(records)
    }
    fn apply_directory_transform(&mut self, change: &Change) -> Result<()> {
        if !change.is_dir {
            return Ok(());
        }
        let records = self.validate_change(change, false)?;
        let baseline = self.baseline_paths(&records)?;
        let state = self.object(change.key)?;
        let old = if let Some(state) = state.filter(|state| !state.paths.is_empty()) {
            state.paths
        } else {
            baseline
                .iter()
                .map(|path| self.transform_path(change.key.volume, path))
                .collect::<Result<Vec<_>>>()?
        };
        self.shadow_range(&records);
        if change.paths.is_empty() {
            if !old.is_empty() {
                self.delete_directory_subtree(&old, change.key.volume, !records.is_empty())?;
            }
            self.remove_object(change.key)?;
            return Ok(());
        }
        for (index, new) in change.paths.iter().enumerate() {
            if let Some(old) = old.get(index) {
                self.relocate_current_prefix(change.key.volume, old, new, change.key)?;
            }
            if let Some(base) = baseline.get(index) {
                self.update_prefix(change.key.volume, base, new)?;
            }
        }
        Ok(())
    }
    fn commit_change(&mut self, change: Change, identity_only: bool) -> Result<()> {
        let records = self.validate_change(&change, identity_only)?;
        self.shadow_range(&records);
        if change.paths.is_empty() {
            self.remove_object(change.key)?;
            return Ok(());
        }
        let baseline = if change.is_dir {
            self.baseline_paths(&records)?
        } else {
            Vec::new()
        };
        if change.is_dir {
            for (index, path) in change.paths.iter().enumerate() {
                if let Some(base) = baseline.get(index) {
                    self.update_prefix(change.key.volume, base, path)?;
                }
            }
        }
        let mut state = self.object(change.key)?.unwrap_or(ObjectState {
            is_dir: change.is_dir,
            baseline_ordinals: Vec::new(),
            baseline_paths: Vec::new(),
            paths: Vec::new(),
        });
        if state.baseline_ordinals.is_empty() {
            state.baseline_ordinals = records.iter().map(|record| record.ordinal).collect();
        }
        if state.is_dir && state.baseline_paths.is_empty() {
            state.baseline_paths = baseline;
        }
        state.paths = change.paths;
        self.put_object(change.key, &state)
    }
    fn sequence(&self, seq: u64) -> Result<()> {
        if seq != self.last_seq.saturating_add(1) {
            Err(invalid("live feed sequence gap").into())
        } else {
            Ok(())
        }
    }
    fn apply_batch(
        &mut self,
        seq: u64,
        changes: Vec<Change>,
        pending: u64,
        identity_only: bool,
        cursors: &Value,
    ) -> Result<()> {
        self.sequence(seq)?;
        if self.round.is_some() || (changes.is_empty() && pending != 0) {
            if !changes.is_empty() {
                return Err(invalid("nonempty batch interleaved with live round").into());
            }
            self.last_seq = seq;
            self.pending_objects = pending;
            self.batches += 1;
            return Ok(());
        }
        if changes.is_empty() && self.store.cursors()?.as_ref() == Some(cursors) {
            self.last_seq = seq;
            self.pending_objects = pending;
            self.batches += 1;
            return Ok(());
        }
        for change in &changes {
            self.validate_change(change, identity_only)?;
        }
        self.store.begin()?;
        let result = (|| {
            if !identity_only {
                for change in &changes {
                    self.apply_directory_transform(change)?;
                }
            }
            self.flush_pending_deletes()?;
            let count = changes.len() as u64;
            for change in changes {
                self.commit_change(change, identity_only)?;
            }
            self.save_view_sequence(seq)?;
            self.store.commit(cursors, &self.shadowed)?;
            self.changes += count;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = self.store.rollback();
            self.pending_deletes.clear();
            self.pending_delete_bytes = 0;
            self.prefix_cache.borrow_mut().clear();
            self.has_prefixes.set(None);
            self.shadowed = self.store.load_shadow(self.shadowed.len())?;
            return Err(error);
        }
        self.last_seq = seq;
        self.pending_objects = pending;
        self.batches += 1;
        self.store.maintenance()?;
        Ok(())
    }
    fn stage_frame(&mut self, value: &Value) -> Result<()> {
        let seq = value["seq"]
            .as_u64()
            .ok_or_else(|| invalid("round sequence missing"))?;
        self.sequence(seq)?;
        let round = value["round"]
            .as_u64()
            .ok_or_else(|| invalid("round identity missing"))?;
        let pending = value["pending_objects"]
            .as_u64()
            .ok_or_else(|| invalid("round pending missing"))?;
        if !value["cursors"].is_array() {
            return Err(invalid("round cursors missing").into());
        }
        match value["kind"].as_str() {
            Some("round_begin") => {
                if self.round.is_some() {
                    return Err(invalid("nested live round").into());
                }
                self.store.conn.execute("DELETE FROM staging", [])?;
                self.round = Some(round);
                self.stage_ordinal = 0;
            }
            Some("round_changes") => {
                if self.round != Some(round) {
                    return Err(invalid("round changes identity mismatch").into());
                }
                let changes = value["changes"]
                    .as_array()
                    .ok_or_else(|| invalid("round changes missing"))?;
                if changes.len() > 128 {
                    return Err(invalid("round changes frame exceeds object bound").into());
                }
                for value in changes {
                    let change = parse_change(value)?;
                    let identity_only = change_identity_only(value)?;
                    self.validate_change(&change, identity_only)?;
                }
                self.store.conn.execute_batch("BEGIN IMMEDIATE")?;
                let result = (|| -> Result<()> {
                    for change in changes {
                        self.stage_ordinal += 1;
                        self.store.conn.execute(
                            "INSERT INTO staging(ordinal,change) VALUES(?1,?2)",
                            params![self.stage_ordinal, serde_json::to_string(change)?],
                        )?;
                    }
                    self.store.conn.execute_batch("COMMIT")?;
                    Ok(())
                })();
                if let Err(error) = result {
                    let _ = self.store.conn.execute_batch("ROLLBACK");
                    return Err(error);
                }
            }
            Some("round_commit") => {
                if self.round != Some(round) || pending != 0 {
                    return Err(invalid("round commit mismatch").into());
                }
                self.store.begin()?;
                let result = (|| -> Result<()> {
                    for phase in 0..2 {
                        if phase == 1 {
                            self.flush_pending_deletes()?;
                        }
                        let mut ordinal = 0i64;
                        loop {
                            let row:Option<(i64,String)>=self.store.conn.prepare_cached("SELECT ordinal,change FROM staging WHERE ordinal>?1 ORDER BY ordinal LIMIT 1")?.query_row([ordinal],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
                            let Some((next, text)) = row else { break };
                            ordinal = next;
                            self.check_cancel()?;
                            let value: Value = serde_json::from_str(&text)?;
                            let change = parse_change(&value)?;
                            let identity_only = change_identity_only(&value)?;
                            if phase == 0 {
                                if !identity_only {
                                    self.apply_directory_transform(&change)?;
                                }
                            } else {
                                self.commit_change(change, identity_only)?;
                            }
                        }
                    }
                    self.save_view_sequence(seq)?;
                    self.store.commit(&value["cursors"], &self.shadowed)?;
                    Ok(())
                })();
                if let Err(error) = result {
                    let _ = self.store.rollback();
                    self.pending_deletes.clear();
                    self.pending_delete_bytes = 0;
                    self.prefix_cache.borrow_mut().clear();
                    self.has_prefixes.set(None);
                    self.shadowed = self.store.load_shadow(self.shadowed.len())?;
                    return Err(error);
                }
                self.changes = self
                    .changes
                    .saturating_add(u64::try_from(self.stage_ordinal)?);
                self.round = None;
                self.store.maintenance()?;
            }
            _ => return Err(invalid("unexpected round frame").into()),
        }
        self.last_seq = seq;
        self.pending_objects = pending;
        self.batches += 1;
        Ok(())
    }
    fn status(&self) -> Value {
        let shadowed = self
            .shadowed
            .iter()
            .map(|value| value.count_ones() as u64)
            .sum::<u64>();
        let (objects, prefixes) = self.store.counts().unwrap_or((0, 0));
        json!({"live":!self.journal_stopped,"stale":self.stale.is_some(),"error":self.stale,"last_seq":self.last_seq,"batches":self.batches,"changes":self.changes,"objects":objects,"prefix_mappings":prefixes,"shadowed_entries":shadowed,"overlay_bytes":self.shadowed.len()+8*1024*1024,"rebuild_required":self.stale.is_some(),"error_code":if self.stale.is_some(){Some("NeedsRebuild")}else{None},"journal_stopped":self.journal_stopped,"pending_objects":self.pending_objects,"end_seq":self.end_seq,"last_operation":self.last_operation,"resume_cursors":self.store.cursors().ok().flatten()})
    }
}

fn change_identity_only(value: &Value) -> Result<bool> {
    match value.get("identity_only") {
        None => Ok(false),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid("change identity_only must be boolean").into()),
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
    let mut transform_error = None;
    let (base_total, hits) = if !exact_total && limit == 0 {
        // Filter validation and overlay health have already happened above;
        // a limited zero-result query need not touch the immutable index.
        (0, Vec::new())
    } else if filter.active() {
        let mut accept = |path: &[u16], is_dir: bool| {
            let volume = path.first().copied().map_or(0, |v| v as u8);
            match overlay.transform_path(volume, path) {
                Ok(path) => filter.accepts(&path, is_dir),
                Err(error) => {
                    transform_error = Some(error);
                    false
                }
            }
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
    if let Some(error) = transform_error {
        return Err(error);
    }
    let mut total = base_total;
    let mut paths = Vec::with_capacity(limit);
    let mut paths_utf16 = Vec::with_capacity(limit);
    let mut is_dirs = Vec::with_capacity(limit);
    let mut seen = HashSet::<Vec<u16>>::new();
    for hit in hits {
        let volume = hit.path.first().copied().map_or(0, |value| value as u8);
        let path = overlay.transform_path(volume, &hit.path)?;
        if seen.insert(path.clone()) && paths.len() < limit {
            let (text, bad) = decode_path(&path);
            paths.push(text);
            is_dirs.push(hit.is_dir);
            paths_utf16.push(if bad { Some(path) } else { None });
        }
    }
    if exact_total || paths.len() < limit {
        let mut overlay_steps = 0usize;
        let mut statement = overlay
            .store
            .conn
            .prepare("SELECT state FROM objects ORDER BY volume,frn")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let state: ObjectState = serde_json::from_str(row.get_ref(0)?.as_str()?)?;
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

fn feed_batch(value: &Value) -> Result<(u64, Vec<Change>, u64, bool)> {
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
    let identity_only = match object.get("identity_only") {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid("live batch identity_only must be boolean"))?,
        None => false,
    };
    let mut parsed = Vec::with_capacity(changes.len());
    for change in changes {
        let change = parse_change(change)?;
        if identity_only && !change.paths.is_empty() {
            return Err(invalid("identity-only live change must have no paths").into());
        }
        parsed.push(change);
    }
    Ok((seq, parsed, pending_objects, identity_only))
}

fn validate_catchup(value: &Value, begin: &Value, state: &Overlay) -> Result<()> {
    if value["generation"] != begin["generation"]
        || value["scope"] != begin["scope"]
        || value["seq"].as_u64() != Some(state.last_seq)
        || value["pending_objects"] != 0
        || state.pending_objects != 0
        || state.round.is_some()
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
    feed_path: PathBuf,
    ack: Arc<dyn Fn(u64) + Send + Sync>,
) {
    let parent = match feed_path.parent().and_then(|path| path.canonicalize().ok()) {
        Some(parent) => parent,
        None => {
            if let Ok(mut state) = shared.lock() {
                state.mark_stale("live feed parent missing");
            }
            return;
        }
    };
    let mut last_segment = 0u64;
    let mut crossed_bootstrap = false;
    let mut partial = String::new();
    let mut last_activity = Instant::now();
    let mut read_offset = bootstrap_start;
    while !stop.load(Ordering::Acquire) {
        let mut line = String::new();
        match Read::by_ref(&mut reader)
            .take(MAX_FRAME_BYTES + 1)
            .read_line(&mut line)
        {
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
                if partial.len() as u64 > MAX_FRAME_BYTES {
                    if let Ok(mut state) = shared.lock() {
                        state.mark_stale("live feed frame exceeds transport bound");
                    }
                    break;
                }
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
                if kind == Some("rotate") {
                    let rotated = (|| -> Result<BufReader<File>> {
                        let name = value["next"]
                            .as_str()
                            .ok_or_else(|| invalid("rotation filename missing"))?;
                        let digits = name
                            .strip_prefix("feed-")
                            .and_then(|s| s.strip_suffix(".jsonl"))
                            .ok_or_else(|| invalid("invalid rotation filename"))?;
                        if !(6..=20).contains(&digits.len())
                            || !digits.bytes().all(|byte| byte.is_ascii_digit())
                        {
                            return Err(invalid("invalid rotation filename").into());
                        }
                        let segment = digits.parse::<u64>()?;
                        if segment <= last_segment {
                            return Err(invalid("rotation segment did not advance").into());
                        }
                        let through = value["through_seq"]
                            .as_u64()
                            .ok_or_else(|| invalid("rotation sequence missing"))?;
                        if shared
                            .lock()
                            .map_err(|_| invalid("live overlay lock poisoned"))?
                            .last_seq
                            != through
                        {
                            return Err(invalid("rotation sequence mismatch").into());
                        }
                        let next = parent.join(name);
                        if next.symlink_metadata()?.file_type().is_symlink()
                            || next.canonicalize()?.parent() != Some(parent.as_path())
                        {
                            return Err(invalid("rotation escapes protected feed parent").into());
                        }
                        let file = File::open(next)?;
                        if !file.metadata()?.is_file() {
                            return Err(invalid("rotation target is not regular file").into());
                        }
                        last_segment = segment;
                        Ok(BufReader::new(file))
                    })();
                    match rotated {
                        Ok(next) => {
                            reader = next;
                            read_offset = 0;
                            crossed_bootstrap = true;
                        }
                        Err(error) => {
                            if let Ok(mut state) = shared.lock() {
                                state.mark_stale(error.to_string());
                            }
                            break;
                        }
                    }
                    continue;
                }
                if matches!(kind, Some("round_begin" | "round_changes" | "round_commit")) {
                    let result = shared
                        .lock()
                        .map_err(|_| invalid("live overlay lock poisoned"))
                        .and_then(|mut state| {
                            state
                                .stage_frame(&value)
                                .map_err(|error| invalid(&error.to_string()))
                        });
                    if let Err(error) = result {
                        if let Ok(mut state) = shared.lock() {
                            state.mark_stale(error.to_string());
                        }
                        break;
                    }
                } else if kind == Some("batch") {
                    let parsed = feed_batch(&value).and_then(
                        |(seq, changes, pending_objects, identity_only)| {
                            let mut state = shared
                                .lock()
                                .map_err(|_| invalid("live overlay lock poisoned"))?;
                            state.apply_batch(
                                seq,
                                changes,
                                pending_objects,
                                identity_only,
                                &value["cursors"],
                            )
                        },
                    );
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
                        if state.round.is_some() || state.pending_objects != 0 {
                            state.mark_stale("live feed ended before its round committed");
                        } else if end_seq != Some(state.last_seq.saturating_add(1)) {
                            state.mark_stale(
                                "live feed end sequence does not follow the last batch",
                            );
                        } else {
                            state.end_seq = end_seq;
                            state.journal_stopped = true;
                            end_ok = true;
                        }
                    }
                    if end_ok {
                        if let Some(seq) = value["seq"].as_u64() {
                            ack(seq);
                        }
                    }
                    if end_ok
                        && allow_recorded_end
                        && partial.is_empty()
                        && (crossed_bootstrap || read_offset >= bootstrap_cutoff)
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
                if let Some(seq) = value["seq"].as_u64() {
                    ack(seq);
                }
                // Local transaction work must not consume the producer's
                // heartbeat grace period after ACK releases its backpressure.
                last_activity = Instant::now();
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

fn snapshot_binding(begin: &Value, entries: u64, bytes: u64) -> Value {
    json!({"scope":begin["scope"],"generation":begin["generation"],"volumes":begin["volumes"],"snapshot_entries":entries,"snapshot_bytes":bytes})
}

struct ScratchDatabase(PathBuf);
impl Drop for ScratchDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(PathBuf::from(format!("{}{suffix}", self.0.display())));
        }
    }
}

/// Serve the existing paged index with a bounded USN/metadata overlay.
pub struct LiveSearch {
    search: paged::Search,
    query_overlay: Overlay,
    status_cache: RefCell<Value>,
    shared: Arc<Mutex<Overlay>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    _scratch: Option<ScratchDatabase>,
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
        Self::open_inner(generation, feed_jsonl, cancel, false, Arc::new(|_| {}))
    }
    /// Offline inspection of a complete recorded feed; status remains journal_stopped.
    pub fn open_recorded(
        generation: &crate::Generation,
        feed_jsonl: &Path,
        cancel: crate::CancelToken,
    ) -> Result<Self> {
        Self::open_inner(generation, feed_jsonl, cancel, true, Arc::new(|_| {}))
    }
    /// Recovery cursors belong to a fully committed overlay for this exact base.
    pub fn resume_cursors(generation: &crate::Generation) -> Result<Option<Value>> {
        let database = generation.root.join("live-state.sqlite");
        if !database.is_file() {
            return Ok(None);
        }
        let file = File::open(&generation.snapshot)?;
        let bytes = file.metadata()?.len();
        let entries = snapshot_header(&mut BufReader::new(file))?;
        let provenance = provenance_candidates(&generation.identities)
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| invalid("identity provenance sidecar missing"))?;
        let sidecar: Value = serde_json::from_reader(BufReader::new(File::open(&provenance)?))?;
        let begin = provenance_begin(&sidecar);
        validate_provenance(&provenance, begin, bytes, entries)?;
        let store = live_store::Store::open(
            &database,
            &snapshot_binding(begin, entries, bytes),
            usize::try_from(
                entries
                    .checked_add(7)
                    .ok_or_else(|| invalid("shadow bitset overflow"))?
                    / 8,
            )?,
        )?;
        store.cursors()
    }
    pub fn open_with_resume_and_ack(
        generation: &crate::Generation,
        feed_jsonl: &Path,
        cancel: crate::CancelToken,
        ack: Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Result<Self> {
        Self::open_inner(generation, feed_jsonl, cancel, false, ack)
    }
    fn open_inner(
        generation: &crate::Generation,
        feed_jsonl: &Path,
        cancel: crate::CancelToken,
        allow_recorded_end: bool,
        ack: Arc<dyn Fn(u64) + Send + Sync>,
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
        if allow_recorded_end
            && !begin
                .get("resume_cursors")
                .unwrap_or(&Value::Null)
                .is_null()
        {
            return Err(invalid("recorded resumed feed requires its matching initial overlay; replay a complete feed from the base").into());
        }
        let scratch = if allow_recorded_end {
            static NEXT_SCRATCH: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            Some(ScratchDatabase(feed_jsonl.with_file_name(format!(
                "live-replay-{}-{nonce}-{}.sqlite",
                std::process::id(),
                NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
            ))))
        } else {
            None
        };
        let database = scratch
            .as_ref()
            .map(|scratch| scratch.0.clone())
            .unwrap_or_else(|| generation.root.join("live-state.sqlite"));
        let store = live_store::Store::open(
            &database,
            &snapshot_binding(&begin, entries, snapshot_bytes),
            usize::try_from(
                entries
                    .checked_add(7)
                    .ok_or_else(|| invalid("shadow bitset overflow"))?
                    / 8,
            )?,
        )?;
        if !allow_recorded_end
            && begin.get("resume_cursors").unwrap_or(&Value::Null)
                != store.cursors()?.as_ref().unwrap_or(&Value::Null)
        {
            return Err(invalid("live feed resume cursors disagree with committed overlay").into());
        }
        let mut overlay = Overlay::new(snapshot, identities, entries, snapshot_bytes, store)?;
        let query_overlay = Overlay::new(
            snapshot,
            identities,
            entries,
            snapshot_bytes,
            live_store::Store::open_reader(&database)?,
        )?;
        let stop = Arc::new(AtomicBool::new(false));
        overlay.work_stop = Some(stop.clone());
        let shared = Arc::new(Mutex::new(overlay));
        let bootstrap_done = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stop = Arc::clone(&stop);
        let thread_bootstrap_done = Arc::clone(&bootstrap_done);
        let feed_path = feed_jsonl.to_owned();
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
                feed_path,
                ack,
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

        let status_cache = RefCell::new(
            shared
                .lock()
                .map_err(|_| invalid("live overlay lock poisoned"))?
                .status(),
        );
        Ok(Self {
            search,
            query_overlay,
            status_cache,
            shared,
            stop,
            worker: Some(worker),
            _scratch: scratch,
        })
    }
    pub fn query(&mut self, query: &crate::Query) -> Result<Value> {
        self.query_committed(query, true)
    }
    /// Query a stable committed WAL snapshot without taking the writer lock.
    pub fn query_limited_with_cancel(
        &mut self,
        query: &crate::Query,
        cancel: crate::CancelToken,
    ) -> Result<Value> {
        let _guard = crate::cancel::enter(cancel);
        self.query_committed(query, false)
    }
    fn query_committed(&mut self, query: &crate::Query, exact_total: bool) -> Result<Value> {
        crate::cancel::check()?;
        let filter = crate::filter::Filter::new(query)?;
        // Health refresh is constant-time: full status counts are reserved
        // for explicit status requests, rather than every search keystroke.
        match self.shared.try_lock() {
            Ok(state) => {
                self.query_overlay.stale = state.stale.clone();
                self.query_overlay.journal_stopped = state.journal_stopped;
                let mut cache = self.status_cache.borrow_mut();
                cache["stale"] = json!(state.stale.is_some());
                cache["error"] = json!(state.stale);
                cache["journal_stopped"] = json!(state.journal_stopped);
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                let cache = self.status_cache.borrow();
                self.query_overlay.stale = cache["error"].as_str().map(str::to_owned);
                self.query_overlay.journal_stopped = cache["journal_stopped"] == true;
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                self.query_overlay.stale = Some("live overlay lock poisoned".into());
            }
        }
        let overlay = &mut self.query_overlay;
        overlay.store.conn.execute_batch("BEGIN DEFERRED")?;
        let result = (|| -> Result<Value> {
            // First read pins both the persisted bitmap and namespace rows to
            // one complete writer commit. No staging rows enter this view.
            overlay.shadowed = overlay.store.load_shadow(overlay.shadowed.len())?;
            overlay.prefix_cache.borrow_mut().clear();
            overlay.has_prefixes.set(None);
            let seq: Option<String> = overlay
                .store
                .conn
                .query_row(
                    "SELECT value FROM metadata WHERE key='view_seq'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            overlay.last_seq = seq.map(|seq| seq.parse()).transpose()?.unwrap_or(0);
            let value = json!({"query":query.query,"limit":query.limit});
            if exact_total {
                live_query(&mut self.search, overlay, &value, &filter)
            } else {
                live_query_limited(&mut self.search, overlay, &value, &filter)
            }
        })();
        let rollback = overlay.store.conn.execute_batch("ROLLBACK");
        match result {
            Err(error) => Err(error),
            Ok(value) => {
                rollback?;
                Ok(value)
            }
        }
    }
    pub fn status(&self) -> Value {
        match self.shared.try_lock() {
            Ok(state) => {
                let value = state.status();
                *self.status_cache.borrow_mut() = value.clone();
                value
            }
            Err(std::sync::TryLockError::WouldBlock) => self.status_cache.borrow().clone(),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                json!({"stale":true,"error":"live overlay lock poisoned"})
            }
        }
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
