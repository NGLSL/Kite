//! Bounded ZIP extraction followed by a validated, reversible plugin replacement.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{import_plugin_dir, ImportOutcome};

const MAX_ENTRIES: usize = 4096;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RATIO: u64 = 1000;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CompareStringOrdinal(
        a: *const u16,
        a_len: i32,
        b: *const u16,
        b_len: i32,
        ignore_case: i32,
    ) -> i32;
}

fn windows_name_equal(a: &[u16], b: &[u16]) -> bool {
    // Match Windows ordinal casing rather than Unicode lowercase expansions.
    unsafe { CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1) == 2 }
}

struct Scratch(PathBuf);

impl Scratch {
    fn create(parent: &Path) -> Result<Self, String> {
        fs::create_dir_all(parent).map_err(|e| format!("创建插件目录失败: {e}"))?;
        for _ in 0..100 {
            let path = parent.join(format!(
                ".import-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("创建导入临时目录失败: {e}")),
            }
        }
        Err("无法创建导入临时目录".into())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn safe_name(raw: &[u8]) -> Result<(PathBuf, String), String> {
    let name = std::str::from_utf8(raw).map_err(|_| "ZIP 文件名必须使用 UTF-8")?;
    if name.is_empty() || name.starts_with('/') || name.contains('\\') {
        return Err("ZIP 包含非法路径".into());
    }
    let mut path = PathBuf::new();
    let name = name.strip_suffix('/').unwrap_or(name);
    for part in name.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.chars().any(|c| c < ' ' || ":<>\"|?*".contains(c))
        {
            return Err("ZIP 包含非法路径".into());
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(
                    &stem[3..],
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                ))
        {
            return Err("ZIP 包含 Windows 保留文件名".into());
        }
        path.push(part);
    }
    Ok((path, name.to_lowercase()))
}

fn preflight(input: &mut File) -> Result<(), String> {
    let length = input.metadata().map_err(|e| e.to_string())?.len();
    if length > MAX_TOTAL_BYTES || length < 22 {
        return Err("ZIP 文件大小无效或超过 256 MiB 限制".into());
    }
    let tail_len = length.min(22 + u16::MAX as u64 + 20) as usize;
    input
        .seek(SeekFrom::End(-(tail_len as i64)))
        .map_err(|e| e.to_string())?;
    let mut tail = vec![0; tail_len];
    input.read_exact(&mut tail).map_err(|e| e.to_string())?;
    let offset = (0..=tail.len() - 22)
        .rev()
        .find(|&index| {
            tail[index..index + 4] == *b"PK\x05\x06"
                && index + 22 + u16::from_le_bytes([tail[index + 20], tail[index + 21]]) as usize
                    == tail.len()
        })
        .ok_or("ZIP 缺少有效结束记录")?;
    let eocd = &tail[offset..offset + 22];
    if tail[offset + 22..]
        .windows(4)
        .any(|bytes| bytes == b"PK\x05\x06")
    {
        return Err("ZIP 注释包含嵌套结束记录".into());
    }
    let word = |index| u16::from_le_bytes([eocd[index], eocd[index + 1]]);
    let dword = |index| u32::from_le_bytes(eocd[index..index + 4].try_into().unwrap()) as u64;
    let count = word(10) as usize;
    if word(4) != 0 || word(6) != 0 || word(8) as usize != count {
        return Err("不支持分卷 ZIP".into());
    }
    if count > MAX_ENTRIES {
        return Err("ZIP 文件条目超过 4096 限制".into());
    }
    if word(10) == u16::MAX
        || dword(12) == u32::MAX as u64
        || dword(16) == u32::MAX as u64
        || (offset >= 20 && tail[offset - 20..offset - 16] == *b"PK\x06\x07")
    {
        return Err("插件包不支持 ZIP64".into());
    }
    let central_size = dword(12);
    let central_offset = dword(16);
    let end_offset = length - tail.len() as u64 + offset as u64;
    if central_size > 16 * 1024 * 1024
        || central_offset.checked_add(central_size) != Some(end_offset)
    {
        return Err("ZIP 中心目录大小或位置无效".into());
    }
    input
        .seek(SeekFrom::Start(central_offset))
        .map_err(|e| e.to_string())?;
    let mut central = vec![0; central_size as usize];
    input.read_exact(&mut central).map_err(|e| e.to_string())?;
    let mut position = 0;
    for _ in 0..count {
        let header = central
            .get(position..position + 46)
            .ok_or("ZIP 中心目录损坏")?;
        if &header[..4] != b"PK\x01\x02" {
            return Err("ZIP 中心目录损坏".into());
        }
        let field = |index| u16::from_le_bytes([header[index], header[index + 1]]) as usize;
        if field(8) & 1 != 0 || !matches!(field(10), 0 | 8) || field(34) != 0 {
            return Err("ZIP 不支持加密、分卷或此压缩格式".into());
        }
        let name_len = field(28);
        let extra_len = field(30);
        let next = position + 46 + name_len + extra_len + field(32);
        if next > central.len() {
            return Err("ZIP 中心目录损坏".into());
        }
        let mut extra = &central[position + 46 + name_len..position + 46 + name_len + extra_len];
        while !extra.is_empty() {
            if extra.len() < 4 {
                return Err("ZIP 额外字段损坏".into());
            }
            let kind = u16::from_le_bytes([extra[0], extra[1]]);
            let size = u16::from_le_bytes([extra[2], extra[3]]) as usize;
            if extra.len() < 4 + size {
                return Err("ZIP 额外字段损坏".into());
            }
            if matches!(kind, 0x0001 | 0x000a | 0x9901 | 0x6375 | 0x7075) {
                return Err("插件 ZIP 不支持 ZIP64、NTFS、加密或 Unicode 覆盖额外字段".into());
            }
            if kind == 0x5455 {
                let data = &extra[4..4 + size];
                if data.is_empty()
                    || data[0] & !7 != 0
                    || (data.len() != 5 && data.len() != 1 + 4 * data[0].count_ones() as usize)
                {
                    return Err("ZIP 时间戳额外字段损坏".into());
                }
            }
            extra = &extra[4 + size..];
        }
        position = next;
    }
    if position != central.len() {
        return Err("ZIP 中心目录条目数量不一致".into());
    }
    input.rewind().map_err(|e| e.to_string())?;
    Ok(())
}

fn extract(source: &Path, root: &Path) -> Result<(), String> {
    let mut input = File::open(source).map_err(|e| format!("读取 ZIP 失败: {e}"))?;
    preflight(&mut input)?;
    let mut zip = zip::ZipArchive::new(input).map_err(|e| format!("无效的插件 ZIP: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err("ZIP 文件条目超过 4096 限制".into());
    }
    // Validate all metadata before writing any files, including Windows case aliases.
    let mut names = HashSet::new();
    let mut windows_names: Vec<(Vec<u16>, bool)> = Vec::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|e| format!("读取 ZIP 条目失败: {e}"))?;
        let (_, key) = safe_name(entry.name_raw())?;
        if !names.insert(key) {
            return Err("ZIP 包含重复文件名（忽略大小写）".into());
        }
        let original = std::str::from_utf8(entry.name_raw())
            .map_err(|_| "ZIP 文件名必须使用 UTF-8")?
            .trim_end_matches('/');
        let wide: Vec<u16> = original.encode_utf16().collect();
        let ascii = original.is_ascii();
        if windows_names.iter().any(|(previous, previous_ascii)| {
            (!ascii || !previous_ascii) && windows_name_equal(previous, &wide)
        }) {
            return Err("ZIP 包含重复 Windows 文件名".into());
        }
        windows_names.push((wide, ascii));
        let kind = entry.unix_mode().unwrap_or(0) & 0o170000;
        if entry.is_symlink() || !matches!(kind, 0 | 0o100000 | 0o040000) || entry.encrypted() {
            return Err("ZIP 不允许链接、特殊文件或加密条目".into());
        }
        if entry.size() > MAX_FILE_BYTES
            || entry.size() > entry.compressed_size().max(1).saturating_mul(MAX_RATIO)
        {
            return Err("ZIP 条目大小或压缩比超过限制".into());
        }
        total = total.checked_add(entry.size()).ok_or("ZIP 大小溢出")?;
        if total > MAX_TOTAL_BYTES {
            return Err("ZIP 解压大小超过 256 MiB 限制".into());
        }
    }
    fs::create_dir(root).map_err(|e| format!("创建解压目录失败: {e}"))?;
    let mut actual_total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let (relative, _) = safe_name(entry.name_raw())?;
        let target = root.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| format!("解压目录失败: {e}"))?;
            continue;
        }
        fs::create_dir_all(target.parent().ok_or("ZIP 文件缺少父目录")?)
            .map_err(|e| format!("解压目录失败: {e}"))?;
        let mut output = File::options()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| format!("创建解压文件失败: {e}"))?;
        let declared = entry.size();
        let mut written = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = entry
                .read(&mut buffer)
                .map_err(|e| format!("解压 ZIP 失败: {e}"))?;
            if count == 0 {
                break;
            }
            written += count as u64;
            actual_total += count as u64;
            if written > declared || written > MAX_FILE_BYTES || actual_total > MAX_TOTAL_BYTES {
                return Err("ZIP 实际解压大小超过限制".into());
            }
            output
                .write_all(&buffer[..count])
                .map_err(|e| format!("写入解压文件失败: {e}"))?;
        }
        if written != declared {
            return Err("ZIP 解压大小与声明不一致".into());
        }
    }
    Ok(())
}

#[cfg(test)]
fn import_zip(source: &Path, plugins_dir: &Path) -> Result<ImportOutcome, String> {
    import_zip_with_pre_replace(source, plugins_dir, &mut |_| Ok(()))
}

pub(super) fn import_zip_with_pre_replace(
    source: &Path,
    plugins_dir: &Path,
    before_replace: &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<ImportOutcome, String> {
    let scratch = Scratch::create(plugins_dir)?;
    let extracted = scratch.0.join("extracted");
    extract(source, &extracted)?;
    // The archive itself must have one root plugin.json. Do not scan nested plugins.
    let staged = import_plugin_dir(&extracted, &scratch.0.join("validated"))?;
    let dest = plugins_dir.join(&staged.plugin_id);
    let backup = scratch.0.join("previous");
    before_replace(&staged.plugin_id)?;
    let replaced = match fs::symlink_metadata(&dest) {
        Ok(meta) => {
            use std::os::windows::fs::MetadataExt;
            if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
                return Err("目标插件目录不是普通目录".into());
            }
            fs::rename(&dest, &backup).map_err(|e| format!("备份已有插件失败: {e}"))?;
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(format!("检查已有插件失败: {e}")),
    };
    if let Err(error) = fs::rename(&staged.dest, &dest) {
        if replaced {
            if let Err(rollback) = fs::rename(&backup, &dest) {
                // Preserve the backup for manual recovery instead of deleting it on Drop.
                let recovery = scratch.0.clone();
                std::mem::forget(scratch);
                return Err(format!(
                    "安装插件失败: {error}；恢复失败: {rollback}；原插件保留在 {}",
                    recovery.display()
                ));
            }
        }
        return Err(format!("安装插件失败: {error}"));
    }
    Ok(ImportOutcome {
        plugin_id: staged.plugin_id,
        dest,
        replaced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn archive(root: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = root.join("plugin.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, bytes) in entries {
            writer
                .start_file(
                    *name,
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    fn manifest() -> Vec<u8> {
        let mut manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../official-plugins/calculator/plugin.json"
        ))
        .unwrap();
        manifest["plugin"]["id"] = "com.kite.zip-test".into();
        manifest["runtime"]["command"] = "client.exe".into();
        serde_json::to_vec(&manifest).unwrap()
    }

    #[test]
    fn imports_zip_with_installer_and_replaces_after_validation() {
        let root = Scratch::create(&std::env::temp_dir()).unwrap();
        let json = manifest();
        let path = archive(
            &root.0,
            &[
                ("plugin.json", &json),
                ("client.exe", b"MZ"),
                ("installer/install-service.ps1", b"installer"),
                ("licenses/LICENSE", b"license"),
            ],
        );
        let plugins = root.0.join("plugins");
        let outcome = super::super::import_from_path(&path, &plugins)
            .pop()
            .unwrap()
            .unwrap();
        assert!(!outcome.replaced);
        assert_eq!(
            fs::read(outcome.dest.join("installer/install-service.ps1")).unwrap(),
            b"installer"
        );
        fs::write(outcome.dest.join("marker"), b"old").unwrap();
        let second = import_zip(&path, &plugins).unwrap();
        assert!(second.replaced);
        assert!(!second.dest.join("marker").exists());
        assert_eq!(
            fs::read_dir(&plugins).unwrap().count(),
            1,
            "temporary directories cleaned"
        );
    }

    #[test]
    fn rejected_zip_keeps_installed_plugin_and_cleans_temporary_files() {
        let root = Scratch::create(&std::env::temp_dir()).unwrap();
        let json = manifest();
        let plugins = root.0.join("plugins");
        let path = archive(&root.0, &[("plugin.json", &json), ("client.exe", b"MZ")]);
        let installed = import_zip(&path, &plugins).unwrap();
        fs::write(installed.dest.join("marker"), b"original").unwrap();
        let bomb = vec![0u8; 2 * 1024 * 1024];
        let cases: Vec<Vec<(&str, &[u8])>> = vec![
            vec![("plugin.json", &json)],
            vec![("nested/plugin.json", &json), ("nested/client.exe", b"MZ")],
            vec![
                ("plugin.json", &json),
                ("client.exe", b"MZ"),
                ("../escape", b"bad"),
            ],
            vec![
                ("plugin.json", &json),
                ("client.exe", b"MZ"),
                ("CLIENT.EXE", b"duplicate"),
            ],
            vec![
                ("plugin.json", &json),
                ("client.exe", b"MZ"),
                ("bomb.bin", &bomb),
            ],
            vec![("plugin.json", b"bad json"), ("client.exe", b"MZ")],
        ];
        for entries in cases {
            let path = archive(&root.0, &entries);
            assert!(import_zip(&path, &plugins).is_err(), "{entries:?}");
            assert_eq!(
                fs::read(installed.dest.join("marker")).unwrap(),
                b"original"
            );
            assert_eq!(fs::read_dir(&plugins).unwrap().count(), 1);
        }
        fs::write(&path, b"invalid ZIP").unwrap();
        assert!(import_zip(&path, &plugins).is_err());
        assert_eq!(
            fs::read(installed.dest.join("marker")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn rejects_windows_path_aliases() {
        for bad in [
            "/root",
            "C:/root",
            "../root",
            "a/../root",
            "a\\root",
            "a:ads",
            "a//b",
            "a/./b",
            "a. /b",
            "a./b",
            "CON",
            "nul.txt",
            "com1.exe",
            "LPT¹",
        ] {
            assert!(safe_name(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(safe_name(b"a\xff").is_err());
        assert_eq!(
            safe_name("licenses/许可.txt".as_bytes()).unwrap().0,
            Path::new("licenses").join("许可.txt")
        );
    }

    #[test]
    fn rejects_symbolic_links_and_too_many_entries() {
        let root = Scratch::create(&std::env::temp_dir()).unwrap();
        let path = root.0.join("link.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        writer
            .add_symlink("link", "../outside", SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
        assert!(import_zip(&path, &root.0.join("plugins"))
            .unwrap_err()
            .contains("链接"));
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        for index in 0..=MAX_ENTRIES {
            writer
                .start_file(format!("file{index}"), SimpleFileOptions::default())
                .unwrap();
        }
        writer.finish().unwrap();
        assert!(import_zip(&path, &root.0.join("plugins"))
            .unwrap_err()
            .contains("4096"));
    }

    #[test]
    fn pre_replace_runs_only_after_validation_and_failure_preserves_old_plugin() {
        let root = Scratch::create(&std::env::temp_dir()).unwrap();
        let json = manifest();
        let plugins = root.0.join("plugins");
        let path = archive(&root.0, &[("plugin.json", &json), ("client.exe", b"MZ")]);
        let installed = import_zip(&path, &plugins).unwrap();
        fs::write(installed.dest.join("marker"), b"old").unwrap();
        let mut calls = 0;
        let result = super::super::import_from_path_with_pre_replace(&path, &plugins, |id| {
            assert_eq!(id, "com.kite.zip-test");
            assert!(installed.dest.join("marker").is_file());
            calls += 1;
            Err("runtime stop failed".into())
        });
        assert!(result[0].is_err());
        assert_eq!(calls, 1);
        assert_eq!(fs::read(installed.dest.join("marker")).unwrap(), b"old");
        let path = archive(&root.0, &[("plugin.json", &json)]);
        let result = super::super::import_from_path_with_pre_replace(&path, &plugins, |_| {
            panic!("invalid ZIP must not stop an existing runtime")
        });
        assert!(result[0].is_err());
        assert_eq!(fs::read_dir(&plugins).unwrap().count(), 1);
    }

    #[test]
    fn bounds_eocd_allocation_before_zip_library_and_checks_declared_file_limit() {
        let root = Scratch::create(&std::env::temp_dir()).unwrap();
        let path = archive(&root.0, &[("a", b"data")]);
        let original = fs::read(&path).unwrap();
        let eocd = original.len() - 22;
        let mut bytes = original.clone();
        bytes[eocd + 8..eocd + 10].copy_from_slice(&65534u16.to_le_bytes());
        bytes[eocd + 10..eocd + 12].copy_from_slice(&65534u16.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(preflight(&mut File::open(&path).unwrap())
            .unwrap_err()
            .contains("4096"));

        let mut bytes = original.clone();
        let central = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
        bytes[central + 24..central + 28]
            .copy_from_slice(&((MAX_FILE_BYTES + 1) as u32).to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(import_zip(&path, &root.0.join("plugins"))
            .unwrap_err()
            .contains("大小"));

        let mut bytes = original;
        bytes[eocd + 12..eocd + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(preflight(&mut File::open(&path).unwrap())
            .unwrap_err()
            .contains("ZIP64"));
    }

    #[test]
    fn rejects_malformed_timestamp_before_fallback_to_embedded_eocd() {
        let root = Scratch::create(&std::env::temp_dir()).unwrap();
        let path = root.0.join("timestamp.zip");
        let mut hidden_eocd = [0u8; 22];
        hidden_eocd[..4].copy_from_slice(b"PK\x05\x06");
        hidden_eocd[8..10].copy_from_slice(&65000u16.to_le_bytes());
        hidden_eocd[10..12].copy_from_slice(&65000u16.to_le_bytes());
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file(
                "embedded-eocd",
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(&hidden_eocd).unwrap();
        writer.finish().unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let end = bytes.len() - 22;
        let central = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
        let name_len =
            u16::from_le_bytes(bytes[central + 28..central + 30].try_into().unwrap()) as usize;
        let original_size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap());
        bytes[central + 30..central + 32].copy_from_slice(&10u16.to_le_bytes());
        bytes.splice(
            central + 46 + name_len..central + 46 + name_len,
            [0x55, 0x54, 6, 0, 0, 0, 0, 0, 0, 0],
        );
        bytes[end + 10 + 12..end + 10 + 16].copy_from_slice(&(original_size + 10).to_le_bytes());
        fs::write(&path, bytes).unwrap();
        // Never hand this archive to the library's backwards EOCD fallback scanner.
        assert!(preflight(&mut File::open(&path).unwrap())
            .unwrap_err()
            .contains("时间戳"));
    }
}
