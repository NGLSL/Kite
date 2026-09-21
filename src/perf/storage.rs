//! 场景：历史库升级、设置回落、快照 Warm/Cold、原子覆盖写。
//! 职责：storage / snapshot / atomic_file 的代码路径；不跑真实安装器。

use super::report::{print_latency, print_section, synthetic_apps, temp_dir, time_iters};
use crate::app::atomic_file;
use crate::app::snapshot;
use crate::storage::HistoryDb;
use std::time::{Duration, Instant};

pub fn run() {
    print_section("存储升级 / 原子写（fixture，无 UI）");

    let dir = temp_dir("upgrade");
    seed_v1_history_db(&dir.join("kite-history.db"));

    let t = Instant::now();
    let mut db = HistoryDb::open(&dir.join("kite-history.db")).expect("migrate open");
    let open_cost = t.elapsed();
    let _ = db.ensure_schema();
    let usage_kept = db
        .usage_snapshot(&["old-app".to_string()])
        .get("old-app")
        .map(|u| u.launch_count == 3)
        .unwrap_or(false);
    let settings = db.load_settings();
    print_latency(
        "history-db-open-migrate",
        open_cost,
        open_cost,
        open_cost,
        &format!(
            "usage_kept={usage_kept} hotkey_set={} theme={}",
            !settings.hotkey.is_empty(),
            settings.theme_mode
        ),
    );

    // 部分 settings 键：缺字段回落默认
    {
        let p = dir.join("partial.db");
        let mut db = HistoryDb::open(&p).expect("partial open");
        db.save_setting("hotkey", "Ctrl+Alt+K").ok();
        let s = db.load_settings();
        print_latency(
            "settings-partial-load",
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            &format!(
                "hotkey={} theme_default={}",
                s.hotkey,
                s.theme_mode == "dark"
            ),
        );
    }

    // snapshot 兼容加载
    let snap_dir = temp_dir("snapshot");
    let apps = synthetic_apps(40);
    let mut index = crate::model::AppIndex {
        apps: apps.clone(),
        system_entries: Vec::new(),
        retrieval: None,
    };
    index.rebuild_retrieval();
    snapshot::save_to(&snap_dir, &index).expect("save snapshot");
    let t = Instant::now();
    let loaded = snapshot::load_from(&snap_dir);
    let load_cost = t.elapsed();
    let n = loaded.as_ref().map(|i| i.apps.len()).unwrap_or(0);
    print_latency(
        "snapshot-warm-load",
        load_cost,
        load_cost,
        load_cost,
        &format!("apps={n} compatible={}", n == apps.len()),
    );

    // 不兼容 snapshot → 冷启动（拒绝 Warm）
    let _ = std::fs::write(
        snapshot::snapshot_path(&snap_dir),
        br#"{"snapshot_version":99,"search_schema_version":1,"saved_at_unix":0,"apps":[],"system_entries":[]}"#,
    );
    let cold = snapshot::load_from(&snap_dir);
    print_latency(
        "snapshot-incompatible-cold",
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
        &format!("rejected={}", cold.is_none()),
    );

    // 原子覆盖写
    let file = dir.join("atomic.json");
    let payload = vec![b'x'; 64 * 1024];
    let (p50, p95, max) = time_iters(100, || {
        atomic_file::write(&file, &payload).expect("atomic write");
    });
    let bytes_ok = std::fs::read(&file)
        .map(|b| b.len() == payload.len())
        .unwrap_or(false);
    print_latency(
        "atomic-file-overwrite-64k",
        p50,
        p95,
        max,
        &format!("bytes_ok={bytes_ok}"),
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&snap_dir);
}

/// 模拟旧版 user_version=1 的历史库（含 usage/query 行）。
fn seed_v1_history_db(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = rusqlite::Connection::open(path).expect("raw open");
    conn.execute_batch(
        r#"
        PRAGMA user_version = 1;
        CREATE TABLE usage_history (
            item_id TEXT PRIMARY KEY,
            launch_count INTEGER NOT NULL DEFAULT 0,
            last_used_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE query_history (
            query TEXT NOT NULL,
            item_id TEXT NOT NULL,
            count INTEGER NOT NULL DEFAULT 0,
            last_used_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (query, item_id)
        );
        INSERT INTO usage_history VALUES ('old-app', 3, 100);
        INSERT INTO query_history VALUES ('chrome', 'old-app', 2, 100);
        "#,
    )
    .expect("seed v1");
}
