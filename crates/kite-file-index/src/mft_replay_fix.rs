//! Bounded interpretation of already-captured exceptional records, never raw-volume I/O.
use super::*;

pub(super) enum Repair {
    Unlinked(Value),
    Disappeared(Value),
    ResidentNames(Vec<LinkName>, Value),
}
struct Failure {
    detail: Value,
    raw: Option<Vec<u8>>,
    source_line: usize,
}
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
fn parse_record_names(
    raw: &[u8],
    requested: u64,
    is_dir: bool,
    complete: bool,
    strict_layout: bool,
    want_proof: bool,
) -> Result<(Vec<LinkName>, Option<Value>)> {
    if raw.len() < 60 || u64_at(raw, 0) != (requested & 0x0000_ffff_ffff_ffff) {
        return Err(invalid("raw record ordinal mismatch").into());
    }
    let length = u32_at(raw, 8) as usize;
    if length < 48 || length > raw.len() - 12 {
        return Err(invalid("raw record length out of bounds").into());
    }
    let allocated_record = &raw[12..12 + length];
    let used = u32_at(allocated_record, 24) as usize;
    if strict_layout
        && (used < 48
            || used > length
            || used % 8 != 0
            || u32_at(allocated_record, 28) as usize != length)
    {
        return Err(invalid("invalid exact-record used/allocated lengths").into());
    }
    let record = if strict_layout {
        &allocated_record[..used]
    } else {
        allocated_record
    };
    if &record[..4] != b"FILE"
        || u16_at(record, 16) as u64 != requested >> 48
        || u16_at(record, 22) & 3 != if is_dir { 3 } else { 1 }
        || u64_at(record, 32) != 0
    {
        return Err(invalid(
            "raw resident-name repair requires exact live base record and matching type",
        )
        .into());
    }
    let mut offset = u16_at(record, 20) as usize;
    if offset < 48 || offset % 8 != 0 {
        return Err(invalid("invalid first attribute offset").into());
    }
    let mut names = Vec::new();
    let mut filename_attributes = 0u16;
    let mut dos_short_names = 0;
    let mut has_attribute_list = false;
    loop {
        if offset + 4 > record.len() {
            return Err(invalid("raw attribute list lacks end marker").into());
        }
        let kind = u32_at(record, offset);
        if kind == 0xffff_ffff {
            break;
        }
        if offset + 24 > record.len() {
            return Err(invalid("truncated raw attribute header").into());
        }
        let length = u32_at(record, offset + 4) as usize;
        if length < 24
            || length % 8 != 0
            || offset
                .checked_add(length)
                .is_none_or(|end| end > record.len())
        {
            return Err(invalid("raw attribute length out of bounds").into());
        }
        let form = record[offset + 8];
        if strict_layout && (form > 1 || (form == 1 && length < 64)) {
            return Err(invalid("unsupported attribute form/header length").into());
        }
        if strict_layout && form == 0 {
            let start = u16_at(record, offset + 20) as usize;
            let size = u32_at(record, offset + 16) as usize;
            if start < 24 || start.checked_add(size).is_none_or(|end| end > length) {
                return Err(invalid("resident attribute value bounds invalid").into());
            }
        }
        if kind == 0x20 {
            has_attribute_list = true;
            if complete {
                return Err(invalid(
                    "ATTRIBUTE_LIST requires extension records; bounded repair refuses it",
                )
                .into());
            }
        }
        if kind == 0x30 {
            if record[offset + 8] != 0 {
                return Err(invalid("nonresident FILE_NAME unsupported").into());
            }
            let value_offset = u16_at(record, offset + 20) as usize;
            let value_length = u32_at(record, offset + 16) as usize;
            if value_offset < 24
                || value_offset
                    .checked_add(value_length)
                    .is_none_or(|end| end > length)
                || value_length < 66
            {
                return Err(invalid("resident FILE_NAME value out of bounds").into());
            }
            let value = &record[offset + value_offset..offset + value_offset + value_length];
            let count = value[64] as usize;
            let namespace = value[65];
            if count == 0 || namespace > 3 || 66 + count * 2 > value.len() {
                return Err(invalid("invalid resident FILE_NAME").into());
            }
            filename_attributes = filename_attributes
                .checked_add(1)
                .ok_or_else(|| invalid("too many FILE_NAME attributes"))?;
            // Namespace 2 is the DOS 8.3 alias of its long name, not another directory entry.
            if namespace == 2 {
                dos_short_names += 1;
            } else {
                let name: Vec<_> = value[66..66 + count * 2]
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                if name.iter().any(|&c| matches!(c, 0 | 47 | 92)) {
                    return Err(invalid("invalid resident basename").into());
                }
                names.push(LinkName {
                    parent: u64_at(value, 0),
                    name,
                });
            }
        }
        offset += length;
    }
    // Additional consistency check for this captured NTFS 3.x base-record layout.
    // Completeness also requires an exhaustive attribute walk and no ATTRIBUTE_LIST.
    let header_name_count = u16_at(record, 18);
    if complete
        && (names.is_empty()
            || filename_attributes != header_name_count
            || (is_dir && names.len() != 1))
    {
        return Err(invalid("resident filename set/header count mismatch").into());
    }
    let proof = want_proof.then(|| {
        json!({"source":"exact-base-record-resident-FILE_NAME","record_ordinal":requested&0x0000_ffff_ffff_ffff,"record_sequence":requested>>48,"in_use":true,"is_dir":is_dir,"base_record":0,"has_attribute_list":has_attribute_list,"filename_attributes":filename_attributes,"header_name_count":header_name_count,"excluded_dos_short_names":dos_short_names})
    });
    Ok((names, proof))
}
fn parse_resident_names(
    raw: &[u8],
    requested: u64,
    is_dir: bool,
) -> Result<(Vec<LinkName>, Value)> {
    let (names, proof) = parse_record_names(raw, requested, is_dir, true, false, true)?;
    Ok((
        names,
        proof.ok_or_else(|| invalid("resident parser did not produce proof"))?,
    ))
}
// Representatives from fixed system records are not a full-name acceptance gate.
pub(super) fn system_names(
    raw: &[u8],
    requested: u64,
    is_dir: bool,
) -> Result<(Vec<LinkName>, Value)> {
    let (names, proof) = parse_record_names(raw, requested, is_dir, false, true, true)?;
    Ok((
        names,
        proof.ok_or_else(|| invalid("system parser did not produce proof"))?,
    ))
}
// The fast collector uses the same deliberately bounded resident-name gate.
// Reserved2 mismatch rejects this optimization and sends the file to native APIs.
pub(super) fn resident(raw: &[u8], requested: u64, is_dir: bool) -> Result<(Vec<LinkName>, Value)> {
    let (names, proof) = parse_record_names(raw, requested, is_dir, true, true, true)?;
    Ok((
        names,
        proof.ok_or_else(|| invalid("resident parser did not produce proof"))?,
    ))
}

// Fast-only collection needs the exact same validation but never serializes
// the forensic proof that the caller discards.  Keeping this as a thin wrapper
// over `parse_record_names` prevents the acceptance gates from drifting.
pub(super) fn resident_names(raw: &[u8], requested: u64, is_dir: bool) -> Result<Vec<LinkName>> {
    Ok(parse_record_names(raw, requested, is_dir, true, true, false)?.0)
}
pub(super) fn directory(raw: &[u8], requested: u64) -> Result<(LinkName, Value)> {
    let (mut names, proof) = parse_resident_names(raw, requested, true)?;
    Ok((names.remove(0), proof))
}
pub(super) fn metadata_names(
    value: &Value,
    id: u64,
    is_dir: bool,
    serial: u32,
) -> Result<(Vec<LinkName>, Value)> {
    let proof = &value["proof"];
    if value["requested_frn"] != id.to_string()
        || value["volume_serial"] != serial
        || value["is_dir"] != is_dir
        || proof["record_ordinal"] != (id & 0x0000_ffff_ffff_ffff)
        || proof["record_sequence"] != (id >> 48)
        || proof["in_use"] != true
        || proof["is_dir"] != is_dir
        || proof["base_record"] != 0
        || proof["has_attribute_list"] != false
    {
        return Err(invalid("typed exact-record metadata identity/gates mismatch").into());
    }
    let raw = value["names"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid("typed resident names absent"))?;
    let mut names = Vec::new();
    for item in raw {
        let name = units(&item["name_utf16"])?;
        if name.is_empty() || name.iter().any(|c| matches!(*c, 0 | 47 | 92)) {
            return Err(invalid("invalid typed basename").into());
        }
        names.push(LinkName {
            parent: frn(&item["parent_frn"])?,
            name,
        });
    }
    let count = proof["filename_attributes"]
        .as_u64()
        .ok_or_else(|| invalid("typed name count missing"))?;
    let dos = proof["excluded_dos_short_names"]
        .as_u64()
        .ok_or_else(|| invalid("typed DOS count missing"))?;
    if count != names.len() as u64 + dos
        || proof["header_name_count"] != count
        || (is_dir && names.len() != 1)
    {
        return Err(invalid("typed resident name count mismatch").into());
    }
    Ok((names, proof.clone()))
}
pub(super) fn load(
    path: &Path,
    root: &str,
    serial: Option<u32>,
) -> Result<(HashMap<u64, Repair>, Vec<Value>)> {
    let mut failures: HashMap<u64, Failure> = HashMap::new();
    let mut metadata = HashMap::<u64, Value>::new();
    for (index, line) in BufReader::new(fs::File::open(path)?).lines().enumerate() {
        let value: Value = serde_json::from_str(&line?)?;
        if value["kind"] == "ntfs-name-metadata" {
            metadata.insert(frn(&value["requested_frn"])?, value);
        } else if value["kind"] == "ntfs-raw" {
            let id = frn(&value["requested_frn"])?;
            if let Some(failure) = failures.get_mut(&id) {
                if value["raw_bytes"].is_array() {
                    failure.raw = Some(bytes(&value["raw_bytes"])?);
                }
            }
        } else if value["error"].is_string() && value["frn"].is_string() {
            failures.insert(
                frn(&value["frn"])?,
                Failure {
                    detail: value,
                    raw: None,
                    source_line: index + 1,
                },
            );
        }
    }
    let mut repairs = HashMap::new();
    let mut unhandled = Vec::new();
    for (id, failure) in failures {
        let mut evidence = json!({"volume":root,"frn":id.to_string(),"source_details_line":failure.source_line,"original_error":failure.detail["error"]});
        if let Some(info) = failure.detail["information"]
            .as_array()
            .filter(|info| info.len() == 13)
        {
            if let Some(serial) = serial {
                if info
                    .iter()
                    .any(|v| v.as_u64().is_none_or(|n| n > u32::MAX as u64))
                    || info[7] != serial
                {
                    return Err(
                        invalid("fast native repair information/volume serial mismatch").into(),
                    );
                }
            }
            let actual = (info[11].as_u64().unwrap_or(0) << 32) | info[12].as_u64().unwrap_or(0);
            if actual == id
                && info[10] == 0
                && info[0].as_u64().is_some_and(|flags| flags & 0x10 == 0)
            {
                if let Ok(path) = units(&failure.detail["current_path_utf16"]) {
                    let prefix: Vec<_> = format!(r"{root}\$Extend\$Deleted\")
                        .encode_utf16()
                        .collect();
                    if path.starts_with(&prefix) && path.len() > prefix.len() {
                        evidence["reason"] = json!("verified-zero-link-internal-object");
                        evidence["same_frn_verified"] = json!(true);
                        evidence["number_of_links"] = json!(0);
                        evidence["internal_directory"] = json!(r"$Extend\$Deleted");
                        repairs.insert(id, Repair::Unlinked(evidence));
                        continue;
                    }
                }
            }
        }
        if let Some(value) = metadata.remove(&id) {
            let serial = serial
                .ok_or_else(|| invalid("typed fast metadata requires manifest volume serial"))?;
            if value["volume_serial"] != serial {
                return Err(invalid("typed metadata serial mismatch").into());
            }
            if failure.detail["error"]
                .as_str()
                .is_some_and(|e| e.contains("OpenFileById"))
            {
                if let (Some(ordinal), Some(sequence)) = (
                    value["returned_ordinal"].as_u64(),
                    value["returned_sequence"].as_u64(),
                ) {
                    if ordinal < (id & 0x0000_ffff_ffff_ffff)
                        || (ordinal == (id & 0x0000_ffff_ffff_ffff) && sequence != id >> 48)
                    {
                        evidence["reason"] = json!("original-file-reference-disappeared");
                        evidence["requested_ordinal"] = json!(id & 0x0000_ffff_ffff_ffff);
                        evidence["requested_sequence"] = json!(id >> 48);
                        evidence["returned_ordinal"] = json!(ordinal);
                        evidence["returned_sequence"] = json!(sequence);
                        repairs.insert(id, Repair::Disappeared(evidence));
                        continue;
                    }
                }
            }
            match metadata_names(&value, id, false, serial) {
                Ok((names, proof)) => {
                    evidence["reason"] = json!("locked-file-names-recovered-from-parsed-metadata");
                    evidence["proof"] = proof;
                    repairs.insert(id, Repair::ResidentNames(names, evidence));
                    continue;
                }
                Err(error) => evidence["parsed_metadata_repair_error"] = json!(error.to_string()),
            }
        }
        if let Some(raw) = &failure.raw {
            if raw.len() >= 60
                && &raw[12..16] == b"FILE"
                && failure.detail["error"]
                    .as_str()
                    .is_some_and(|e| e.contains("OpenFileById"))
            {
                let requested_ordinal = id & 0x0000_ffff_ffff_ffff;
                let actual_ordinal = u64_at(raw, 0);
                let actual_sequence = u16_at(raw, 28) as u64;
                if actual_ordinal < requested_ordinal
                    || (actual_ordinal == requested_ordinal && actual_sequence != id >> 48)
                {
                    evidence["reason"] = json!("original-file-reference-disappeared");
                    evidence["requested_ordinal"] = json!(requested_ordinal);
                    evidence["requested_sequence"] = json!(id >> 48);
                    evidence["returned_ordinal"] = json!(actual_ordinal);
                    evidence["returned_sequence"] = json!(actual_sequence);
                    repairs.insert(id, Repair::Disappeared(evidence));
                    continue;
                }
            }
            match parse_resident_names(raw, id, false) {
                Ok((names, proof)) => {
                    evidence["reason"] = json!("locked-file-names-recovered");
                    evidence["proof"] = proof;
                    repairs.insert(id, Repair::ResidentNames(names, evidence));
                    continue;
                }
                Err(error) => evidence["raw_repair_error"] = json!(error.to_string()),
            }
        }
        unhandled.push(evidence);
    }
    Ok((repairs, unhandled))
}
