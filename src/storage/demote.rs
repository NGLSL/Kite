//! 用户降权：对某入口表达「别再推到前面」，可恢复。
//! 只影响排序，不删除索引项、不改启动目标。

use rusqlite::params;

use crate::storage::HistoryDb;

/// 降权扣分：须 > HISTORY_BOOST_MAX(160)，避免被历史正向偏好轻易抵消；
/// 仍限制在非保护层，不把 Name Exact 挤出前排。
pub const DEMOTE_PENALTY: i32 = 180;

impl HistoryDb {
    pub fn ensure_demote_schema(&mut self) -> rusqlite::Result<()> {
        self.conn_mut().execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS demoted (
                item_id TEXT PRIMARY KEY,
                demoted_at INTEGER NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn demote_item(&mut self, item_id: &str, now: i64) -> rusqlite::Result<()> {
        if item_id.is_empty() {
            return Ok(());
        }
        self.conn_mut().execute(
            "INSERT INTO demoted (item_id, demoted_at) VALUES (?1, ?2)
             ON CONFLICT(item_id) DO UPDATE SET demoted_at = excluded.demoted_at",
            params![item_id, now],
        )?;
        Ok(())
    }

    pub fn undemote_item(&mut self, item_id: &str) -> rusqlite::Result<()> {
        self.conn_mut()
            .execute("DELETE FROM demoted WHERE item_id = ?1", params![item_id])?;
        Ok(())
    }

    pub fn demoted_ids(&self) -> Vec<String> {
        let Ok(mut stmt) = self
            .conn()
            .prepare("SELECT item_id FROM demoted ORDER BY demoted_at DESC")
        else {
            return Vec::new();
        };
        stmt.query_map([], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    pub fn is_demoted(&self, item_id: &str) -> bool {
        self.conn()
            .query_row(
                "SELECT 1 FROM demoted WHERE item_id = ?1",
                params![item_id],
                |_| Ok(()),
            )
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::HistoryDb;

    fn temp_db() -> HistoryDb {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("kite-demote-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let nanos = std::time::SystemTime::now().elapsed().unwrap().as_nanos();
        let path = dir.join(format!("{nanos}-{n}.db"));
        HistoryDb::open(&path).expect("open temp db")
    }

    #[test]
    fn demote_undemote_roundtrip() {
        let mut db = temp_db();
        db.demote_item("app-a", 100).unwrap();
        assert!(db.is_demoted("app-a"));
        assert!(!db.is_demoted("app-b"));
        db.undemote_item("app-a").unwrap();
        assert!(!db.is_demoted("app-a"));
    }
}
