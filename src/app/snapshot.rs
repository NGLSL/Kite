//! last-good 索引快照：Warm Start 直接恢复，Full 完成后写盘。
//!
//! 不缓存 RetrievalIndex；只保存扫描阶段已算好的条目字段。
//! 搜索字段算法变更时提升 `SEARCH_SCHEMA_VERSION`，旧缓存直接废弃。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::model::{AppIndex, AppItem};

pub const SNAPSHOT_VERSION: u32 = 1;
pub const SEARCH_SCHEMA_VERSION: u32 = 1;
/// Warm Start 资格：超过该年龄的 last-good 不恢复，走冷 Bootstrap。
/// Full 连续失败时避免「越用越旧」；成功 Full 会覆盖并刷新时间戳。
const MAX_WARM_AGE_SECS: u64 = 24 * 3600;

const FILE_NAME: &str = "index-snapshot.json";

/// 应用数据目录（与历史库同级，不是图标 cache 目录）。
static RUNTIME_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 启动时注入数据目录；snapshot 读写都走这里，避免误用 icon cache 路径。
pub fn init(data_dir: PathBuf) {
    let _ = RUNTIME_DATA_DIR.set(data_dir);
}

fn runtime_data_dir() -> PathBuf {
    RUNTIME_DATA_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("com.kite.launcher")
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAppItem {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_src: Option<String>,
    pub source: String,
    #[serde(default)]
    pub is_lnk: bool,
    #[serde(default)]
    pub search_keywords: Vec<String>,
    #[serde(default)]
    pub search_context: Vec<String>,
    #[serde(default)]
    pub normalized_name: String,
    #[serde(default)]
    pub normalized_display: String,
    #[serde(default)]
    pub pinyin: String,
    #[serde(default)]
    pub pinyin_initials: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAppSnapshot {
    pub snapshot_version: u32,
    pub search_schema_version: u32,
    /// Full 写入时刻（unix 秒）；旧快照缺字段为 0，视为过期。
    #[serde(default)]
    pub saved_at_unix: u64,
    pub apps: Vec<CachedAppItem>,
    pub system_entries: Vec<CachedAppItem>,
}

impl CachedAppItem {
    pub fn from_item(item: &AppItem) -> Self {
        Self {
            id: item.id.clone(),
            name: item.name.clone(),
            display_name: item.display_name.clone(),
            target: item.target.clone(),
            args: item.args.clone(),
            working_dir: item.working_dir.clone(),
            icon: item.icon.clone(),
            icon_src: item.icon_src.clone(),
            source: item.source.clone(),
            is_lnk: item.is_lnk,
            search_keywords: item.search_keywords.clone(),
            search_context: item.search_context.clone(),
            normalized_name: item.normalized_name.clone(),
            normalized_display: item.normalized_display.clone(),
            pinyin: item.pinyin.clone(),
            pinyin_initials: item.pinyin_initials.clone(),
        }
    }

    pub fn into_item(self) -> AppItem {
        AppItem {
            id: self.id,
            name: self.name,
            display_name: self.display_name,
            target: self.target,
            args: self.args,
            working_dir: self.working_dir,
            icon: self.icon,
            icon_src: self.icon_src,
            source: self.source,
            is_lnk: self.is_lnk,
            normalized_name: self.normalized_name,
            normalized_display: self.normalized_display,
            pinyin: self.pinyin,
            pinyin_initials: self.pinyin_initials,
            search_keywords: self.search_keywords,
            search_context: self.search_context,
        }
    }
}

pub fn snapshot_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

pub fn from_index(index: &AppIndex) -> CachedAppSnapshot {
    from_index_at(index, crate::storage::now_ts().max(0) as u64)
}

fn from_index_at(index: &AppIndex, saved_at_unix: u64) -> CachedAppSnapshot {
    CachedAppSnapshot {
        snapshot_version: SNAPSHOT_VERSION,
        search_schema_version: SEARCH_SCHEMA_VERSION,
        saved_at_unix,
        apps: index.apps.iter().map(CachedAppItem::from_item).collect(),
        system_entries: index
            .system_entries
            .iter()
            .map(CachedAppItem::from_item)
            .collect(),
    }
}

pub fn into_index(snapshot: CachedAppSnapshot) -> AppIndex {
    let mut index = AppIndex {
        apps: snapshot.apps.into_iter().map(CachedAppItem::into_item).collect(),
        system_entries: snapshot
            .system_entries
            .into_iter()
            .map(CachedAppItem::into_item)
            .collect(),
        retrieval: None,
    };
    index.rebuild_retrieval();
    index
}

fn is_compatible(snapshot: &CachedAppSnapshot) -> bool {
    snapshot.snapshot_version == SNAPSHOT_VERSION
        && snapshot.search_schema_version == SEARCH_SCHEMA_VERSION
        && !snapshot.apps.is_empty()
}

/// 版本兼容且未超过 Warm 最大年龄。
fn is_warm_eligible(snapshot: &CachedAppSnapshot, now_unix: u64) -> bool {
    is_compatible(snapshot)
        && now_unix.saturating_sub(snapshot.saved_at_unix) <= MAX_WARM_AGE_SECS
}

/// Full 成功后写 last-good。Bootstrap/不完整结果不要调用。
pub fn save(index: &AppIndex) -> std::io::Result<()> {
    save_to(&runtime_data_dir(), index)
}

pub fn save_to(data_dir: &Path, index: &AppIndex) -> std::io::Result<()> {
    let snapshot = from_index(index);
    let path = snapshot_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec(&snapshot)?;
    std::fs::write(&tmp, json)?;
    // Windows 的 rename 不会覆盖已存在目标；与 ScanCache 一致：先删再改名。
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Warm Start：版本兼容且未超过 24h 则返回可搜索 AppIndex；否则 None（冷启动）。
pub fn load() -> Option<AppIndex> {
    load_from(&runtime_data_dir())
}

pub fn load_from(data_dir: &Path) -> Option<AppIndex> {
    let path = snapshot_path(data_dir);
    let bytes = std::fs::read(&path).ok()?;
    let snapshot: CachedAppSnapshot = serde_json::from_slice(&bytes).ok()?;
    let now = crate::storage::now_ts().max(0) as u64;
    if !is_warm_eligible(&snapshot, now) {
        crate::log::info(&format!(
            "index snapshot incompatible/expired/empty; cold start (path={}, age_s={}, saved={})",
            path.display(),
            now.saturating_sub(snapshot.saved_at_unix),
            snapshot.saved_at_unix
        ));
        return None;
    }
    let index = into_index(snapshot);
    crate::log::info(&format!(
        "warm start from snapshot n={} ({})",
        index.apps.len(),
        path.display()
    ));
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_item() -> AppItem {
        let mut item = AppItem::scanned(
            "id-1".into(),
            "Demo App".into(),
            r"C:\Demo\app.exe".into(),
            None,
            None,
            "start-menu",
        );
        item.icon = Some(r"C:\icons\demo.png".into());
        item.icon_src = Some(item.target.clone());
        item.attach_search_fields();
        item
    }

    #[test]
    fn snapshot_roundtrip_keeps_search_fields_and_icon() {
        let mut index = AppIndex {
            apps: vec![sample_item()],
            system_entries: Vec::new(),
            retrieval: None,
        };
        index.rebuild_retrieval();
        let restored = into_index(from_index(&index));
        assert_eq!(restored.apps.len(), 1);
        let app = &restored.apps[0];
        assert_eq!(app.name, "Demo App");
        assert_eq!(app.icon.as_deref(), Some(r"C:\icons\demo.png"));
        assert_eq!(app.normalized_name, "demo app");
        assert!(!app.pinyin_initials.is_empty());
        assert!(restored.retrieval.is_some());
    }

    #[test]
    fn incompatible_schema_is_rejected() {
        let mut snapshot = from_index(&AppIndex {
            apps: vec![sample_item()],
            system_entries: Vec::new(),
            retrieval: None,
        });
        snapshot.search_schema_version += 1;
        assert!(!is_compatible(&snapshot));

        snapshot.search_schema_version = SEARCH_SCHEMA_VERSION;
        snapshot.apps.clear();
        assert!(!is_compatible(&snapshot), "empty snapshot must not warm-start");
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("kite-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut index = AppIndex {
            apps: vec![sample_item()],
            system_entries: Vec::new(),
            retrieval: None,
        };
        index.rebuild_retrieval();
        save_to(&dir, &index).unwrap();
        let loaded = load_from(&dir).expect("compatible snapshot loads");
        assert_eq!(loaded.apps[0].target, index.apps[0].target);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_save_overwrites_existing_snapshot() {
        let dir = std::env::temp_dir().join(format!("kite-snap-ow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut first = AppIndex {
            apps: vec![sample_item()],
            system_entries: Vec::new(),
            retrieval: None,
        };
        first.rebuild_retrieval();
        save_to(&dir, &first).unwrap();

        let mut second_item = sample_item();
        second_item.name = "Demo App v2".into();
        second_item.display_name = "Demo App v2".into();
        second_item.attach_search_fields();
        let mut second = AppIndex {
            apps: vec![second_item],
            system_entries: Vec::new(),
            retrieval: None,
        };
        second.rebuild_retrieval();
        save_to(&dir, &second).expect("overwrite must succeed on Windows");

        let loaded = load_from(&dir).expect("second save loads");
        assert_eq!(loaded.apps[0].name, "Demo App v2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_snapshot_is_not_warm_eligible() {
        let mut snapshot = from_index(&AppIndex {
            apps: vec![sample_item()],
            system_entries: Vec::new(),
            retrieval: None,
        });
        let now = snapshot.saved_at_unix;
        assert!(is_warm_eligible(&snapshot, now));
        assert!(is_warm_eligible(&snapshot, now + MAX_WARM_AGE_SECS));
        assert!(!is_warm_eligible(&snapshot, now + MAX_WARM_AGE_SECS + 1));
        // 旧格式缺 saved_at_unix → 0 → 过期
        snapshot.saved_at_unix = 0;
        assert!(!is_warm_eligible(&snapshot, MAX_WARM_AGE_SECS + 10));
    }
}
