//! Bounded-resident XPRESS name blocks; generated index I/O is unbuffered, 4KiB aligned.
use super::{decode_path, invalid, snapshot_header, u32_at, u64_at, utf16_units, Result};
use memchr::memmem::Finder;
use serde_json::{json, Value};
use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Instant;
const MAGIC: &[u8; 8] = b"KFPAGED1";
const ALIGN: usize = 4096;
const IO: usize = 256 * 1024;
const RAW: usize = 64 * 1024;
const PACKED: usize = RAW * 2 + 4096;
const NO_BUFFERING: u32 = 0x2000_0000;
#[link(name = "Cabinet")]
extern "system" {
    fn CreateCompressor(algorithm: u32, alloc: *const c_void, handle: *mut *mut c_void) -> i32;
    fn CreateDecompressor(algorithm: u32, alloc: *const c_void, handle: *mut *mut c_void) -> i32;
    fn CloseCompressor(handle: *mut c_void) -> i32;
    fn CloseDecompressor(handle: *mut c_void) -> i32;
    fn Compress(
        handle: *mut c_void,
        input: *const c_void,
        len: usize,
        output: *mut c_void,
        cap: usize,
        written: *mut usize,
    ) -> i32;
    fn Decompress(
        handle: *mut c_void,
        input: *const c_void,
        len: usize,
        output: *mut c_void,
        cap: usize,
        written: *mut usize,
    ) -> i32;
}
struct Codec {
    handle: *mut c_void,
    compress: bool,
}
impl Codec {
    fn new(compress: bool) -> Result<Self> {
        let mut handle = std::ptr::null_mut();
        let ok = unsafe {
            if compress {
                CreateCompressor(3, std::ptr::null(), &mut handle)
            } else {
                CreateDecompressor(3, std::ptr::null(), &mut handle)
            }
        };
        if ok == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self { handle, compress })
    }
    fn run(&self, input: &[u8], output: &mut [u8]) -> Result<usize> {
        let mut written = 0;
        let ok = unsafe {
            if self.compress {
                Compress(
                    self.handle,
                    input.as_ptr().cast(),
                    input.len(),
                    output.as_mut_ptr().cast(),
                    output.len(),
                    &mut written,
                )
            } else {
                Decompress(
                    self.handle,
                    input.as_ptr().cast(),
                    input.len(),
                    output.as_mut_ptr().cast(),
                    output.len(),
                    &mut written,
                )
            }
        };
        if ok == 0 {
            return Err(io::Error::last_os_error().into());
        }
        if written > output.len() {
            return Err(invalid("codec returned out-of-bounds size").into());
        }
        Ok(written)
    }
}
impl Drop for Codec {
    fn drop(&mut self) {
        unsafe {
            if self.compress {
                CloseCompressor(self.handle);
            } else {
                CloseDecompressor(self.handle);
            }
        }
    }
}
struct Aligned {
    pointer: *mut u8,
    layout: Layout,
}
impl Aligned {
    fn new() -> Result<Self> {
        let layout = Layout::from_size_align(IO, ALIGN)?;
        let pointer = unsafe { alloc_zeroed(layout) };
        if pointer.is_null() {
            return Err(invalid("aligned allocation failed").into());
        }
        Ok(Self { pointer, layout })
    }
    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.pointer, IO) }
    }
    fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.pointer, IO) }
    }
}
impl Drop for Aligned {
    fn drop(&mut self) {
        unsafe { dealloc(self.pointer, self.layout) }
    }
}
fn aligned_length(n: u64) -> Result<u64> {
    n.checked_add(ALIGN as u64 - 1)
        .map(|x| x / ALIGN as u64 * ALIGN as u64)
        .ok_or_else(|| invalid("aligned length overflow").into())
}
struct DirectReader {
    file: File,
    buffer: Aligned,
    length: u64,
    position: u64,
    start: u64,
    valid: usize,
}
impl DirectReader {
    fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NO_BUFFERING)
            .open(path)?;
        let length = file.metadata()?.len();
        if length < ALIGN as u64 || length % ALIGN as u64 != 0 {
            return Err(invalid("paged index must have 4KiB padded physical length").into());
        }
        Ok(Self {
            file,
            buffer: Aligned::new()?,
            length,
            position: 0,
            start: 0,
            valid: 0,
        })
    }
    fn seek_to(&mut self, offset: u64) -> Result<()> {
        if offset > self.length {
            return Err(invalid("direct seek outside file").into());
        }
        self.position = offset;
        Ok(())
    }
}
impl Read for DirectReader {
    fn read(&mut self, dest: &mut [u8]) -> io::Result<usize> {
        if dest.is_empty() || self.position == self.length {
            return Ok(0);
        }
        if self.valid == 0
            || self.position < self.start
            || self.position >= self.start + self.valid as u64
        {
            self.start = self.position / ALIGN as u64 * ALIGN as u64;
            let count = (self.length - self.start).min(IO as u64) as usize;
            self.file.seek(SeekFrom::Start(self.start))?;
            let n = self.file.read(&mut self.buffer.bytes_mut()[..count])?;
            if n != count {
                return Err(invalid("short aligned index read; no cached fallback"));
            }
            self.valid = n;
        }
        let offset = (self.position - self.start) as usize;
        let count = dest.len().min(self.valid - offset);
        dest[..count].copy_from_slice(&self.buffer.bytes()[offset..offset + count]);
        self.position += count as u64;
        Ok(count)
    }
}
struct DirectWriter {
    file: File,
    buffer: Aligned,
    used: usize,
    logical: u64,
}
impl DirectWriter {
    fn new(path: &Path) -> Result<Self> {
        Ok(Self {
            file: OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(NO_BUFFERING)
                .open(path)?,
            buffer: Aligned::new()?,
            used: 0,
            logical: 0,
        })
    }
    fn bytes(&mut self, mut data: &[u8]) -> Result<()> {
        while !data.is_empty() {
            let n = data.len().min(IO - self.used);
            self.buffer.bytes_mut()[self.used..self.used + n].copy_from_slice(&data[..n]);
            self.used += n;
            self.logical = self
                .logical
                .checked_add(n as u64)
                .ok_or_else(|| invalid("index length overflow"))?;
            data = &data[n..];
            if self.used == IO {
                let n = self.file.write(self.buffer.bytes())?;
                if n != IO {
                    return Err(invalid("short aligned index write").into());
                }
                self.used = 0;
            }
        }
        Ok(())
    }
    fn finish(mut self, header: &[u8]) -> Result<u64> {
        if header.len() != ALIGN {
            return Err(invalid("header alignment invalid").into());
        }
        if self.used > 0 {
            let n = aligned_length(self.used as u64)? as usize;
            self.buffer.bytes_mut()[self.used..n].fill(0);
            let written = self.file.write(&self.buffer.bytes()[..n])?;
            if written != n {
                return Err(invalid("short final aligned write").into());
            }
        }
        self.buffer.bytes_mut()[..ALIGN].copy_from_slice(header);
        self.file.seek(SeekFrom::Start(0))?;
        if self.file.write(&self.buffer.bytes()[..ALIGN])? != ALIGN {
            return Err(invalid("short aligned header write").into());
        }
        self.file.sync_all()?;
        Ok(aligned_length(self.logical)?)
    }
}
#[derive(Clone, Copy)]
struct Header {
    entries: u64,
    snapshot: u64,
    pool: u64,
    invalid: u64,
    unique: u64,
    logical: u64,
    blocks: u64,
}
impl Header {
    fn bytes(self) -> Vec<u8> {
        let mut b = vec![0; ALIGN];
        b[..8].copy_from_slice(MAGIC);
        for (i, v) in [
            self.entries,
            self.snapshot,
            self.pool,
            self.invalid,
            self.unique,
            self.logical,
            self.blocks,
            3,
            RAW as u64,
            ALIGN as u64,
        ]
        .into_iter()
        .enumerate()
        {
            b[8 + i * 8..16 + i * 8].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
    fn names_start(self) -> Result<u64> {
        (ALIGN as u64)
            .checked_add(
                self.entries
                    .checked_mul(8)
                    .ok_or_else(|| invalid("entry bytes overflow"))?,
            )
            .ok_or_else(|| invalid("entry offset overflow").into())
    }
    fn read(reader: &mut DirectReader, snapshot: u64, count: u64) -> Result<Self> {
        let mut b = vec![0; ALIGN];
        reader.read_exact(&mut b)?;
        if &b[..8] != MAGIC
            || u64_at(&b, 64) != 3
            || u64_at(&b, 72) != RAW as u64
            || u64_at(&b, 80) != ALIGN as u64
        {
            return Err(invalid("unsupported paged header/codec/alignment").into());
        }
        let h = Self {
            entries: u64_at(&b, 8),
            snapshot: u64_at(&b, 16),
            pool: u64_at(&b, 24),
            invalid: u64_at(&b, 32),
            unique: u64_at(&b, 40),
            logical: u64_at(&b, 48),
            blocks: u64_at(&b, 56),
        };
        if h.entries != count
            || h.snapshot != snapshot
            || snapshot >= 1u64 << 32
            || h.invalid > count
            || h.unique > count
            || h.unique > u32::MAX as u64
            || (count > 0 && h.unique == 0)
            || h.pool > u32::MAX as u64
            || h.blocks > h.unique
            || (h.unique > 0 && h.blocks == 0)
            || h.logical < h.names_start()?
            || aligned_length(h.logical)? != reader.length
        {
            return Err(invalid("invalid paged counts/length/snapshot").into());
        }
        Ok(h)
    }
}
fn check_entry(bytes: &[u8], h: Header, last: &mut u64, first: &mut bool) -> Result<()> {
    let id = u32_at(bytes, 0) as u64;
    let offset = u32_at(bytes, 4) as u64;
    if id >= h.unique
        || offset < 16
        || offset + 5 > h.snapshot
        || (*first && offset != 16)
        || (!*first && offset < *last + 5)
    {
        return Err(invalid("entry ID/snapshot offset invalid").into());
    }
    *first = false;
    *last = offset;
    Ok(())
}
fn block_write(
    writer: &mut DirectWriter,
    codec: &Codec,
    raw: &[u8],
    names: u32,
    packed: &mut [u8],
) -> Result<()> {
    crate::cancel::check()?;
    let n = codec.run(raw, packed)?;
    for value in [names, u32::try_from(raw.len())?, u32::try_from(n)?, 0] {
        writer.bytes(&value.to_le_bytes())?;
    }
    writer.bytes(&packed[..n])
}
pub(super) fn build(snapshot: &Path, compact: &Path, index: &Path) -> Result<()> {
    let started = Instant::now();
    let mut source = File::open(snapshot)?;
    let snapshot_bytes = source.metadata()?.len();
    let count = snapshot_header(&mut source)?;
    if snapshot_bytes >= 1u64 << 32 || count > snapshot_bytes.saturating_sub(16) / 5 {
        return Err(invalid("invalid/oversized snapshot").into());
    }
    let compact_file = File::open(compact)?;
    let compact_bytes = compact_file.metadata()?.len();
    let mut entries = BufReader::with_capacity(IO, compact_file);
    let mut b = [0u8; 48];
    entries.read_exact(&mut b)?;
    if &b[..8] != b"KFINDEX3" || u64_at(&b, 8) != count || u64_at(&b, 16) != snapshot_bytes {
        return Err(invalid("compact source header mismatch").into());
    }
    let pool = u64_at(&b, 24);
    let invalid_count = u64_at(&b, 32);
    let unique = u64_at(&b, 40);
    let table_start = 48u64
        .checked_add(
            count
                .checked_mul(8)
                .ok_or_else(|| invalid("entry overflow"))?,
        )
        .ok_or_else(|| invalid("entry overflow"))?;
    let pool_start = table_start
        .checked_add(
            unique
                .checked_add(1)
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| invalid("dictionary overflow"))?,
        )
        .ok_or_else(|| invalid("dictionary overflow"))?;
    if unique > count
        || unique > u32::MAX as u64
        || pool > u32::MAX as u64
        || invalid_count > count
        || pool_start.checked_add(pool) != Some(compact_bytes)
    {
        return Err(invalid("invalid compact source counts/size").into());
    }
    let mut h = Header {
        entries: count,
        snapshot: snapshot_bytes,
        pool,
        invalid: invalid_count,
        unique,
        logical: 0,
        blocks: 0,
    };
    let mut writer = DirectWriter::new(index)?;
    writer.bytes(&[0; ALIGN])?;
    let mut chunk = vec![0u8; IO];
    let mut remaining = count * 8;
    let mut last = 0;
    let mut first = true;
    while remaining > 0 {
        let n = remaining.min(IO as u64) as usize;
        entries.read_exact(&mut chunk[..n])?;
        for r in chunk[..n].chunks_exact(8) {
            check_entry(r, h, &mut last, &mut first)?;
        }
        writer.bytes(&chunk[..n])?;
        remaining -= n as u64;
    }
    let mut offsets = BufReader::with_capacity(IO, File::open(compact)?);
    offsets.seek(SeekFrom::Start(table_start))?;
    let mut names = BufReader::with_capacity(IO, File::open(compact)?);
    names.seek(SeekFrom::Start(pool_start))?;
    let mut word = [0; 4];
    offsets.read_exact(&mut word)?;
    if u32::from_le_bytes(word) != 0 {
        return Err(invalid("compact first dictionary offset nonzero").into());
    }
    let codec = Codec::new(true)?;
    let mut raw = Vec::with_capacity(RAW);
    let mut packed = vec![0u8; PACKED];
    let mut block_names = 0u32;
    let mut previous = 0u32;
    let mut name = Vec::new();
    for _ in 0..unique {
        offsets.read_exact(&mut word)?;
        let end = u32::from_le_bytes(word);
        if end < previous || end as u64 > pool {
            return Err(invalid("compact dictionary offset invalid").into());
        }
        let length = (end - previous) as usize;
        if length + 4 > RAW {
            return Err(invalid("basename exceeds paged 64KiB block limit").into());
        }
        name.resize(length, 0);
        names.read_exact(&mut name)?;
        std::str::from_utf8(&name)?;
        if raw.len() + 4 + length > RAW {
            block_write(&mut writer, &codec, &raw, block_names, &mut packed)?;
            h.blocks += 1;
            raw.clear();
            block_names = 0;
        }
        raw.extend_from_slice(&(length as u32).to_le_bytes());
        raw.extend_from_slice(&name);
        block_names += 1;
        previous = end;
    }
    if previous as u64 != pool {
        return Err(invalid("compact dictionary tail mismatch").into());
    }
    if block_names > 0 {
        block_write(&mut writer, &codec, &raw, block_names, &mut packed)?;
        h.blocks += 1;
    }
    h.logical = writer.logical;
    let index_bytes = writer.finish(&h.bytes())?;
    crate::diagnostic!(
        "{}",
        json!({"format":"KFPAGED1","entries":count,"distinct_names":unique,"snapshot_bytes":snapshot_bytes,"invalid_utf16_paths":invalid_count,"build_ms":started.elapsed().as_secs_f64()*1000.0,"transcode_ms":started.elapsed().as_secs_f64()*1000.0,"index_bytes":index_bytes,"logical_bytes":h.logical,"compact_source_bytes":compact_bytes,"original_name_pool_bytes":pool,"entry_table_bytes":count*8,"compressed_dictionary_bytes":h.logical-h.names_start()?,"blocks":h.blocks,"algorithm":"XPRESS","generated_index_io":"FILE_FLAG_NO_BUFFERING, 4096-byte alignment; no cached fallback","build_cost_excludes_compact_build":true})
    );
    Ok(())
}
/// Opt-in build with a temporary RAM dictionary, no intermediate compact file.
pub(super) fn build_in_memory(snapshot: &Path, index: &Path) -> Result<()> {
    let started = Instant::now();
    let parts = super::compact::build_parts(snapshot)?;
    let dictionary_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut h = Header {
        entries: parts.count,
        snapshot: parts.snapshot_bytes,
        pool: parts.pool.len() as u64,
        invalid: parts.invalid_utf16_paths,
        unique: parts.offsets.len() as u64 - 1,
        logical: 0,
        blocks: 0,
    };
    let mut writer = DirectWriter::new(index)?;
    writer.bytes(&[0; ALIGN])?;
    writer.bytes(&parts.entries)?;
    let codec = Codec::new(true)?;
    let mut raw = Vec::with_capacity(RAW);
    let mut packed = vec![0u8; PACKED];
    let mut block_names = 0u32;
    for offsets in parts.offsets.windows(2) {
        let name = &parts.pool[offsets[0] as usize..offsets[1] as usize];
        if name.len() + 4 > RAW {
            return Err(invalid("basename exceeds paged 64KiB block limit").into());
        }
        if raw.len() + 4 + name.len() > RAW {
            block_write(&mut writer, &codec, &raw, block_names, &mut packed)?;
            h.blocks += 1;
            raw.clear();
            block_names = 0;
        }
        raw.extend_from_slice(&(name.len() as u32).to_le_bytes());
        raw.extend_from_slice(name);
        block_names += 1;
    }
    if block_names > 0 {
        block_write(&mut writer, &codec, &raw, block_names, &mut packed)?;
        h.blocks += 1;
    }
    h.logical = writer.logical;
    let index_bytes = writer.finish(&h.bytes())?;
    crate::diagnostic!(
        "{}",
        json!({"format":"KFPAGED1","build_mode":"in-memory-no-compact-intermediate","entries":h.entries,"distinct_names":h.unique,"snapshot_bytes":h.snapshot,"invalid_utf16_paths":h.invalid,"build_ms":started.elapsed().as_secs_f64()*1000.0,"dictionary_ms":dictionary_ms,"index_bytes":index_bytes,"logical_bytes":h.logical,"original_name_pool_bytes":h.pool,"entry_table_bytes":h.entries*8,"compressed_dictionary_bytes":h.logical-h.names_start()?,"blocks":h.blocks,"algorithm":"XPRESS","generated_index_io":"FILE_FLAG_NO_BUFFERING, 4096-byte alignment; no cached fallback","build_cost_excludes_compact_build":false})
    );
    Ok(())
}
pub(crate) struct Search {
    reader: DirectReader,
    snapshot: File,
    h: Header,
    codec: Codec,
    raw: Vec<u8>,
    packed: Vec<u8>,
    table: Vec<u8>,
    bits: Vec<u8>,
}

/// A baseline path selected by the paged name scan.
fn read_hit(file: &mut File, bytes: u64, offset: u64, buffer: &mut Vec<u8>) -> Result<BaseHit> {
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0; 5];
    file.read_exact(&mut header)?;
    let length = u32_at(&header, 1) as u64;
    if header[0] > 1
        || length % 2 != 0
        || offset
            .checked_add(5)
            .and_then(|n| n.checked_add(length))
            .is_none_or(|end| end > bytes)
    {
        return Err(invalid("invalid snapshot hit record").into());
    }
    buffer.resize(length as usize, 0);
    file.read_exact(buffer)?;
    Ok(BaseHit {
        path: utf16_units(buffer),
        is_dir: header[0] != 0,
    })
}
pub(crate) struct BaseHit {
    pub(crate) path: Vec<u16>,
    pub(crate) is_dir: bool,
}

impl Search {
    fn scan_names(&mut self, terms: Option<&[Finder<'_>]>, check_cancel: bool) -> Result<()> {
        self.reader.seek_to(self.h.names_start()?)?;
        let mut id = 0u64;
        let mut pool = 0u64;
        for _ in 0..self.h.blocks {
            // A limited interactive query installs the thread-local token;
            // checking once per compressed block bounds cancellation latency
            // without adding work to the exact-total query path.
            if check_cancel {
                crate::cancel::check()?;
            }
            let mut b = [0u8; 16];
            self.reader.read_exact(&mut b)?;
            let count = u32_at(&b, 0) as u64;
            let raw = u32_at(&b, 4) as usize;
            let packed = u32_at(&b, 8) as usize;
            if count == 0
                || raw > RAW
                || raw < 4
                || packed == 0
                || packed > PACKED
                || u32_at(&b, 12) != 0
                || count > self.h.unique - id
                || self
                    .reader
                    .position
                    .checked_add(packed as u64)
                    .is_none_or(|n| n > self.h.logical)
            {
                return Err(invalid("compressed block header/count/length invalid").into());
            }
            self.reader.read_exact(&mut self.packed[..packed])?;
            if self.codec.run(&self.packed[..packed], &mut self.raw)? != raw {
                return Err(invalid("decompressed length mismatch").into());
            }
            let mut position = 0usize;
            for _ in 0..count {
                if position + 4 > raw {
                    return Err(invalid("truncated dictionary length").into());
                }
                let length = u32_at(&self.raw, position) as usize;
                position += 4;
                let end = position
                    .checked_add(length)
                    .filter(|x| *x <= raw)
                    .ok_or_else(|| invalid("dictionary name boundary invalid"))?;
                let name = &self.raw[position..end];
                if let Some(terms) = terms {
                    if terms.iter().all(|t| t.find(name).is_some()) {
                        self.bits[id as usize / 8] |= 1 << (id as usize % 8);
                    }
                } else {
                    std::str::from_utf8(name)?;
                }
                pool += length as u64;
                position = end;
                id += 1;
            }
            if position != raw {
                return Err(invalid("dictionary block has trailing bytes").into());
            }
        }
        if id != self.h.unique || pool != self.h.pool || self.reader.position != self.h.logical {
            return Err(invalid("dictionary totals/logical end mismatch").into());
        }
        Ok(())
    }
    fn open(snapshot: &Path, index: &Path) -> Result<Self> {
        let mut snapshot = File::open(snapshot)?;
        let size = snapshot.metadata()?.len();
        let count = snapshot_header(&mut snapshot)?;
        if count > size.saturating_sub(16) / 5 {
            return Err(invalid("snapshot count invalid").into());
        }
        let mut reader = DirectReader::open(index)?;
        let h = Header::read(&mut reader, size, count)?;
        let bits = usize::try_from(
            h.unique
                .checked_add(7)
                .ok_or_else(|| invalid("bitset overflow"))?
                / 8,
        )?;
        let mut s = Self {
            reader,
            snapshot,
            h,
            codec: Codec::new(false)?,
            raw: vec![0; RAW],
            packed: vec![0; PACKED],
            table: vec![0; IO],
            bits: vec![0; bits],
        };
        s.reader.seek_to(ALIGN as u64)?;
        let mut remaining = h.entries * 8;
        let mut last = 0;
        let mut first = true;
        while remaining > 0 {
            let n = remaining.min(IO as u64) as usize;
            s.reader.read_exact(&mut s.table[..n])?;
            for record in s.table[..n].chunks_exact(8) {
                check_entry(record, h, &mut last, &mut first)?;
            }
            remaining -= n as u64;
        }
        s.scan_names(None, true)?;
        Ok(s)
    }
    fn query_terms(
        &mut self,
        query: &str,
        shadowed: Option<&[u8]>,
        limit: usize,
        accept: Option<&mut dyn FnMut(&[u16], bool) -> bool>,
    ) -> Result<(u64, Vec<BaseHit>)> {
        self.query_terms_inner(query, shadowed, limit, accept, false)
    }

    /// Search the base with the interactive result limit as an early-stop
    /// condition.  The full query above deliberately keeps scanning to return
    /// an exact total; this variant is used only when the caller omits total.
    fn query_terms_limited(
        &mut self,
        query: &str,
        shadowed: Option<&[u8]>,
        limit: usize,
        accept: Option<&mut dyn FnMut(&[u16], bool) -> bool>,
    ) -> Result<Vec<BaseHit>> {
        if limit == 0 {
            if let Some(bits) = shadowed {
                let required = self
                    .h
                    .entries
                    .checked_add(7)
                    .ok_or_else(|| invalid("shadow bitset length overflow"))?
                    / 8;
                if bits.len() != usize::try_from(required)? {
                    return Err(invalid("shadow bitset does not match paged entry count").into());
                }
            }
            return Ok(Vec::new());
        }
        let (_, hits) = self.query_terms_inner(query, shadowed, limit, accept, true)?;
        Ok(hits)
    }

    fn query_terms_inner(
        &mut self,
        query: &str,
        shadowed: Option<&[u8]>,
        limit: usize,
        mut accept: Option<&mut dyn FnMut(&[u16], bool) -> bool>,
        stop_after_limit: bool,
    ) -> Result<(u64, Vec<BaseHit>)> {
        if let Some(bits) = shadowed {
            let required = self
                .h
                .entries
                .checked_add(7)
                .ok_or_else(|| invalid("shadow bitset length overflow"))?
                / 8;
            let required = usize::try_from(required)?;
            if bits.len() != required {
                return Err(invalid("shadow bitset does not match paged entry count").into());
            }
        }
        let lowercase = query.to_lowercase();
        let terms: Vec<_> = lowercase
            .split_whitespace()
            .map(|x| Finder::new(x.as_bytes()))
            .collect();
        self.bits.fill(0);
        self.scan_names(Some(&terms), stop_after_limit)?;
        let mut total = 0u64;
        let mut selected = Vec::with_capacity(limit);
        let mut filtered = Vec::with_capacity(limit);
        let mut path_buffer = Vec::new();
        self.reader.seek_to(ALIGN as u64)?;
        let mut remaining = self.h.entries * 8;
        let mut ordinal = 0usize;
        let mut records_since_cancel_check = 0usize;
        while remaining > 0 {
            if stop_after_limit {
                crate::cancel::check()?;
            }
            let n = remaining.min(IO as u64) as usize;
            self.reader.read_exact(&mut self.table[..n])?;
            let mut reached_limit = false;
            for record in self.table[..n].chunks_exact(8) {
                if stop_after_limit {
                    if records_since_cancel_check >= 256 {
                        crate::cancel::check()?;
                        records_since_cancel_check = 0;
                    }
                    records_since_cancel_check += 1;
                }
                if let Some(bits) = shadowed {
                    if bits[ordinal / 8] & (1 << (ordinal % 8)) != 0 {
                        ordinal += 1;
                        continue;
                    }
                }
                let id = u32_at(record, 0) as usize;
                if self.bits[id / 8] & (1 << (id % 8)) != 0 {
                    let offset = u32_at(record, 4) as u64;
                    if let Some(predicate) = accept.as_deref_mut() {
                        let hit = read_hit(
                            &mut self.snapshot,
                            self.h.snapshot,
                            offset,
                            &mut path_buffer,
                        )?;
                        if predicate(&hit.path, hit.is_dir) {
                            total += 1;
                            if filtered.len() < limit {
                                filtered.push(hit);
                                reached_limit = stop_after_limit && filtered.len() >= limit;
                            }
                        }
                    } else {
                        total += 1;
                        if selected.len() < limit {
                            selected.push(offset);
                            reached_limit = stop_after_limit && selected.len() >= limit;
                        }
                    }
                }
                ordinal += 1;
                if reached_limit {
                    break;
                }
            }
            if reached_limit {
                break;
            }
            remaining -= n as u64;
        }
        let mut result = filtered;
        for offset in selected {
            result.push(read_hit(
                &mut self.snapshot,
                self.h.snapshot,
                offset,
                &mut path_buffer,
            )?);
        }
        Ok((total, result))
    }

    pub(crate) fn query_base_with_shadow(
        &mut self,
        query: &str,
        shadowed: &[u8],
        limit: usize,
    ) -> Result<(u64, Vec<BaseHit>)> {
        self.query_terms(query, Some(shadowed), limit, None)
    }

    pub(crate) fn query_base_filtered_with_shadow(
        &mut self,
        query: &str,
        shadowed: &[u8],
        limit: usize,
        accept: &mut dyn FnMut(&[u16], bool) -> bool,
    ) -> Result<(u64, Vec<BaseHit>)> {
        self.query_terms(query, Some(shadowed), limit, Some(accept))
    }

    pub(crate) fn query_base_limited_with_shadow(
        &mut self,
        query: &str,
        shadowed: &[u8],
        limit: usize,
    ) -> Result<Vec<BaseHit>> {
        self.query_terms_limited(query, Some(shadowed), limit, None)
    }

    pub(crate) fn query_base_filtered_limited_with_shadow(
        &mut self,
        query: &str,
        shadowed: &[u8],
        limit: usize,
        accept: &mut dyn FnMut(&[u16], bool) -> bool,
    ) -> Result<Vec<BaseHit>> {
        self.query_terms_limited(query, Some(shadowed), limit, Some(accept))
    }

    pub(crate) fn entries(&self) -> usize {
        self.h.entries as usize
    }

    pub(crate) fn index_bytes(&self) -> usize {
        self.reader.length as usize
    }

    pub(crate) fn open_for_live(snapshot: &Path, index: &Path) -> Result<Self> {
        Self::open(snapshot, index)
    }

    fn query(&mut self, input: &Value) -> Result<Value> {
        let started = Instant::now();
        let query = input["query"]
            .as_str()
            .ok_or_else(|| invalid("query must be a string"))?;
        let limit = match input.get("limit") {
            None => 20,
            Some(v) => v
                .as_u64()
                .filter(|v| *v <= 1000)
                .ok_or_else(|| invalid("limit must be an integer from 0 to 1000"))?
                as usize,
        };
        let (total, selected) = self.query_terms(query, None, limit, None)?;
        let mut paths = Vec::with_capacity(selected.len());
        let mut paths_utf16 = Vec::with_capacity(selected.len());
        for hit in selected {
            let (text, bad) = decode_path(&hit.path);
            paths.push(text);
            paths_utf16.push(if bad { Some(hit.path) } else { None });
        }
        Ok(
            json!({"query":query,"total":total,"paths":paths,"paths_utf16":paths_utf16,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0}),
        )
    }
}
pub(super) fn serve(snapshot: &Path, index: &Path) -> Result<()> {
    let started = Instant::now();
    let mut s = Search::open(snapshot, index)?;
    let mut out = io::stdout().lock();
    writeln!(
        out,
        "{}",
        json!({"ready":true,"format":"KFPAGED1","entries":s.h.entries,"distinct_names":s.h.unique,"index_bytes":s.reader.length,"invalid_utf16_paths":s.h.invalid,"load_ms":started.elapsed().as_secs_f64()*1000.0,"query_bitset_bytes":s.bits.len(),"generated_index_io":"FILE_FLAG_NO_BUFFERING","io_alignment":ALIGN,"io_buffer_bytes":IO,"raw_block_bytes":RAW,"algorithm":"XPRESS","resident_scope":"fixed I/O/codec buffers and match bitset; snapshot paths read on demand; hardware caches uncontrolled"})
    )?;
    out.flush()?;
    for line in io::stdin().lock().lines() {
        let response: Result<Value> = serde_json::from_str(&line?)
            .map_err(Into::into)
            .and_then(|v| s.query(&v));
        writeln!(
            out,
            "{}",
            response.unwrap_or_else(|e| json!({"error":e.to_string()}))
        )?;
        out.flush()?;
    }
    Ok(())
}
