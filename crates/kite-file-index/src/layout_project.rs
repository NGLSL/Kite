//! Offline projection of official names-only FILE_LAYOUT frames.
use crate::{invalid, mft, u32_at, u64_at, Result, SNAP_MAGIC};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};
use std::time::Instant;
struct Name {
    parent: u64,
    start: u32,
    len: u32,
    flags: u32,
}
struct Entry {
    id: u64,
    attrs: u32,
    start: u32,
    count: u32,
}
struct Data {
    entries: Vec<Entry>,
    names: Vec<Name>,
    pool: Vec<u16>,
    index: HashMap<u64, usize>,
    frames: u64,
    flags: [u64; 4],
}

// Optional incremental side table: fixed `<BB6xQII>` records in snapshot
// order, kept separate from the existing KFSNAP02 bytes.
struct IdentityWriter {
    output: BufWriter<File>,
    offset: u64,
    ordinal: u64,
    records: u64,
}

impl IdentityWriter {
    fn create(path: &Path) -> Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        Ok(Self {
            output: BufWriter::with_capacity(1024 * 1024, file),
            offset: 16,
            ordinal: 0,
            records: 0,
        })
    }

    fn record(&mut self, volume: u8, is_dir: bool, frn: u64, path: &[u16]) -> Result<()> {
        let bytes = path
            .len()
            .checked_mul(std::mem::size_of::<u16>())
            .ok_or_else(|| invalid("UTF16 path length overflow"))?;
        let record_bytes = 5u64
            .checked_add(u64::try_from(bytes)?)
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
fn add(a: usize, b: usize, end: usize) -> Result<usize> {
    a.checked_add(b)
        .filter(|x| *x <= end)
        .ok_or_else(|| invalid("layout offset outside entry/buffer").into())
}
fn load(path: &Path) -> Result<Data> {
    let mut input = BufReader::new(File::open(path)?);
    let mut magic = [0; 9];
    input.read_exact(&mut magic)?;
    if &magic != b"KFLAYOUT1" {
        return Err(invalid("expected 9-byte KFLAYOUT1").into());
    }
    let mut d = Data {
        entries: Vec::new(),
        names: Vec::new(),
        pool: Vec::new(),
        index: HashMap::new(),
        frames: 0,
        flags: [0; 4],
    };
    loop {
        crate::cancel::check()?;
        let mut h = [0; 4];
        if input.read(&mut h[..1])? == 0 {
            break;
        }
        input.read_exact(&mut h[1..])?;
        let length = u32::from_le_bytes(h) as usize;
        if !(16..=16 * 1024 * 1024).contains(&length) {
            return Err(invalid("layout frame length unsupported").into());
        }
        let mut b = vec![0; length];
        input.read_exact(&mut b)?;
        d.frames += 1;
        let count = u32_at(&b, 0) as usize;
        let mut off = u32_at(&b, 4) as usize;
        if count == 0 || count > (length - 16) / 40 || off < 16 || off % 8 != 0 {
            return Err(invalid("invalid layout header/count").into());
        }
        for i in 0..count {
            add(off, 40, length)?;
            if u32_at(&b, off) != 1 {
                return Err(invalid("unsupported FILE_LAYOUT_ENTRY version").into());
            }
            let next = u32_at(&b, off + 4) as usize;
            let end = if i + 1 == count {
                if next != 0 {
                    return Err(invalid("file chain longer than count").into());
                }
                length
            } else {
                if next < 40 || next % 8 != 0 {
                    return Err(invalid("file overlap/nonprogress").into());
                }
                add(off, next, length)?
            };
            if u32_at(&b, off + 28) != 0 || u32_at(&b, off + 32) != 0 || u32_at(&b, off + 36) != 0 {
                return Err(invalid("unrequested stream/extra entries").into());
            }
            let id = u64_at(&b, off + 16);
            if d.index.insert(id, d.entries.len()).is_some() {
                return Err(invalid("duplicate full FRN in layout").into());
            }
            let start = u32::try_from(d.names.len())?;
            let first = u32_at(&b, off + 24) as usize;
            let mut seen = HashSet::new();
            if first != 0 {
                if first < 40 || first % 8 != 0 {
                    return Err(invalid("invalid name offset").into());
                }
                let mut no = add(off, first, end)?;
                loop {
                    add(no, 24, end)?;
                    let next = u32_at(&b, no) as usize;
                    let flags = u32_at(&b, no + 4);
                    let parent = u64_at(&b, no + 8);
                    let len = u32_at(&b, no + 16) as usize;
                    if flags > 3 || len == 0 || len % 2 != 0 {
                        return Err(invalid("unknown name flags/invalid UTF16 length").into());
                    }
                    let finish = add(no, 24 + len, end)?;
                    let units: Vec<u16> = b[no + 24..finish]
                        .chunks_exact(2)
                        .map(|x| u16::from_le_bytes([x[0], x[1]]))
                        .collect();
                    if units.iter().any(|c| matches!(*c, 0 | 47 | 92)) {
                        return Err(invalid("name contains path separator/NUL").into());
                    }
                    if !seen.insert((parent, units.clone())) {
                        return Err(invalid("duplicate owner parent/name edge").into());
                    }
                    d.flags[flags as usize] += 1;
                    d.names.push(Name {
                        parent,
                        start: u32::try_from(d.pool.len())?,
                        len: u32::try_from(units.len())?,
                        flags,
                    });
                    d.pool.extend(units);
                    if next == 0 {
                        break;
                    }
                    if next < 24 + len || next % 8 != 0 {
                        return Err(invalid("name overlap/nonprogress").into());
                    }
                    no = add(no, next, end)?;
                }
            }
            d.entries.push(Entry {
                id,
                attrs: u32_at(&b, off + 12),
                start,
                count: u32::try_from(d.names.len() - start as usize)?,
            });
            off = end;
        }
    }
    Ok(d)
}
fn names<'a>(d: &'a Data, e: &Entry) -> impl Iterator<Item = usize> + 'a {
    let start = e.start as usize;
    let end = start + e.count as usize;
    (start..end).filter(|&i| d.names[i].flags != 2)
}
fn text<'a>(d: &'a Data, n: &Name) -> &'a [u16] {
    &d.pool[n.start as usize..n.start as usize + n.len as usize]
}
fn write_units_fast(output: &mut impl Write, units: &[u16]) -> Result<()> {
    let bytes = units
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or_else(|| invalid("UTF16 path length overflow"))?;
    output.write_all(&u32::try_from(bytes)?.to_le_bytes())?;
    #[cfg(target_endian = "little")]
    {
        // SAFETY: Windows uses little-endian UTF-16, matching the snapshot
        // encoding; the byte view is valid for the duration of write_all.
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
fn emit_path(
    output: &mut impl Write,
    identities: &mut Option<IdentityWriter>,
    volume: u8,
    frn: u64,
    is_dir: bool,
    path: &[u16],
) -> Result<()> {
    write_path_fast(output, is_dir, path)?;
    if let Some(side) = identities.as_mut() {
        side.record(volume, is_dir, frn, path)?;
    }
    Ok(())
}
fn path(d: &Data, n: usize, root: u64, drive: &[u16]) -> Result<Vec<u16>> {
    let mut chain = vec![n];
    let mut parent = d.names[n].parent;
    while parent != root {
        let e = &d.entries[*d
            .index
            .get(&parent)
            .ok_or_else(|| invalid("parent full FRN missing/sequence changed"))?];
        if e.attrs & 16 == 0 {
            return Err(invalid("parent is not directory").into());
        }
        let mut choices = names(d, e);
        let part = choices
            .next()
            .ok_or_else(|| invalid("directory has no long name"))?;
        if choices.next().is_some() {
            return Err(invalid("multiple directory long-name edges unsupported").into());
        }
        if chain.contains(&part) {
            return Err(invalid("directory cycle").into());
        }
        chain.push(part);
        parent = d.names[part].parent;
    }
    let mut out = drive.to_vec();
    for &i in chain.iter().rev() {
        out.push(92);
        out.extend_from_slice(text(d, &d.names[i]));
    }
    Ok(out)
}
fn cached_path(
    d: &Data,
    n: usize,
    root: u64,
    drive: &[u16],
    directory_paths: &HashMap<u64, Vec<u16>>,
) -> Result<Vec<u16>> {
    let name = &d.names[n];
    let mut out = if name.parent == root {
        drive.to_vec()
    } else if let Some(parent) = directory_paths.get(&name.parent) {
        parent.clone()
    } else {
        return path(d, n, root, drive);
    };
    out.push(92);
    out.extend_from_slice(text(d, name));
    Ok(out)
}
fn zero_evidence(capture: &Path, manifest: &Value) -> Result<HashMap<(String, u64), Value>> {
    let mut result = HashMap::new();
    let Some(relative) = manifest["verified_zero_link_supplement"].as_str() else {
        return Ok(result);
    };
    if manifest["zero_link_supplement_is_separate_observation"] != true {
        return Err(invalid("zero-link supplement needs separate-observation declaration").into());
    }
    for line in BufReader::new(File::open(capture.join(relative))?).lines() {
        let value: Value = serde_json::from_str(&line?)?;
        if value["result"] != "verified-same-FRN-zero-links" {
            continue;
        }
        let volume = value["volume"]
            .as_str()
            .ok_or_else(|| invalid("zero evidence missing volume"))?
            .to_owned();
        let id = value["frn"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| invalid("zero evidence invalid full FRN"))?;
        let info = value["information"]
            .as_array()
            .filter(|a| a.len() == 13)
            .ok_or_else(|| invalid("zero evidence invalid handle information"))?;
        let info = info
            .iter()
            .map(|x| {
                x.as_u64()
                    .filter(|v| *v <= u32::MAX as u64)
                    .ok_or_else(|| invalid("invalid info u32"))
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        if id >> 48 == 0
            || value["verified_frn"] != value["frn"]
            || info[10] != 0
            || ((info[11] << 32) | info[12]) != id
            || value["is_directory"].as_bool() != Some(info[0] & 16 != 0)
            || value.get("error").is_some()
        {
            return Err(invalid("zero evidence identity/type/link validation failed").into());
        }
        if let Some(v) = manifest["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["volume"] == volume)
        {
            if v["volume_serial"].as_u64() != Some(info[7]) {
                return Err(invalid("zero evidence volume serial mismatch").into());
            }
        }
        if result.insert((volume, id), value).is_some() {
            return Err(invalid("duplicate zero-link evidence").into());
        }
    }
    Ok(result)
}
fn ascii_name_eq(units: &[u16], expected: &[u8]) -> bool {
    units.len() == expected.len()
        && units.iter().zip(expected).all(|(&unit, &byte)| {
            let folded = if (b'A' as u16..=b'Z' as u16).contains(&unit) {
                unit + (b'a' - b'A') as u16
            } else {
                unit
            };
            folded == byte as u16
        })
}

fn manifest_u32(value: &Value, label: &str) -> Result<u32> {
    let value = value
        .as_u64()
        .filter(|value| *value <= u32::MAX as u64)
        .ok_or_else(|| invalid(label))?;
    Ok(value as u32)
}

fn manifest_frn(value: &Value, label: &str) -> Result<u64> {
    let value = value
        .as_str()
        .ok_or_else(|| invalid(label))?
        .parse::<u64>()?;
    Ok(value)
}

fn layout_capture_file(capture: &Path, value: &Value) -> Result<std::path::PathBuf> {
    let name = value
        .as_str()
        .ok_or_else(|| invalid("layout file must be a normal filename"))?;
    let path = Path::new(name);
    if name.is_empty()
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(invalid("layout file must be a normal filename").into());
    }
    let capture = capture.canonicalize()?;
    let file = capture.join(path).canonicalize()?;
    if !file.starts_with(&capture) {
        return Err(invalid("layout file escapes capture").into());
    }
    Ok(file)
}

fn replace_manifest(path: &Path, manifest: &Value) -> Result<()> {
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(".verify-zero-link.tmp");
    let temporary = Path::new(&temporary);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    serde_json::to_writer_pretty(&mut output, manifest)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    drop(output);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
        }
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                0x0000_0001 | 0x0000_0008,
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            let _ = std::fs::remove_file(temporary);
            return Err(error.into());
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(temporary, path)?;
    Ok(())
}

/// Verify the exact root -> `$Extend` -> `$Deleted` candidate set through
/// `OpenFileById`, without reparsing the layout in a second process.  The
/// resulting JSONL and manifest fields intentionally match the historical
/// Python helper so `project` consumes the same evidence contract.
pub fn verify_zero_links(capture: &Path) -> Result<()> {
    let started = Instant::now();
    let manifest_path = capture.join("manifest.json");
    let mut manifest: Value = serde_json::from_reader(File::open(&manifest_path)?)?;
    if manifest["format"] != "KFLAYOUT1"
        || manifest["complete"] != true
        || manifest["content_streams_requested"] != false
        || manifest["raw_MFT"] != false
    {
        return Err(invalid("requires complete names-only non-raw-MFT capture").into());
    }
    let volumes = manifest["volumes"]
        .as_array()
        .filter(|volumes| !volumes.is_empty())
        .ok_or_else(|| invalid("manifest has no volumes"))?;
    let mut seen = HashSet::new();
    let mut plans = Vec::with_capacity(volumes.len());
    for volume in volumes {
        let drive = volume["volume"]
            .as_str()
            .ok_or_else(|| invalid("manifest volume missing drive"))?;
        if drive.len() != 2
            || !drive.as_bytes()[0].is_ascii_uppercase()
            || drive.as_bytes()[1] != b':'
            || !seen.insert(drive.to_owned())
        {
            return Err(invalid("manifest volume must be a unique uppercase drive").into());
        }
        if volume["complete"] != true || volume["eof_win32"] != 38 {
            return Err(format!("{drive}: incomplete volume capture").into());
        }
        let serial = manifest_u32(&volume["volume_serial"], "invalid volume serial")?;
        let root = manifest_frn(&volume["root_frn"], "invalid root FRN")?;
        if root >> 48 == 0 {
            return Err(format!("{drive}: root FRN has no sequence").into());
        }
        let information = volume["root_information"]
            .as_array()
            .filter(|values| values.len() == 13)
            .ok_or_else(|| invalid("root_information must have 13 values"))?;
        let information = information
            .iter()
            .map(|value| manifest_u32(value, "invalid root_information value"))
            .collect::<Result<Vec<_>>>()?;
        if (((information[11] as u64) << 32) | information[12] as u64) != root
            || information[7] != serial
            || information[0] & 16 == 0
        {
            return Err(format!("{drive}: root information mismatch").into());
        }
        let expected = ["frames", "file_entries", "name_entries"]
            .into_iter()
            .map(|field| {
                volume[field]
                    .as_u64()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| invalid("invalid layout counts"))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let file = layout_capture_file(capture, &volume["file"])?;
        let data = load(&file)?;
        let actual = [
            data.frames,
            u64::try_from(data.entries.len())?,
            u64::try_from(data.names.len())?,
        ];
        if actual != expected.as_slice() {
            return Err(format!(
                "{drive}: manifest counts {:?} != layout counts {:?}",
                expected, actual
            )
            .into());
        }
        let root_entry = data
            .index
            .get(&root)
            .copied()
            .ok_or_else(|| invalid("root absent"))?;
        if data.entries[root_entry].attrs & 16 == 0 {
            return Err(format!("{drive}: root absent/not directory").into());
        }
        let mut extend = HashSet::new();
        let mut deleted_edges = Vec::new();
        for entry in &data.entries {
            for name_index in names(&data, entry) {
                let name = &data.names[name_index];
                let units = text(&data, name);
                if name.parent == root && entry.attrs & 16 != 0 && ascii_name_eq(units, b"$extend")
                {
                    extend.insert(entry.id);
                }
                if ascii_name_eq(units, b"$deleted") {
                    deleted_edges.push((entry.id, name.parent, entry.attrs));
                }
            }
        }
        let deleted: HashSet<u64> = deleted_edges
            .into_iter()
            .filter(|(_, parent, attrs)| {
                extend.contains(parent) && attrs & 16 != 0 && *parent >> 48 != 0
            })
            .map(|(id, _, _)| id)
            .collect();
        let mut candidates = BTreeMap::new();
        for entry in &data.entries {
            for name_index in names(&data, entry) {
                let name = &data.names[name_index];
                if deleted.contains(&name.parent) && entry.id >> 48 != 0 && name.parent >> 48 != 0 {
                    candidates.insert(entry.id, entry.attrs);
                }
            }
        }
        plans.push((drive.to_owned(), serial, candidates));
    }

    let proof_path = capture.join("zero-link-proof.jsonl");
    let mut output = BufWriter::with_capacity(
        1024 * 1024,
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&proof_path)?,
    );
    let total: u64 = plans
        .iter()
        .map(|(_, _, candidates)| candidates.len() as u64)
        .sum();
    let mut verified = 0u64;
    let mut zero = 0u64;
    let mut positive = 0u64;
    let mut failed = 0u64;
    let mut volume_stats = Vec::with_capacity(plans.len());
    for (drive, serial, candidates) in &plans {
        let mut local_verified = 0u64;
        let mut local_zero = 0u64;
        let mut local_positive = 0u64;
        let mut local_failed = 0u64;
        for (&frn, &attrs) in candidates {
            let expected_is_dir = attrs & 16 != 0;
            match mft::verify_metadata_id(drive, frn, *serial, expected_is_dir) {
                Ok(information) => {
                    let observed = ((information[11] as u64) << 32) | information[12] as u64;
                    let actual_is_dir = information[0] & 16 != 0;
                    if observed != frn || actual_is_dir != expected_is_dir {
                        failed += 1;
                        local_failed += 1;
                        serde_json::to_writer(
                            &mut output,
                            &json!({"volume":drive,"frn":frn.to_string(),"result":"failed-identity-or-information","error":"metadata identity/type mismatch"}),
                        )?;
                        output.write_all(b"\n")?;
                        continue;
                    }
                    let is_zero = information[10] == 0;
                    serde_json::to_writer(
                        &mut output,
                        &json!({"volume":drive,"frn":frn.to_string(),"verified_frn":observed.to_string(),"information":information,"is_directory":actual_is_dir,"nNumberOfLinks":information[10],"result":if is_zero {"verified-same-FRN-zero-links"} else {"verified-same-FRN-positive-links"}}),
                    )?;
                    output.write_all(b"\n")?;
                    verified += 1;
                    local_verified += 1;
                    if is_zero {
                        zero += 1;
                        local_zero += 1;
                    } else {
                        positive += 1;
                        local_positive += 1;
                    }
                }
                Err(error) => {
                    failed += 1;
                    local_failed += 1;
                    serde_json::to_writer(
                        &mut output,
                        &json!({"volume":drive,"frn":frn.to_string(),"result":"failed-identity-or-information","error":error.to_string()}),
                    )?;
                    output.write_all(b"\n")?;
                }
            }
        }
        volume_stats.push(json!({
            "volume": drive,
            "candidate_count": candidates.len(),
            "attempted": candidates.len(),
            "identity_verified": local_verified,
            "zero_links": local_zero,
            "positive_links": local_positive,
            "failed": local_failed,
        }));
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    if failed != 0 {
        return Err(
            format!("metadata verification failed for {failed} candidate operation(s)").into(),
        );
    }
    manifest["verified_zero_link_supplement"] = json!("zero-link-proof.jsonl");
    manifest["zero_link_supplement_is_separate_observation"] = json!(true);
    manifest["supplement_ms"] = json!(started.elapsed().as_secs_f64() * 1000.0);
    manifest["zero_link_supplement_stats"] = json!({
        "candidate_count": total,
        "attempted": total,
        "identity_verified": verified,
        "zero_links": zero,
        "positive_links": positive,
        "failed": failed,
        "volumes": volume_stats,
    });
    replace_manifest(&manifest_path, &manifest)?;
    Ok(())
}

fn project(
    capture: &Path,
    snapshot: &Path,
    report: &mut Value,
    identities_raw: Option<&Path>,
) -> Result<()> {
    let m: Value = serde_json::from_reader(File::open(capture.join("manifest.json"))?)?;
    if m["format"] != "KFLAYOUT1"
        || m["complete"] != true
        || m["content_streams_requested"] != false
    {
        return Err(invalid("requires explicit complete names-only KFLAYOUT1 scope").into());
    }
    let file_search_scope = match m.get("file_search_scope") {
        None => false,
        Some(value) if value == mft::FILE_SEARCH_SCOPE => true,
        _ => return Err(invalid("unsupported file_search_scope").into()),
    };
    if file_search_scope {
        report["file_search_scope"] = json!(mft::FILE_SEARCH_SCOPE);
    }
    let volumes = m["volumes"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid("no volumes"))?;
    let mut zeros = zero_evidence(capture, &m)?;
    report["zero_link_supplement_is_separate_observation"] =
        json!(m.get("verified_zero_link_supplement").is_some());
    report["source_scope"] = m["scope"].clone();
    let mut out = BufWriter::with_capacity(
        1024 * 1024,
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(snapshot)?,
    );
    let mut identities = identities_raw.map(IdentityWriter::create).transpose()?;
    if let Some(path) = identities_raw {
        report["identity_side_table_path"] = json!(path.to_string_lossy());
    }
    out.write_all(SNAP_MAGIC)?;
    out.write_all(&0u64.to_le_bytes())?;
    let mut total = 0u64;
    let mut seen = HashSet::new();
    for volume in volumes {
        let started = Instant::now();
        let drive = volume["volume"]
            .as_str()
            .ok_or_else(|| invalid("missing volume"))?;
        if drive.len() != 2
            || !drive.as_bytes()[0].is_ascii_uppercase()
            || drive.as_bytes()[1] != b':'
            || !seen.insert(drive)
        {
            return Err(invalid("invalid/duplicate volume").into());
        }
        if volume["complete"] != true || volume["eof_win32"] != 38 {
            return Err(invalid("volume did not finish at EOF38").into());
        }
        let file = Path::new(
            volume["file"]
                .as_str()
                .ok_or_else(|| invalid("missing file"))?,
        );
        if file
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(invalid("layout filename outside capture").into());
        }
        let file = capture.join(file).canonicalize()?;
        if !file.starts_with(capture.canonicalize()?) {
            return Err(invalid("layout resolves outside capture").into());
        }
        let d = load(&file)?;
        for (field, actual) in [
            ("frames", d.frames),
            ("file_entries", u64::try_from(d.entries.len())?),
            ("name_entries", u64::try_from(d.names.len())?),
        ] {
            let expected = volume[field]
                .as_u64()
                .ok_or_else(|| invalid("complete manifest count must be a u64"))?;
            if actual != expected {
                return Err(format!(
                    "{drive} {field}: manifest expects {expected}, layout contains {actual}"
                )
                .into());
            }
        }
        let load_ms = started.elapsed().as_secs_f64() * 1000.0;
        mft::sample_memory(report)?;
        let root = volume["root_frn"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| invalid("invalid root FRN"))?;
        let ri = d.index.get(&root).ok_or_else(|| invalid("root absent"))?;
        if d.entries[*ri].attrs & 16 == 0 {
            return Err(invalid("root not directory").into());
        }
        let stage = Instant::now();
        let prefix: Vec<u16> = drive.encode_utf16().collect();
        let volume_byte = drive
            .as_bytes()
            .first()
            .copied()
            .ok_or_else(|| invalid("missing volume letter"))?;
        let mut directory_paths = HashMap::with_capacity(
            d.entries
                .iter()
                .filter(|entry| entry.attrs & 16 != 0)
                .count(),
        );
        directory_paths.insert(root, prefix.clone());
        for entry in &d.entries {
            if entry.id == root || entry.attrs & 16 == 0 {
                continue;
            }
            let mut choices = names(&d, entry);
            let Some(name_index) = choices.next() else {
                continue;
            };
            if choices.next().is_some() {
                // Keep the original ambiguity error on the path that uses
                // this directory; do not cache an arbitrary parent name.
                continue;
            }
            if let Ok(path) = cached_path(&d, name_index, root, &prefix, &directory_paths) {
                directory_paths.insert(entry.id, path);
            }
        }
        emit_path(&mut out, &mut identities, volume_byte, root, true, &prefix)?;
        let mut count = 1u64;
        let mut folders = 1u64;
        let mut files = 0u64;
        let mut reserved_objects = 0u64;
        let mut reserved_names = 0u64;
        let mut reserved_raw_names = 0u64;
        let mut excluded = 0u64;
        let mut excluded_names = 0u64;
        let mut unnamed = Vec::new();
        let mut invalid_paths = 0u64;
        for (entry_number, e) in d.entries.iter().enumerate() {
            if entry_number & 0x3fff == 0 {
                crate::cancel::check()?;
            }
            if e.id == root {
                continue;
            }
            if file_search_scope && mft::reserved_metadata_file(e.id, e.attrs & 16 != 0) {
                // Source inventory was already checked above; scope does not change it.
                reserved_objects += 1;
                reserved_names += names(&d, e).count() as u64;
                reserved_raw_names += u64::from(e.count);
                if let Some(proof) = zeros.remove(&(drive.to_owned(), e.id)) {
                    if proof["is_directory"].as_bool() != Some(false) {
                        return Err(invalid("zero supplement/layout type mismatch").into());
                    }
                }
                continue;
            }
            if let Some(proof) = zeros.remove(&(drive.to_owned(), e.id)) {
                if proof["is_directory"].as_bool() != Some(e.attrs & 16 != 0) {
                    return Err(invalid("zero supplement/layout type mismatch").into());
                }
                excluded += 1;
                excluded_names += names(&d, e).count() as u64;
                continue;
            }
            if e.count == 0 {
                let ordinal = e.id & 0x0000_ffff_ffff_ffff;
                let known =
                    (12..=15).contains(&ordinal) && e.id >> 48 == ordinal && e.attrs & 16 == 0;
                unnamed.push(json!({"frn":e.id.to_string(),"known_unnamed_NTFS_slot_12_15":known}));
                if !known {
                    return Err(invalid(
                        "unknown zero-name owner: cannot establish named-path completeness",
                    )
                    .into());
                }
                continue;
            }
            if names(&d, e).next().is_none() {
                return Err(invalid("owner has only DOS names").into());
            }
            for n in names(&d, e) {
                let p = cached_path(&d, n, root, &prefix, &directory_paths)
                    .map_err(|error| format!("{drive} FRN {}: {error}", e.id))?;
                if String::from_utf16(&p).is_err() {
                    invalid_paths += 1;
                }
                emit_path(
                    &mut out,
                    &mut identities,
                    volume_byte,
                    e.id,
                    e.attrs & 16 != 0,
                    &p,
                )?;
                count += 1;
                if e.attrs & 16 != 0 {
                    folders += 1;
                } else {
                    files += 1;
                }
            }
        }
        out.flush()?;
        total += count;
        report["entries"] = json!(total);
        report["volumes"].as_array_mut().unwrap().push(json!({"volume":drive,"entries":count,"files":files,"folders":folders,"frames":d.frames,"source_file_entries":d.entries.len(),"source_name_flags_counts":d.flags,"DOS_only_aliases_not_projected":d.flags[2],"excluded_reserved_file_objects":reserved_objects,"excluded_reserved_long_names":reserved_names,"excluded_reserved_raw_names_including_DOS":reserved_raw_names,"excluded_verified_zero_link_objects":excluded,"excluded_verified_zero_link_names":excluded_names,"unnamed_objects":unnamed,"invalid_utf16_paths":invalid_paths,"load_parse_ms":load_ms,"path_write_ms":stage.elapsed().as_secs_f64()*1000.0,"status":"complete-returned-name-closure"}));
        if file_search_scope {
            for (field, value) in [
                ("excluded_reserved_file_objects", reserved_objects),
                ("excluded_reserved_long_names", reserved_names),
                (
                    "excluded_reserved_raw_names_including_DOS",
                    reserved_raw_names,
                ),
            ] {
                report[field] = json!(report[field].as_u64().unwrap_or(0) + value);
            }
        }
        mft::sample_memory(report)?;
    }
    out.flush()?;
    let mut file = out.into_inner()?;
    file.seek(SeekFrom::Start(8))?;
    file.write_all(&total.to_le_bytes())?;
    file.sync_all()?;
    if let Some(side) = identities {
        side.finish(total)?;
        report["identity_side_table_records"] = json!(total);
        report["identity_side_table_record_bytes"] = json!(24u64);
    }
    report["unused_zero_link_evidence"] = json!(zeros.len());
    Ok(())
}
pub fn run(capture: &Path, snapshot: &Path) -> Result<()> {
    let started = Instant::now();
    let mut report = json!({"mode":"project-file-layout","status":"running","entries":0,"volumes":[],"source_contract":"all API-returned non-DOS names with full parent closure; flags0/1/3 retained, flags2 omitted; known unnamed slots reported; explicit verified-zero-link evidence only","coverage_claim":"returned-name closure within selected completed scope, not atomic snapshot or independent full-hardlink proof","private_commit_bytes":0,"peak_working_set_bytes":0});
    let result = project(capture, snapshot, &mut report, None);
    report["project_ms"] = json!(started.elapsed().as_secs_f64() * 1000.0);
    match &result {
        Ok(()) => report["status"] = json!("complete-returned-name-closure"),
        Err(error) => {
            report["status"] = json!("partial");
            report["error"] = json!(error.to_string());
        }
    }
    let mut sidecar = snapshot.as_os_str().to_os_string();
    sidecar.push(".json");
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(Path::new(&sidecar))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    crate::diagnostic!("{report}");
    result
}

pub fn run_mapped(capture: &Path, snapshot: &Path, identities_raw: &Path) -> Result<()> {
    let started = Instant::now();
    let mut report = json!({"mode":"project-file-layout-mapped","status":"running","entries":0,"volumes":[],"source_contract":"all API-returned non-DOS names with full parent closure; flags0/1/3 retained, flags2 omitted; known unnamed slots reported; explicit verified-zero-link evidence only","coverage_claim":"returned-name closure within selected completed scope, not atomic snapshot or independent full-hardlink proof","private_commit_bytes":0,"peak_working_set_bytes":0,"identity_side_table":"24-byte records in snapshot order; no header"});
    let result = project(capture, snapshot, &mut report, Some(identities_raw));
    report["project_ms"] = json!(started.elapsed().as_secs_f64() * 1000.0);
    match &result {
        Ok(()) => report["status"] = json!("complete-returned-name-closure"),
        Err(error) => {
            report["status"] = json!("partial");
            report["error"] = json!(error.to_string());
        }
    }
    let mut sidecar = snapshot.as_os_str().to_os_string();
    sidecar.push(".json");
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(Path::new(&sidecar))?;
    serde_json::to_writer_pretty(&mut f, &report)?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    crate::diagnostic!("{report}");
    result
}
