//! Search aliases from executable version resources.
//!
//! Full 扫描曾对每个 exe 串行调用 GetFileVersionInfoW，实测 ~12s/200 应用。
//! 这里改为：路径戳缓存 + 线程池并行 miss，把热路径压到秒级。

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};

use super::util::normalize_path_key;

const MAX_VERSION_RESOURCE: u32 = 16 * 1024 * 1024;
const FIELDS: [&str; 4] = [
    "FileDescription",
    "ProductName",
    "InternalName",
    "OriginalFilename",
];
const CACHE_VERSION: u32 = 1;
/// 并行读取版本资源的工作线程数（受 AV/磁盘限制，8 足够吃满）。
const WORKERS: usize = 8;

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: HashMap<String, MetaEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct MetaEntry {
    mtime_nanos: u64,
    size: u64,
    keywords: Vec<String>,
}

/// exe 版本资源关键词缓存（按 path + mtime + size）。
pub(super) struct MetaCache {
    file: PathBuf,
    entries: HashMap<String, MetaEntry>,
    seen: std::collections::HashSet<String>,
    pub hits: usize,
    pub misses: usize,
}

impl MetaCache {
    pub fn load(icon_dir: &Path) -> Self {
        let file = icon_dir.join("meta-cache-v1.json");
        let entries = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
            .filter(|f| f.version == CACHE_VERSION)
            .map(|f| f.entries)
            .unwrap_or_default();
        Self {
            file,
            entries,
            seen: std::collections::HashSet::new(),
            hits: 0,
            misses: 0,
        }
    }

    fn lookup(&mut self, path: &Path) -> Option<Vec<String>> {
        let key = normalize_path_key(&path.to_string_lossy());
        let Some(entry) = self.entries.get(&key) else {
            self.misses += 1;
            return None;
        };
        let Some((mtime, size)) = stamp(path) else {
            self.misses += 1;
            return None;
        };
        if entry.mtime_nanos == mtime && entry.size == size {
            self.hits += 1;
            self.seen.insert(key);
            Some(entry.keywords.clone())
        } else {
            self.misses += 1;
            None
        }
    }

    fn record(&mut self, path: &Path, keywords: Vec<String>) {
        let key = normalize_path_key(&path.to_string_lossy());
        let Some((mtime, size)) = stamp(path) else {
            return;
        };
        self.seen.insert(key.clone());
        self.entries.insert(
            key,
            MetaEntry {
                mtime_nanos: mtime,
                size,
                keywords,
            },
        );
    }

    pub fn save(&self) {
        let file = CacheFile {
            version: CACHE_VERSION,
            entries: self
                .entries
                .iter()
                .filter(|(k, _)| self.seen.contains(*k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };
        let Ok(json) = serde_json::to_vec(&file) else {
            return;
        };
        if let Err(error) = crate::app::atomic_file::write(&self.file, &json) {
            crate::log::info(&format!(
                "meta cache write failed path={} err={error}",
                self.file.display()
            ));
        }
    }
}

fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((mtime.as_nanos() as u64, meta.len()))
}

/// 批量取关键词：缓存命中直接用，miss 并行读版本资源。
/// 返回与 `paths` 等长的列表。
pub(super) fn executable_keywords_batch(
    paths: &[PathBuf],
    cache: &mut MetaCache,
) -> Vec<Vec<String>> {
    let mut out = vec![Vec::new(); paths.len()];
    let mut pending: Vec<usize> = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        if let Some(kw) = cache.lookup(path) {
            out[i] = kw;
        } else {
            pending.push(i);
        }
    }
    if pending.is_empty() {
        return out;
    }

    let chunk = pending.len().div_ceil(WORKERS).max(1);
    let extracted = Mutex::new(HashMap::<usize, Vec<String>>::new());
    std::thread::scope(|s| {
        for part in pending.chunks(chunk) {
            let part = part.to_vec();
            let paths = paths;
            let extracted = &extracted;
            s.spawn(move || {
                for i in part {
                    let kw = executable_keywords(&paths[i]);
                    if let Ok(mut g) = extracted.lock() {
                        g.insert(i, kw);
                    }
                }
            });
        }
    });

    let extracted = extracted.into_inner().unwrap_or_default();
    for (i, kw) in extracted {
        cache.record(&paths[i], kw.clone());
        out[i] = kw;
    }
    out
}

pub(super) fn executable_keywords(path: &Path) -> Vec<String> {
    // shell: / 虚拟路径不做文件探测
    let path_str = path.to_string_lossy();
    if path_str.starts_with("shell:") || path_str.starts_with("::{") {
        return Vec::new();
    }
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return Vec::new();
    }
    let wide_path = wide_os(path.as_os_str());
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide_path.as_ptr()), None) };
    if size == 0 || size > MAX_VERSION_RESOURCE {
        return Vec::new();
    }
    let mut block = vec![0u8; size as usize];
    if unsafe {
        GetFileVersionInfoW(
            PCWSTR(wide_path.as_ptr()),
            None,
            size,
            block.as_mut_ptr().cast(),
        )
    }
    .is_err()
    {
        return Vec::new();
    }

    let mut translations = query_translations(&block);
    if !translations.contains(&(0x0409, 0x04b0)) {
        translations.push((0x0409, 0x04b0));
    }
    let mut values = Vec::new();
    for field in FIELDS {
        for &(language, code_page) in &translations {
            let key = format!(r"\StringFileInfo\{language:04x}{code_page:04x}\{field}");
            if let Some(value) = query_string(&block, &key) {
                if value.len() <= 512
                    && !values
                        .iter()
                        .any(|known: &String| known.eq_ignore_ascii_case(&value))
                {
                    values.push(value);
                }
                break;
            }
        }
    }
    values
}

fn query_translations(block: &[u8]) -> Vec<(u16, u16)> {
    let Some((ptr, len)) = query_value(block, r"\VarFileInfo\Translation") else {
        return Vec::new();
    };
    let count = len as usize / 4;
    let words = (0..count * 2)
        .map(|index| unsafe { ptr.cast::<u16>().add(index).read_unaligned() })
        .collect::<Vec<_>>();
    words
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

fn query_string(block: &[u8], key: &str) -> Option<String> {
    let (ptr, len) = query_value(block, key)?;
    let units = (0..len as usize)
        .map(|index| unsafe { ptr.cast::<u16>().add(index).read_unaligned() })
        .collect::<Vec<_>>();
    let value = String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .trim()
        .to_string();
    (!value.is_empty()).then_some(value)
}

fn query_value(block: &[u8], key: &str) -> Option<(*mut c_void, u32)> {
    let wide_key = wide(key);
    let mut ptr = std::ptr::null_mut();
    let mut len = 0u32;
    let ok = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            PCWSTR(wide_key.as_ptr()),
            &mut ptr,
            &mut len,
        )
    };
    (ok.as_bool() && !ptr.is_null() && len > 0).then_some((ptr, len))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_os(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_searchable_metadata_from_windows_executable() {
        let windows = std::env::var_os("WINDIR").unwrap();
        let fields = executable_keywords(&Path::new(&windows).join("System32/notepad.exe"));
        assert!(
            !fields.is_empty(),
            "Notepad should expose version search metadata"
        );
    }

    #[test]
    fn invalid_executable_has_no_metadata() {
        let file = std::env::temp_dir().join(format!("kite-metadata-{}.exe", std::process::id()));
        std::fs::write(&file, b"fixture").unwrap();
        assert!(executable_keywords(&file).is_empty());
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn batch_hits_cache_after_first_extract() {
        let dir = std::env::temp_dir().join(format!("kite-meta-cache-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut cache = MetaCache::load(&dir);
        let windows = std::env::var_os("WINDIR").unwrap();
        let path = PathBuf::from(&windows).join("System32/notepad.exe");
        let first = executable_keywords_batch(&[path.clone()], &mut cache);
        assert!(!first[0].is_empty());
        assert_eq!(cache.misses, 1);
        let second = executable_keywords_batch(&[path], &mut cache);
        assert_eq!(first[0], second[0]);
        assert_eq!(cache.hits, 1);
        cache.save();
        let reloaded = MetaCache::load(&dir);
        assert!(reloaded.entries.len() >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_paths_are_skipped() {
        assert!(executable_keywords(Path::new("shell:AppsFolder\\Foo")).is_empty());
    }

    #[test]
    fn save_overwrites_existing_meta_cache() {
        let dir = std::env::temp_dir().join(format!("kite-meta-ow-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let windows = std::env::var_os("WINDIR").unwrap();
        let path = PathBuf::from(&windows).join("System32/notepad.exe");

        let mut first = MetaCache::load(&dir);
        let _ = executable_keywords_batch(&[path.clone()], &mut first);
        first.save();

        let mut second = MetaCache::load(&dir);
        // 模拟本轮未见到旧条目：save 会裁掉 unseen
        second.seen.clear();
        second.save();
        assert!(MetaCache::load(&dir).entries.is_empty());

        let mut third = MetaCache::load(&dir);
        let _ = executable_keywords_batch(&[path], &mut third);
        third.save();
        assert!(
            !MetaCache::load(&dir).entries.is_empty(),
            "second write after prune must land"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}