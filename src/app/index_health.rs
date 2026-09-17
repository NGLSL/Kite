//! Full Index 健康状态：让「一直吃旧 last-good」变得可见。
//!
//! Warm 策略不变：schema 兼容即可恢复，aged 只记日志与本状态。
//! 状态损坏按默认（未知）处理，不影响搜索/启动。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::atomic_file;

const FILE_NAME: &str = "index-health.json";
/// 与 snapshot 软阈值对齐：仅用于日志与展示，不拒绝 Warm。
pub const WARM_SOFT_AGE_SECS: u64 = 24 * 3600;

static RUNTIME_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 启动时注入数据目录（与 last-good 同级）。
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexHealth {
    /// 最近一次 Full + last-good 写入成功的 unix 秒；0 = 从未。
    #[serde(default)]
    pub last_full_success_unix: u64,
    #[serde(default)]
    pub last_full_failure_unix: u64,
    #[serde(default)]
    pub last_full_failure_reason: String,
    /// 自上次成功以来的连续失败次数。
    #[serde(default)]
    pub full_failure_streak: u32,
    /// 最近一次 Warm load 时 last-good 的年龄（秒）；0 = 无记录。
    #[serde(default)]
    pub last_warm_age_secs: u64,
    #[serde(default)]
    pub warm_from_aged_snapshot: bool,
}

impl IndexHealth {
    pub fn record_full_success(&mut self, now_unix: u64) {
        self.last_full_success_unix = now_unix;
        self.last_full_failure_unix = 0;
        self.last_full_failure_reason.clear();
        self.full_failure_streak = 0;
    }

    pub fn record_full_failure(&mut self, now_unix: u64, reason: &str) {
        self.last_full_failure_unix = now_unix;
        self.last_full_failure_reason = reason.to_string();
        self.full_failure_streak = self.full_failure_streak.saturating_add(1);
    }

    pub fn record_warm_load(&mut self, age_secs: u64, aged: bool) {
        self.last_warm_age_secs = age_secs;
        self.warm_from_aged_snapshot = aged;
    }

    /// 设置页/日志用的一行摘要。
    pub fn summary(&self, now_unix: u64) -> String {
        if self.full_failure_streak > 0 {
            return format!(
                "完整扫描失败 {} 次（最近 {}）",
                self.full_failure_streak,
                self.last_full_failure_reason
            );
        }
        if self.warm_from_aged_snapshot {
            let hours = self.last_warm_age_secs / 3600;
            return format!("快照过旧（约 {hours} 小时），后台完整扫描会刷新");
        }
        if self.last_full_success_unix > 0 {
            let age = now_unix.saturating_sub(self.last_full_success_unix);
            return format!("索引正常（完整扫描 {} 秒前）", age);
        }
        "索引状态未知".to_string()
    }
}

pub fn health_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

pub fn load() -> IndexHealth {
    load_from(&runtime_data_dir())
}

pub fn load_from(data_dir: &Path) -> IndexHealth {
    let path = health_path(data_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return IndexHealth::default(),
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save(health: &IndexHealth) {
    save_to(&runtime_data_dir(), health);
}

pub fn save_to(data_dir: &Path, health: &IndexHealth) {
    let Ok(json) = serde_json::to_vec(health) else {
        return;
    };
    if let Err(error) = atomic_file::write(&health_path(data_dir), &json) {
        crate::log::info(&format!(
            "index health write failed path={} err={error}",
            health_path(data_dir).display()
        ));
    }
}

/// Full 成功（last-good 已写入）后调用。
pub fn record_full_success() {
    record_full_success_to(&runtime_data_dir());
}

pub fn record_full_success_to(data_dir: &Path) {
    let now = crate::storage::now_ts().max(0) as u64;
    let mut health = load_from(data_dir);
    health.record_full_success(now);
    crate::log::info(&format!(
        "index health full success now={now} streak={}",
        health.full_failure_streak
    ));
    save_to(data_dir, &health);
}

/// Full 失败（当前以 last-good 写入失败为准）后调用。
pub fn record_full_failure(reason: &str) {
    record_full_failure_to(&runtime_data_dir(), reason);
}

pub fn record_full_failure_to(data_dir: &Path, reason: &str) {
    let now = crate::storage::now_ts().max(0) as u64;
    let mut health = load_from(data_dir);
    health.record_full_failure(now, reason);
    crate::log::info(&format!(
        "index health full failure streak={} reason={reason}",
        health.full_failure_streak
    ));
    save_to(data_dir, &health);
}

/// Warm load 成功后调用；aged 不阻止恢复。
pub fn record_warm_load(age_secs: u64, aged: bool) {
    record_warm_load_to(&runtime_data_dir(), age_secs, aged);
}

pub fn record_warm_load_to(data_dir: &Path, age_secs: u64, aged: bool) {
    let mut health = load_from(data_dir);
    health.record_warm_load(age_secs, aged);
    if aged {
        crate::log::info(&format!(
            "index health warm aged age_s={age_secs} streak={}",
            health.full_failure_streak
        ));
    }
    save_to(data_dir, &health);
}

/// 当前是否处于「应提示用户」的不健康态（连续失败或 aged）。
pub fn is_degraded(health: &IndexHealth) -> bool {
    health.full_failure_streak > 0 || health.warm_from_aged_snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kite-health-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn success_clears_failure_streak() {
        let mut h = IndexHealth::default();
        h.record_full_failure(100, "scan boom");
        h.record_full_failure(200, "scan boom");
        assert_eq!(h.full_failure_streak, 2);
        h.record_full_success(300);
        assert_eq!(h.full_failure_streak, 0);
        assert!(h.last_full_failure_reason.is_empty());
        assert_eq!(h.last_full_success_unix, 300);
    }

    #[test]
    fn failure_increments_streak_and_keeps_reason() {
        let mut h = IndexHealth::default();
        h.record_full_success(50);
        h.record_full_failure(60, "snapshot save: access denied");
        assert_eq!(h.full_failure_streak, 1);
        assert_eq!(h.last_full_success_unix, 50, "上次成功时间保留");
        assert!(h.last_full_failure_reason.contains("access denied"));
        h.record_full_failure(70, "snapshot save: access denied");
        assert_eq!(h.full_failure_streak, 2);
    }

    #[test]
    fn warm_load_records_age_and_aged_flag() {
        let mut h = IndexHealth::default();
        h.record_warm_load(WARM_SOFT_AGE_SECS + 10, true);
        assert!(h.warm_from_aged_snapshot);
        assert_eq!(h.last_warm_age_secs, WARM_SOFT_AGE_SECS + 10);
        h.record_warm_load(60, false);
        assert!(!h.warm_from_aged_snapshot);
    }

    #[test]
    fn corrupt_or_missing_file_loads_default() {
        let dir = temp_dir("corrupt");
        assert_eq!(load_from(&dir), IndexHealth::default());
        std::fs::write(health_path(&dir), b"not json").unwrap();
        assert_eq!(load_from(&dir), IndexHealth::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_roundtrip_overwrites() {
        let dir = temp_dir("roundtrip");
        let mut h = IndexHealth::default();
        h.record_full_failure(10, "first");
        save_to(&dir, &h);
        let mut h2 = load_from(&dir);
        assert_eq!(h2.full_failure_streak, 1);
        h2.record_full_success(20);
        save_to(&dir, &h2);
        let reloaded = load_from(&dir);
        assert_eq!(reloaded.full_failure_streak, 0);
        assert_eq!(reloaded.last_full_success_unix, 20);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_and_degraded_reflect_states() {
        let mut h = IndexHealth::default();
        assert_eq!(h.summary(1000), "索引状态未知");
        assert!(!is_degraded(&h));

        h.record_full_success(900);
        assert!(h.summary(1000).contains("索引正常"));
        assert!(!is_degraded(&h));

        h.record_warm_load(WARM_SOFT_AGE_SECS, true);
        assert!(h.summary(1000).contains("过旧"));
        assert!(is_degraded(&h));

        h.record_full_success(1000);
        assert_eq!(h.full_failure_streak, 0);
        h.record_full_failure(1001, "save failed");
        assert!(h.summary(1002).contains("失败"));
        assert!(is_degraded(&h));
    }
}
