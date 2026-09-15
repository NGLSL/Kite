//! SQLite 历史库：启动频次、最近使用、Query→App 配对。
//! 手写 SQL，无 ORM。连接可被 `Mutex` 串行使用。

pub mod pins;
pub mod settings;

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, params_from_iter, Connection};

pub struct HistoryDb {
    conn: Connection,
}

/// 一条应用的历史加权原料。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UsageStats {
    pub launch_count: i64,
    pub last_used_at: i64,
}

/// Query→App 配对统计。`last_used_at` 预留调试与后续策略。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct QueryPairStats {
    pub count: i64,
    pub last_used_at: i64,
}

impl HistoryDb {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS usage_history (
                item_id TEXT PRIMARY KEY,
                launch_count INTEGER NOT NULL DEFAULT 0,
                last_used_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS query_history (
                query TEXT NOT NULL,
                item_id TEXT NOT NULL,
                count INTEGER NOT NULL DEFAULT 0,
                last_used_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (query, item_id)
            );
            "#,
        )?;
        let mut db = Self { conn };
        db.ensure_schema()?;
        db.ensure_pins_schema()?;
        Ok(db)
    }

    pub(crate) fn raw_conn(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn raw_conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// 记录一次启动；`query_norm` 为空则只记 Usage，不记 Query History。
    /// 用户暂停记录时整体跳过（设置读取一次，SQLite 本地读开销可忽略）。
    pub fn record_launch(
        &mut self,
        item_id: &str,
        query_norm: &str,
        now: i64,
    ) -> rusqlite::Result<()> {
        if !self.history_recording_enabled() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO usage_history (item_id, launch_count, last_used_at)
             VALUES (?1, 1, ?2)
             ON CONFLICT(item_id) DO UPDATE SET
               launch_count = launch_count + 1,
               last_used_at = excluded.last_used_at",
            params![item_id, now],
        )?;

        if !query_norm.is_empty() {
            self.conn.execute(
                "INSERT INTO query_history (query, item_id, count, last_used_at)
                 VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT(query, item_id) DO UPDATE SET
                   count = count + 1,
                   last_used_at = excluded.last_used_at",
                params![query_norm, item_id, now],
            )?;
        }
        Ok(())
    }

    /// 一次查询拿回多条 usage；搜索热路径按 id 批量取，避免每条结果两次 SQL。
    pub fn usage_snapshot(&self, ids: &[String]) -> HashMap<String, UsageStats> {
        if ids.is_empty() {
            return HashMap::new();
        }
        let sql = format!(
            "SELECT item_id, launch_count, last_used_at FROM usage_history
             WHERE item_id IN ({})",
            placeholders(ids.len())
        );
        let Ok(mut stmt) = self.conn.prepare(&sql) else {
            return HashMap::new();
        };
        stmt.query_map(params_from_iter(ids.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                UsageStats {
                    launch_count: row.get(1)?,
                    last_used_at: row.get(2)?,
                },
            ))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// 清空使用历史（Usage + Query History）。固定项不属于历史，保留。
    pub fn clear_history(&mut self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM usage_history", [])?;
        self.conn.execute("DELETE FROM query_history", [])?;
        Ok(())
    }

    /// 将旧 item_id 的 Usage / Query History / Pin / Alias 迁移到 new_id。
    /// 仅在确认 launch identity 相同（由调用方保证）时调用；不把历史转给不同 target。
    pub fn remap_item_id(&mut self, old_id: &str, new_id: &str) -> rusqlite::Result<bool> {
        if old_id == new_id || old_id.is_empty() || new_id.is_empty() {
            return Ok(false);
        }
        let mut moved = false;

        // usage：合并计数，保留较新 last_used_at
        let existing: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT launch_count, last_used_at FROM usage_history WHERE item_id = ?1",
                params![old_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        if let Some((count, last)) = existing {
            moved = true;
            self.conn.execute(
                "INSERT INTO usage_history (item_id, launch_count, last_used_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(item_id) DO UPDATE SET
                   launch_count = usage_history.launch_count + excluded.launch_count,
                   last_used_at = max(usage_history.last_used_at, excluded.last_used_at)",
                params![new_id, count, last],
            )?;
            self.conn.execute(
                "DELETE FROM usage_history WHERE item_id = ?1",
                params![old_id],
            )?;
        }

        // query_history：按 (query, item_id) 合并
        let pairs: Vec<(String, i64, i64)> = {
            let mut stmt = self.conn.prepare(
                "SELECT query, count, last_used_at FROM query_history WHERE item_id = ?1",
            )?;
            let rows = stmt.query_map(params![old_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            rows.filter_map(|r| r.ok()).collect()
        };
        for (query, count, last) in pairs {
            moved = true;
            self.conn.execute(
                "INSERT INTO query_history (query, item_id, count, last_used_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(query, item_id) DO UPDATE SET
                   count = query_history.count + excluded.count,
                   last_used_at = max(query_history.last_used_at, excluded.last_used_at)",
                params![query, new_id, count, last],
            )?;
        }
        self.conn.execute(
            "DELETE FROM query_history WHERE item_id = ?1",
            params![old_id],
        )?;

        // pin：只改 id，保留 pinned_at
        let pinned: Option<i64> = self
            .conn
            .query_row(
                "SELECT pinned_at FROM pinned WHERE item_id = ?1",
                params![old_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(at) = pinned {
            moved = true;
            self.conn.execute(
                "INSERT INTO pinned (item_id, pinned_at) VALUES (?1, ?2)
                 ON CONFLICT(item_id) DO UPDATE SET pinned_at = max(pinned.pinned_at, excluded.pinned_at)",
                params![new_id, at],
            )?;
            self.conn
                .execute("DELETE FROM pinned WHERE item_id = ?1", params![old_id])?;
        }

        // user_aliases.target_id
        let n = self.conn.execute(
            "UPDATE user_aliases SET target_id = ?1 WHERE target_id = ?2",
            params![new_id, old_id],
        )?;
        if n > 0 {
            moved = true;
        }

        Ok(moved)
    }

    /// 索引发布后：把 legacy id（含 source）迁移到 stable id。
    pub fn migrate_legacy_ids_for_items(
        &mut self,
        items: &[(String, String, Option<String>)], // (stable_id, target, args)
    ) -> usize {
        use crate::app::scanner::util::{legacy_item_id, KNOWN_SOURCES};
        let mut migrated = 0usize;
        for (stable, target, args) in items {
            for source in KNOWN_SOURCES {
                let legacy = legacy_item_id(target, args.as_deref(), source);
                if legacy == *stable {
                    continue;
                }
                match self.remap_item_id(&legacy, stable) {
                    Ok(true) => migrated += 1,
                    Ok(false) => {}
                    Err(e) => {
                        crate::log::info(&format!("id remap {legacy} -> {stable} failed: {e}"));
                    }
                }
            }
        }
        if migrated > 0 {
            crate::log::info(&format!(
                "migrated {migrated} legacy item ids to stable ids"
            ));
        }
        migrated
    }

    /// 最近启动的应用 id，按 last_used_at 降序。
    pub fn recent_ids(&self, limit: usize) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT item_id FROM usage_history
             WHERE last_used_at > 0
             ORDER BY last_used_at DESC
             LIMIT ?1",
        )?;
        let ids = stmt
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// 批量取 Query→App 配对；缺行即无历史（不进结果表）。
    pub fn query_pair_snapshot(
        &self,
        query_norm: &str,
        ids: &[String],
    ) -> HashMap<String, QueryPairStats> {
        if ids.is_empty() || query_norm.is_empty() {
            return HashMap::new();
        }
        let sql = format!(
            "SELECT item_id, count, last_used_at FROM query_history
             WHERE query = ?1 AND item_id IN ({})",
            placeholders(ids.len())
        );
        let Ok(mut stmt) = self.conn.prepare(&sql) else {
            return HashMap::new();
        };
        let joined = std::iter::once(query_norm.to_string())
            .chain(ids.iter().cloned())
            .collect::<Vec<_>>();
        stmt.query_map(params_from_iter(joined.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                QueryPairStats {
                    count: row.get(1)?,
                    last_used_at: row.get(2)?,
                },
            ))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// 全量 Usage 快照（统一排序在截断前需要完整候选集的个性化）。
    pub fn usage_all(&self) -> HashMap<String, UsageStats> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT item_id, launch_count, last_used_at FROM usage_history",
        ) else {
            return HashMap::new();
        };
        stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                UsageStats {
                    launch_count: row.get(1)?,
                    last_used_at: row.get(2)?,
                },
            ))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// 某 Query 的全部配对快照（不按候选 id 过滤）。
    pub fn query_pairs_for(&self, query_norm: &str) -> HashMap<String, QueryPairStats> {
        if query_norm.is_empty() {
            return HashMap::new();
        }
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT item_id, count, last_used_at FROM query_history WHERE query = ?1",
        ) else {
            return HashMap::new();
        };
        stmt.query_map(params![query_norm], |row| {
            Ok((
                row.get::<_, String>(0)?,
                QueryPairStats {
                    count: row.get(1)?,
                    last_used_at: row.get(2)?,
                },
            ))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }
}

/// 生成 `?,?,?` 占位符。
fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// 秒级时间戳。
pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> HistoryDb {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("kite-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}-{n}.db", uuid_like()));
        HistoryDb::open(&path).expect("open temp db")
    }

    fn uuid_like() -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(format!("{:?}", std::time::Instant::now()).as_bytes());
        format!(
            "{:x}",
            h.finalize()[..8]
                .iter()
                .fold(0u64, |a, b| (a << 8) | *b as u64)
        )
    }

    #[test]
    fn recent_ids_ordered_by_last_used() {
        let mut db = temp_db();
        db.record_launch("app-a", "", 100).unwrap();
        db.record_launch("app-b", "", 300).unwrap();
        db.record_launch("app-c", "", 200).unwrap();
        db.record_launch("app-a", "", 400).unwrap(); // a 更新为最新
        let ids = db.recent_ids(10).unwrap();
        assert_eq!(ids, vec!["app-a", "app-b", "app-c"]);
    }

    #[test]
    fn recent_ids_respects_limit() {
        let mut db = temp_db();
        for i in 0..5 {
            db.record_launch(&format!("app-{i}"), "", 100 + i).unwrap();
        }
        let ids = db.recent_ids(2).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "app-4");
        assert_eq!(ids[1], "app-3");
    }

    #[test]
    fn recent_ids_empty_when_no_launch() {
        let db = temp_db();
        assert!(db.recent_ids(10).unwrap().is_empty());
    }

    #[test]
    fn preferred_browser_roundtrip() {
        let mut db = temp_db();
        assert_eq!(db.preferred_browser(), None);
        db.set_preferred_browser("edge").unwrap();
        assert_eq!(db.preferred_browser().as_deref(), Some("edge"));
        db.set_preferred_browser("chrome").unwrap();
        assert_eq!(db.preferred_browser().as_deref(), Some("chrome"));
    }

    #[test]
    fn search_template_roundtrip() {
        let mut db = temp_db();
        assert_eq!(db.search_url_template(), None);
        db.set_search_url_template("https://www.google.com/search?q={searchTerms}")
            .unwrap();
        assert_eq!(
            db.search_url_template().as_deref(),
            Some("https://www.google.com/search?q={searchTerms}")
        );
    }

    #[test]
    fn usage_snapshot_matches_recorded_values() {
        let mut db = temp_db();
        db.record_launch("app-a", "a", 100).unwrap();
        db.record_launch("app-a", "a", 150).unwrap();
        db.record_launch("app-b", "a", 200).unwrap();
        // app-c 没有记录,应缺席
        let ids = vec!["app-a".into(), "app-b".into(), "app-c".into()];
        let snap = db.usage_snapshot(&ids);
        assert_eq!(snap.len(), 2);
        assert_eq!(
            snap["app-a"],
            UsageStats {
                launch_count: 2,
                last_used_at: 150
            }
        );
        assert_eq!(
            snap["app-b"],
            UsageStats {
                launch_count: 1,
                last_used_at: 200
            }
        );
        assert!(!snap.contains_key("app-c"));

        let pairs = db.query_pair_snapshot("a", &ids);
        assert_eq!(pairs.len(), 2);
        assert_eq!(
            pairs["app-a"],
            QueryPairStats {
                count: 2,
                last_used_at: 150
            }
        );
        assert_eq!(
            pairs["app-b"],
            QueryPairStats {
                count: 1,
                last_used_at: 200
            }
        );
        assert!(!pairs.contains_key("app-c"));
    }

    #[test]
    fn snapshots_empty_inputs() {
        let db = temp_db();
        assert!(db.usage_snapshot(&[]).is_empty());
        assert!(db.query_pair_snapshot("a", &[]).is_empty());
        assert!(db.query_pair_snapshot("", &["app-a".into()]).is_empty());
    }

    #[test]
    fn usage_snapshot_ignores_missing_rows() {
        let mut db = temp_db();
        db.record_launch("only", "", 1).unwrap();
        let ids: Vec<String> = (0..50).map(|i| format!("ghost-{i}")).collect();
        assert!(db.usage_snapshot(&ids).is_empty(), "全缺行时返回空表");
    }

    #[test]
    fn clear_history_empties_usage_and_pairs() {
        let mut db = temp_db();
        db.record_launch("app-a", "aa", 100).unwrap();
        db.record_launch("app-b", "bb", 200).unwrap();
        db.clear_history().unwrap();
        assert!(
            db.recent_ids(10).unwrap().is_empty(),
            "清空后最近使用应为空"
        );
        let snap = db.usage_snapshot(&["app-a".into(), "app-b".into()]);
        assert!(snap.is_empty());
        let pairs = db.query_pair_snapshot("aa", &["app-a".into()]);
        assert!(pairs.is_empty());
    }

    #[test]
    fn record_launch_skipped_when_paused() {
        let mut db = temp_db();
        db.save_setting("history_recording", "0").unwrap();
        db.record_launch("app-a", "aa", 100).unwrap();
        assert!(
            db.recent_ids(10).unwrap().is_empty(),
            "暂停记录后不得新增启动次数"
        );
        let pairs = db.query_pair_snapshot("aa", &["app-a".into()]);
        assert!(pairs.is_empty(), "暂停记录后不得新增 Query History");

        // 恢复记录后正常写入
        db.save_setting("history_recording", "1").unwrap();
        db.record_launch("app-a", "aa", 200).unwrap();
        assert_eq!(db.recent_ids(10).unwrap(), vec!["app-a".to_string()]);
    }

    #[test]
    fn remap_item_id_moves_history_pin_and_alias() {
        let mut db = temp_db();
        db.record_launch("old-id", "q", 100).unwrap();
        db.pin_item("old-id", 50).unwrap();
        db.set_alias("oa", Some("old-id"), "Old App").unwrap();

        let moved = db.remap_item_id("old-id", "new-id").unwrap();
        assert!(moved);
        assert!(db.recent_ids(10).unwrap().contains(&"new-id".to_string()));
        assert!(!db.recent_ids(10).unwrap().contains(&"old-id".to_string()));
        assert!(db.pinned_ids().contains(&"new-id".to_string()));
        let alias = db
            .list_aliases()
            .unwrap()
            .into_iter()
            .find(|a| a.alias == "oa")
            .expect("alias remains");
        assert_eq!(alias.target_id.as_deref(), Some("new-id"));
        let pairs = db.query_pair_snapshot("q", &["new-id".into()]);
        assert!(pairs.contains_key("new-id"));
    }

    #[test]
    fn remap_does_not_invent_history_for_different_target() {
        let mut db = temp_db();
        // 空库 remap 到 new-id：不凭空创建历史
        let moved = db.remap_item_id("ghost", "new-id").unwrap();
        assert!(!moved);
        assert!(db.recent_ids(10).unwrap().is_empty());
        assert!(db.pinned_ids().is_empty());
    }

    #[test]
    fn migrate_legacy_ids_for_items_uses_known_sources() {
        use crate::app::scanner::util::{legacy_item_id, stable_item_id};
        let mut db = temp_db();
        let target = r"C:\Apps\Foo\foo.exe";
        let stable = stable_item_id(target, None);
        let legacy = legacy_item_id(target, None, "start-menu");
        assert_ne!(legacy, stable);
        db.record_launch(&legacy, "foo", 100).unwrap();
        db.pin_item(&legacy, 10).unwrap();

        let n = db.migrate_legacy_ids_for_items(&[(stable.clone(), target.into(), None)]);
        assert!(n >= 1);
        assert!(db.pinned_ids().contains(&stable));
        assert!(db.recent_ids(5).unwrap().contains(&stable));
    }

    #[test]
    fn stable_id_ignores_display_source() {
        use crate::app::scanner::util::{legacy_item_id, stable_item_id};
        let t = r"C:\Tools\app.exe";
        assert_eq!(stable_item_id(t, None), stable_item_id(t, None));
        assert_ne!(
            legacy_item_id(t, None, "desktop"),
            legacy_item_id(t, None, "start-menu")
        );
        assert_eq!(stable_item_id(t, Some("-x")), stable_item_id(t, Some("-x")));
        assert_ne!(stable_item_id(t, None), stable_item_id(t, Some("-x")));
    }
}
