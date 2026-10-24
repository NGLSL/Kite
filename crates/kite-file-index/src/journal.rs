//! Bounded, read-only USN polling. Saves names/identity, never file content.
use super::*;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

const REASONS: u32 = 0x0001_3300; // create/delete, both rename records, hardlink change
const POLL_BUFFER_BYTES: usize = 64 * 1024;
const ROUND_INTERVAL: Duration = Duration::from_millis(100);
const CHANGE_BATCH: usize = 128;
const CHANGE_BATCH_BYTES: usize = 1024 * 1024;
#[link(name = "kernel32")]
extern "system" {
    fn QueryPerformanceCounter(counter: *mut i64) -> i32;
    fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
}
fn qpc_ns() -> Result<u64> {
    let (mut ticks, mut frequency) = (0i64, 0i64);
    if unsafe { QueryPerformanceCounter(&mut ticks) } == 0
        || unsafe { QueryPerformanceFrequency(&mut frequency) } == 0
        || ticks < 0
        || frequency <= 0
    {
        return Err(last_error("QueryPerformanceCounter/Frequency").into());
    }
    Ok(u64::try_from(
        ticks as u128 * 1_000_000_000 / frequency as u128,
    )?)
}
fn drive(root: &OsString) -> Result<String> {
    let s = root
        .to_str()
        .ok_or_else(|| invalid("volume must be ASCII"))?;
    let b = s.as_bytes();
    if b.len() != 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return Err(invalid("volume must be a drive letter such as D:").into());
    }
    Ok(s.to_ascii_uppercase())
}
fn query(volume: &OwnedHandle) -> Result<Value> {
    let mut out = [0u8; 128];
    let n = ioctl(volume, 0x0009_00f4, &[], &mut out)?;
    if n < 56 {
        return Err(invalid("short USN journal data").into());
    }
    let id = u64_at(&out, 0);
    let first = u64_at(&out, 8);
    let next = u64_at(&out, 16);
    let lowest = u64_at(&out, 24);
    if first > next || lowest > next || next > i64::MAX as u64 {
        return Err(invalid("invalid journal bounds").into());
    }
    Ok(
        json!({"journal_id":id.to_string(),"first_usn":first.to_string(),
        "next_usn":next.to_string(),"lowest_valid_usn":lowest.to_string()}),
    )
}
fn number(v: &Value, key: &str) -> Result<u64> {
    Ok(v[key]
        .as_str()
        .ok_or_else(|| invalid("missing decimal identity field"))?
        .parse()?)
}
fn json_new(path: &Path, value: &Value) -> Result<()> {
    let mut out = BufWriter::new(OpenOptions::new().write(true).create_new(true).open(path)?);
    serde_json::to_writer_pretty(&mut out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    out.get_ref().sync_all()?;
    Ok(())
}
// Segment size controls transport I/O, never the lifetime of file search.
// The consumer acknowledges this session's frames after durable staging.
const FEED_SEGMENT_BYTES: u64 = 32 * 1024 * 1024;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
struct FeedWriter {
    out: BufWriter<File>,
    path: PathBuf,
    segment: u64,
    first_retained: u64,
    segment_bytes: u64,
    acknowledged: Arc<AtomicU64>,
    bytes: u64,
    seq: u64,
    last_output: Instant,
    last_pending: usize,
    last_cursors: Vec<Value>,
}
impl FeedWriter {
    fn segment_path(&self, segment: u64) -> PathBuf {
        if segment == 0 {
            self.path.clone()
        } else {
            self.path.with_file_name(format!("feed-{segment:06}.jsonl"))
        }
    }

    fn reclaim(&mut self) -> Result<()> {
        let acknowledged = self.acknowledged.load(Ordering::Acquire);
        while self.first_retained < self.segment {
            let path = self.segment_path(self.first_retained);
            let mut file = File::open(&path)?;
            let length = file.metadata()?.len();
            file.seek(SeekFrom::Start(length.saturating_sub(512)))?;
            let mut tail = Vec::new();
            file.read_to_end(&mut tail)?;
            let line = tail
                .split(|b| *b == b'\n')
                .rfind(|line| !line.is_empty())
                .ok_or_else(|| invalid("journal segment rotation footer missing"))?;
            let footer: Value = serde_json::from_slice(line)?;
            let through = footer["through_seq"]
                .as_u64()
                .filter(|_| footer["kind"] == "rotate")
                .ok_or_else(|| invalid("journal segment rotation footer invalid"))?;
            // A frame in a later segment proves the tail has switched away
            // from this one; equality alone could precede reading rotate.
            if acknowledged <= through {
                break;
            }
            drop(file);
            std::fs::remove_file(path)?;
            self.first_retained += 1;
        }
        Ok(())
    }
}
fn emit(out: &mut FeedWriter, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if out.segment_bytes >= FEED_SEGMENT_BYTES {
        let segment = out
            .segment
            .checked_add(1)
            .ok_or_else(|| invalid("journal segment number overflow"))?;
        let path = out.segment_path(segment);
        // Make the successor available before advertising it to the reader.
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.sync_all()?;
        let footer = json!({"kind":"rotate", "next":path.file_name().unwrap().to_string_lossy(),
            "through_seq":out.seq});
        let footer = serde_json::to_vec(&footer)?;
        out.out.write_all(&footer)?;
        out.out.write_all(b"\n")?;
        out.out.flush()?;
        out.bytes += footer.len() as u64 + 1;
        out.out = BufWriter::with_capacity(1024 * 1024, file);
        out.segment = segment;
        out.segment_bytes = 0;
    }
    out.out.write_all(&bytes)?;
    out.out.write_all(b"\n")?;
    out.out.flush()?;
    out.bytes = out
        .bytes
        .checked_add(bytes.len() as u64 + 1)
        .ok_or_else(|| invalid("journal feed size overflow"))?;
    out.segment_bytes += bytes.len() as u64 + 1;
    out.reclaim()?;
    Ok(())
}

fn emit_frame(out: &mut FeedWriter, mut frame: Value) -> Result<()> {
    let seq = out
        .seq
        .checked_add(1)
        .ok_or_else(|| invalid("journal sequence overflow"))?;
    frame["seq"] = json!(seq);
    frame["observed_qpc_ns"] = json!(qpc_ns()?);
    emit(out, &frame)?;
    out.seq = seq;
    out.last_output = Instant::now();
    out.last_pending = usize::try_from(frame["pending_objects"].as_u64().unwrap_or(0))?;
    out.last_cursors = frame["cursors"].as_array().cloned().unwrap_or_default();
    Ok(())
}

fn emit_batch(
    out: &mut FeedWriter,
    changes: Vec<Value>,
    cursors: &[Value],
    pending: usize,
    identity_only: bool,
) -> Result<()> {
    let mut batch = json!({"kind":"batch","changes":changes,
        "cursors":cursors,"pending_objects":pending});
    if identity_only {
        batch["identity_only"] = json!(true);
    }
    if out.seq % 10 == 0 {
        batch["watcher_memory"] = memory_now()?;
    }
    emit_frame(out, batch)
}

fn memory_now() -> Result<Value> {
    let mut memory = MemoryCounters {
        cb: std::mem::size_of::<MemoryCounters>() as u32,
        faults: 0,
        values: [0; 9],
    };
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut memory, memory.cb) } == 0 {
        return Err(last_error("GetProcessMemoryInfo").into());
    }
    Ok(json!({"working_set_bytes":memory.values[1],
        "peak_working_set_bytes":memory.values[0],"private_commit_bytes":memory.values[8]}))
}
pub(super) fn begin(checkpoint: &Path, roots: &[OsString]) -> Result<()> {
    let _privilege = backup_privilege()?;
    if roots.is_empty() {
        return Err(invalid("at least one volume required").into());
    }
    let mut volumes = Vec::new();
    for root in roots {
        let root = drive(root)?;
        if volumes.iter().any(|v: &Value| v["volume"] == root) {
            return Err(invalid("duplicate journal volume").into());
        }
        let volume = open(&format!(r"\\.\{root}"), 0x8000_0000, 0)?;
        let root_handle = open(&format!(r"\\?\{root}\"), 0, 0x0220_0000)?;
        let identity = info(&root_handle)?;
        let journal = query(&volume)?;
        volumes.push(json!({"volume":root,"volume_serial":identity[7],
            "root_frn":info_frn(&identity).to_string(),"journal_id":journal["journal_id"],
            "start_usn":journal["next_usn"],"initial_bounds":journal}));
    }
    let v = json!({"kind":"begin","scope":FILE_SEARCH_SCOPE,
        "generation":format!("{}-{}",std::process::id(),qpc_ns()?),
        "volumes":volumes,"observed_qpc_ns":qpc_ns()?,
        "read_only":true,"poll_ms":100});
    json_new(checkpoint, &v)?;
    crate::diagnostic!("{v}");
    Ok(())
}
struct Volume {
    handle: OwnedHandle,
    root: String,
    serial: u32,
    journal: u64,
    cursor: u64,
    record_size: usize,
}
#[derive(Clone)]
struct Changed {
    is_dir: bool,
}

// GET_NTFS_FILE_RECORD returns the requested in-use ordinal, or the nearest
// preceding one. A different ordinal/sequence proves the original FRN is gone;
// an OpenFileById failure alone (including delete-pending) does not.
fn disappeared(volume: &Volume, id: u64) -> Result<bool> {
    // NTFS_FILE_RECORD_OUTPUT_BUFFER includes ABI tail padding. Match the
    // existing collector's +16 allocation; only the 12-byte header is payload.
    let mut record = vec![0; volume.record_size + 16];
    let n = ioctl(&volume.handle, 0x0009_0068, &id.to_le_bytes(), &mut record)?;
    if n < 12 + volume.record_size
        || u32_at(&record, 8) as usize != volume.record_size
        || &record[12..16] != b"FILE"
    {
        return Err(invalid("invalid journal refresh MFT record length/signature").into());
    }
    let ordinal = u64_at(&record, 0) & 0x0000_ffff_ffff_ffff;
    let requested = id & 0x0000_ffff_ffff_ffff;
    if ordinal > requested || u32_at(&record, 56) as u64 != ordinal {
        return Err(invalid("invalid journal refresh MFT record ordinal").into());
    }
    let sequence = u16::from_le_bytes([record[28], record[29]]) as u64;
    let in_use = u16::from_le_bytes([record[34], record[35]]) & 1 != 0;
    Ok(ordinal != requested || sequence != id >> 48 || !in_use)
}
fn exact_id(volume: &Volume, frn: u64) -> Result<(OwnedHandle, [u32; 13])> {
    let id = FileId {
        size: std::mem::size_of::<FileId>() as u32,
        kind: 0,
        id: [frn, 0],
    };
    let handle = unsafe {
        OpenFileById(
            volume.handle.0,
            &id,
            0,
            7,
            std::ptr::null_mut(),
            0x0220_0000,
        )
    };
    if handle as isize == -1 {
        return Err(io::Error::last_os_error().into());
    }
    let handle = OwnedHandle(handle);
    let information = info(&handle)?;
    if info_frn(&information) != frn || information[7] != volume.serial {
        return Err(invalid("journal refresh identity/volume mismatch").into());
    }
    Ok((handle, information))
}
fn paths(volume: &Volume, id: u64, change: &Changed) -> Result<Vec<Vec<u16>>> {
    let (handle, information) = match exact_id(volume, id) {
        Ok(v) => v,
        Err(error) => {
            let code = error
                .downcast_ref::<io::Error>()
                .and_then(|e| e.raw_os_error());
            if matches!(code, Some(2 | 3 | 5 | 87 | 303 | 1168)) && disappeared(volume, id)? {
                return Ok(Vec::new());
            }
            return Err(error);
        }
    };
    let is_dir = information[0] & 16 != 0;
    if is_dir != change.is_dir {
        return Err(invalid("journal type changed at exact FRN").into());
    }
    if reserved_metadata_file(id, is_dir) || information[10] == 0 {
        return Ok(Vec::new());
    }
    if is_dir || information[10] == 1 {
        let mut path = final_path(&handle, &volume.root)?;
        if is_dir && path.len() == 3 && path[2] == b'\\' as u16 {
            path.pop();
        }
        return Ok(vec![path]);
    }
    let (_, links) = native_links(&handle, information[10])?;
    let mut paths = Vec::with_capacity(links.len());
    for link in links {
        let (parent, pi) = exact_id(volume, link.parent)?;
        if pi[0] & 16 == 0 {
            return Err(invalid("hardlink parent is not a directory").into());
        }
        let mut path = final_path(&parent, &volume.root)?;
        if path.last() != Some(&(b'\\' as u16)) {
            path.push(b'\\' as u16);
        }
        path.extend(link.name);
        if paths.contains(&path) {
            return Err(invalid("duplicate refreshed hardlink path").into());
        }
        paths.push(path);
    }
    // Reject a link-set race instead of treating a partial set as current.
    let after = info(&handle)?;
    if after[10] != information[10] || info_frn(&after) != id || after[7] != volume.serial {
        return Err(invalid("hardlink set changed while refreshing").into());
    }
    Ok(paths)
}
fn poll(
    volume: &mut Volume,
    cutoff: u64,
    pending: &mut Pending,
    vi: usize,
    buffer: &mut [u8],
    record_count: &mut u64,
) -> Result<bool> {
    crate::cancel::check()?;
    let journal = validate_journal(volume)?;
    if cutoff > number(&journal, "next_usn")? {
        return Err(invalid("journal catch-up target moved backwards").into());
    }
    if volume.cursor < cutoff {
        let mut input = [0u8; 48];
        input[..8].copy_from_slice(&volume.cursor.to_le_bytes());
        input[8..12].copy_from_slice(&REASONS.to_le_bytes());
        input[32..40].copy_from_slice(&volume.journal.to_le_bytes());
        input[40..42].copy_from_slice(&2u16.to_le_bytes());
        input[42..44].copy_from_slice(&2u16.to_le_bytes());
        let n = match ioctl(&volume.handle, 0x0009_00bb, &input, buffer) {
            // Retention can advance after validate_journal and before READ.
            Err(error) if error.raw_os_error() == Some(1181) => {
                return Err(crate::NeedsRebuild(format!(
                    "{} journal records expired while reading",
                    volume.root
                ))
                .into());
            }
            result => result?,
        };
        if n < 8 {
            return Err(invalid("short READ_USN_JOURNAL output").into());
        }
        let next = u64_at(buffer, 0);
        if next <= volume.cursor {
            return Err(invalid("journal read made no progress before cutoff").into());
        }
        let mut at = 8;
        let mut page = Vec::new();
        while at < n {
            crate::cancel::check()?;
            let mut name = Vec::new();
            let node = parse_record(&buffer[at..n], &mut name)?;
            let length = u32_at(buffer, at) as usize;
            let usn = u64_at(buffer, at + 24);
            let reason = u32_at(buffer, at + 40);
            if usn < volume.cursor
                || usn >= next
                || reason & REASONS == 0
                || name.iter().any(|x| matches!(*x, 0 | 47 | 92))
            {
                return Err(invalid("USN record cursor/reason/name invalid").into());
            }
            if usn >= cutoff {
                break;
            }
            if !reserved_metadata_file(node.frn, node.is_dir) {
                page.push((vi, node.frn, node.is_dir));
            }
            *record_count += 1;
            at += length;
        }
        pending.add_page(&page)?;
        volume.cursor = next.min(cutoff);
    }
    Ok(volume.cursor < cutoff)
}

fn validate_journal(volume: &Volume) -> Result<Value> {
    let journal = query(&volume.handle)?;
    if number(&journal, "journal_id")? != volume.journal
        || volume.cursor < number(&journal, "first_usn")?.max(number(&journal, "lowest_valid_usn")?)
        || volume.cursor > number(&journal, "next_usn")?
    {
        return Err(crate::NeedsRebuild(format!(
            "{} journal identity/retention changed",
            volume.root
        ))
        .into());
    }
    Ok(journal)
}

fn validate_journals(volumes: &[Volume]) -> Result<()> {
    for volume in volumes {
        crate::cancel::check()?;
        validate_journal(volume)?;
    }
    Ok(())
}

// Only a read page and a refresh page are resident. The coalesced identity
// set lives in a session-owned database and has no object-count ceiling.
struct Pending {
    db: rusqlite::Connection,
    count: usize,
}
impl Pending {
    fn new(feed: &Path) -> Result<Self> {
        let path = feed.with_extension("pending.sqlite");
        drop(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?,
        );
        let db = rusqlite::Connection::open(path)?;
        // This is replayable scratch, not the durable applied state. The
        // ordinary consumer alone publishes a recovery cursor after commit.
        db.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=OFF;
            PRAGMA cache_size=-4096; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;
            CREATE TABLE pending(volume INTEGER NOT NULL, frn BLOB NOT NULL,
            is_dir INTEGER NOT NULL, PRIMARY KEY(volume,frn)) WITHOUT ROWID;",
        )?;
        Ok(Self { db, count: 0 })
    }
    fn add_page(&mut self, changes: &[(usize, u64, bool)]) -> Result<()> {
        let tx = self.db.transaction()?;
        let mut added = 0;
        {
            let mut insert = tx.prepare_cached(
                "INSERT OR IGNORE INTO pending(volume,frn,is_dir) VALUES(?1,?2,?3)",
            )?;
            let mut existing =
                tx.prepare_cached("SELECT is_dir FROM pending WHERE volume=?1 AND frn=?2")?;
            for &(volume, frn, is_dir) in changes {
                let key = frn.to_be_bytes();
                if insert.execute(rusqlite::params![volume as i64, &key[..], is_dir])? != 0 {
                    added += 1;
                } else if existing.query_row(rusqlite::params![volume as i64, &key[..]], |row| {
                    row.get::<_, bool>(0)
                })? != is_dir
                {
                    return Err(invalid("journal identity changed directory type").into());
                }
            }
        }
        tx.commit()?;
        self.count += added;
        Ok(())
    }
    fn page(&self, after: Option<(usize, u64)>) -> Result<Vec<((usize, u64), Changed)>> {
        let mut query = self.db.prepare_cached(
            "SELECT volume,frn,is_dir FROM pending
             WHERE (volume,frn)>(?1,?2)
             ORDER BY volume,frn LIMIT 128",
        )?;
        let (volume, frn) = after.map_or((-1, 0), |(v, f)| (v as i64, f));
        let key = frn.to_be_bytes();
        let mut rows = query.query(rusqlite::params![volume, &key[..]])?;
        let mut page = Vec::with_capacity(CHANGE_BATCH);
        while let Some(row) = rows.next()? {
            let volume = usize::try_from(row.get::<_, i64>(0)?)?;
            let frn: Vec<u8> = row.get(1)?;
            let frn = u64::from_be_bytes(
                frn.as_slice()
                    .try_into()
                    .map_err(|_| invalid("invalid pending identity"))?,
            );
            page.push((
                (volume, frn),
                Changed {
                    is_dir: row.get(2)?,
                },
            ));
        }
        Ok(page)
    }
    fn clear(&mut self) -> Result<()> {
        self.db.execute("DELETE FROM pending", [])?;
        self.count = 0;
        Ok(())
    }
}

fn refresh_pending(
    volumes: &[Volume],
    pending: &Pending,
    out: &mut FeedWriter,
    cursors: &[Value],
) -> Result<u64> {
    let round = out
        .seq
        .checked_add(1)
        .ok_or_else(|| invalid("round overflow"))?;
    emit_frame(
        out,
        json!({"kind":"round_begin", "round":round,
        "cursors":cursors,"pending_objects":pending.count}),
    )?;
    let mut after = None;
    let mut changes = Vec::with_capacity(CHANGE_BATCH);
    let mut bytes = 0;
    let mut remaining = pending.count;
    let mut disappeared_count = 0;
    loop {
        let page = pending.page(after)?;
        if page.is_empty() {
            break;
        }
        for (key @ (vi, id), change) in page {
            crate::cancel::check()?;
            let mut failed_since = None::<Instant>;
            let paths = loop {
                crate::cancel::check()?;
                if out.last_output.elapsed() >= HEARTBEAT_INTERVAL {
                    validate_journals(volumes)?;
                    emit_frame(
                        out,
                        json!({"kind":"round_changes","round":round,
                        "changes":std::mem::take(&mut changes),"cursors":cursors,
                        "pending_objects":remaining}),
                    )?;
                    bytes = 0;
                }
                match paths(&volumes[vi], id, &change) {
                    Ok(paths) => break paths,
                    Err(error) => {
                        if failed_since.get_or_insert_with(Instant::now).elapsed()
                            >= Duration::from_secs(3)
                        {
                            return Err(format!(
                                "{} FRN {id} refresh failed: {error}",
                                volumes[vi].root
                            )
                            .into());
                        }
                        std::thread::sleep(ROUND_INTERVAL);
                    }
                }
            };
            let identity_only = paths.is_empty();
            disappeared_count += u64::from(identity_only);
            let item = json!({"volume":volumes[vi].root,"frn":id.to_string(),
                "is_dir":change.is_dir,"paths_utf16":paths,"identity_only":identity_only});
            let item_bytes = serde_json::to_vec(&item)?.len();
            if !changes.is_empty() && bytes + item_bytes > CHANGE_BATCH_BYTES {
                emit_frame(
                    out,
                    json!({"kind":"round_changes","round":round,
                    "changes":std::mem::take(&mut changes),"cursors":cursors,
                    "pending_objects":remaining}),
                )?;
                bytes = 0;
            }
            changes.push(item);
            bytes += item_bytes;
            remaining -= 1;
            after = Some(key);
            if changes.len() >= CHANGE_BATCH || bytes >= CHANGE_BATCH_BYTES {
                validate_journals(volumes)?;
                emit_frame(
                    out,
                    json!({"kind":"round_changes","round":round,
                    "changes":std::mem::take(&mut changes),"cursors":cursors,
                    "pending_objects":remaining}),
                )?;
                bytes = 0;
            }
        }
    }
    if !changes.is_empty() {
        emit_frame(
            out,
            json!({"kind":"round_changes","round":round,
            "changes":changes,"cursors":cursors,"pending_objects":0}),
        )?;
    }
    // No recovery cursor is published before all paths are refreshed and
    // journal continuity is checked. The consumer commits both together.
    validate_journals(volumes)?;
    emit_frame(
        out,
        json!({"kind":"round_commit","round":round,
        "cursors":cursors,"pending_objects":0}),
    )?;
    Ok(disappeared_count)
}
pub(super) fn watch(checkpoint: &Path, feed: &Path) -> Result<()> {
    watch_resumable(checkpoint, feed, None, Arc::new(AtomicU64::new(0)))
}

pub(super) fn watch_resumable(
    checkpoint: &Path,
    feed: &Path,
    resume: Option<Value>,
    acknowledged: Arc<AtomicU64>,
) -> Result<()> {
    crate::cancel::check()?;
    let _privilege = backup_privilege()?;
    let mut begin: Value = serde_json::from_reader(BufReader::new(File::open(checkpoint)?))?;
    if begin["kind"] != "begin" || begin["scope"] != FILE_SEARCH_SCOPE {
        return Err(invalid("unsupported journal checkpoint").into());
    }
    let original = begin["volumes"]
        .as_array()
        .filter(|volumes| !volumes.is_empty() && volumes.len() <= 26)
        .ok_or_else(|| invalid("journal checkpoint needs 1..26 volumes"))?;
    let mut starts = Vec::with_capacity(original.len());
    if let Some(resume) = &resume {
        let cursors = resume
            .as_array()
            .filter(|v| v.len() == original.len())
            .ok_or_else(|| invalid("resume volume inventory mismatch"))?;
        for (cursor, volume) in cursors.iter().zip(original) {
            if cursor.as_object().is_none_or(|v| v.len() != 3)
                || cursor["volume"] != volume["volume"]
                || number(cursor, "journal_id")? != number(volume, "journal_id")?
                || number(cursor, "next_usn")? < number(volume, "start_usn")?
                || number(cursor, "next_usn")? > i64::MAX as u64
            {
                return Err(invalid("resume identity/cursor mismatch").into());
            }
            starts.push(number(cursor, "next_usn")?);
        }
    } else {
        for volume in original {
            starts.push(number(volume, "start_usn")?);
        }
    }
    // Provenance fields remain the immutable capture identity. Only this
    // invocation's starting cursors differ, bound to durable consumer state.
    begin["resume_cursors"] = resume.unwrap_or(Value::Null);
    let started = Instant::now();
    let mut out = FeedWriter {
        out: BufWriter::with_capacity(
            1024 * 1024,
            OpenOptions::new().write(true).create_new(true).open(feed)?,
        ),
        path: feed.to_path_buf(),
        segment: 0,
        first_retained: 0,
        segment_bytes: 0,
        acknowledged,
        bytes: 0,
        seq: 0,
        last_output: Instant::now(),
        last_pending: 0,
        last_cursors: Vec::new(),
    };
    emit(&mut out, &begin)?;
    let mut report = json!({"mode":"journal-watch","pid":std::process::id(),
        "start_qpc_ns":qpc_ns()?,"status":"running","metadata_only":true,
        "journal_mutations":false,"raw_records_saved":false,
        "feed_segment_bytes":FEED_SEGMENT_BYTES,"pending_storage":"sqlite",
        "pending_cache_bytes":4*1024*1024,"refresh_batch_objects":CHANGE_BATCH,
        "heartbeat_ms":HEARTBEAT_INTERVAL.as_millis(),
        "peak_pending_objects":0,"reclaimed_objects":0});
    let result = (|| -> Result<()> {
        let mut volumes = Vec::new();
        for (vi, v) in begin["volumes"].as_array().unwrap().iter().enumerate() {
            let root = drive(&OsString::from(
                v["volume"]
                    .as_str()
                    .ok_or_else(|| invalid("missing volume"))?,
            ))?;
            if volumes.iter().any(|volume: &Volume| volume.root == root) {
                return Err(invalid("duplicate journal checkpoint volume").into());
            }
            let handle = open(&format!(r"\\.\{root}"), 0x8000_0000, 0)?;
            let rh = open(&format!(r"\\?\{root}\"), 0, 0x0220_0000)?;
            let ri = info(&rh)?;
            if Some(ri[7] as u64) != v["volume_serial"].as_u64()
                || info_frn(&ri) != number(v, "root_frn")?
            {
                return Err(invalid("journal volume/root identity changed").into());
            }
            let mut geometry = [0u8; 96];
            if ioctl(&handle, 0x0009_0064, &[], &mut geometry)? < geometry.len()
                || u64_at(&geometry, 0) as u32 != ri[7]
            {
                return Err(invalid("journal NTFS volume geometry/identity mismatch").into());
            }
            let record_size = u32_at(&geometry, 48) as usize;
            if !(512..=1024 * 1024).contains(&record_size) {
                return Err(invalid("unsupported journal MFT record size").into());
            }
            volumes.push(Volume {
                handle,
                root,
                serial: ri[7],
                journal: number(v, "journal_id")?,
                cursor: starts[vi],
                record_size,
            });
        }
        let mut targets = Vec::new();
        for volume in &volumes {
            let current = validate_journal(volume)?;
            targets.push(
                json!({"volume":volume.root,"journal_id":volume.journal.to_string(),
                "next_usn":current["next_usn"],"volume_serial":volume.serial}),
            );
        }
        let mut catchup_emitted = false;
        let mut buffer = vec![0; POLL_BUFFER_BYTES];
        let mut pending = Pending::new(feed)?;
        let mut next_volume = 0;
        let mut round_targets = targets
            .iter()
            .map(|target| number(target, "next_usn"))
            .collect::<Result<Vec<_>>>()?;
        let mut round_complete = false;
        let mut records = 0u64;
        let mut changed = 0u64;
        while !crate::cancel::cancelled() {
            let tick = Instant::now();
            if round_complete {
                round_targets = volumes
                    .iter()
                    .map(|volume| number(&query(&volume.handle)?, "next_usn"))
                    .collect::<Result<Vec<_>>>()?;
            }
            let previous_records = records;
            for offset in 0..volumes.len() {
                let vi = (next_volume + offset) % volumes.len();
                poll(
                    &mut volumes[vi],
                    round_targets[vi],
                    &mut pending,
                    vi,
                    &mut buffer,
                    &mut records,
                )?;
            }
            next_volume = (next_volume + 1) % volumes.len();
            report["peak_pending_objects"] = json!(report["peak_pending_objects"]
                .as_u64()
                .unwrap_or(0)
                .max(pending.count as u64));
            let cursors: Vec<_> = volumes
                .iter()
                .map(|v| {
                    json!({"volume":v.root,
                "journal_id":v.journal.to_string(),"next_usn":v.cursor.to_string()})
                })
                .collect();
            let more = volumes
                .iter()
                .zip(&round_targets)
                .any(|(volume, &target)| volume.cursor < target);
            let had_changes = !more && pending.count != 0;
            if had_changes {
                let before = pending.count;
                let reclaimed = refresh_pending(&volumes, &pending, &mut out, &cursors)?;
                changed += before as u64;
                report["reclaimed_objects"] =
                    json!(report["reclaimed_objects"].as_u64().unwrap_or(0) + reclaimed);
                pending.clear()?;
            }
            round_complete = !more;
            let caught_up_now = !catchup_emitted
                && round_complete
                && pending.count == 0
                && volumes.iter().zip(&targets).all(|(volume, target)| {
                    number(target, "next_usn").is_ok_and(|target| volume.cursor >= target)
                });
            // Collection heartbeats may report read cursors, but the consumer
            // persists them only with pending=0 outside an unfinished round.
            if out.seq == 0
                || pending.count != out.last_pending
                || caught_up_now
                || (!catchup_emitted && cursors != out.last_cursors)
                || out.last_output.elapsed() >= HEARTBEAT_INTERVAL
            {
                validate_journals(&volumes)?;
                emit_batch(&mut out, Vec::new(), &cursors, pending.count, false)?;
            }
            if caught_up_now {
                let seq = out.seq;
                emit(
                    &mut out,
                    &json!({"kind":"caught_up","seq":seq,
                    "generation":begin["generation"],"scope":begin["scope"],
                    "targets":targets,"cursors":cursors,"pending_objects":0}),
                )?;
                catchup_emitted = true;
            }
            if !(more && records != previous_records) {
                if let Some(delay) = ROUND_INTERVAL.checked_sub(tick.elapsed()) {
                    std::thread::sleep(delay);
                }
            }
        }
        report["batches"] = json!(out.seq);
        report["journal_records"] = json!(records);
        report["object_refreshes"] = json!(changed);
        report["status"] = json!("complete");
        let seq = out.seq + 1;
        emit(
            &mut out,
            &json!({"kind":"end","seq":seq,"observed_qpc_ns":qpc_ns()?}),
        )?;
        Ok(())
    })();
    if let Err(error) = &result {
        report["status"] = json!("failed");
        report["error"] = json!(error.to_string());
        let code = if error.downcast_ref::<crate::NeedsRebuild>().is_some() {
            "NeedsRebuild"
        } else {
            "MetadataReaderFailed"
        };
        report["error_code"] = json!(code);
        let message: String = error.to_string().chars().take(8192).collect();
        let _ = emit(
            &mut out,
            &json!({"kind":"error","error_code":code,"error":message}),
        );
    }
    report["feed_bytes"] = json!(out.bytes);
    report["feed_segments"] = json!(out.segment + 1);
    report["wall_ms"] = json!(started.elapsed().as_secs_f64() * 1000.0);
    report["end_qpc_ns"] = json!(qpc_ns()?);
    sample_memory(&mut report)?;
    let mut name = feed.as_os_str().to_os_string();
    name.push(".report.json");
    json_new(Path::new(&name), &report)?;
    crate::diagnostic!("{report}");
    result
}
