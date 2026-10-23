//! Read-only Windows API adapter for the standalone MFT experiment.
#[path = "mft_full.rs"]
mod full;
#[path = "journal.rs"]
mod journal;
#[path = "mft_layout.rs"]
mod layout;
use crate::{invalid, u32_at, u64_at, Result};
use serde_json::{json, Value};
use std::ffi::{c_void, OsString};
use std::io::{self, Read, Write};
use std::path::Path;

type Handle = *mut c_void;
#[repr(C)]
struct EnumData {
    cursor: u64,
    low_usn: i64,
    high_usn: i64,
}
#[repr(C)]
struct FileId {
    size: u32,
    kind: u32,
    id: [u64; 2],
}
#[repr(C)]
struct MemoryCounters {
    cb: u32,
    faults: u32,
    values: [usize; 9],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Privileges {
    count: u32,
    luid_low: u32,
    luid_high: i32,
    attributes: u32,
}
#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut u32) -> i32;
    fn AdjustTokenPrivileges(
        token: Handle,
        disable: i32,
        state: *const Privileges,
        length: u32,
        previous: *mut Privileges,
        returned: *mut u32,
    ) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        creation: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn DeviceIoControl(
        handle: Handle,
        code: u32,
        input: *const c_void,
        input_len: u32,
        output: *mut c_void,
        output_len: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn GetFileInformationByHandle(handle: Handle, info: *mut u32) -> i32;
    fn OpenFileById(
        volume: Handle,
        id: *const FileId,
        access: u32,
        share: u32,
        security: *mut c_void,
        flags: u32,
    ) -> Handle;
    fn GetFinalPathNameByHandleW(handle: Handle, path: *mut u16, length: u32, flags: u32) -> u32;
    fn GetCurrentProcess() -> Handle;
    fn K32GetProcessMemoryInfo(process: Handle, counters: *mut MemoryCounters, size: u32) -> i32;
    fn SetLastError(error: u32);
}
// Process-token privilege is shared by concurrent volume/session readers.
// Restore its original state only after the last reader guard has dropped.
struct PrivilegeState {
    token: usize,
    previous: Privileges,
    users: usize,
}
static BACKUP_PRIVILEGE: std::sync::Mutex<Option<PrivilegeState>> = std::sync::Mutex::new(None);
struct BackupPrivilege;
impl Drop for BackupPrivilege {
    fn drop(&mut self) {
        if let Ok(mut shared) = BACKUP_PRIVILEGE.lock() {
            if let Some(state) = shared.as_mut() {
                state.users -= 1;
                if state.users == 0 {
                    let state = shared.take().unwrap();
                    unsafe {
                        AdjustTokenPrivileges(
                            state.token as Handle,
                            0,
                            &state.previous,
                            0,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                        );
                        CloseHandle(state.token as Handle);
                    }
                }
            }
        }
    }
}
fn backup_privilege() -> Result<BackupPrivilege> {
    let mut shared = BACKUP_PRIVILEGE
        .lock()
        .map_err(|_| invalid("backup privilege lock poisoned"))?;
    if let Some(state) = shared.as_mut() {
        state.users += 1;
        return Ok(BackupPrivilege);
    }
    *shared = Some(enable_backup_privilege()?);
    Ok(BackupPrivilege)
}
fn enable_backup_privilege() -> Result<PrivilegeState> {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), 0x28, &mut token) } == 0 {
        return Err(last_error("OpenProcessToken").into());
    }
    let token = OwnedHandle(token);
    let mut state = Privileges {
        count: 1,
        luid_low: 0,
        luid_high: 0,
        attributes: 2,
    };
    let name: Vec<_> = "SeBackupPrivilege".encode_utf16().chain(Some(0)).collect();
    if unsafe { LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut state.luid_low) } == 0 {
        return Err(last_error("LookupPrivilegeValueW").into());
    }
    let mut previous = Privileges {
        count: 0,
        luid_low: 0,
        luid_high: 0,
        attributes: 0,
    };
    let mut returned = 0;
    unsafe {
        SetLastError(0);
    }
    if unsafe {
        AdjustTokenPrivileges(
            token.0,
            0,
            &state,
            std::mem::size_of::<Privileges>() as u32,
            &mut previous,
            &mut returned,
        )
    } == 0
    {
        return Err(last_error("AdjustTokenPrivileges").into());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(0) {
        return Err(format!("AdjustTokenPrivileges: {error}").into());
    }
    let raw = token.0 as usize;
    std::mem::forget(token);
    Ok(PrivilegeState {
        token: raw,
        previous,
        users: 1,
    })
}
struct OwnedHandle(Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn last_error(operation: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(
        error.kind(),
        format!(
            "{operation}: Win32 {}: {error}",
            error.raw_os_error().unwrap_or(-1)
        ),
    )
}
fn open(name: &str, access: u32, flags: u32) -> Result<OwnedHandle> {
    crate::cancel::check()?;
    let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            7,
            std::ptr::null_mut(),
            3,
            flags,
            std::ptr::null_mut(),
        )
    };
    if handle as isize == -1 {
        return Err(last_error(&format!("{name}: CreateFileW")).into());
    }
    Ok(OwnedHandle(handle))
}
fn info(handle: &OwnedHandle) -> Result<[u32; 13]> {
    let mut information = [0; 13];
    if unsafe { GetFileInformationByHandle(handle.0, information.as_mut_ptr()) } == 0 {
        return Err(last_error("GetFileInformationByHandle").into());
    }
    Ok(information)
}
fn info_frn(information: &[u32; 13]) -> u64 {
    ((information[11] as u64) << 32) | information[12] as u64
}
fn open_id(volume: &OwnedHandle, frn: u64) -> Result<(OwnedHandle, [u32; 13])> {
    crate::cancel::check()?;
    let id = FileId {
        size: std::mem::size_of::<FileId>() as u32,
        kind: 0,
        id: [frn, 0],
    };
    let handle = unsafe { OpenFileById(volume.0, &id, 0, 7, std::ptr::null_mut(), 0x0220_0000) };
    if handle as isize == -1 {
        return Err(last_error("OpenFileById").into());
    }
    let handle = OwnedHandle(handle);
    let information = info(&handle)?;
    if info_frn(&information) != frn {
        return Err(invalid("OpenFileById returned a different FRN").into());
    }
    Ok((handle, information))
}
fn ioctl(handle: &OwnedHandle, code: u32, input: &[u8], output: &mut [u8]) -> io::Result<usize> {
    if crate::cancel::cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "metadata operation cancelled",
        ));
    }
    let mut size = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle.0,
            code,
            input.as_ptr().cast(),
            input.len() as u32,
            output.as_mut_ptr().cast(),
            output.len() as u32,
            &mut size,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if size as usize > output.len() {
        return Err(invalid("DeviceIoControl returned an invalid size"));
    }
    Ok(size as usize)
}
pub(crate) fn sample_memory(report: &mut Value) -> Result<()> {
    let mut memory = MemoryCounters {
        cb: std::mem::size_of::<MemoryCounters>() as u32,
        faults: 0,
        values: [0; 9],
    };
    if unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut memory,
            std::mem::size_of::<MemoryCounters>() as u32,
        )
    } == 0
    {
        return Err(last_error("GetProcessMemoryInfo").into());
    }
    report["peak_working_set_bytes"] = json!(memory.values[0]);
    report["private_commit_bytes"] = json!(report["private_commit_bytes"]
        .as_u64()
        .unwrap_or(0)
        .max(memory.values[8] as u64));
    report["peak_pagefile_bytes"] = json!(memory.values[7]);
    Ok(())
}
struct Node {
    frn: u64,
    parent: u64,
    name_start: u32,
    name_len: u32,
    is_dir: bool,
}
fn parse_record(record: &[u8], names: &mut Vec<u16>) -> Result<Node> {
    if record.len() < 60 {
        return Err(invalid("truncated USN record").into());
    }
    let length = u32_at(record, 0) as usize;
    let major = u16::from_le_bytes([record[4], record[5]]);
    if length < 60 || length % 8 != 0 || length > record.len() || major != 2 {
        return Err(
            format!("unsupported/invalid USN record: version {major}, length {length}").into(),
        );
    }
    let name_len = u16::from_le_bytes([record[56], record[57]]) as usize;
    let name_offset = u16::from_le_bytes([record[58], record[59]]) as usize;
    if name_offset < 60
        || name_offset % 2 != 0
        || name_len % 2 != 0
        || name_offset + name_len > length
    {
        return Err(invalid("invalid USN filename bounds").into());
    }
    let name_start = u32::try_from(names.len())?;
    names.extend(
        record[name_offset..name_offset + name_len]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]])),
    );
    Ok(Node {
        frn: u64_at(record, 8),
        parent: u64_at(record, 16),
        name_start,
        name_len: (name_len / 2) as u32,
        is_dir: u32_at(record, 52) & 0x10 != 0,
    })
}
fn parse_block(block: &[u8], nodes: &mut Vec<Node>, names: &mut Vec<u16>) -> Result<()> {
    if block.len() < 8 {
        return Err(invalid("truncated ENUM block").into());
    }
    let mut offset = 8;
    while offset < block.len() {
        if offset % 8 != 0 {
            return Err(invalid("unaligned ENUM record").into());
        }
        let node = parse_record(&block[offset..], names)?;
        let length = u32_at(block, offset) as usize;
        nodes.push(node);
        offset += length;
    }
    Ok(())
}
fn write_frame(output: &mut impl Write, bytes: &[u8]) -> Result<()> {
    output.write_all(&u32::try_from(bytes.len())?.to_le_bytes())?;
    output.write_all(bytes)?;
    Ok(())
}
fn read_frame(input: &mut impl Read, maximum: usize) -> Result<Option<Vec<u8>>> {
    let mut length = [0; 4];
    match input.read(&mut length[..1])? {
        0 => return Ok(None),
        _ => input.read_exact(&mut length[1..])?,
    }
    let length = u32::from_le_bytes(length) as usize;
    if length > maximum {
        return Err(invalid("capture frame exceeds expected size").into());
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}
pub(crate) fn write_path(output: &mut impl Write, is_dir: bool, path: &[u16]) -> Result<()> {
    output.write_all(&[u8::from(is_dir)])?;
    write_units(output, path)
}
fn write_units(output: &mut impl Write, path: &[u16]) -> Result<()> {
    output.write_all(&u32::try_from(path.len() * 2)?.to_le_bytes())?;
    for unit in path {
        output.write_all(&unit.to_le_bytes())?;
    }
    Ok(())
}
fn read_units(input: &mut impl Read) -> Result<Vec<u16>> {
    let bytes =
        read_frame(input, 131072)?.ok_or_else(|| invalid("truncated UTF16 capture path"))?;
    if bytes.len() % 2 != 0 {
        return Err(invalid("odd UTF16 capture path length").into());
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect())
}
fn final_path(handle: &OwnedHandle, root: &str) -> Result<Vec<u16>> {
    let mut buffer = vec![0; 32769];
    let length =
        unsafe { GetFinalPathNameByHandleW(handle.0, buffer.as_mut_ptr(), buffer.len() as u32, 0) }
            as usize;
    if length == 0 {
        return Err(last_error("GetFinalPathNameByHandleW").into());
    }
    if length >= buffer.len() {
        return Err(invalid("final path exceeds supported Windows path length").into());
    }
    buffer.truncate(length);
    let prefix: Vec<_> = format!(r"\\?\{root}\").encode_utf16().collect();
    if !buffer.starts_with(&prefix) {
        return Err(invalid("final path is not on the requested volume").into());
    }
    Ok(buffer[4..].to_vec())
}
fn open_path(path: &[u16], expected_frn: u64) -> Result<(OwnedHandle, [u32; 13])> {
    let wide: Vec<_> = r"\\?\"
        .encode_utf16()
        .chain(path.iter().copied())
        .chain(Some(0))
        .collect();
    let file = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            7,
            std::ptr::null_mut(),
            3,
            0x0220_0000,
            std::ptr::null_mut(),
        )
    };
    if file as isize == -1 {
        return Err(last_error("CreateFileW(no-follow source)").into());
    }
    let file = OwnedHandle(file);
    let information = info(&file)?;
    if info_frn(&information) != expected_frn || information[0] & 0x10 != 0 {
        return Err(invalid("source FRN/type changed").into());
    }
    Ok((file, information))
}
#[repr(C)]
struct IoStatus {
    status: isize,
    information: usize,
}
#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationFile(
        handle: Handle,
        ios: *mut IoStatus,
        buffer: *mut c_void,
        length: u32,
        class: u32,
    ) -> i32;
}
struct LinkName {
    parent: u64,
    name: Vec<u16>,
}
#[derive(Debug)]
struct NativeLinkFailure {
    description: String,
    ntstatus: u32,
    raw: Vec<u8>,
}
impl std::fmt::Display for NativeLinkFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description)
    }
}
impl std::error::Error for NativeLinkFailure {}
fn parse_native_links(raw: &[u8], expected: u32) -> Result<Vec<LinkName>> {
    if raw.len() < 8 || u32_at(raw, 0) == 0 || u32_at(raw, 0) as usize > raw.len() {
        return Err(invalid("invalid FILE_LINKS_INFORMATION size").into());
    }
    let count = u32_at(raw, 4);
    if count == 0 || count != expected || count > 65536 {
        return Err(format!("hardlink count mismatch: handle={expected}, returned={count}").into());
    }
    let mut offset = 8;
    let mut links = Vec::with_capacity(count as usize);
    for index in 0..count {
        if offset % 8 != 0 || offset + 20 > raw.len() {
            return Err(invalid("unaligned/truncated hardlink entry").into());
        }
        let next = u32_at(raw, offset) as usize;
        let parent = u64_at(raw, offset + 8);
        let length = u32_at(raw, offset + 16) as usize;
        let end = offset
            + 20
            + length
                .checked_mul(2)
                .ok_or_else(|| invalid("hardlink name length overflow"))?;
        if length == 0 || end > raw.len() {
            return Err(invalid("hardlink name out of bounds").into());
        }
        let name: Vec<_> = raw[offset + 20..end]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        if name.iter().any(|&c| c == 0 || c == 47 || c == 92) {
            return Err(invalid("hardlink basename contains a path separator/NUL").into());
        }
        links.push(LinkName { parent, name });
        if index + 1 == count {
            if next != 0 {
                return Err(invalid("hardlink count ended before linked list").into());
            }
        } else {
            if next % 8 != 0
                || next < 20 + length * 2
                || offset.checked_add(next).is_none_or(|end| end >= raw.len())
            {
                return Err(invalid("invalid hardlink NextEntryOffset").into());
            }
            offset += next;
        }
    }
    Ok(links)
}
fn native_links(handle: &OwnedHandle, expected: u32) -> Result<(Vec<u8>, Vec<LinkName>)> {
    let mut buffer = vec![0; 4096];
    loop {
        let mut ios = IoStatus {
            status: 0,
            information: 0,
        };
        let status = unsafe {
            NtQueryInformationFile(
                handle.0,
                &mut ios,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                46,
            )
        } as u32;
        let size = ios.information.min(buffer.len());
        if status == 0 {
            let raw = buffer[..size].to_vec();
            return match parse_native_links(&raw, expected) {
                Ok(names) => Ok((raw, names)),
                Err(error) => Err(NativeLinkFailure {
                    description: error.to_string(),
                    ntstatus: status,
                    raw,
                }
                .into()),
            };
        }
        if matches!(status, 0x8000_0005 | 0xc000_0004 | 0xc000_0023) {
            let needed = (u32_at(&buffer, 0) as usize).max(buffer.len() * 2);
            if needed <= 16 * 1024 * 1024 {
                buffer.resize(needed, 0);
                continue;
            }
        }
        return Err(NativeLinkFailure {
            description: format!(
                "NtQueryInformationFile(FileHardLinkInformation): NTSTATUS 0x{status:08x}"
            ),
            ntstatus: status,
            raw: buffer[..size].to_vec(),
        }
        .into());
    }
}
pub fn run(snapshot: &Path, roots: &[OsString]) -> Result<()> {
    full::run(snapshot, roots)
}
pub fn replay(capture: &Path, snapshot: &Path) -> Result<()> {
    full::replay(capture, snapshot)
}

pub fn capture_layout(output: &Path, roots: &[String]) -> Result<()> {
    layout::run(output, roots)
}

pub fn run_fast(snapshot: &Path, roots: &[OsString]) -> Result<()> {
    full::run_fast(snapshot, roots)
}

pub fn run_fast_mapped(snapshot: &Path, roots: &[OsString], identities: &Path) -> Result<()> {
    full::run_fast_mapped(snapshot, roots, identities)
}
pub fn run_fast_mapped_workers(
    snapshot: &Path,
    roots: &[OsString],
    identities: &Path,
    workers: usize,
) -> Result<()> {
    full::run_fast_mapped_workers(snapshot, roots, identities, workers)
}

pub fn replay_mapped(capture: &Path, snapshot: &Path, identities: &Path) -> Result<()> {
    full::replay_mapped(capture, snapshot, identities)
}
pub(crate) fn verify_metadata_id(
    root: &str,
    id: u64,
    serial: u32,
    is_dir: bool,
) -> Result<[u32; 13]> {
    let _privilege = backup_privilege()?;
    let volume = open(&format!(r"\\.\{root}"), 0x8000_0000, 0)?;
    let (_handle, information) = open_id(&volume, id)?;
    if information[7] != serial || (information[0] & 16 != 0) != is_dir {
        return Err(invalid("metadata ID verification volume/type mismatch").into());
    }
    Ok(information)
}

pub fn journal_begin(checkpoint: &Path, roots: &[OsString]) -> Result<()> {
    journal::begin(checkpoint, roots)
}
pub fn journal_watch_continuous(checkpoint: &Path, feed: &Path) -> Result<()> {
    journal::watch(checkpoint, feed)
}

// Explicit search scope only: NTFS reserved file records, never directories/subtrees.
pub(crate) const FILE_SEARCH_SCOPE: &str = "file-search-exclude-ntfs-reserved-files-0-15-v1";
pub(crate) fn reserved_metadata_file(frn: u64, is_dir: bool) -> bool {
    !is_dir && (frn & 0x0000_ffff_ffff_ffff) < 16
}
