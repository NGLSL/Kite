//! Bounded, read-only USN polling. Saves names/identity, never file content.
use super::*;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::time::{Duration, Instant};

const REASONS: u32 = 0x0001_3300; // create/delete, both rename records, hardlink change
const FILE_DELETE: u32 = 0x200;
const MAX_PENDING: usize = 65_536;
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
// Session feeds are append-only, but bounded even for an indefinitely running
// service. Keep room for the terminal error so consumers cannot remain live.
const MAX_FEED_BYTES: u64 = 512 * 1024 * 1024;
const ERROR_RESERVE_BYTES: u64 = 64 * 1024;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
struct FeedWriter {
    out: BufWriter<File>,
    bytes: u64,
}
fn emit(out: &mut FeedWriter, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let next = out
        .bytes
        .checked_add(bytes.len() as u64)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| invalid("journal feed size overflow"))?;
    let limit = if value["kind"] == "error" {
        MAX_FEED_BYTES
    } else {
        MAX_FEED_BYTES - ERROR_RESERVE_BYTES
    };
    if next > limit {
        return Err(crate::NeedsRebuild(
            "session journal feed reached its 512 MiB limit; rebuild required".into(),
        )
        .into());
    }
    out.out.write_all(&bytes)?;
    out.out.write_all(b"\n")?;
    out.out.flush()?;
    out.bytes = next;
    Ok(())
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
}
#[derive(Clone)]
struct Changed {
    is_dir: bool,
    reasons: u32,
    since: Instant,
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
            if change.reasons & FILE_DELETE != 0 && matches!(code, Some(2 | 3 | 87 | 303 | 1168)) {
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
    pending: &mut BTreeMap<(usize, u64), Changed>,
    vi: usize,
    buffer: &mut [u8],
    record_count: &mut u64,
) -> Result<()> {
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
    let cutoff = number(&journal, "next_usn")?;
    let mut pages = 0;
    while volume.cursor < cutoff {
        pages += 1;
        if pages > 4096 {
            return Err(invalid("journal catch-up exceeded bounded batch").into());
        }
        let mut input = [0u8; 48];
        input[..8].copy_from_slice(&volume.cursor.to_le_bytes());
        input[8..12].copy_from_slice(&REASONS.to_le_bytes());
        input[32..40].copy_from_slice(&volume.journal.to_le_bytes());
        input[40..42].copy_from_slice(&2u16.to_le_bytes());
        input[42..44].copy_from_slice(&2u16.to_le_bytes());
        let n = ioctl(&volume.handle, 0x0009_00bb, &input, buffer)?;
        if n < 8 {
            return Err(invalid("short READ_USN_JOURNAL output").into());
        }
        let next = u64_at(buffer, 0);
        if next <= volume.cursor {
            return Err(invalid("journal read made no progress before cutoff").into());
        }
        let mut at = 8;
        while at < n {
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
            *record_count += 1;
            if !reserved_metadata_file(node.frn, node.is_dir) {
                pending
                    .entry((vi, node.frn))
                    .and_modify(|c| c.reasons |= reason)
                    .or_insert(Changed {
                        is_dir: node.is_dir,
                        reasons: reason,
                        since: Instant::now(),
                    });
            }
            if pending.len() > MAX_PENDING {
                return Err(invalid("journal pending-object limit: full rebuild required").into());
            }
            at += length;
        }
        volume.cursor = next;
    }
    Ok(())
}
pub(super) fn watch(checkpoint: &Path, feed: &Path) -> Result<()> {
    crate::cancel::check()?;
    let _privilege = backup_privilege()?;
    let begin: Value = serde_json::from_reader(BufReader::new(File::open(checkpoint)?))?;
    if begin["kind"] != "begin" || begin["scope"] != FILE_SEARCH_SCOPE {
        return Err(invalid("unsupported journal checkpoint").into());
    }
    let started = Instant::now();
    let mut out = FeedWriter {
        out: BufWriter::with_capacity(
            1024 * 1024,
            OpenOptions::new().write(true).create_new(true).open(feed)?,
        ),
        bytes: 0,
    };
    emit(&mut out, &begin)?;
    let mut report = json!({"mode":"journal-watch","pid":std::process::id(),
        "start_qpc_ns":qpc_ns()?,"status":"running","metadata_only":true,
        "journal_mutations":false,"raw_records_saved":false,"max_feed_bytes":MAX_FEED_BYTES,"heartbeat_ms":HEARTBEAT_INTERVAL.as_millis()});
    let result = (|| -> Result<()> {
        let mut volumes = Vec::new();
        for v in begin["volumes"]
            .as_array()
            .ok_or_else(|| invalid("missing volumes"))?
        {
            let root = drive(&OsString::from(
                v["volume"]
                    .as_str()
                    .ok_or_else(|| invalid("missing volume"))?,
            ))?;
            let handle = open(&format!(r"\\.\{root}"), 0x8000_0000, 0)?;
            let rh = open(&format!(r"\\?\{root}\"), 0, 0x0220_0000)?;
            let ri = info(&rh)?;
            if Some(ri[7] as u64) != v["volume_serial"].as_u64()
                || info_frn(&ri) != number(v, "root_frn")?
            {
                return Err(invalid("journal volume/root identity changed").into());
            }
            volumes.push(Volume {
                handle,
                root,
                serial: ri[7],
                journal: number(v, "journal_id")?,
                cursor: number(v, "start_usn")?,
            });
        }
        // Immutable base begin is unchanged; these are this invocation's catch-up targets.
        let mut targets = Vec::new();
        for volume in &volumes {
            let current = query(&volume.handle)?;
            if number(&current, "journal_id")? != volume.journal {
                return Err(invalid("journal identity changed before catch-up").into());
            }
            targets.push(json!({"volume":volume.root,"journal_id":volume.journal.to_string(),"next_usn":current["next_usn"],"volume_serial":volume.serial}));
        }
        let mut catchup_emitted = false;
        let mut buffer = vec![0; 1024 * 1024];
        let mut pending = BTreeMap::new();
        let mut seq = 0u64;
        let mut records = 0u64;
        let mut changed = 0u64;
        let mut last_output = Instant::now();
        let mut last_pending = 0usize;
        let mut last_cursors = Vec::<Value>::new();
        while !crate::cancel::cancelled() {
            let tick = Instant::now();
            for (vi, volume) in volumes.iter_mut().enumerate() {
                poll(volume, &mut pending, vi, &mut buffer, &mut records)?;
            }
            let mut changes = Vec::new();
            let mut done = Vec::new();
            for (&(vi, id), change) in &pending {
                match paths(&volumes[vi], id, change) {
                    Ok(paths) => {
                        changes.push(json!({"volume":volumes[vi].root,"frn":id.to_string(),
                            "is_dir":change.is_dir,"paths_utf16":paths}));
                        done.push((vi, id));
                    }
                    Err(error) if change.since.elapsed() < Duration::from_secs(3) => {
                        let _ = error; // retry only inside the explicit, bounded pending window
                    }
                    Err(error) => {
                        return Err(format!(
                            "{} FRN {id} refresh failed: {error}",
                            volumes[vi].root
                        )
                        .into())
                    }
                }
            }
            for key in done {
                pending.remove(&key);
            }
            changed += changes.len() as u64;
            let cursors:Vec<_> = volumes.iter().map(|v|json!({"volume":v.root,"journal_id":v.journal.to_string(),"next_usn":v.cursor.to_string()})).collect();
            let caught_up_now = !catchup_emitted
                && pending.is_empty()
                && volumes.iter().zip(&targets).all(|(volume, target)| {
                    number(target, "next_usn").is_ok_and(|target| volume.cursor >= target)
                });
            // After startup, cursor-only motion can wait for the next heartbeat.
            // Metadata polling and refresh retries still run on every 100ms tick.
            let emit_batch = seq == 0
                || !changes.is_empty()
                || pending.len() != last_pending
                || caught_up_now
                || (!catchup_emitted && cursors != last_cursors)
                || last_output.elapsed() >= HEARTBEAT_INTERVAL;
            if emit_batch {
                let next_seq = seq
                    .checked_add(1)
                    .ok_or_else(|| invalid("journal sequence overflow"))?;
                let mut batch = json!({"kind":"batch","seq":next_seq,"changes":changes,"cursors":cursors,"observed_qpc_ns":qpc_ns()?,"pending_objects":pending.len()});
                if next_seq % 10 == 1 {
                    batch["watcher_memory"] = memory_now()?;
                }
                emit(&mut out, &batch)?;
                seq = next_seq;
                last_pending = pending.len();
                last_cursors = cursors;
                last_output = Instant::now();
                if caught_up_now {
                    emit(
                        &mut out,
                        &json!({"kind":"caught_up","seq":seq,"generation":begin["generation"],"scope":begin["scope"],"targets":targets,"cursors":batch["cursors"],"pending_objects":0}),
                    )?;
                    catchup_emitted = true;
                }
            }
            if tick.elapsed() < Duration::from_millis(100) {
                std::thread::sleep(Duration::from_millis(100) - tick.elapsed());
            }
        }
        if !pending.is_empty() {
            return Err(invalid("pending refreshes remain when journal watcher stops").into());
        }
        report["batches"] = json!(seq);
        report["journal_records"] = json!(records);
        report["object_refreshes"] = json!(changed);
        report["status"] = json!("complete");
        emit(
            &mut out,
            &json!({"kind":"end","seq":seq+1,"observed_qpc_ns":qpc_ns()?}),
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
        // Bounded text leaves the reserved terminal frame below 64 KiB even
        // with JSON escaping; the report sidecar retains the complete error.
        let message: String = error.to_string().chars().take(8192).collect();
        let _ = emit(
            &mut out,
            &json!({"kind":"error","error_code":code,"error":message}),
        );
    }
    report["feed_bytes"] = json!(out.bytes);
    report["wall_ms"] = json!(started.elapsed().as_secs_f64() * 1000.0);
    report["end_qpc_ns"] = json!(qpc_ns()?);
    sample_memory(&mut report)?;
    let mut name = feed.as_os_str().to_os_string();
    name.push(".report.json");
    json_new(Path::new(&name), &report)?;
    crate::diagnostic!("{report}");
    result
}
