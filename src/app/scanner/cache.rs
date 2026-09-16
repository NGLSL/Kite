//! lnk 解析结果增量缓存：按 (路径, mtime, 大小) 复用上次的 COM 解析。
//! fast 扫描的大头是 IShellLink COM 解析（首个目录还叠加 COM 冷启动，实测 100ms+）；
//! 文件未变时直接复用，热重建只剩目录枚举 + 文件 stat。损坏/版本不符一律按空处理。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::util::normalize_path_key;

/// 与 lnk::resolve_lnk 的四元组一致：(target, args, working_dir, icon_src)。
/// icon_src 自 v2 起可能为 `path,index`；旧缓存丢序号，升级 version 强制重解析。
pub type ResolvedLnk = (String, Option<String>, Option<String>, Option<String>);

const CACHE_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: HashMap<String, CacheEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct CacheEntry {
    mtime_nanos: u64,
    size: u64,
    target: String,
    args: Option<String>,
    working_dir: Option<String>,
    icon_src: Option<String>,
}

#[derive(Default)]
pub struct ScanCache {
    file: PathBuf,
    entries: HashMap<String, CacheEntry>,
    /// 本次扫描见过的路径；save 时只保留这些，自动清掉已删除的 lnk。
    seen: HashSet<String>,
    pub hits: usize,
    pub misses: usize,
}

impl ScanCache {
    /// 从 `icon_dir` 的上级目录加载；失败按空缓存处理。
    pub fn load(icon_dir: &Path) -> ScanCache {
        let file = cache_file(icon_dir);
        let entries = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
            .filter(|f| f.version == CACHE_VERSION)
            .map(|f| f.entries)
            .unwrap_or_default();
        ScanCache {
            file,
            entries,
            seen: HashSet::new(),
            hits: 0,
            misses: 0,
        }
    }

    /// 文件未变时返回缓存解析结果，否则 miss（由调用方走 COM 解析）。
    pub fn lookup(&mut self, path: &Path) -> Option<ResolvedLnk> {
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
            Some((
                entry.target.clone(),
                entry.args.clone(),
                entry.working_dir.clone(),
                entry.icon_src.clone(),
            ))
        } else {
            self.misses += 1;
            None
        }
    }

    /// 记录一次解析结果；命中与新增都要记，刷新时间戳并保活。
    pub fn record(&mut self, path: &Path, resolved: &ResolvedLnk) {
        let key = normalize_path_key(&path.to_string_lossy());
        let Some((mtime, size)) = stamp(path) else {
            return;
        };
        self.seen.insert(key.clone());
        self.entries.insert(
            key,
            CacheEntry {
                mtime_nanos: mtime,
                size,
                target: resolved.0.clone(),
                args: resolved.1.clone(),
                working_dir: resolved.2.clone(),
                icon_src: resolved.3.clone(),
            },
        );
    }

    /// 原子写盘（tmp + rename）；失败静默，下次全量扫描即可。
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
        let tmp = self.file.with_extension("json.tmp");
        if std::fs::write(&tmp, &json).is_ok() {
            // Windows does not replace an existing destination with rename.
            // Remove the old snapshot first so subsequent scans can refresh
            // the cache instead of silently keeping stale entries.
            if self.file.exists() {
                let _ = std::fs::remove_file(&self.file);
            }
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }
}

/// (mtime 纳秒, size)；任一不可得视为不可缓存。
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((mtime.as_nanos() as u64, meta.len()))
}

/// 缓存放在图标缓存目录内，避免多实例或并行扫描共用同一临时文件。
fn cache_file(icon_dir: &Path) -> PathBuf {
    icon_dir.join("scan-cache-v1.json")
}

/// UWP/AppsFolder 枚举结果缓存：COM 枚举约数百毫秒，且无廉价文件 mtime。
/// 仅 Full 收集 UWP；TTL 内自动复用，设置/托盘「重新扫描」强制刷新。
/// 30min：装完 Store 应用后下次 Full 会较快吃到，又不至于每次 Full 都 COM 全枚举。
const UWP_CACHE_VERSION: u32 = 1;
const UWP_CACHE_TTL_SECS: u64 = 30 * 60;

#[derive(Serialize, Deserialize, Clone)]
pub struct CachedUwpItem {
    pub id: String,
    pub name: String,
    pub target: String,
    pub source: String,
    pub icon_src: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct UwpCacheFile {
    version: u32,
    saved_at_unix: u64,
    items: Vec<CachedUwpItem>,
}

#[derive(Default)]
pub struct UwpCache {
    file: PathBuf,
    saved_at_unix: u64,
    items: Vec<CachedUwpItem>,
    loaded: bool,
}

impl UwpCache {
    pub fn load(icon_dir: &Path) -> UwpCache {
        let file = icon_dir.join("uwp-cache-v1.json");
        let parsed = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<UwpCacheFile>(&bytes).ok())
            .filter(|f| f.version == UWP_CACHE_VERSION);
        match parsed {
            Some(f) => UwpCache {
                file,
                saved_at_unix: f.saved_at_unix,
                items: f.items,
                loaded: true,
            },
            None => UwpCache {
                file,
                ..UwpCache::default()
            },
        }
    }

    /// TTL 内且未强制刷新时返回缓存条目。
    pub fn fresh_items(&self, force: bool, now_unix: u64) -> Option<Vec<CachedUwpItem>> {
        if force || !self.loaded || self.items.is_empty() {
            return None;
        }
        if now_unix.saturating_sub(self.saved_at_unix) > UWP_CACHE_TTL_SECS {
            return None;
        }
        Some(self.items.clone())
    }

    pub fn save(&self, items: Vec<CachedUwpItem>, now_unix: u64) {
        let file = UwpCacheFile {
            version: UWP_CACHE_VERSION,
            saved_at_unix: now_unix,
            items,
        };
        let Ok(json) = serde_json::to_vec(&file) else {
            return;
        };
        let tmp = self.file.with_extension("json.tmp");
        if std::fs::write(&tmp, &json).is_ok() {
            if self.file.exists() {
                let _ = std::fs::remove_file(&self.file);
            }
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kite-scan-cache-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    fn resolved() -> ResolvedLnk {
        (
            r"C:\apps\foo.exe".into(),
            Some("--x".into()),
            Some(r"C:\apps".into()),
            Some(r"C:\apps\foo.exe".into()),
        )
    }

    #[test]
    fn uwp_cache_ttl_and_force() {
        let d = temp_dir("uwp");
        let cache = UwpCache::load(&d);
        assert!(cache.fresh_items(false, 1_000).is_none(), "空缓存不命中");
        let items = vec![CachedUwpItem {
            id: "a".into(),
            name: "App".into(),
            target: "shell:AppsFolder\\a".into(),
            source: "uwp".into(),
            icon_src: Some("logo.png".into()),
        }];
        cache.save(items.clone(), 1_000);
        let reloaded = UwpCache::load(&d);
        let hit = reloaded
            .fresh_items(false, 1_000 + 60)
            .expect("TTL 内应命中");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].source, "uwp");
        assert!(reloaded.fresh_items(true, 1_000 + 60).is_none(), "force 忽略缓存");
        assert!(
            reloaded.fresh_items(false, 1_000 + UWP_CACHE_TTL_SECS + 1).is_none(),
            "超过 30min TTL 不命中"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn save_then_load_hits_same_file() {
        let dir = temp_dir("roundtrip");
        let lnk = dir.join("a.lnk");
        std::fs::write(&lnk, b"x").unwrap();

        let mut cache = ScanCache::load(&dir);
        assert_eq!(cache.lookup(&lnk), None, "首次必然 miss");
        cache.record(&lnk, &resolved());
        cache.save();

        let mut again = ScanCache::load(&dir);
        assert_eq!(
            again.lookup(&lnk).as_ref(),
            Some(&resolved()),
            "未变化应命中"
        );
        assert_eq!(again.hits, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changed_file_misses() {
        let dir = temp_dir("stale");
        let lnk = dir.join("b.lnk");
        std::fs::write(&lnk, b"old").unwrap();
        let mut cache = ScanCache::load(&dir);
        cache.record(&lnk, &resolved());
        cache.save();

        std::fs::write(&lnk, b"new content").unwrap();
        let mut again = ScanCache::load(&dir);
        assert_eq!(again.lookup(&lnk), None, "内容变化（size 变）必须 miss");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_or_version_mismatch_loads_empty() {
        let dir = temp_dir("corrupt");
        std::fs::write(cache_file(&dir), b"not json").unwrap();
        assert!(ScanCache::load(&dir).lookup(Path::new("x")).is_none());

        let bad = CacheFile {
            version: 99,
            entries: HashMap::new(),
        };
        std::fs::write(cache_file(&dir), serde_json::to_vec(&bad).unwrap()).unwrap();
        let mut cache = ScanCache::load(&dir);
        let lnk = dir.join("c.lnk");
        std::fs::write(&lnk, b"x").unwrap();
        assert_eq!(cache.lookup(&lnk), None, "版本不符按空缓存");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleted_entries_dropped_on_save() {
        let dir = temp_dir("prune");
        let lnk = dir.join("d.lnk");
        std::fs::write(&lnk, b"x").unwrap();
        let mut cache = ScanCache::load(&dir);
        cache.record(&lnk, &resolved());
        // 第二次扫描没再见到该文件：新缓存实例的 seen 为空 → 保存后条目被清掉
        let second = ScanCache::load(&dir);
        second.save();
        let mut again = ScanCache::load(&dir);
        assert_eq!(again.lookup(&lnk), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
