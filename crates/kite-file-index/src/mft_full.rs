//! Capture once with elevation; rebuild the same snapshot from saved API outputs offline.
use super::*;
use crate::SNAP_MAGIC;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
#[path = "mft_replay_fix.rs"]
mod replay_fix;

fn save_json(path: &Path, value: &Value) -> Result<()> {
    let mut file = fs::File::create(path)?;
    {
        let mut buffered = BufWriter::with_capacity(1024 * 1024, &mut file);
        serde_json::to_writer_pretty(&mut buffered, value)?;
        buffered.write_all(b"\n")?;
        buffered.flush()?;
    }
    file.sync_all()?;
    Ok(())
}
fn event(output: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}

// The shared snapshot writer historically emitted one `write_all` call per
// UTF-16 code unit.  A full D: capture contains hundreds of millions of code
// units, so the call boundary itself becomes measurable even behind a
// BufWriter.  Windows targets are little-endian; write the existing UTF-16
// representation as one byte slice without changing the on-disk format.
fn write_units_fast(output: &mut impl Write, units: &[u16]) -> Result<()> {
    let bytes = units
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or_else(|| invalid("UTF16 path length overflow"))?;
    output.write_all(&u32::try_from(bytes)?.to_le_bytes())?;
    #[cfg(target_endian = "little")]
    {
        // SAFETY: `u16` is two bytes and the slice remains borrowed for the
        // duration of write_all.  The experiment only targets little-endian
        // Windows, which is also the byte order of the snapshot format.
        let raw = unsafe { std::slice::from_raw_parts(units.as_ptr().cast::<u8>(), bytes) };
        output.write_all(raw)?;
    }
    #[cfg(target_endian = "big")]
    {
        for unit in units {
            output.write_all(&unit.to_le_bytes())?;
        }
    }
    Ok(())
}

fn write_path_fast(output: &mut impl Write, is_dir: bool, path: &[u16]) -> Result<()> {
    output.write_all(&[u8::from(is_dir)])?;
    write_units_fast(output, path)
}

/// Optional side table for the incremental prototype.  It deliberately stays
/// out of the existing snapshot format: one fixed 24-byte `<BB6xQII>` record
/// is appended for each path record in exactly the same order.  `offset` and `ordinal`
/// refer to the snapshot record about to be emitted (the 16-byte snapshot
/// header is therefore the initial offset).
struct IdentityWriter {
    output: BufWriter<fs::File>,
    offset: u64,
    ordinal: u64,
    records: u64,
}

impl IdentityWriter {
    fn create(path: &Path) -> Result<Self> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        Ok(Self {
            output: BufWriter::with_capacity(1024 * 1024, file),
            offset: 16,
            ordinal: 0,
            records: 0,
        })
    }

    fn record(&mut self, volume: u8, is_dir: bool, frn: u64, path: &[u16]) -> Result<()> {
        let path_bytes = path
            .len()
            .checked_mul(std::mem::size_of::<u16>())
            .ok_or_else(|| invalid("UTF16 path length overflow"))?;
        let record_bytes = 5u64
            .checked_add(u64::try_from(path_bytes)?)
            .ok_or_else(|| invalid("snapshot record length overflow"))?;
        let offset = u32::try_from(self.offset)
            .map_err(|_| invalid("snapshot offset exceeds identity side-table range"))?;
        let ordinal = u32::try_from(self.ordinal)
            .map_err(|_| invalid("snapshot ordinal exceeds identity side-table range"))?;
        let mut row = [0u8; 24];
        row[0] = volume;
        row[1] = u8::from(is_dir);
        row[8..16].copy_from_slice(&frn.to_le_bytes());
        row[16..20].copy_from_slice(&offset.to_le_bytes());
        row[20..24].copy_from_slice(&ordinal.to_le_bytes());
        self.output.write_all(&row)?;
        self.offset = self
            .offset
            .checked_add(record_bytes)
            .ok_or_else(|| invalid("snapshot offset overflow"))?;
        self.ordinal += 1;
        self.records += 1;
        Ok(())
    }

    fn finish(mut self, expected_records: u64) -> Result<()> {
        if self.records != expected_records || self.ordinal != expected_records {
            return Err(invalid("identity side-table count does not match snapshot").into());
        }
        self.output.flush()?;
        self.output.get_ref().sync_all()?;
        Ok(())
    }
}

fn emit_path(
    snapshot: &mut impl Write,
    identities: &mut Option<IdentityWriter>,
    volume: u8,
    frn: u64,
    is_dir: bool,
    path: &[u16],
) -> Result<()> {
    write_path_fast(snapshot, is_dir, path)?;
    if let Some(side) = identities.as_mut() {
        side.record(volume, is_dir, frn, path)?;
    }
    Ok(())
}
fn bytes(value: &Value) -> Result<Vec<u8>> {
    value
        .as_array()
        .ok_or_else(|| invalid("expected byte array"))?
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|&n| n <= 255)
                .map(|n| n as u8)
                .ok_or_else(|| invalid("invalid capture byte").into())
        })
        .collect()
}
fn units(value: &Value) -> Result<Vec<u16>> {
    value
        .as_array()
        .ok_or_else(|| invalid("expected UTF16 array"))?
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|&n| n <= 65535)
                .map(|n| n as u16)
                .ok_or_else(|| invalid("invalid capture UTF16 unit").into())
        })
        .collect()
}
fn frn(value: &Value) -> Result<u64> {
    Ok(value
        .as_str()
        .ok_or_else(|| invalid("expected FRN string"))?
        .parse()?)
}
fn directories(nodes: &[Node]) -> Result<HashMap<u64, usize>> {
    let mut result = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if index & 0x3fff == 0 {
            crate::cancel::check()?;
        }
        if node.is_dir && result.insert(node.frn, index).is_some() {
            return Err(invalid("duplicate directory FRN").into());
        }
    }
    Ok(result)
}
struct Failure {
    reason: &'static str,
    parent: u64,
}
fn resolve(
    index: usize,
    nodes: &[Node],
    names: &[u16],
    dirs: &HashMap<u64, usize>,
    root_frn: u64,
    root: &[u16],
    overrides: &HashMap<u64, Vec<u16>>,
) -> std::result::Result<Vec<u16>, Failure> {
    let node = &nodes[index];
    if node.frn == root_frn {
        return Ok(root.to_vec());
    }
    if let Some(path) = overrides.get(&node.frn) {
        return Ok(path.clone());
    }
    let mut chain = vec![index];
    let mut parent = node.parent;
    let prefix = loop {
        if parent == root_frn {
            break root;
        }
        if let Some(path) = overrides.get(&parent) {
            break path;
        }
        let Some(&part) = dirs.get(&parent) else {
            return Err(Failure {
                reason: "missing-parent",
                parent,
            });
        };
        if chain.contains(&part) {
            return Err(Failure {
                reason: "cycle",
                parent,
            });
        }
        chain.push(part);
        parent = nodes[part].parent;
    };
    let mut path = prefix.to_vec();
    for &part in chain.iter().rev() {
        let node = &nodes[part];
        let start = node.name_start as usize;
        path.push(b'\\' as u16);
        path.extend_from_slice(&names[start..start + node.name_len as usize]);
    }
    Ok(path)
}
struct FastState {
    serial: u32,
    record_size: usize,
    buffer: Vec<u8>,
    calls: u64,
    record_ms: f64,
    fast_files: u64,
    native_files: u64,
    native_ms: f64,
    native_link_calls: u64,
    native_link_ms: f64,
    fallback_reasons: HashMap<String, u64>,
    system_added: u64,
    system_root_verified: bool,
    system_unnamed: Vec<Value>,
    system_out_of_scope: Vec<Value>,
    system_unknown: Vec<Value>,
    workers: usize,
    pool: Option<FastReadPool>,
    pending: Option<HashMap<u64, VecDeque<std::result::Result<Vec<LinkName>, String>>>>,
}
impl FastState {
    fn new(volume: &OwnedHandle, root: &str, expected_root: u64) -> Result<Self> {
        let directory = open(&format!(r"{root}\"), 0, 0x0200_0000)?;
        let information = info(&directory)?;
        if info_frn(&information) != expected_root || information[0] & 16 == 0 {
            return Err(invalid("fast root identity/type changed").into());
        }
        let serial = information[7];
        let mut geometry = [0u8; 96];
        if ioctl(volume, 0x90064, &[], &mut geometry)? < 96 {
            return Err(invalid("short fast NTFS volume data").into());
        }
        if u64_at(&geometry, 0) as u32 != serial {
            return Err(invalid("fast volume/root serial mismatch").into());
        }
        let record_size = u32_at(&geometry, 48) as usize;
        if !(512..=1024 * 1024).contains(&record_size) {
            return Err(invalid("unsupported fast record size").into());
        }
        Ok(Self {
            serial,
            record_size,
            buffer: vec![0; record_size + 16],
            calls: 0,
            record_ms: 0.0,
            fast_files: 0,
            native_files: 0,
            native_ms: 0.0,
            native_link_calls: 0,
            native_link_ms: 0.0,
            fallback_reasons: HashMap::new(),
            system_added: 0,
            system_root_verified: false,
            system_unnamed: Vec::new(),
            system_out_of_scope: Vec::new(),
            system_unknown: Vec::new(),
            workers: 1,
            pool: None,
            pending: None,
        })
    }
    fn read(&mut self, volume: &OwnedHandle, id: u64) -> Result<usize> {
        self.calls += 1;
        let started = Instant::now();
        let result = ioctl(volume, 0x90068, &id.to_le_bytes(), &mut self.buffer);
        self.record_ms += started.elapsed().as_secs_f64() * 1000.0;
        let size = result?;
        if size < 12 + self.record_size
            || u32_at(&self.buffer, 8) as usize != self.record_size
            || &self.buffer[12..16] != b"FILE"
        {
            return Err(invalid("invalid fast FSCTL record length/signature").into());
        }
        let actual = u64_at(&self.buffer, 0) & 0x0000_ffff_ffff_ffff;
        if actual > id & 0x0000_ffff_ffff_ffff || u32_at(&self.buffer, 56) as u64 != actual {
            return Err(invalid("invalid fast FSCTL returned ordinal").into());
        }
        Ok(size)
    }
    fn reject(&mut self, reason: &str) {
        *self.fallback_reasons.entry(reason.to_owned()).or_default() += 1;
    }
    fn configure_workers(&mut self, root: &str, expected_root: u64, workers: usize) -> Result<()> {
        if !matches!(workers, 1 | 2) {
            return Err(invalid("fast record workers must be 1 or 2").into());
        }
        self.workers = workers;
        if workers == 2 {
            self.pool = Some(FastReadPool::new(root, expected_root, workers)?);
        }
        Ok(())
    }
    fn prepare_batch(&mut self, nodes: &[Node], scope: bool) -> Result<()> {
        if self.workers == 1 {
            return Ok(());
        }
        let pool = self
            .pool
            .as_mut()
            .ok_or_else(|| invalid("parallel fast record pool is not initialized"))?;
        let tasks: Vec<_> = nodes
            .iter()
            .filter(|node| !node.is_dir && !(scope && reserved_metadata_file(node.frn, false)))
            .map(|node| ReadTask {
                slot: 0,
                frn: node.frn,
                is_dir: node.is_dir,
            })
            .collect();
        if tasks.is_empty() {
            self.pending = Some(HashMap::new());
            return Ok(());
        }
        let mut tasks = tasks;
        for (slot, task) in tasks.iter_mut().enumerate() {
            task.slot = slot;
        }
        let batch = pool.read_tasks(&tasks)?;
        self.calls += batch.calls;
        self.record_ms += batch.record_ms;
        let mut pending = HashMap::with_capacity(batch.results.len());
        for outcome in batch.results {
            pending
                .entry(outcome.frn)
                .or_insert_with(VecDeque::new)
                .push_back(outcome.result);
        }
        let pending_results = pending.values().map(VecDeque::len).sum::<usize>();
        if pending_results != tasks.len() {
            return Err(invalid("parallel fast record batch lost a result").into());
        }
        self.pending = Some(pending);
        Ok(())
    }
    fn read_names(&mut self, volume: &OwnedHandle, id: u64, is_dir: bool) -> Result<Vec<LinkName>> {
        if self.workers > 1 {
            if let Some(pending) = self.pending.as_mut() {
                let queue = pending
                    .get_mut(&id)
                    .ok_or_else(|| invalid("parallel fast record batch missing FRN"))?;
                let result = queue
                    .pop_front()
                    .ok_or_else(|| invalid("parallel fast record batch exhausted FRN"))?;
                if queue.is_empty() {
                    pending.remove(&id);
                }
                return result.map_err(|error| error.into());
            }
        }
        let size = self.read(volume, id)?;
        replay_fix::resident_names(&self.buffer[..size], id, is_dir)
    }
    fn finish_workers(&mut self) -> Result<()> {
        if let Some(pool) = self.pool.take() {
            pool.finish()?;
        }
        self.pending = None;
        Ok(())
    }
    fn summary(&self) -> Value {
        json!({"get_record_calls":self.calls,"get_record_native_ms":self.record_ms,"record_size":self.record_size,"volume_serial":self.serial,"get_record_workers":self.workers,"exact_resident_fast_files":self.fast_files,"native_branch_files":self.native_files,"native_branch_ms":self.native_ms,"native_hardlink_calls":self.native_link_calls,"native_hardlink_ms":self.native_link_ms,"fallback_reasons":self.fallback_reasons,"system_nodes_added":self.system_added,"system_root_identity_verified":self.system_root_verified,"system_unnamed_slots":self.system_unnamed,"out_of_scope_reserved_system_records":self.system_out_of_scope,"system_unknown_records":self.system_unknown,"raw_record_bytes_saved":false,"fast_source":"exact-base/resident-FILE_NAME/no-ATTRIBUTE_LIST; Reserved2 mismatch falls back, never filters names"})
    }
}

const FAST_READ_BATCH: usize = 2048;

struct ReadTask {
    slot: usize,
    frn: u64,
    is_dir: bool,
}

struct ReadOutcome {
    slot: usize,
    frn: u64,
    result: std::result::Result<Vec<LinkName>, String>,
    failure_kind: Option<&'static str>,
    identity_valid: bool,
}

enum ReadCommand {
    Batch(Vec<ReadTask>),
    Stop,
}

enum ReadReply {
    Ready(std::result::Result<(), String>),
    Batch {
        results: Vec<ReadOutcome>,
        calls: u64,
        record_ms: f64,
    },
}

struct ReadBatch {
    results: Vec<ReadOutcome>,
    calls: u64,
    record_ms: f64,
}

struct FastReadPool {
    senders: Vec<Sender<ReadCommand>>,
    replies: Receiver<ReadReply>,
    joins: Vec<JoinHandle<()>>,
}

impl FastReadPool {
    fn new(root: &str, expected_root: u64, workers: usize) -> Result<Self> {
        if !matches!(workers, 1 | 2) {
            return Err(invalid("parallel fast record pool supports one or two workers").into());
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        let mut senders = Vec::with_capacity(workers);
        let mut joins = Vec::with_capacity(workers);
        for _ in 0..workers {
            let (command_tx, command_rx) = mpsc::channel();
            let reply_tx = reply_tx.clone();
            let root = root.to_owned();
            joins.push(thread::spawn(move || {
                let setup = (|| -> Result<(OwnedHandle, FastState)> {
                    let volume = open(&format!(r"\\.\{root}"), 0x8000_0000, 0)?;
                    let state = FastState::new(&volume, &root, expected_root)?;
                    Ok((volume, state))
                })();
                let (volume, mut state) = match setup {
                    Ok(value) => {
                        let _ = reply_tx.send(ReadReply::Ready(Ok(())));
                        value
                    }
                    Err(error) => {
                        let _ = reply_tx.send(ReadReply::Ready(Err(error.to_string())));
                        return;
                    }
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        ReadCommand::Stop => break,
                        ReadCommand::Batch(tasks) => {
                            let mut results = Vec::with_capacity(tasks.len());
                            for task in tasks {
                                let (result, failure_kind, identity_valid) =
                                    match state.read(&volume, task.frn) {
                                        Err(error) => {
                                            (Err(error.to_string()), Some("get-record"), false)
                                        }
                                        Ok(size) => {
                                            let identity_valid = probe_record_identity(
                                                &state.buffer[..size],
                                                task.frn,
                                                task.is_dir,
                                            );
                                            match replay_fix::resident_names(
                                                &state.buffer[..size],
                                                task.frn,
                                                task.is_dir,
                                            ) {
                                                Ok(names) => (Ok(names), None, identity_valid),
                                                Err(error) => (
                                                    Err(error.to_string()),
                                                    Some("resident-parser"),
                                                    identity_valid,
                                                ),
                                            }
                                        }
                                    };
                                results.push(ReadOutcome {
                                    slot: task.slot,
                                    frn: task.frn,
                                    result,
                                    failure_kind,
                                    identity_valid,
                                });
                            }
                            let _ = reply_tx.send(ReadReply::Batch {
                                results,
                                calls: state.calls,
                                record_ms: state.record_ms,
                            });
                            state.calls = 0;
                            state.record_ms = 0.0;
                        }
                    }
                }
            }));
            senders.push(command_tx);
        }
        drop(reply_tx);
        for _ in 0..workers {
            loop {
                match reply_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(ReadReply::Ready(Ok(()))) => break,
                    Ok(ReadReply::Ready(Err(error))) => {
                        for sender in &senders {
                            let _ = sender.send(ReadCommand::Stop);
                        }
                        for join in joins {
                            let _ = join.join();
                        }
                        return Err(
                            format!("parallel fast record worker setup failed: {error}").into()
                        );
                    }
                    Ok(ReadReply::Batch { .. }) => {
                        for sender in &senders {
                            let _ = sender.send(ReadCommand::Stop);
                        }
                        for join in joins {
                            let _ = join.join();
                        }
                        return Err(
                            invalid("parallel fast record worker replied before ready").into()
                        );
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if joins.iter().any(JoinHandle::is_finished) {
                            for sender in &senders {
                                let _ = sender.send(ReadCommand::Stop);
                            }
                            for join in joins {
                                let _ = join.join();
                            }
                            return Err(
                                invalid("parallel fast record worker exited during setup").into()
                            );
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        for sender in &senders {
                            let _ = sender.send(ReadCommand::Stop);
                        }
                        for join in joins {
                            let _ = join.join();
                        }
                        return Err(
                            invalid("parallel fast record worker setup channel closed").into()
                        );
                    }
                }
            }
        }
        Ok(Self {
            senders,
            replies: reply_rx,
            joins,
        })
    }
    fn read_tasks(&self, tasks: &[ReadTask]) -> Result<ReadBatch> {
        if tasks.is_empty() {
            return Ok(ReadBatch {
                results: Vec::new(),
                calls: 0,
                record_ms: 0.0,
            });
        }
        let worker_count = self.senders.len();
        let mut chunks: Vec<Vec<ReadTask>> = (0..worker_count).map(|_| Vec::new()).collect();
        for task in tasks {
            chunks[task.slot % worker_count].push(ReadTask {
                slot: task.slot,
                frn: task.frn,
                is_dir: task.is_dir,
            });
        }
        let mut expected = 0;
        for (sender, chunk) in self.senders.iter().zip(chunks) {
            if !chunk.is_empty() {
                sender
                    .send(ReadCommand::Batch(chunk))
                    .map_err(|_| invalid("parallel fast record worker input channel closed"))?;
                expected += 1;
            }
        }
        let mut results = Vec::with_capacity(tasks.len());
        let mut calls = 0;
        let mut record_ms = 0.0;
        for _ in 0..expected {
            loop {
                match self.replies.recv_timeout(Duration::from_millis(100)) {
                    Ok(ReadReply::Batch {
                        results: batch,
                        calls: batch_calls,
                        record_ms: batch_ms,
                    }) => {
                        results.extend(batch);
                        calls += batch_calls;
                        record_ms += batch_ms;
                        break;
                    }
                    Ok(ReadReply::Ready(_)) => {
                        return Err(
                            invalid("parallel fast record worker sent duplicate ready").into()
                        )
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if self.joins.iter().any(JoinHandle::is_finished) {
                            return Err(
                                invalid("parallel fast record worker exited during batch").into()
                            );
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(invalid(
                            "parallel fast record worker channel closed during batch",
                        )
                        .into())
                    }
                }
            }
        }
        results.sort_by_key(|outcome| outcome.slot);
        if results.len() != tasks.len()
            || results
                .iter()
                .zip(tasks)
                .any(|(outcome, task)| outcome.slot != task.slot || outcome.frn != task.frn)
        {
            return Err(invalid("parallel fast record worker returned an incomplete batch").into());
        }
        Ok(ReadBatch {
            results,
            calls,
            record_ms,
        })
    }
    fn finish(mut self) -> Result<()> {
        for sender in &self.senders {
            sender
                .send(ReadCommand::Stop)
                .map_err(|_| invalid("parallel fast record worker stop channel closed"))?;
        }
        for join in self.joins.drain(..) {
            join.join()
                .map_err(|_| invalid("parallel fast record worker panicked"))?;
        }
        Ok(())
    }
}

impl Drop for FastReadPool {
    fn drop(&mut self) {
        for sender in &self.senders {
            let _ = sender.send(ReadCommand::Stop);
        }
        for join in self.joins.drain(..) {
            let _ = join.join();
        }
    }
}

pub(super) struct ProbeRecord {
    pub frn: u64,
    pub class: String,
    pub identity_valid: bool,
    pub names: Vec<(u64, Vec<u16>)>,
}

pub(super) struct ProbeRun {
    pub workers: usize,
    pub root_frn: u64,
    pub volume_serial: u32,
    pub wall_ms: f64,
    pub native_ms: f64,
    pub calls: u64,
    pub records: Vec<ProbeRecord>,
    pub memory: Value,
}

fn probe_u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes
        .get(offset..offset.checked_add(2)?)
        .map(|value| u16::from_le_bytes([value[0], value[1]]))
}

/// Check the identity fields that make a GET_RECORD response belong to the
/// requested FRN.  The strict resident parser still owns name/layout
/// acceptance; this narrow check separates an invalid sequence/base/type
/// response from a valid record whose names need fallback.
fn probe_record_identity(raw: &[u8], requested: u64, is_dir: bool) -> bool {
    let ordinal = requested & 0x0000_ffff_ffff_ffff;
    if raw.len() < 12 + 40 || u64_at(raw, 0) != ordinal {
        return false;
    }
    let length = u32_at(raw, 8) as usize;
    if length < 48 || length > raw.len() - 12 {
        return false;
    }
    let record = &raw[12..12 + length];
    if record.get(..4) != Some(b"FILE")
        || probe_u16_at(record, 16).map(u64::from) != Some(requested >> 48)
        || probe_u16_at(record, 22).map(|flags| flags & 3) != Some(if is_dir { 3 } else { 1 })
        || u64_at(record, 32) != 0
    {
        return false;
    }
    true
}

pub(super) fn probe_volume_identity(root: &str) -> Result<(u64, u32)> {
    let volume = open(&format!(r"\\.\{root}"), 0x8000_0000, 0)?;
    let root_handle = open(&format!(r"{root}\"), 0, 0x0200_0000)?;
    let information = info(&root_handle)?;
    if information[0] & 16 == 0 {
        return Err(invalid("probe volume root is not a directory").into());
    }
    let serial = information[7];
    let mut geometry = [0u8; 96];
    if ioctl(&volume, 0x90064, &[], &mut geometry)? < 96 {
        return Err(invalid("probe volume geometry is incomplete").into());
    }
    if u64_at(&geometry, 0) as u32 != serial {
        return Err(invalid("probe volume/root serial mismatch").into());
    }
    Ok((info_frn(&information), serial))
}

fn probe_error_class(error: &str, failure_kind: Option<&str>) -> String {
    let upper = error.to_ascii_uppercase();
    if failure_kind == Some("get-record") {
        "io".to_owned()
    } else if upper.contains("ATTRIBUTE_LIST") {
        "attribute-list".to_owned()
    } else if upper.contains("FSCTL")
        || upper.contains("DEVICEIOCONTROL")
        || upper.contains("GET_RECORD")
        || upper.contains("WIN32")
    {
        "io".to_owned()
    } else if upper.contains("ORDINAL")
        || upper.contains("SIGNATURE")
        || upper.contains("FRN")
        || upper.contains("IDENTITY")
        || upper.contains("RECORD")
    {
        "identity-or-layout".to_owned()
    } else {
        "other".to_owned()
    }
}

fn make_probe_record(
    frn: u64,
    result: std::result::Result<Vec<LinkName>, String>,
    failure_kind: Option<&'static str>,
    identity_valid: bool,
) -> ProbeRecord {
    match result {
        Ok(names) => ProbeRecord {
            frn,
            class: "ok".to_owned(),
            identity_valid,
            names: names
                .into_iter()
                .map(|link| (link.parent, link.name))
                .collect(),
        },
        Err(error) => {
            let class = if !identity_valid && failure_kind != Some("get-record") {
                "identity-or-layout".to_owned()
            } else {
                probe_error_class(&error, failure_kind)
            };
            ProbeRecord {
                frn,
                class,
                identity_valid,
                names: Vec::new(),
            }
        }
    }
}

pub(super) fn probe_record_workers(
    root: &str,
    targets: &[(u64, bool)],
    workers: usize,
) -> Result<ProbeRun> {
    if targets.is_empty() {
        return Err(invalid("fast record probe sample is empty").into());
    }
    if !matches!(workers, 1 | 2) {
        return Err(invalid("fast record probe workers must be 1 or 2").into());
    }
    let (expected_root, volume_serial) = probe_volume_identity(root)?;
    let started = Instant::now();
    if workers == 1 {
        let volume = open(&format!(r"\\.\{root}"), 0x8000_0000, 0)?;
        let mut state = FastState::new(&volume, root, expected_root)?;
        let mut records = Vec::with_capacity(targets.len());
        for &(frn, is_dir) in targets {
            let (result, failure_kind, identity_valid) = match state.read(&volume, frn) {
                Err(error) => (Err(error.to_string()), Some("get-record"), false),
                Ok(size) => {
                    let identity_valid = probe_record_identity(&state.buffer[..size], frn, is_dir);
                    match replay_fix::resident_names(&state.buffer[..size], frn, is_dir) {
                        Ok(names) => (Ok(names), None, identity_valid),
                        Err(error) => (
                            Err(error.to_string()),
                            Some("resident-parser"),
                            identity_valid,
                        ),
                    }
                }
            };
            records.push(make_probe_record(frn, result, failure_kind, identity_valid));
        }
        let mut memory = json!({});
        if let Err(error) = sample_memory(&mut memory) {
            memory["error"] = json!(error.to_string());
        }
        return Ok(ProbeRun {
            workers,
            root_frn: expected_root,
            volume_serial,
            wall_ms: started.elapsed().as_secs_f64() * 1000.0,
            native_ms: state.record_ms,
            calls: state.calls,
            records,
            memory,
        });
    }
    let pool = FastReadPool::new(root, expected_root, workers)?;
    let mut records: Vec<Option<ProbeRecord>> = (0..targets.len()).map(|_| None).collect();
    let mut calls = 0;
    let mut native_ms = 0.0;
    for (batch_index, batch) in targets.chunks(FAST_READ_BATCH).enumerate() {
        let start = batch_index * FAST_READ_BATCH;
        let tasks: Vec<_> = batch
            .iter()
            .enumerate()
            .map(|(offset, &(frn, is_dir))| ReadTask {
                slot: offset,
                frn,
                is_dir,
            })
            .collect();
        let result = pool.read_tasks(&tasks)?;
        calls += result.calls;
        native_ms += result.record_ms;
        for outcome in result.results {
            let index = start + outcome.slot;
            let record = make_probe_record(
                outcome.frn,
                outcome.result,
                outcome.failure_kind,
                outcome.identity_valid,
            );
            if index >= records.len() || records[index].is_some() {
                return Err(invalid("fast record probe returned duplicate slot").into());
            }
            records[index] = Some(record);
        }
    }
    pool.finish()?;
    let records: Vec<_> = records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            record.ok_or_else(|| {
                format!("fast record probe lost result at sample index {index}").into()
            })
        })
        .collect::<Result<_>>()?;
    let mut memory = json!({});
    if let Err(error) = sample_memory(&mut memory) {
        memory["error"] = json!(error.to_string());
    }
    Ok(ProbeRun {
        workers,
        root_frn: expected_root,
        volume_serial,
        wall_ms: started.elapsed().as_secs_f64() * 1000.0,
        native_ms,
        calls,
        records,
        memory,
    })
}

fn add_system_nodes(
    volume: &OwnedHandle,
    root: &str,
    root_frn: u64,
    nodes: &mut Vec<Node>,
    names: &mut Vec<u16>,
    events: &mut impl Write,
    state: &mut FastState,
) -> Result<()> {
    for ordinal in 0u64..16 {
        let mut observed =
            json!({"kind":"system-record-unknown","ordinal":ordinal,"volume_serial":state.serial});
        let result = (|| -> Result<()> {
            let size = state.read(volume, ordinal)?;
            let raw = &state.buffer[..size];
            observed["returned_ordinal"] = json!(u64_at(raw, 0) & 0x0000_ffff_ffff_ffff);
            observed["returned_sequence"] = json!(u16::from_le_bytes([raw[28], raw[29]]));
            observed["returned_flags"] = json!(u16::from_le_bytes([raw[34], raw[35]]));
            observed["returned_base_frn"] = json!(u64_at(raw, 44).to_string());
            if u64_at(raw, 0) & 0x0000_ffff_ffff_ffff != ordinal {
                return Err(invalid("system record requested ordinal not in use/exact").into());
            }
            let sequence = u16::from_le_bytes([raw[28], raw[29]]) as u64;
            let flags = u16::from_le_bytes([raw[34], raw[35]]);
            if sequence == 0 || flags & 1 == 0 || u64_at(raw, 44) != 0 {
                return Err(invalid("system record sequence/inuse/base invalid").into());
            }
            let id = sequence << 48 | ordinal;
            let is_dir = flags & 2 != 0;
            if reserved_metadata_file(id, is_dir) {
                let evidence = json!({"kind":"system-reserved-file-out-of-scope","frn":id.to_string(),"ordinal":ordinal,"sequence":sequence,"volume_serial":state.serial,"returned_flags":flags,"base_frn":"0","file_search_scope":FILE_SEARCH_SCOPE});
                event(events, &evidence)?;
                state.system_out_of_scope.push(evidence);
                return Ok(());
            }
            if id == root_frn {
                if !is_dir {
                    return Err(invalid("system root identity is not directory").into());
                }
                event(
                    events,
                    &json!({"kind":"system-root-identity","frn":id.to_string(),"volume_serial":state.serial,"ordinal":ordinal,"sequence":sequence,"in_use":true,"is_dir":true,"base_frn":"0","already_emitted_as_drive_root":true}),
                )?;
                state.system_root_verified = true;
                return Ok(());
            }
            let (mut links, mut proof) = replay_fix::system_names(raw, id, is_dir)?;
            observed["proof"] = proof.clone();
            observed["representative_names"] = json!(links
                .iter()
                .map(|n| json!({"parent_frn":n.parent.to_string(),"name_utf16":n.name}))
                .collect::<Vec<_>>());
            if links.is_empty()
                && proof["has_attribute_list"] == false
                && proof["header_name_count"] == 0
            {
                let evidence = json!({"kind":"system-unnamed-slot","frn":id.to_string(),"ordinal":ordinal,"sequence":sequence,"proof":proof,"volume_serial":state.serial});
                event(events, &evidence)?;
                state.system_unnamed.push(evidence);
                return Ok(());
            }
            if nodes.iter().any(|node| node.frn == id) {
                return Ok(());
            }
            let needs_handle =
                links.is_empty() || (is_dir && replay_fix::resident(raw, id, true).is_err());
            if needs_handle {
                let (handle, information) = open_id(volume, id)?;
                if information[7] != state.serial
                    || (information[0] & 16 != 0) != is_dir
                    || information[10] == 0
                    || (is_dir && information[10] != 1)
                {
                    return Err(invalid(
                        "system representative handle serial/type/link-state invalid",
                    )
                    .into());
                }
                let full = final_path(&handle, root)?;
                let split = full
                    .iter()
                    .rposition(|c| *c == 92)
                    .ok_or_else(|| invalid("system current path lacks parent"))?;
                let parent = if split == 2 {
                    root_frn
                } else {
                    let parent_units: Vec<_> =
                        full[..split].iter().copied().chain(Some(0)).collect();
                    let raw = unsafe {
                        CreateFileW(
                            parent_units.as_ptr(),
                            0,
                            7,
                            std::ptr::null_mut(),
                            3,
                            0x0220_0000,
                            std::ptr::null_mut(),
                        )
                    };
                    if raw as isize == -1 {
                        return Err(last_error("system current parent metadata open").into());
                    }
                    let h = OwnedHandle(raw);
                    let info = info(&h)?;
                    if info[7] != state.serial || info[0] & 16 == 0 {
                        return Err(invalid("system current parent identity invalid").into());
                    }
                    info_frn(&info)
                };
                links = vec![LinkName {
                    parent,
                    name: full[split + 1..].to_vec(),
                }];
                proof["representative_handle_information"] = json!(information);
                proof["representative_current_path_utf16"] = json!(full);
            }
            let link = links
                .first()
                .ok_or_else(|| invalid("system record has unknown name set"))?;
            if link.name.is_empty() {
                return Err(invalid("empty system basename").into());
            }
            let start = u32::try_from(names.len())?;
            let length = u32::try_from(link.name.len())?;
            names.extend_from_slice(&link.name);
            nodes.push(Node {
                frn: id,
                parent: link.parent,
                name_start: start,
                name_len: length,
                is_dir,
            });
            state.system_added += 1;
            event(
                events,
                &json!({"kind":"system-name-node","frn":id.to_string(),"parent_frn":link.parent.to_string(),"name_utf16":link.name,"is_dir":is_dir,"volume_serial":state.serial,"proof":proof,"representative_only":true}),
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            observed["error"] = json!(error.to_string());
            event(events, &observed)?;
            state.system_unknown.push(observed);
        }
    }
    Ok(())
}
fn forensic(
    volume: &OwnedHandle,
    id: u64,
    events: &mut impl Write,
    fast: Option<&mut FastState>,
    is_dir: bool,
) -> Result<Option<(u64, u64)>> {
    let Some(fast) = fast else {
        forensic_legacy(volume, id, events)?;
        return Ok(None);
    };
    let mut detail = json!({"kind":"ntfs-name-metadata","requested_frn":id.to_string(),"volume_serial":fast.serial,"raw_record_bytes_saved":false});
    let returned_identity = match fast.read(volume, id) {
        Err(error) => {
            detail["metadata_error"] = json!(error.to_string());
            None
        }
        Ok(size) => {
            let b = &fast.buffer[..size];
            let returned_ordinal = u64_at(b, 0) & 0x0000_ffff_ffff_ffff;
            let returned_sequence = u16::from_le_bytes([b[28], b[29]]) as u64;
            detail["returned_ordinal"] = json!(returned_ordinal);
            detail["returned_sequence"] = json!(returned_sequence);
            detail["returned_in_use"] = json!(u16::from_le_bytes([b[34], b[35]]) & 1 != 0);
            detail["returned_base_frn"] = json!(u64_at(b, 44).to_string());
            detail["volume_serial"] = json!(fast.serial);
            match replay_fix::resident(b, id, is_dir) {
                Ok((links, proof)) => {
                    detail["proof"] = proof;
                    detail["is_dir"] = json!(is_dir);
                    detail["names"] = json!(links
                        .into_iter()
                        .map(|n| json!({"parent_frn":n.parent.to_string(),"name_utf16":n.name}))
                        .collect::<Vec<_>>());
                }
                Err(error) => detail["metadata_error"] = json!(error.to_string()),
            }
            Some((returned_ordinal, returned_sequence))
        }
    };
    event(events, &detail).map(|()| returned_identity)
}
fn forensic_legacy(_volume: &OwnedHandle, _frn: u64, _events: &mut impl Write) -> Result<()> {
    Err(invalid("legacy raw-record capture is disabled in the product kernel").into())
}

fn recover(
    volume: &OwnedHandle,
    root: &str,
    root_frn: u64,
    nodes: &mut Vec<Node>,
    names: &mut Vec<u16>,
    events: &mut impl Write,
    mut fast: Option<&mut FastState>,
) -> Result<HashMap<u64, Vec<u16>>> {
    let mut dirs = directories(nodes)?;
    let mut attempted = HashSet::new();
    let mut pending: Vec<_> = nodes
        .iter()
        .map(|n| n.parent)
        .filter(|parent| *parent != root_frn && !dirs.contains_key(parent))
        .collect();
    while let Some(parent) = pending.pop() {
        crate::cancel::check()?;
        if parent == root_frn || dirs.contains_key(&parent) || !attempted.insert(parent) {
            continue;
        }
        let mut parent_information = None;
        let result = (|| -> Result<(Vec<u8>, [u32; 13])> {
            let (handle, information) = open_id(volume, parent)?;
            parent_information = Some(information);
            if fast
                .as_ref()
                .is_some_and(|state| information[7] != state.serial)
            {
                return Err(invalid("missing parent handle volume mismatch").into());
            }
            if information[0] & 0x10 == 0 {
                return Err(invalid("missing parent FRN is not a directory").into());
            }
            let mut raw = vec![0; 65536];
            let size = ioctl(&handle, 0x0009_00eb, &[2, 0, 2, 0], &mut raw)
                .map_err(|e| format!("FSCTL_READ_FILE_USN_DATA: {e}"))?;
            raw.truncate(size);
            Ok((raw, information))
        })();
        match result {
            Ok((raw, information)) => {
                // Persist even malformed/unsupported returned records before parsing.
                let pool_before = names.len();
                let parsed = parse_record(&raw, names).and_then(|node| {
                    if node.frn != parent || !node.is_dir {
                        Err(invalid("parent USN identity/type mismatch").into())
                    } else {
                        Ok(node)
                    }
                });
                let parse_error = parsed.as_ref().err().map(|error| error.to_string());
                if fast.is_some() {
                    let mut value = json!({"kind":"parent-name","requested_frn":parent.to_string(),"information":information,"accepted":parsed.is_ok(),"parse_error":parse_error});
                    if let Ok(node) = &parsed {
                        let start = node.name_start as usize;
                        value["parent_frn"] = json!(node.parent.to_string());
                        value["name_utf16"] = json!(&names[start..start + node.name_len as usize]);
                    }
                    event(events, &value)?;
                } else {
                    event(
                        events,
                        &json!({"kind":"parent-usn","requested_frn":parent.to_string(),"raw_bytes":raw,"information":information,"accepted":parsed.is_ok(),"parse_error":parse_error}),
                    )?;
                }
                match parsed {
                    Ok(node) => {
                        pending.push(node.parent);
                        dirs.insert(parent, nodes.len());
                        nodes.push(node);
                    }
                    Err(_) => {
                        names.truncate(pool_before);
                        forensic(volume, parent, events, fast.as_deref_mut(), true)?;
                    }
                }
            }
            Err(error) => {
                event(
                    events,
                    &json!({"kind":"parent-usn","requested_frn":parent.to_string(),"error":error.to_string()}),
                )?;
                let mut resident_recovered = false;
                if let Some(state) = fast.as_deref_mut() {
                    // A valid GET_RECORD resident FILE_NAME set is enough to
                    // restore this missing directory when the USN-data query
                    // rejects its output buffer (the D: $Deleted case).  It
                    // uses the same strict parser as the normal fast branch;
                    // no raw record is persisted and the parent is queued for
                    // the same closure walk as a USN-recovered directory.
                    let information = parent_information.or_else(|| {
                        open_id(volume, parent)
                            .ok()
                            .map(|(_, information)| information)
                    });
                    if let Some(information) = information {
                        if information[7] == state.serial
                            && information[0] & 0x10 != 0
                            && info_frn(&information) == parent
                        {
                            if let Ok(size) = state.read(volume, parent) {
                                let raw = &state.buffer[..size];
                                if let Ok((links, proof)) = replay_fix::resident(raw, parent, true)
                                {
                                    if links.len() == 1 {
                                        let link = &links[0];
                                        if !link.name.is_empty() {
                                            let start = u32::try_from(names.len())?;
                                            let length = u32::try_from(link.name.len())?;
                                            names.extend_from_slice(&link.name);
                                            let node = Node {
                                                frn: parent,
                                                parent: link.parent,
                                                name_start: start,
                                                name_len: length,
                                                is_dir: true,
                                            };
                                            event(
                                                events,
                                                &json!({"kind":"parent-name","requested_frn":parent.to_string(),"information":information,"accepted":true,"parse_error":Value::Null,"parent_frn":link.parent.to_string(),"name_utf16":link.name,"proof":proof,"source":"exact-base-resident-FILE_NAME"}),
                                            )?;
                                            pending.push(node.parent);
                                            dirs.insert(parent, nodes.len());
                                            nodes.push(node);
                                            resident_recovered = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if !resident_recovered {
                    forensic(volume, parent, events, fast.as_deref_mut(), true)?;
                }
            }
        }
    }
    let root_units: Vec<_> = root.encode_utf16().collect();
    let mut overrides = HashMap::new();
    for index in 0..nodes.len() {
        if resolve(
            index,
            nodes,
            names,
            &dirs,
            root_frn,
            &root_units,
            &overrides,
        )
        .is_ok()
        {
            continue;
        }
        let node = &nodes[index];
        let result = (|| -> Result<(Vec<u16>, [u32; 13])> {
            let (handle, information) = open_id(volume, node.frn)?;
            if fast
                .as_ref()
                .is_some_and(|state| information[7] != state.serial)
            {
                return Err(invalid("recovery handle volume mismatch").into());
            }
            if (information[0] & 0x10 != 0) != node.is_dir {
                return Err(invalid("final-path node type changed").into());
            }
            Ok((final_path(&handle, root)?, information))
        })();
        match result {
            Ok((path, information)) => {
                event(
                    events,
                    &json!({"kind":"final-path","frn":node.frn.to_string(),"path_utf16":path,"information":information}),
                )?;
                overrides.insert(node.frn, path);
            }
            Err(error) => {
                let start = node.name_start as usize;
                event(
                    events,
                    &json!({"kind":"final-path","frn":node.frn.to_string(),"parent_frn":node.parent.to_string(),"name_utf16":&names[start..start+node.name_len as usize],"is_dir":node.is_dir,"error":error.to_string()}),
                )?;
                forensic(volume, node.frn, events, fast.as_deref_mut(), node.is_dir)?;
            }
        }
    }
    Ok(overrides)
}
fn replay_recovery(
    path: &Path,
    nodes: &mut Vec<Node>,
    names: &mut Vec<u16>,
    root_frn: u64,
    excluded: &HashSet<u64>,
    serial: Option<u32>,
    file_search_scope: bool,
) -> Result<(HashMap<u64, Vec<u16>>, Vec<Value>)> {
    let mut overrides = HashMap::new();
    let mut raw_parents = Vec::new();
    let mut typed_directory_repairs = Vec::new();
    for (index, line) in BufReader::new(fs::File::open(path)?).lines().enumerate() {
        let value: Value = serde_json::from_str(&line?)?;
        if value.get("error").is_some() {
            continue;
        }
        match value["kind"].as_str() {
            Some("parent-usn") => {
                if value["accepted"] != true {
                    continue;
                }
                let node = parse_record(&bytes(&value["raw_bytes"])?, names)?;
                if node.frn != frn(&value["requested_frn"])? || !node.is_dir {
                    return Err(invalid("replay parent identity mismatch").into());
                }
                nodes.push(node);
            }
            Some("system-reserved-file-out-of-scope") => {
                let id = frn(&value["frn"])?;
                let flags = value["returned_flags"]
                    .as_u64()
                    .ok_or_else(|| invalid("out-of-scope record flags missing"))?;
                if !file_search_scope
                    || value["file_search_scope"] != FILE_SEARCH_SCOPE
                    || value["volume_serial"].as_u64() != serial.map(u64::from)
                    || !reserved_metadata_file(id, flags & 2 != 0)
                    || flags & 1 == 0
                    || id >> 48 == 0
                    || value["ordinal"] != (id & 0x0000_ffff_ffff_ffff)
                    || value["sequence"] != (id >> 48)
                    || value["base_frn"] != "0"
                {
                    return Err(invalid("out-of-scope system metadata identity mismatch").into());
                }
            }
            Some("system-root-identity") => {
                if frn(&value["frn"])? != root_frn
                    || value["volume_serial"].as_u64() != serial.map(u64::from)
                    || value["in_use"] != true
                    || value["is_dir"] != true
                    || value["base_frn"] != "0"
                    || value["ordinal"] != (root_frn & 0x0000_ffff_ffff_ffff)
                    || value["sequence"] != (root_frn >> 48)
                {
                    return Err(invalid("replay system root identity mismatch").into());
                }
            }
            Some("system-unnamed-slot") => {}
            Some("system-name-node") => {
                let id = frn(&value["frn"])?;
                let serial =
                    serial.ok_or_else(|| invalid("system node requires fast capture serial"))?;
                let proof = &value["proof"];
                let is_dir = value["is_dir"]
                    .as_bool()
                    .ok_or_else(|| invalid("system node type missing"))?;
                if value["volume_serial"] != serial
                    || id & 0x0000_ffff_ffff_ffff >= 16
                    || proof["record_ordinal"] != (id & 0x0000_ffff_ffff_ffff)
                    || proof["record_sequence"] != (id >> 48)
                    || proof["in_use"] != true
                    || proof["base_record"] != 0
                    || proof["is_dir"] != is_dir
                {
                    return Err(invalid("system node exact metadata proof mismatch").into());
                }
                if nodes.iter().any(|node| node.frn == id) {
                    return Err(invalid("duplicate replay system node").into());
                }
                let text = units(&value["name_utf16"])?;
                if text.is_empty() || text.iter().any(|c| matches!(*c, 0 | 47 | 92)) {
                    return Err(invalid("system replay basename invalid").into());
                }
                let start = u32::try_from(names.len())?;
                let length = u32::try_from(text.len())?;
                names.extend_from_slice(&text);
                nodes.push(Node {
                    frn: id,
                    parent: frn(&value["parent_frn"])?,
                    name_start: start,
                    name_len: length,
                    is_dir,
                });
            }
            Some("parent-name") => {
                if value["accepted"] != true {
                    continue;
                }
                let id = frn(&value["requested_frn"])?;
                let serial =
                    serial.ok_or_else(|| invalid("typed parent requires fast manifest serial"))?;
                let information = value["information"]
                    .as_array()
                    .filter(|a| a.len() == 13)
                    .ok_or_else(|| invalid("typed parent information missing"))?;
                let actual = (information[11]
                    .as_u64()
                    .ok_or_else(|| invalid("invalid parent FRN high"))?
                    << 32)
                    | information[12]
                        .as_u64()
                        .ok_or_else(|| invalid("invalid parent FRN low"))?;
                if actual != id
                    || information[7] != serial
                    || !information[0].as_u64().is_some_and(|x| x & 16 != 0)
                {
                    return Err(invalid("typed parent identity/serial/type mismatch").into());
                }
                let text = units(&value["name_utf16"])?;
                if text.is_empty() || text.iter().any(|c| matches!(*c, 0 | 47 | 92)) {
                    return Err(invalid("invalid typed parent basename").into());
                }
                let start = u32::try_from(names.len())?;
                let length = u32::try_from(text.len())?;
                names.extend_from_slice(&text);
                nodes.push(Node {
                    frn: id,
                    parent: frn(&value["parent_frn"])?,
                    name_start: start,
                    name_len: length,
                    is_dir: true,
                });
            }
            Some("ntfs-name-metadata") => {
                if value["is_dir"] != true || !value["names"].is_array() {
                    continue;
                }
                let id = frn(&value["requested_frn"])?;
                if nodes.iter().any(|n| n.frn == id) {
                    continue;
                }
                let (mut links, _) = replay_fix::metadata_names(
                    &value,
                    id,
                    true,
                    serial.ok_or_else(|| invalid("typed directory requires serial"))?,
                )?;
                let link = links.remove(0);
                let start = u32::try_from(names.len())?;
                let length = u32::try_from(link.name.len())?;
                names.extend_from_slice(&link.name);
                nodes.push(Node {
                    frn: id,
                    parent: link.parent,
                    name_start: start,
                    name_len: length,
                    is_dir: true,
                });
                typed_directory_repairs.push(value.clone());
            }
            Some("final-path") => {
                if let Some(serial) = serial {
                    let id = frn(&value["frn"])?;
                    let information = value["information"]
                        .as_array()
                        .filter(|a| a.len() == 13)
                        .ok_or_else(|| invalid("typed final-path information missing"))?;
                    if information
                        .iter()
                        .any(|v| v.as_u64().is_none_or(|n| n > u32::MAX as u64))
                    {
                        return Err(invalid("typed final-path information invalid").into());
                    }
                    let actual = (information[11].as_u64().unwrap() << 32)
                        | information[12].as_u64().unwrap();
                    if actual != id || information[7] != serial {
                        return Err(invalid("typed final-path identity/serial mismatch").into());
                    }
                    if let Some(node) = nodes.iter().find(|node| node.frn == id) {
                        if (information[0].as_u64().unwrap() & 16 != 0) != node.is_dir {
                            return Err(invalid("typed final-path type mismatch").into());
                        }
                    }
                }
                overrides.insert(frn(&value["frn"])?, units(&value["path_utf16"])?);
            }
            Some("ntfs-raw") => {
                if value["raw_bytes"].is_array() {
                    raw_parents.push((
                        frn(&value["requested_frn"])?,
                        bytes(&value["raw_bytes"])?,
                        index + 1,
                    ));
                }
            }
            _ => return Err(invalid("unknown recovery capture event").into()),
        }
    }
    // A recovered full path also needs its real directory entries in the output.
    // Excluded/unlinked files must not make their internal parent directory required.
    let mut known: HashSet<_> = nodes
        .iter()
        .filter(|node| node.is_dir)
        .map(|node| node.frn)
        .collect();
    known.insert(root_frn);
    let mut required: HashSet<_> = nodes
        .iter()
        .filter(|node| !excluded.contains(&node.frn))
        .map(|node| node.parent)
        .filter(|parent| !known.contains(parent))
        .collect();
    let mut directory_repairs = typed_directory_repairs;
    loop {
        let mut progress = false;
        for (id, raw, line) in &raw_parents {
            if !required.contains(id) || known.contains(id) {
                continue;
            }
            let (name, mut proof) = replay_fix::directory(raw, *id)?;
            let name_start = u32::try_from(names.len())?;
            let name_len = u32::try_from(name.name.len())?;
            names.extend_from_slice(&name.name);
            nodes.push(Node {
                frn: *id,
                parent: name.parent,
                name_start,
                name_len,
                is_dir: true,
            });
            known.insert(*id);
            required.remove(id);
            if !known.contains(&name.parent) {
                required.insert(name.parent);
            }
            proof["frn"] = json!(id.to_string());
            proof["parent_frn"] = json!(name.parent.to_string());
            proof["name_utf16"] = json!(name.name);
            proof["source_repairs_line"] = json!(line);
            directory_repairs.push(proof);
            progress = true;
        }
        if !progress {
            break;
        }
    }
    if !required.is_empty() {
        let mut ids: Vec<_> = required.into_iter().map(|id| id.to_string()).collect();
        ids.sort();
        return Err(format!(
            "required directory entries lack exact captured metadata: {}",
            ids.join(",")
        )
        .into());
    }
    Ok((overrides, directory_repairs))
}

// A file result frame is FRN(u64), status(u8), path count(u32), then UTF16 paths.
// 0 = complete native hardlink names; 1 = verified same-FRN single link;
// 2 = unknown/failed (known paths may be present); 3 = legacy unqueried file.
// Status 3 remains partial on replay even if parent resolution later improves.
// 4 = verified zero-link $Deleted internal object; 5 = proven disappeared FRN.
// 6 = exact-base resident FILE_NAME set, no ATTRIBUTE_LIST (fast mode only).
// 7 = explicitly out-of-scope reserved metadata file, not a successful name query.
fn write_links(output: &mut impl Write, id: u64, status: u8, paths: &[Vec<u16>]) -> Result<()> {
    output.write_all(&id.to_le_bytes())?;
    output.write_all(&[status])?;
    output.write_all(&u32::try_from(paths.len())?.to_le_bytes())?;
    for path in paths {
        write_units_fast(output, path)?;
    }
    Ok(())
}
fn read_links(input: &mut impl Read, id: u64) -> Result<(u8, Vec<Vec<u16>>)> {
    let mut header = [0; 13];
    input.read_exact(&mut header)?;
    if u64_at(&header, 0) != id || header[8] > 7 {
        return Err(invalid("hardlink capture FRN/status mismatch").into());
    }
    let count = u32_at(&header, 9) as usize;
    if count > 65536 {
        return Err(invalid("hardlink capture path count too large").into());
    }
    let mut paths = Vec::with_capacity(count);
    for _ in 0..count {
        paths.push(read_units(input)?);
    }
    Ok((header[8], paths))
}
fn live_links(
    volume: &OwnedHandle,
    node: &Node,
    path: Option<&[u16]>,
    root: &str,
    details: &mut impl Write,
    mut parent_path: impl FnMut(u64) -> Result<(Vec<u16>, Value)>,
    mut fast: Option<&mut FastState>,
) -> Result<(u8, Vec<Vec<u16>>, Option<Value>)> {
    let mut path_open_error = None;
    let mut current_path_error = None;
    let opened = match path.map(|path| open_path(path, node.frn)) {
        Some(Ok((handle, information))) => Ok((handle, information, path.map(<[u16]>::to_vec))),
        attempted => {
            path_open_error =
                attempted.and_then(|value| value.err().map(|error| error.to_string()));
            (|| -> Result<_> {
                let (handle, information) = open_id(volume, node.frn)?;
                if information[0] & 0x10 != 0 {
                    return Err(invalid("file became a directory").into());
                }
                let current = match final_path(&handle, root) {
                    Ok(path) => Some(path),
                    Err(error) => {
                        current_path_error = Some(error.to_string());
                        None
                    }
                };
                Ok((handle, information, current))
            })()
        }
    };
    let (handle, information, current) = match opened {
        Ok(value) => value,
        Err(error) => {
            event(
                details,
                &json!({"kind":"file-source","frn":node.frn.to_string(),"path_utf16":path,"path_open_error":path_open_error,"error":error.to_string()}),
            )?;
            let returned_identity =
                forensic(volume, node.frn, details, fast.as_deref_mut(), false)?;
            let disappearance = returned_identity
                .filter(|&(ordinal, sequence)| {
                    let requested_ordinal = node.frn & 0x0000_ffff_ffff_ffff;
                    let requested_sequence = node.frn >> 48;
                    error.to_string().contains("OpenFileById")
                        && (ordinal < requested_ordinal
                            || (ordinal == requested_ordinal && sequence != requested_sequence))
                })
                .map(|(returned_ordinal, returned_sequence)| {
                    json!({
                        "volume": root,
                        "frn": node.frn.to_string(),
                        "reason": "original-file-reference-disappeared",
                        "original_error": error.to_string(),
                        "requested_ordinal": node.frn & 0x0000_ffff_ffff_ffff,
                        "requested_sequence": node.frn >> 48,
                        "returned_ordinal": returned_ordinal,
                        "returned_sequence": returned_sequence,
                        "proof": {
                            "phase": "live-collection",
                            "source": "strict-GET_RECORD-identity",
                            "same_ordinal_checked": true,
                            "ordinal_changed": returned_ordinal
                                != (node.frn & 0x0000_ffff_ffff_ffff),
                            "sequence_changed": returned_sequence != node.frn >> 48,
                            "raw_record_bytes_saved": false
                        }
                    })
                });
            return Ok((
                if disappearance.is_some() { 5 } else { 2 },
                if disappearance.is_some() {
                    Vec::new()
                } else {
                    path.map(<[u16]>::to_vec).into_iter().collect()
                },
                disappearance,
            ));
        }
    };
    if fast
        .as_ref()
        .is_some_and(|state| information[7] != state.serial)
    {
        return Err(invalid("native exception handle volume serial mismatch").into());
    }
    if information[10] == 0 {
        let prefix: Vec<_> = format!(r"{root}\$Extend\$Deleted\")
            .encode_utf16()
            .collect();
        if current
            .as_ref()
            .is_some_and(|path| path.starts_with(&prefix) && path.len() > prefix.len())
        {
            event(
                details,
                &json!({"kind":"excluded-unlinked-internal-object","frn":node.frn.to_string(),"information":information,"current_path_utf16":current}),
            )?;
            return Ok((4, Vec::new(), None));
        }
    }
    if information[10] == 1 && current.is_some() {
        if path.is_none() || path_open_error.is_some() {
            event(
                details,
                &json!({"kind":"file-source","frn":node.frn.to_string(),"path_utf16":path,"source":"verified-id-current-path","path_open_error":path_open_error,"information":information,"current_path_utf16":current}),
            )?;
        }
        return Ok((1, vec![current.unwrap()], None));
    }
    let mut source = json!({"kind":"native-hardlinks","frn":node.frn.to_string(),"path_utf16":path,"source":if path.is_some()&&path_open_error.is_none() {"verified-no-follow-path"} else {"verified-id"},"path_open_error":path_open_error,"information":information,"current_path_utf16":current,"current_path_error":current_path_error});
    let query_started = Instant::now();
    let native_result = native_links(&handle, information[10]);
    if let Some(state) = fast.as_deref_mut() {
        state.native_link_calls += 1;
        state.native_link_ms += query_started.elapsed().as_secs_f64() * 1000.0;
    }
    match native_result {
        Ok((raw, links)) => {
            source["raw_bytes"] = json!(raw);
            source["ntstatus"] = json!(0);
            let mut paths = Vec::new();
            let mut parents = Vec::new();
            let mut failure = None;
            for link in links {
                match parent_path(link.parent) {
                    Ok((mut parent, detail)) => {
                        parents.push(detail);
                        parent.push(b'\\' as u16);
                        parent.extend_from_slice(&link.name);
                        if !paths.contains(&parent) {
                            paths.push(parent);
                        }
                    }
                    Err(error) => {
                        failure = Some(format!("hardlink parent {}: {error}", link.parent));
                        break;
                    }
                }
            }
            source["parents"] = json!(parents);
            if paths.len() != information[10] as usize && failure.is_none() {
                failure = Some("hardlink paths were not distinct".to_owned());
            }
            if let Some(error) = failure {
                source["error"] = json!(error);
                source["partial_paths_utf16"] = json!(paths);
                event(details, &source)?;
                forensic(volume, node.frn, details, fast.as_deref_mut(), false)?;
                if let Some(current) = current {
                    if !paths.contains(&current) {
                        paths.push(current);
                    }
                }
                Ok((2, paths, None))
            } else {
                event(details, &source)?;
                Ok((0, paths, None))
            }
        }
        Err(error) => {
            source["error"] = json!(error.to_string());
            if let Some(error) = error.downcast_ref::<NativeLinkFailure>() {
                source["raw_bytes"] = json!(error.raw);
                source["ntstatus"] = json!(error.ntstatus);
            }
            event(details, &source)?;
            forensic(volume, node.frn, details, fast.as_deref_mut(), false)?;
            Ok((2, current.into_iter().collect(), None))
        }
    }
}
struct Counts {
    entries: u64,
    files: u64,
    folders: u64,
    unresolved: u64,
    cycles: u64,
    link_failures: u64,
    original_unresolved: u64,
    original_cycles: u64,
    repaired_files: u64,
    excluded_unlinked: u64,
    excluded_disappeared: u64,
    raw_recovered: u64,
}
impl Counts {
    fn new() -> Self {
        Self {
            entries: 0,
            files: 0,
            folders: 0,
            unresolved: 0,
            cycles: 0,
            link_failures: 0,
            original_unresolved: 0,
            original_cycles: 0,
            repaired_files: 0,
            excluded_unlinked: 0,
            excluded_disappeared: 0,
            raw_recovered: 0,
        }
    }
    fn update(&self, report: &mut Value) {
        report["entries"] = json!(self.entries);
        report["files"] = json!(self.files);
        report["folders"] = json!(self.folders);
        report["unresolved_paths"] = json!(self.unresolved);
        report["cycles"] = json!(self.cycles);
        report["hardlink_failures"] = json!(self.link_failures);
        report["original_unresolved_paths"] = json!(self.original_unresolved);
        report["original_cycles"] = json!(self.original_cycles);
        report["files_repaired_after_parent_chain_failure"] = json!(self.repaired_files);
        report["final_unresolved_paths"] = json!(self.unresolved + self.cycles);
        report["excluded_unlinked_internal_objects"] = json!(self.excluded_unlinked);
        report["excluded_disappeared_during_capture"] = json!(self.excluded_disappeared);
        report["locked_files_recovered_from_raw_metadata"] = json!(self.raw_recovered);
    }
}
fn original_failures(
    nodes: &[Node],
    _names: &[u16],
    root_frn: u64,
    _root: &str,
) -> Result<(u64, u64)> {
    let dirs = directories(nodes)?;
    // This pass only reports the original parent-chain failure counts.  It
    // does not need to materialize any path text, so follow directory FRNs
    // with generation marks instead of allocating a `Vec`/scanning it for
    // every record as `resolve` does for actual path emission.
    let mut marks = vec![0u32; nodes.len()];
    let mut generation = 0u32;
    let mut unresolved = 0;
    let mut cycles = 0;
    for node in nodes {
        if node.frn == root_frn {
            continue;
        }
        generation = generation.wrapping_add(1).max(1);
        let mut parent = node.parent;
        loop {
            if parent == root_frn {
                break;
            }
            let Some(&directory) = dirs.get(&parent) else {
                unresolved += 1;
                break;
            };
            if marks[directory] == generation {
                unresolved += 1;
                cycles += 1;
                break;
            }
            marks[directory] = generation;
            parent = nodes[directory].parent;
        }
    }
    Ok((unresolved, cycles))
}

fn cached_resolve(
    index: usize,
    nodes: &[Node],
    names: &[u16],
    dirs: &HashMap<u64, usize>,
    directory_paths: &HashMap<u64, Vec<u16>>,
    root_frn: u64,
    root: &[u16],
    overrides: &HashMap<u64, Vec<u16>>,
) -> std::result::Result<Vec<u16>, Failure> {
    let node = &nodes[index];
    if node.frn == root_frn {
        return Ok(root.to_vec());
    }
    if let Some(path) = overrides.get(&node.frn) {
        return Ok(path.clone());
    }
    if node.is_dir {
        return directory_paths
            .get(&node.frn)
            .cloned()
            .ok_or(Failure {
                reason: "missing-parent",
                parent: node.parent,
            })
            .or_else(|_| resolve(index, nodes, names, dirs, root_frn, root, overrides));
    }
    let parent = if node.parent == root_frn {
        root.to_vec()
    } else if let Some(path) = directory_paths.get(&node.parent) {
        path.clone()
    } else {
        return resolve(index, nodes, names, dirs, root_frn, root, overrides);
    };
    let start = node.name_start as usize;
    let mut path = parent;
    path.push(b'\\' as u16);
    path.extend_from_slice(&names[start..start + node.name_len as usize]);
    Ok(path)
}

fn cached_parent_path(
    parent: u64,
    nodes: &[Node],
    names: &[u16],
    dirs: &HashMap<u64, usize>,
    directory_paths: &HashMap<u64, Vec<u16>>,
    root_frn: u64,
    root: &[u16],
    overrides: &HashMap<u64, Vec<u16>>,
) -> Result<Vec<u16>> {
    if parent == root_frn {
        return Ok(root.to_vec());
    }
    if let Some(path) = directory_paths.get(&parent) {
        return Ok(path.clone());
    }
    let index = dirs
        .get(&parent)
        .ok_or_else(|| invalid("hardlink parent is absent from captured directory closure"))?;
    cached_resolve(
        *index,
        nodes,
        names,
        dirs,
        directory_paths,
        root_frn,
        root,
        overrides,
    )
    .map_err(|failure| invalid(failure.reason).into())
}
#[allow(clippy::too_many_arguments)]
fn write_volume(
    snapshot: &mut impl Write,
    identities: &mut Option<IdentityWriter>,
    root: &str,
    root_frn: u64,
    nodes: &[Node],
    names: &[u16],
    overrides: &HashMap<u64, Vec<u16>>,
    capture: &Path,
    live: Option<&OwnedHandle>,
    original: (u64, u64),
    replay_repairs: Option<&HashMap<u64, replay_fix::Repair>>,
    counts: &mut Counts,
    report: &mut Value,
    mut fast: Option<&mut FastState>,
) -> Result<Value> {
    let started = Instant::now();
    counts.original_unresolved += original.0;
    counts.original_cycles += original.1;
    let dirs = directories(nodes)?;
    // Resolve each captured directory once.  File paths and every additional
    // hardlink name can then reuse its parent path instead of walking the
    // same parent chain for every record.  The cache lives only for this
    // projection call and is dropped before the low-resident search process
    // starts.
    let mut directory_paths = HashMap::with_capacity(dirs.len());
    let root_units: Vec<_> = root.encode_utf16().collect();
    for (&frn, &index) in &dirs {
        if let Ok(path) = resolve(index, nodes, names, &dirs, root_frn, &root_units, overrides) {
            directory_paths.insert(frn, path);
        }
    }
    let deleted_prefix: Vec<_> = format!(r"{root}\$Extend\$Deleted\")
        .encode_utf16()
        .collect();
    let base = capture.join(root.trim_end_matches(':'));
    let mut link_output = if live.is_some() {
        Some(BufWriter::with_capacity(
            1024 * 1024,
            fs::File::create(base.with_extension("links"))?,
        ))
    } else {
        None
    };
    let mut link_input = if live.is_none() {
        Some(BufReader::with_capacity(
            1024 * 1024,
            fs::File::open(base.with_extension("links"))?,
        ))
    } else {
        None
    };
    let mut errors = if live.is_some() {
        Some(BufWriter::with_capacity(
            1024 * 1024,
            fs::File::create(base.with_extension("details.jsonl"))?,
        ))
    } else {
        None
    };
    let volume_byte = root
        .as_bytes()
        .first()
        .copied()
        .ok_or_else(|| invalid("missing volume letter"))?;
    emit_path(
        snapshot,
        identities,
        volume_byte,
        root_frn,
        true,
        &root_units,
    )?;
    counts.entries += 1;
    counts.folders += 1;
    let mut unresolved = 0;
    let mut cycles = 0;
    let mut failures = 0;
    let mut single = 0;
    let mut multi = 0;
    let mut extra = 0;
    let mut file_queries = 0;
    let mut samples = Vec::new();
    let mut snapshot_count = 1;
    let mut hardlink_ms = 0.0;
    let mut repaired_files = 0;
    let mut excluded_unlinked = 0;
    let mut excluded_disappeared = 0;
    let mut raw_recovered = 0;
    let mut typed_recovered = 0u64;
    let allow_status6 = fast.is_some() || report["source_capture_mode"] == "collect-mft-fast";
    let file_search_scope = report["file_search_scope"] == FILE_SEARCH_SCOPE;
    if let Some(state) = fast.as_deref_mut() {
        let workers = state.workers;
        state.configure_workers(root, root_frn, workers)?;
    }
    let mut excluded_reserved_files = 0u64;
    let mut excluded_reserved_saved_paths = 0u64;
    for (index, node) in nodes.iter().enumerate() {
        if index & 0x3fff == 0 {
            crate::cancel::check()?;
        }
        if index % FAST_READ_BATCH == 0 {
            if let (Some(_volume), Some(state)) = (live, fast.as_deref_mut()) {
                state.prepare_batch(
                    &nodes[index..(index + FAST_READ_BATCH).min(nodes.len())],
                    file_search_scope,
                )?;
            }
        }
        if node.frn == root_frn {
            continue;
        }
        if file_search_scope && reserved_metadata_file(node.frn, node.is_dir) {
            excluded_reserved_files += 1;
            if live.is_some() {
                write_links(link_output.as_mut().unwrap(), node.frn, 7, &[])?;
            } else {
                let (_observed_status, paths) = read_links(link_input.as_mut().unwrap(), node.frn)?;
                excluded_reserved_saved_paths += paths.len() as u64;
            }
            continue;
        }
        let resolved = cached_resolve(
            index,
            nodes,
            names,
            &dirs,
            &directory_paths,
            root_frn,
            &root_units,
            overrides,
        );
        let original_failure = resolved.as_ref().err();
        let detail=original_failure.map(|failure| {
            let start=node.name_start as usize;
            json!({"kind":"parent-chain-failure-after-directory-recovery","reason":failure.reason,"frn":node.frn.to_string(),"parent_frn":node.parent.to_string(),"failed_parent_frn":failure.parent.to_string(),"name_utf16":&names[start..start+node.name_len as usize],"is_dir":node.is_dir})
        });
        if let (Some(output), Some(detail)) = (errors.as_mut(), detail.as_ref()) {
            event(output, detail)?;
        }
        if node.is_dir {
            match resolved {
                Ok(path) => {
                    emit_path(snapshot, identities, volume_byte, node.frn, true, &path)?;
                    counts.entries += 1;
                    counts.folders += 1;
                    snapshot_count += 1;
                }
                Err(failure) => {
                    if failure.reason == "cycle" {
                        cycles += 1;
                        counts.cycles += 1;
                    } else {
                        unresolved += 1;
                        counts.unresolved += 1;
                    }
                    if samples.len() < 10 {
                        samples.push(detail.unwrap());
                    }
                }
            }
            continue;
        }
        let (mut status, mut paths, live_repair) = if let Some(volume) = live {
            let query_started = Instant::now();
            let serial = fast.as_ref().map(|state| state.serial);
            let mut parent_path = |parent: u64| -> Result<(Vec<u16>, Value)> {
                if parent == root_frn {
                    return Ok((
                        root_units.clone(),
                        json!({"parent_frn":parent.to_string(),"source":"volume-root"}),
                    ));
                }
                if let Some(path) = directory_paths.get(&parent) {
                    return Ok((
                        path.clone(),
                        json!({"parent_frn":parent.to_string(),"source":"mft-parent-chain","path_utf16":path}),
                    ));
                }
                if let Some(&index) = dirs.get(&parent) {
                    if let Ok(path) = cached_resolve(
                        index,
                        nodes,
                        names,
                        &dirs,
                        &directory_paths,
                        root_frn,
                        &root_units,
                        overrides,
                    ) {
                        return Ok((
                            path.clone(),
                            json!({"parent_frn":parent.to_string(),"source":"mft-parent-chain","path_utf16":path}),
                        ));
                    }
                }
                let (handle, information) = open_id(volume, parent)?;
                if serial.is_some_and(|serial| information[7] != serial) {
                    return Err(invalid("parent handle volume serial mismatch").into());
                }
                if information[0] & 0x10 == 0 {
                    return Err(invalid("hardlink parent is not a directory").into());
                }
                let path = final_path(&handle, root)?;
                Ok((
                    path.clone(),
                    json!({"parent_frn":parent.to_string(),"source":"verified-parent-handle","path_utf16":path,"information":information}),
                ))
            };
            let mut fast_result = None;
            if let Some(state) = fast.as_deref_mut() {
                let prefix = &deleted_prefix;
                if resolved
                    .as_ref()
                    .is_ok_and(|path| path.starts_with(&prefix))
                {
                    state.reject("internal-deleted-link-state-candidate");
                } else {
                    let candidate = (|| -> Result<Vec<Vec<u16>>> {
                        let links = state.read_names(volume, node.frn, false)?;
                        let mut paths = Vec::new();
                        for link in links {
                            let mut path = cached_parent_path(
                                link.parent,
                                nodes,
                                names,
                                &dirs,
                                &directory_paths,
                                root_frn,
                                &root_units,
                                overrides,
                            )?;
                            path.push(b'\\' as u16);
                            path.extend_from_slice(&link.name);
                            if path.starts_with(&prefix) {
                                return Err(invalid("internal-deleted-link-state-candidate").into());
                            }
                            if !paths.contains(&path) {
                                paths.push(path);
                            }
                        }
                        if paths.is_empty() {
                            return Err(invalid("empty exact resident name set").into());
                        }
                        Ok(paths)
                    })();
                    match candidate {
                        Ok(paths) => {
                            state.fast_files += 1;
                            fast_result = Some((6, paths));
                        }
                        Err(error) => state.reject(&error.to_string()),
                    }
                }
            }
            let result = if let Some((status, paths)) = fast_result {
                (status, paths, None)
            } else {
                if let Some(state) = fast.as_deref_mut() {
                    state.native_files += 1;
                }
                let native_started = Instant::now();
                let result = live_links(
                    volume,
                    node,
                    resolved.as_deref().ok(),
                    root,
                    errors.as_mut().unwrap(),
                    &mut parent_path,
                    fast.as_deref_mut(),
                )?;
                if let Some(state) = fast.as_deref_mut() {
                    state.native_ms += native_started.elapsed().as_secs_f64() * 1000.0;
                }
                result
            };
            write_links(link_output.as_mut().unwrap(), node.frn, result.0, &result.1)?;
            hardlink_ms += query_started.elapsed().as_secs_f64() * 1000.0;
            result
        } else {
            let (status, paths) = read_links(link_input.as_mut().unwrap(), node.frn)?;
            (status, paths, None)
        };
        if let Some(evidence) = live_repair {
            report["live_identity_repairs"]
                .as_array_mut()
                .unwrap()
                .push(evidence);
        }
        if status == 7 {
            return Err(
                invalid("reserved-file exclusion frame outside explicit matching scope").into(),
            );
        }
        if status == 6 && !allow_status6 {
            return Err(invalid("fast status6 requires explicit fast capture provenance").into());
        }
        if (2..=5).contains(&status) {
            if let Some(repair) = replay_repairs.and_then(|repairs| repairs.get(&node.frn)) {
                match repair {
                    replay_fix::Repair::Unlinked(evidence) => {
                        status = 4;
                        paths.clear();
                        report["replay_repairs"]
                            .as_array_mut()
                            .unwrap()
                            .push(evidence.clone());
                    }
                    replay_fix::Repair::Disappeared(evidence) => {
                        status = 5;
                        paths.clear();
                        report["replay_repairs"]
                            .as_array_mut()
                            .unwrap()
                            .push(evidence.clone());
                    }
                    replay_fix::Repair::ResidentNames(links, evidence) => {
                        let repaired = (|| -> Result<Vec<Vec<u16>>> {
                            let mut paths = Vec::new();
                            for link in links {
                                let mut parent = cached_parent_path(
                                    link.parent,
                                    nodes,
                                    names,
                                    &dirs,
                                    &directory_paths,
                                    root_frn,
                                    &root_units,
                                    overrides,
                                )
                                .map_err(|_| invalid("raw filename parent remains unresolved"))?;
                                parent.push(b'\\' as u16);
                                parent.extend_from_slice(&link.name);
                                if !paths.contains(&parent) {
                                    paths.push(parent);
                                }
                            }
                            if paths.is_empty() {
                                return Err(invalid("raw filename repair was empty").into());
                            }
                            Ok(paths)
                        })();
                        match repaired {
                            Ok(repaired) => {
                                if evidence["reason"]
                                    == "locked-file-names-recovered-from-parsed-metadata"
                                {
                                    status = 6;
                                    typed_recovered += 1;
                                } else {
                                    status = 0;
                                    raw_recovered += 1;
                                    counts.raw_recovered += 1;
                                }
                                paths = repaired;
                                report["replay_repairs"]
                                    .as_array_mut()
                                    .unwrap()
                                    .push(evidence.clone());
                            }
                            Err(error) => report["unhandled_replay_failures"]
                                .as_array_mut()
                                .unwrap()
                                .push(
                                    json!({"frn":node.frn.to_string(),"error":error.to_string()}),
                                ),
                        }
                    }
                }
            }
        }
        if status == 4 {
            excluded_unlinked += 1;
            counts.excluded_unlinked += 1;
            continue;
        }
        if status == 5 {
            excluded_disappeared += 1;
            counts.excluded_disappeared += 1;
            continue;
        }
        let complete = (status <= 1 || status == 6) && !paths.is_empty();
        if !complete {
            failures += 1;
            counts.link_failures += 1;
        }
        if status == 1 {
            single += 1;
        }
        if status != 3 {
            file_queries += 1;
        }
        if !complete && paths.is_empty() {
            if original_failure.is_some_and(|failure| failure.reason == "cycle") {
                cycles += 1;
                counts.cycles += 1;
            } else {
                unresolved += 1;
                counts.unresolved += 1;
            }
            if samples.len() < 10 {
                samples.push(detail.unwrap_or_else(||json!({"kind":"unqueried-or-empty-names","frn":node.frn.to_string(),"saved_status":status})));
            }
        }
        if complete && original_failure.is_some() {
            repaired_files += 1;
            counts.repaired_files += 1;
            if let Some(output) = errors.as_mut() {
                event(
                    output,
                    &json!({"kind":"file-repaired-after-parent-chain-failure","frn":node.frn.to_string(),"file_result_status":status}),
                )?;
            }
        }
        let mut unique = Vec::new();
        for path in paths.drain(..) {
            if path.len() < 3 || path[..2] != root_units || path[2] != b'\\' as u16 {
                return Err(invalid("captured hardlink path outside requested volume").into());
            }
            if !unique.contains(&path) {
                unique.push(path);
            }
        }
        if unique.len() > 1 {
            multi += 1;
            extra += unique.len() - 1;
        }
        for path in unique {
            emit_path(snapshot, identities, volume_byte, node.frn, false, &path)?;
            counts.entries += 1;
            counts.files += 1;
            snapshot_count += 1;
        }
        if index % 16384 == 0 {
            counts.update(report);
            sample_memory(report)?;
        }
    }
    if let Some(input) = link_input.as_mut() {
        let mut extra = [0; 1];
        if input.read(&mut extra)? != 0 {
            return Err(invalid("trailing hardlink capture records").into());
        }
    }
    if let Some(output) = link_output.as_mut() {
        output.flush()?;
        output.get_ref().sync_all()?;
    }
    if let Some(output) = errors.as_mut() {
        output.flush()?;
        output.get_ref().sync_all()?;
    }
    snapshot.flush()?;
    counts.update(report);
    if file_search_scope {
        for (field, value) in [
            ("excluded_reserved_file_nodes", excluded_reserved_files),
            (
                "excluded_reserved_saved_path_candidates",
                excluded_reserved_saved_paths,
            ),
        ] {
            report[field] = json!(report[field].as_u64().unwrap_or(0) + value);
        }
    }
    if let Some(state) = fast.as_deref_mut() {
        state.finish_workers()?;
    }
    sample_memory(report)?;
    Ok(
        json!({"volume":root,"status":if unresolved+cycles+failures==0 {"complete"} else {"partial"},"records":nodes.len(),"entries":snapshot_count,"path_write_and_hardlinks_ms":started.elapsed().as_secs_f64()*1000.0,"hardlink_ms":hardlink_ms,"path_write_ms":started.elapsed().as_secs_f64()*1000.0-hardlink_ms,"unresolved_paths":unresolved,"final_unresolved_paths":unresolved+cycles,"original_unresolved_paths":original.0,"original_cycles":original.1,"files_repaired_after_parent_chain_failure":repaired_files,"cycles":cycles,"hardlink_failures":failures,"verified_single_link_files":single,"files_with_multiple_names":multi,"extra_hardlink_names":extra,"file_queries":file_queries,"root_frn":root_frn.to_string(),"unresolved_samples":samples,"handle_path_repairs":overrides.len(),"excluded_unlinked_internal_objects":excluded_unlinked,"excluded_disappeared_during_capture":excluded_disappeared,"locked_files_recovered_from_raw_metadata":raw_recovered,"locked_files_recovered_from_parsed_metadata":typed_recovered,"excluded_reserved_file_nodes":excluded_reserved_files,"excluded_reserved_saved_path_candidates":excluded_reserved_saved_paths}),
    )
}
const ENUM_BUFFER_SIZE: usize = 8 * 1024 * 1024;

fn enumerate(
    volume: &OwnedHandle,
    capture: &Path,
    nodes: &mut Vec<Node>,
    names: &mut Vec<u16>,
    report: &mut Value,
) -> Result<usize> {
    let mut output = BufWriter::with_capacity(ENUM_BUFFER_SIZE, fs::File::create(capture)?);
    let mut data = EnumData {
        cursor: 0,
        low_usn: 0,
        high_usn: i64::MAX,
    };
    let mut buffer = vec![0; ENUM_BUFFER_SIZE];
    let mut frames = 0usize;
    let mut parse_error = None;
    loop {
        let input = unsafe {
            std::slice::from_raw_parts(
                (&data as *const EnumData).cast::<u8>(),
                std::mem::size_of::<EnumData>(),
            )
        };
        let size = match ioctl(volume, 0x0009_00b3, input, &mut buffer) {
            Ok(size) => size,
            Err(error) if error.raw_os_error() == Some(38) => break,
            Err(error) => return Err(format!("FSCTL_ENUM_USN_DATA: {error}").into()),
        };
        // Persist the raw result before any interpretation that could fail.
        write_frame(&mut output, &buffer[..size])?;
        frames += 1;
        if size < 8 || u64_at(&buffer, 0) <= data.cursor {
            return Err(invalid("invalid/nonadvancing ENUM cursor").into());
        }
        if let Err(error) = parse_block(&buffer[..size], nodes, names) {
            if parse_error.is_none() {
                parse_error = Some(error.to_string());
            }
        }
        data.cursor = u64_at(&buffer, 0);
        sample_memory(report)?;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    if let Some(error) = parse_error {
        return Err(format!("raw ENUM completed but interpretation failed: {error}").into());
    }
    if nodes.is_empty() {
        return Err(invalid("MFT enumeration returned no records").into());
    }
    Ok(frames)
}
fn load_enum(path: &Path, nodes: &mut Vec<Node>, names: &mut Vec<u16>) -> Result<()> {
    let mut input = BufReader::with_capacity(1024 * 1024, fs::File::open(path)?);
    let mut previous = 0;
    while let Some(block) = read_frame(&mut input, ENUM_BUFFER_SIZE)? {
        if block.len() < 8 || u64_at(&block, 0) <= previous {
            return Err(invalid("invalid replay ENUM cursor").into());
        }
        previous = u64_at(&block, 0);
        parse_block(&block, nodes, names)?;
    }
    if nodes.is_empty() {
        return Err(invalid("empty ENUM capture").into());
    }
    Ok(())
}
fn begin_snapshot(path: &Path) -> Result<BufWriter<fs::File>> {
    let mut output = BufWriter::with_capacity(1024 * 1024, fs::File::create(path)?);
    output.write_all(SNAP_MAGIC)?;
    output.write_all(&0u64.to_le_bytes())?;
    Ok(output)
}
fn finish_snapshot(mut output: BufWriter<fs::File>, count: u64) -> Result<()> {
    output.flush()?;
    let mut file = output.into_inner()?;
    file.seek(SeekFrom::Start(8))?;
    file.write_all(&count.to_le_bytes())?;
    file.sync_all()?;
    Ok(())
}
fn check_counts(counts: &Counts) -> Result<()> {
    if counts.unresolved + counts.cycles + counts.link_failures != 0 {
        return Err(format!(
            "partial snapshot: {} unresolved paths, {} cycles, {} unknown hardlink sets",
            counts.unresolved, counts.cycles, counts.link_failures
        )
        .into());
    }
    Ok(())
}
fn normalize(root: &OsString) -> Result<String> {
    let root = root
        .to_str()
        .ok_or_else(|| invalid("expected ASCII drive letter"))?
        .trim_end_matches(['\\', '/'])
        .to_ascii_uppercase();
    if root.len() != 2 || !root.as_bytes()[0].is_ascii_alphabetic() || root.as_bytes()[1] != b':' {
        return Err(format!("invalid volume: {root}").into());
    }
    Ok(root)
}
fn collect(
    snapshot: &Path,
    roots: &[OsString],
    report: &mut Value,
    fast_mode: bool,
    identities_raw: Option<&Path>,
    workers: usize,
) -> Result<()> {
    let mut volumes = Vec::new();
    for root in roots {
        let root = normalize(root)?;
        if volumes.iter().any(|(name, _, _)| name == &root) {
            return Err(invalid("duplicate volume").into());
        }
        let volume = open(&format!(r"\\.\{root}"), 0x8000_0000, 0)?;
        let root_handle = open(&format!(r"{root}\"), 0, 0x0200_0000)?;
        let root_frn = info_frn(&info(&root_handle)?);
        volumes.push((root, volume, root_frn));
    }
    let mut capture_name = snapshot.as_os_str().to_os_string();
    capture_name.push(".capture");
    let capture = PathBuf::from(capture_name);
    fs::create_dir(&capture)?;
    report["capture_directory"] = json!(capture.to_string_lossy());
    let mut manifest = json!({"format":"KFMFTC01","volumes":[],"complete":false});
    if fast_mode {
        manifest["file_search_scope"] = json!(FILE_SEARCH_SCOPE);
        report["file_search_scope"] = json!(FILE_SEARCH_SCOPE);
        report["path_semantics"] = json!("all named ordinary file/directory paths and hardlinks; only file objects at NTFS reserved ordinals 0..15 excluded; all directories retained; verified zero-link internal objects and proven disappeared FRNs separately excluded");
        manifest["mode"] = json!("collect-mft-fast");
        manifest["raw_record_bytes_saved"] = json!(false);
        manifest["capture_payload"]=json!("ENUM_USN name metadata, complete-name results and parsed identity/proof only; no NTFS FILE-record byte copies or resident DATA values");
        manifest["links_status6_source"] = json!("exact-base-resident-name-set-no-ATTRIBUTE_LIST");
        manifest["links_status7_source"] =
            json!("out-of-scope reserved file object, not successful name verification");
        manifest["get_record_workers"] = json!(workers);
        report["get_record_workers"] = json!(workers);
    }
    for (root, _, root_frn) in &volumes {
        manifest["volumes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"volume":root,"root_frn":root_frn.to_string(),"capture_complete":false}));
    }
    save_json(&capture.join("manifest.json"), &manifest)?;
    let mut output = begin_snapshot(snapshot)?;
    let mut identities = identities_raw.map(IdentityWriter::create).transpose()?;
    if let Some(path) = identities_raw {
        report["identity_side_table_path"] = json!(path.to_string_lossy());
    }
    let mut counts = Counts::new();
    let mut volume_errors = Vec::new();
    for (index, (root, volume, root_frn)) in volumes.into_iter().enumerate() {
        let result = (|| -> Result<Value> {
            let base = capture.join(root.trim_end_matches(':'));
            let mut fast = if fast_mode {
                let mut state = FastState::new(&volume, &root, root_frn)?;
                state.workers = workers;
                Some(state)
            } else {
                None
            };
            if let Some(state) = &fast {
                manifest["volumes"][index]["volume_serial"] = json!(state.serial);
            }
            let mut nodes = Vec::new();
            let mut names = Vec::new();
            let started = Instant::now();
            let enum_frames = enumerate(
                &volume,
                &base.with_extension("usn"),
                &mut nodes,
                &mut names,
                report,
            )?;
            let enum_ms = started.elapsed().as_secs_f64() * 1000.0;
            let original_records = nodes.len();
            let mut repairs = BufWriter::with_capacity(
                1024 * 1024,
                fs::File::create(base.with_extension("repairs.jsonl"))?,
            );
            let system_started = Instant::now();
            if let Some(state) = fast.as_mut() {
                add_system_nodes(
                    &volume,
                    &root,
                    root_frn,
                    &mut nodes,
                    &mut names,
                    &mut repairs,
                    state,
                )?;
                counts.link_failures += state.system_unknown.len() as u64;
            }
            let system_record_ms = system_started.elapsed().as_secs_f64() * 1000.0;
            let original_started = Instant::now();
            let original = original_failures(&nodes, &names, root_frn, &root)?;
            let original_parent_check_ms = original_started.elapsed().as_secs_f64() * 1000.0;
            let started = Instant::now();
            let overrides = recover(
                &volume,
                &root,
                root_frn,
                &mut nodes,
                &mut names,
                &mut repairs,
                fast.as_mut(),
            )?;
            repairs.flush()?;
            repairs.get_ref().sync_all()?;
            let recovery_ms = started.elapsed().as_secs_f64() * 1000.0;
            let mut stats = write_volume(
                &mut output,
                &mut identities,
                &root,
                root_frn,
                &nodes,
                &names,
                &overrides,
                &capture,
                Some(&volume),
                original,
                None,
                &mut counts,
                report,
                fast.as_mut(),
            )?;
            if let Some(state) = &fast {
                stats["fast_record"] = state.summary();
                stats["system_record_ms"] = json!(system_record_ms);
                manifest["volumes"][index]["system_unknown_records"] = json!(state.system_unknown);
                manifest["volumes"][index]["out_of_scope_reserved_system_records"] =
                    json!(state.system_out_of_scope);
                if !state.system_unknown.is_empty() {
                    stats["status"] = json!("partial");
                }
            }
            stats["enum_ms"] = json!(enum_ms);
            stats["enum_frames"] = json!(enum_frames);
            stats["enum_buffer_bytes"] = json!(ENUM_BUFFER_SIZE);
            stats["original_parent_check_ms"] = json!(original_parent_check_ms);
            stats["recovery_ms"] = json!(recovery_ms);
            stats["original_records"] = json!(original_records);
            stats["parent_records_recovered"] = json!(
                nodes.len()
                    - original_records
                    - fast.as_ref().map_or(0, |state| state.system_added as usize)
            );
            manifest["volumes"][index]["capture_complete"] = json!(true);
            save_json(&capture.join("manifest.json"), &manifest)?;
            Ok(stats)
        })();
        match result {
            Ok(stats) => report["volumes"].as_array_mut().unwrap().push(stats),
            Err(error) => {
                let error = format!("{root}: {error}");
                report["volumes"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"volume":root,"status":"error","error":error}));
                volume_errors.push(error);
            }
        }
    }
    finish_snapshot(output, counts.entries)?;
    if let Some(side) = identities {
        side.finish(counts.entries)?;
        report["identity_side_table_records"] = json!(counts.entries);
        report["identity_side_table_record_bytes"] = json!(24u64);
    }
    manifest["complete"] = json!(volume_errors.is_empty());
    save_json(&capture.join("manifest.json"), &manifest)?;
    if !volume_errors.is_empty() {
        return Err(volume_errors.join("; ").into());
    }
    check_counts(&counts)
}
fn known_root_basename_observation(value: &Value, root_frn: u64, serial: u32) -> bool {
    let proof = &value["proof"];
    let names = value["representative_names"].as_array();
    root_frn & 0x0000_ffff_ffff_ffff == 5
        && value["kind"] == "system-record-unknown"
        && value["error"] == "empty system basename"
        && value["ordinal"] == 5
        && value["volume_serial"] == serial
        && value["returned_ordinal"] == 5
        && value["returned_sequence"] == (root_frn >> 48)
        && value["returned_flags"].as_u64().is_some_and(|x| x & 3 == 3)
        && value["returned_base_frn"] == "0"
        && proof["record_ordinal"] == 5
        && proof["record_sequence"] == (root_frn >> 48)
        && proof["base_record"] == 0
        && proof["in_use"] == true
        && proof["is_dir"] == true
        && names.is_some_and(|n| {
            n.len() == 1
                && n[0]["parent_frn"] == root_frn.to_string()
                && n[0]["name_utf16"] == json!([46])
        })
}
fn replay_inner(
    capture: &Path,
    snapshot: &Path,
    report: &mut Value,
    identities_raw: Option<&Path>,
) -> Result<()> {
    report["capture_directory"] = json!(capture.to_string_lossy());
    let source_sidecar = capture.with_extension("json");
    if source_sidecar.exists() {
        let source: Value = serde_json::from_reader(fs::File::open(&source_sidecar)?)?;
        report["original_collect_ms"] = source["collect_ms"].clone();
        report["original_collect_status"] = source["status"].clone();
        report["original_hardlink_failures"] = source["hardlink_failures"].clone();
    }
    let manifest: Value = serde_json::from_reader(fs::File::open(capture.join("manifest.json"))?)?;
    if manifest["format"] != "KFMFTC01" || manifest["complete"] != true {
        return Err(
            invalid("capture was not completed; cannot reproduce an entire snapshot").into(),
        );
    }
    report["source_capture_mode"] = manifest["mode"].clone();
    let file_search_scope = match manifest.get("file_search_scope") {
        None => false,
        Some(value) if value == FILE_SEARCH_SCOPE => true,
        _ => return Err(invalid("unsupported file_search_scope").into()),
    };
    if file_search_scope {
        report["file_search_scope"] = json!(FILE_SEARCH_SCOPE);
        report["path_semantics"] = json!("all named ordinary file/directory paths and hardlinks; only file objects at NTFS reserved ordinals 0..15 excluded; all directories retained; verified zero-link internal objects and proven disappeared FRNs separately excluded");
    }
    let mut output = begin_snapshot(snapshot)?;
    let mut identities = identities_raw.map(IdentityWriter::create).transpose()?;
    if let Some(path) = identities_raw {
        report["identity_side_table_path"] = json!(path.to_string_lossy());
    }
    let mut counts = Counts::new();
    for volume in manifest["volumes"]
        .as_array()
        .ok_or_else(|| invalid("missing captured volumes"))?
    {
        if volume["capture_complete"] != true {
            return Err(invalid("volume capture incomplete").into());
        }
        let root = normalize(&OsString::from(
            volume["volume"]
                .as_str()
                .ok_or_else(|| invalid("missing volume"))?,
        ))?;
        let base = capture.join(root.trim_end_matches(':'));
        let root_frn = frn(&volume["root_frn"])?;
        let mut nodes = Vec::new();
        let mut names = Vec::new();
        load_enum(&base.with_extension("usn"), &mut nodes, &mut names)?;
        let original = original_failures(&nodes, &names, root_frn, &root)?;
        let serial = if manifest["mode"] == "collect-mft-fast" {
            Some(u32::try_from(
                volume["volume_serial"]
                    .as_u64()
                    .ok_or_else(|| invalid("fast capture serial missing"))?,
            )?)
        } else {
            None
        };
        if serial.is_some() {
            let unknown = volume["system_unknown_records"]
                .as_array()
                .ok_or_else(|| invalid("fast system coverage declaration missing"))?;
            for observation in unknown {
                let reserved_observation = observation["returned_ordinal"]
                    .as_u64()
                    .zip(observation["returned_sequence"].as_u64())
                    .zip(observation["returned_flags"].as_u64())
                    .is_some_and(|((ordinal, sequence), flags)| {
                        ordinal < 16
                            && sequence > 0
                            && sequence <= u16::MAX as u64
                            && flags & 1 != 0
                            && reserved_metadata_file(sequence << 48 | ordinal, flags & 2 != 0)
                            && observation["ordinal"] == ordinal
                            && observation["volume_serial"] == serial.unwrap()
                            && observation["returned_base_frn"] == "0"
                    });
                if file_search_scope && reserved_observation {
                    if !report["out_of_scope_system_observations"].is_array() {
                        report["out_of_scope_system_observations"] = json!([]);
                    }
                    report["out_of_scope_system_observations"]
                        .as_array_mut()
                        .unwrap()
                        .push(observation.clone());
                } else if known_root_basename_observation(observation, root_frn, serial.unwrap()) {
                    if !report["resolved_root_observations"].is_array() {
                        report["resolved_root_observations"] = json!([]);
                    }
                    report["resolved_root_observations"].as_array_mut().unwrap().push(json!({"volume":root,"root_frn":root_frn.to_string(),"reason":"verified drive root is emitted once as D:-style root, not as an empty child basename","original_observation":observation}));
                } else {
                    counts.link_failures += 1;
                    report["unhandled_replay_failures"]
                        .as_array_mut()
                        .unwrap()
                        .push(observation.clone());
                }
            }
        }
        let (repairs, unhandled) =
            replay_fix::load(&base.with_extension("details.jsonl"), &root, serial)?;

        let excluded: HashSet<_> = repairs
            .iter()
            .filter_map(|(id, repair)| match repair {
                replay_fix::Repair::Unlinked(_) | replay_fix::Repair::Disappeared(_) => Some(*id),
                _ => None,
            })
            .collect();
        let (overrides, mut directory_repairs) = replay_recovery(
            &base.with_extension("repairs.jsonl"),
            &mut nodes,
            &mut names,
            root_frn,
            &excluded,
            serial,
            file_search_scope,
        )?;
        let mut out_of_scope_failures = Vec::new();
        for failure in unhandled {
            let excluded = file_search_scope
                && failure["frn"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .is_some_and(|id| {
                        nodes
                            .iter()
                            .find(|node| node.frn == id)
                            .is_some_and(|node| reserved_metadata_file(id, node.is_dir))
                    });
            if excluded {
                out_of_scope_failures.push(failure);
            } else {
                report["unhandled_replay_failures"]
                    .as_array_mut()
                    .unwrap()
                    .push(failure);
            }
        }
        if !out_of_scope_failures.is_empty() {
            if !report["out_of_scope_recorded_failures"].is_array() {
                report["out_of_scope_recorded_failures"] = json!([]);
            }
            report["out_of_scope_recorded_failures"]
                .as_array_mut()
                .unwrap()
                .extend(out_of_scope_failures);
        }
        for proof in &mut directory_repairs {
            proof["volume"] = json!(root);
        }
        let directories_recovered = directory_repairs.len();
        report["directory_projection_repairs"]
            .as_array_mut()
            .unwrap()
            .extend(directory_repairs);
        report["recovered_missing_directories"] = json!(report["directory_projection_repairs"]
            .as_array()
            .unwrap()
            .len());
        let mut stats = write_volume(
            &mut output,
            &mut identities,
            &root,
            root_frn,
            &nodes,
            &names,
            &overrides,
            capture,
            None,
            original,
            Some(&repairs),
            &mut counts,
            report,
            None,
        )?;
        stats["recovered_missing_directories"] = json!(directories_recovered);
        report["volumes"].as_array_mut().unwrap().push(stats);
    }
    finish_snapshot(output, counts.entries)?;
    if let Some(side) = identities {
        side.finish(counts.entries)?;
        report["identity_side_table_records"] = json!(counts.entries);
        report["identity_side_table_record_bytes"] = json!(24u64);
    }
    check_counts(&counts)
}
fn execute(
    snapshot: &Path,
    mode: &str,
    operation: impl FnOnce(&mut Value) -> Result<()>,
) -> Result<()> {
    let started = Instant::now();
    let mut report = json!({"status":"running","mode":mode,"entries":0,"files":0,"folders":0,"volumes":[],"unresolved_paths":0,"cycles":0,"hardlink_failures":0,"private_commit_bytes":0,"peak_working_set_bytes":0,"private_commit_sampling":"maximum observed at enumeration-buffer and output-stage boundaries","path_semantics":"all named file/directory paths; verified zero-link $Deleted internal objects and proven disappeared FRNs are explicitly excluded; unknown sets remain partial","non_atomic_collection":true,"replay_repairs":[],"live_identity_repairs":[],"unhandled_replay_failures":[],"directory_projection_repairs":[],"recovered_missing_directories":0});
    let _privilege = if matches!(
        mode,
        "collect-mft"
            | "collect-mft-fast"
            | "collect-mft-fast-mapped"
            | "collect-mft-fast-mapped-workers"
    ) {
        match backup_privilege() {
            Ok(guard) => {
                report["backup_privilege_enabled"] = json!(true);
                Some(guard)
            }
            Err(error) => {
                report["backup_privilege_enabled"] = json!(false);
                report["backup_privilege_error"] = json!(error.to_string());
                None
            }
        }
    } else {
        None
    };
    let result = operation(&mut report);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    if mode.starts_with("replay-mft") {
        report["replay_ms"] = json!(elapsed_ms);
    } else {
        report["collect_ms"] = json!(elapsed_ms);
    }
    match &result {
        Ok(()) => report["status"] = json!("complete"),
        Err(error) => {
            report["status"] = json!("error");
            report["error"] = json!(error.to_string());
        }
    }
    if let Err(error) = sample_memory(&mut report) {
        report["memory_error"] = json!(error.to_string());
    }
    let mut sidecar = snapshot.as_os_str().to_os_string();
    sidecar.push(".json");
    save_json(Path::new(&sidecar), &report)?;
    crate::diagnostic!("{report}");
    result
}
pub(super) fn run(snapshot: &Path, roots: &[OsString]) -> Result<()> {
    execute(snapshot, "collect-mft", |report| {
        collect(snapshot, roots, report, false, None, 1)
    })
}
pub(super) fn replay(capture: &Path, snapshot: &Path) -> Result<()> {
    execute(snapshot, "replay-mft", |report| {
        replay_inner(capture, snapshot, report, None)
    })
}

pub(super) fn replay_mapped(capture: &Path, snapshot: &Path, identities_raw: &Path) -> Result<()> {
    execute(snapshot, "replay-mft-mapped", |report| {
        replay_inner(capture, snapshot, report, Some(identities_raw))
    })
}

pub(super) fn run_fast(snapshot: &Path, roots: &[OsString]) -> Result<()> {
    execute(snapshot, "collect-mft-fast", |report| {
        collect(snapshot, roots, report, true, None, 1)
    })
}

pub(super) fn run_fast_mapped(
    snapshot: &Path,
    roots: &[OsString],
    identities_raw: &Path,
) -> Result<()> {
    execute(snapshot, "collect-mft-fast-mapped", |report| {
        collect(snapshot, roots, report, true, Some(identities_raw), 1)
    })
}

pub(super) fn run_fast_mapped_workers(
    snapshot: &Path,
    roots: &[OsString],
    identities_raw: &Path,
    workers: usize,
) -> Result<()> {
    execute(snapshot, "collect-mft-fast-mapped-workers", |report| {
        collect(snapshot, roots, report, true, Some(identities_raw), workers)
    })
}
