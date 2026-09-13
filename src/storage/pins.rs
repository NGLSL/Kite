//! 固定结果（Pin）：用户手动置顶的 AppItem id，跨重启持久化。
//! 与使用历史无关：清空历史不影响固定项。

use rusqlite::params;

use crate::storage::HistoryDb;

impl HistoryDb {
    /// 初始化 pinned 表（在 ensure_schema 之后调用也安全）。
    pub fn ensure_pins_schema(&mut self) -> rusqlite::Result<()> {
        self.conn_mut().execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS pinned (
                item_id TEXT PRIMARY KEY,
                pinned_at INTEGER NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    /// 固定一个结果；重复固定刷新时间（最近固定的排最前）。
    pub fn pin_item(&mut self, item_id: &str, now: i64) -> rusqlite::Result<()> {
        if item_id.is_empty() {
            return Ok(());
        }
        self.conn_mut().execute(
            "INSERT INTO pinned (item_id, pinned_at) VALUES (?1, ?2)
             ON CONFLICT(item_id) DO UPDATE SET pinned_at = excluded.pinned_at",
            params![item_id, now],
        )?;
        Ok(())
    }

    pub fn unpin_item(&mut self, item_id: &str) -> rusqlite::Result<()> {
        self.conn_mut().execute(
            "DELETE FROM pinned WHERE item_id = ?1",
            params![item_id],
        )?;
        Ok(())
    }

    /// 全部固定项 id，按固定时间降序（最近固定在前）。
    pub fn pinned_ids(&self) -> Vec<String> {
        let Ok(mut stmt) = self
            .conn()
            .prepare("SELECT item_id FROM pinned ORDER BY pinned_at DESC")
        else {
            return Vec::new();
        };
        stmt.query_map([], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::HistoryDb;

    fn temp_db() -> HistoryDb {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("kite-pins-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let nanos = std::time::SystemTime::now().elapsed().unwrap().as_nanos();
        let path = dir.join(format!("{nanos}-{n}.db"));
        HistoryDb::open(&path).expect("open temp db")
    }

    #[test]
    fn pin_unpin_roundtrip() {
        let mut db = temp_db();
        db.pin_item("app-a", 100).unwrap();
        db.pin_item("app-b", 200).unwrap();
        // 最近固定的在前
        assert_eq!(db.pinned_ids(), vec!["app-b".to_string(), "app-a".to_string()]);

        db.unpin_item("app-b").unwrap();
        assert_eq!(db.pinned_ids(), vec!["app-a".to_string()]);

        db.unpin_item("ghost").unwrap();
        assert_eq!(db.pinned_ids().len(), 1);
    }

    #[test]
    fn re_pin_moves_to_front() {
        let mut db = temp_db();
        db.pin_item("app-a", 100).unwrap();
        db.pin_item("app-b", 200).unwrap();
        db.pin_item("app-a", 300).unwrap();
        assert_eq!(db.pinned_ids(), vec!["app-a".to_string(), "app-b".to_string()]);
    }

    #[test]
    fn clear_history_keeps_pins() {
        let mut db = temp_db();
        db.pin_item("app-a", 100).unwrap();
        db.record_launch("app-a", "a", 100).unwrap();
        db.clear_history().unwrap();
        assert_eq!(db.pinned_ids(), vec!["app-a".to_string()]);
        assert!(db.recent_ids(10).unwrap().is_empty());
    }

    #[test]
    fn empty_and_blank_ids_ignored() {
        let mut db = temp_db();
        db.pin_item("", 100).unwrap();
        assert!(db.pinned_ids().is_empty());
    }

    #[test]
    fn settings_table_still_works() {
        let db = temp_db();
        let s = db.load_settings();
        assert!(!s.search_files);
    }
}
