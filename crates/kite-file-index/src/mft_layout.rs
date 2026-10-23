//! Read-only names-only FSCTL_QUERY_FILE_LAYOUT capture, not a raw MFT image.
use super::*;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::BufWriter;
use std::time::Instant;
const BUFFER_BYTES: usize = 8 * 1024 * 1024;
const FSCTL_QUERY_FILE_LAYOUT: u32 = 0x0009_0277;
fn new_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}
fn drive(s: &str) -> Result<String> {
    let s = s.trim_end_matches('\\').to_ascii_uppercase();
    if s.len() != 2 || !s.as_bytes()[0].is_ascii_alphabetic() || s.as_bytes()[1] != b':' {
        return Err(invalid("volume must be C: or C:\\").into());
    }
    Ok(s)
}
#[derive(Default)]
struct Batch {
    files: u64,
    names: u64,
    file_flags: u32,
    name_flags: u32,
    output_flags: u32,
}
fn validate(bytes: &[u8]) -> Result<Batch> {
    if bytes.len() < 16 {
        return Err(invalid("layout output shorter than 16-byte header").into());
    }
    let count = u32_at(bytes, 0) as usize;
    let mut file = u32_at(bytes, 4) as usize;
    if count == 0 || count > (bytes.len() - 16) / 40 || file < 16 || file % 8 != 0 {
        return Err(invalid("empty/no-progress layout or invalid file count/first offset").into());
    }
    let mut stats = Batch {
        output_flags: u32_at(bytes, 8),
        ..Batch::default()
    };
    for index in 0..count {
        if file.checked_add(40).is_none_or(|end| end > bytes.len()) {
            return Err(invalid("FILE_LAYOUT_ENTRY out of bounds").into());
        }
        if u32_at(bytes, file) != 1 {
            return Err(invalid("unsupported FILE_LAYOUT_ENTRY version (requires 1)").into());
        }
        let next = u32_at(bytes, file + 4) as usize;
        let end = if index + 1 == count {
            if next != 0 {
                return Err(invalid("layout file count ends before file chain").into());
            }
            bytes.len()
        } else {
            if next < 40
                || next % 8 != 0
                || file.checked_add(next).is_none_or(|end| end > bytes.len())
            {
                return Err(invalid("invalid overlapping/non-progressing NextFileOffset").into());
            }
            file + next
        };
        stats.files += 1;
        stats.file_flags |= u32_at(bytes, file + 8);
        // No stream/extra-info inclusion bits were requested. Unexpected shapes stay unsupported.
        if u32_at(bytes, file + 28) != 0
            || u32_at(bytes, file + 32) != 0
            || u32_at(bytes, file + 36) != 0
        {
            return Err(invalid("unrequested stream/extra-info output").into());
        }
        let first = u32_at(bytes, file + 24) as usize;
        if first != 0 {
            if first < 40 || first % 8 != 0 || file.checked_add(first).is_none_or(|n| n >= end) {
                return Err(invalid("invalid FirstNameOffset").into());
            }
            let mut name = file + first;
            loop {
                if name.checked_add(24).is_none_or(|n| n > end) {
                    return Err(invalid("NAME entry header crosses FILE boundary").into());
                }
                let length = u32_at(bytes, name + 16) as usize;
                let next = u32_at(bytes, name) as usize;
                if length == 0
                    || length % 2 != 0
                    || name.checked_add(24 + length).is_none_or(|n| n > end)
                {
                    return Err(invalid("NAME UTF16 byte length/bounds invalid").into());
                }
                stats.names += 1;
                stats.name_flags |= u32_at(bytes, name + 4);
                if next == 0 {
                    break;
                }
                if next < 24 + length
                    || next % 8 != 0
                    || name.checked_add(next).is_none_or(|n| n >= end)
                {
                    return Err(invalid("overlapping/non-progressing NextNameOffset").into());
                }
                name += next;
            }
        }
        file = end;
    }
    Ok(stats)
}
fn volume(root: &str, output: &Path, state: &mut Value, report: &mut Value) -> Result<()> {
    let started = Instant::now();
    let mut artifact: Option<BufWriter<File>> = None;
    let filename = format!("{}.layout", &root[..1]);
    let mut result = (|| -> Result<()> {
        let directory = open(&format!(r"{root}\"), 0, 0x0200_0000)?;
        let information = info(&directory)?;
        if information[0] & 0x10 == 0 {
            return Err(invalid("root metadata handle is not a directory").into());
        }
        state["root_frn"] = json!(info_frn(&information).to_string());
        state["volume_serial"] = json!(information[7]);
        state["root_information"] = json!(information);
        drop(directory);
        // Backup privilege and backup-open intent are separate inputs to CreateFileW.
        // Keep the previous flags=0 diagnosis intact as the control observation.
        state["volume_open_flags"] = json!(0x0200_0000u32);
        state["volume_open_access"] = json!(0x8000_0000u32);
        let handle = open(&format!(r"\\.\{root}"), 0x8000_0000, 0x0200_0000)?;
        let mut words = vec![0u64; BUFFER_BYTES / 8];
        // METHOD_NEITHER pointers are backed by u64 allocations for eight-byte alignment.
        let buffer = unsafe {
            std::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), BUFFER_BYTES)
        };
        let mut previous = Vec::<u8>::new();
        let mut seen_first = BTreeSet::new();
        let mut first = true;
        loop {
            let flags = if first { 3u32 } else { 2u32 };
            let input = [(flags as u64) << 32, 0, 0, 0];
            let input_bytes =
                unsafe { std::slice::from_raw_parts(input.as_ptr().cast::<u8>(), 32) };
            state["api_calls"] = json!(state["api_calls"].as_u64().unwrap() + 1);
            let begin = Instant::now();
            let call = ioctl(&handle, FSCTL_QUERY_FILE_LAYOUT, input_bytes, buffer);
            state["native_ms"] = json!(
                state["native_ms"].as_f64().unwrap() + begin.elapsed().as_secs_f64() * 1000.0
            );
            let returned = match call {
                Ok(n) => n,
                Err(error) if error.raw_os_error() == Some(38) => {
                    state["eof_win32"] = json!(38);
                    state["complete"] = json!(true);
                    break;
                }
                Err(error) => {
                    state["win32_error"] = json!(error.raw_os_error());
                    state["failed_request_flags"] = json!(flags);
                    return Err(error.into());
                }
            };
            let bytes = &buffer[..returned];
            if artifact.is_none() {
                let mut file =
                    BufWriter::with_capacity(1024 * 1024, new_file(&output.join(&filename))?);
                file.write_all(b"KFLAYOUT1")?;
                artifact = Some(file);
                state["file"] = json!(filename);
            }
            let file = artifact.as_mut().unwrap();
            file.write_all(&u32::try_from(returned)?.to_le_bytes())?;
            file.write_all(bytes)?;
            state["data_bytes"] = json!(state["data_bytes"].as_u64().unwrap() + returned as u64);
            state["frames"] = json!(state["frames"].as_u64().unwrap() + 1);
            let batch = validate(bytes)?;
            if previous == bytes {
                return Err(invalid("layout repeated identical batch: no progress").into());
            }
            let first_file = u32_at(bytes, 4) as usize;
            if !seen_first.insert(u64_at(bytes, first_file + 16)) {
                return Err(invalid(
                    "layout repeated first-file identity across batches: progress unverified",
                )
                .into());
            }
            previous.clear();
            previous.extend_from_slice(bytes);
            state["file_entries"] = json!(state["file_entries"].as_u64().unwrap() + batch.files);
            state["name_entries"] = json!(state["name_entries"].as_u64().unwrap() + batch.names);
            state["observed_output_flags_or"] = json!(
                state["observed_output_flags_or"].as_u64().unwrap() | batch.output_flags as u64
            );
            state["observed_file_flags_or"] =
                json!(state["observed_file_flags_or"].as_u64().unwrap() | batch.file_flags as u64);
            state["observed_name_flags_or"] =
                json!(state["observed_name_flags_or"].as_u64().unwrap() | batch.name_flags as u64);
            first = false;
            sample_memory(report)?;
        }
        Ok(())
    })();
    // Preserve and sync a partial frame artifact on validation/API failure as well.
    if let Some(mut file) = artifact {
        let synced = (|| -> Result<()> {
            file.flush()?;
            file.into_inner()?.sync_all()?;
            state["artifact_bytes"] = json!(fs::metadata(output.join(&filename))?.len());
            Ok(())
        })();
        if let Err(error) = synced {
            state["artifact_sync_error"] = json!(error.to_string());
            if result.is_ok() {
                result = Err(error);
            }
        }
    }
    state["capture_ms"] = json!(started.elapsed().as_secs_f64() * 1000.0);
    if let Err(error) = &result {
        state["complete"] = json!(false);
        state["error"] = json!(error.to_string());
        state["status"] = json!("error");
    } else {
        state["status"] = json!("complete");
    }
    result
}
pub fn run(output: &Path, roots: &[String]) -> Result<()> {
    if roots.is_empty() || roots.len() > 26 {
        return Err(invalid("provide 1..26 volumes").into());
    }
    let roots = roots.iter().map(|s| drive(s)).collect::<Result<Vec<_>>>()?;
    if roots.iter().collect::<BTreeSet<_>>().len() != roots.len() {
        return Err(invalid("duplicate volume").into());
    }
    fs::create_dir(output)?;
    let mut manifest = new_file(&output.join("manifest.json"))?;
    let started = Instant::now();
    let mut report = json!({"format":"KFLAYOUT1","magic_bytes":9,"frame_length_bytes":4,"purpose":"metadata-name-enumeration","representation":"filesystem-query-file-layout-framed","complete":false,"status":"running","volumes":[],"fsctl":FSCTL_QUERY_FILE_LAYOUT,"input_bytes":32,"filter_type":0,"filter_entry_count":0,"first_request_flags":3,"continuation_flags":2,"buffer_bytes":BUFFER_BYTES,"file_entry_version":1,"content_streams_requested":false,"raw_MFT":false,"source_writes":false,"USN_mutations":false,"cache_state":"uncontrolled; no cold-cache claim","private_commit_bytes":0,"peak_working_set_bytes":0});
    let _guard = match backup_privilege() {
        Ok(g) => {
            report["backup_privilege_enabled"] = json!(true);
            Some(g)
        }
        Err(error) => {
            report["backup_privilege_enabled"] = json!(false);
            report["backup_privilege_error"] = json!(error.to_string());
            None
        }
    };
    let mut failures = 0;
    for root in roots {
        let mut state = json!({"volume":root,"complete":false,"status":"running","api_calls":0,"native_ms":0.0,"data_bytes":0,"file_entries":0,"name_entries":0,"frames":0,"observed_output_flags_or":0,"observed_file_flags_or":0,"observed_name_flags_or":0});
        if volume(&root, output, &mut state, &mut report).is_err() {
            failures += 1;
        }
        report["volumes"].as_array_mut().unwrap().push(state);
    }
    report["complete"] = json!(failures == 0);
    report["status"] = json!(if failures == 0 { "complete" } else { "error" });
    report["failed_volumes"] = json!(failures);
    if let Err(error) = sample_memory(&mut report) {
        report["memory_error"] = json!(error.to_string());
    }
    report["capture_ms"] = json!(started.elapsed().as_secs_f64() * 1000.0);
    serde_json::to_writer_pretty(&mut manifest, &report)?;
    manifest.write_all(b"\n")?;
    manifest.sync_all()?;
    crate::diagnostic!("{report}");
    if failures > 0 {
        return Err("one or more QUERY_FILE_LAYOUT volumes failed; see manifest.json".into());
    }
    Ok(())
}
