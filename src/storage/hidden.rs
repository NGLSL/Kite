//! Hidden application entry preferences.
//!
//! Hidden state is keyed by the stable launch identity (`AppItem::id`) and
//! keeps enough display metadata for the settings list even when an index
//! snapshot no longer contains the target.

use std::collections::HashSet;

use rusqlite::params;

use crate::model::AppItem;
use crate::storage::HistoryDb;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenItem {
    pub item_id: String,
    pub display_name: String,
    pub target: String,
    pub args: Option<String>,
    pub hidden_at: i64,
}

impl HistoryDb {
    /// Hide one stable application launch identity while retaining management
    /// metadata for the settings view.
    pub fn hide_item(&mut self, item: &AppItem, now: i64) -> rusqlite::Result<()> {
        if item.id.trim().is_empty() {
            return Ok(());
        }
        self.conn_mut().execute(
            "INSERT INTO hidden_items (item_id, display_name, target, args, hidden_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(item_id) DO UPDATE SET
               display_name = excluded.display_name,
               target = excluded.target,
               args = excluded.args,
               hidden_at = excluded.hidden_at",
            params![
                item.id,
                if item.display_name.trim().is_empty() {
                    &item.name
                } else {
                    &item.display_name
                },
                item.target,
                item.args,
                now
            ],
        )?;
        Ok(())
    }

    pub fn restore_item(&mut self, item_id: &str) -> rusqlite::Result<()> {
        if item_id.trim().is_empty() {
            return Ok(());
        }
        self.conn_mut().execute(
            "DELETE FROM hidden_items WHERE item_id = ?1",
            params![item_id],
        )?;
        Ok(())
    }

    pub fn hidden_ids(&self) -> rusqlite::Result<HashSet<String>> {
        let mut stmt = self.conn().prepare("SELECT item_id FROM hidden_items")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn list_hidden_items(&self) -> rusqlite::Result<Vec<HiddenItem>> {
        let mut stmt = self.conn().prepare(
            "SELECT item_id, display_name, target, args, hidden_at
             FROM hidden_items
             ORDER BY hidden_at DESC, item_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(HiddenItem {
                item_id: row.get(0)?,
                display_name: row.get(1)?,
                target: row.get(2)?,
                args: row.get(3)?,
                hidden_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::AppItem;
    use crate::storage::HistoryDb;

    fn temp_path() -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!("kite-hidden-{}", std::process::id()))
            .join(format!("{n}.db"))
    }

    fn app(id: &str) -> AppItem {
        AppItem::scanned(
            id.into(),
            "Display Name".into(),
            r"C:\Apps\sample.exe".into(),
            Some("--profile test".into()),
            None,
            "test",
        )
    }

    #[test]
    fn hide_list_restore_roundtrip_survives_reopen() {
        let path = temp_path();
        {
            let mut db = HistoryDb::open(&path).unwrap();
            db.hide_item(&app("sample"), 100).unwrap();
            let rows = db.list_hidden_items().unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].item_id, "sample");
            assert_eq!(rows[0].args.as_deref(), Some("--profile test"));
            assert!(db.hidden_ids().unwrap().contains("sample"));
        }
        {
            let mut db = HistoryDb::open(&path).unwrap();
            assert_eq!(db.list_hidden_items().unwrap().len(), 1);
            db.restore_item("sample").unwrap();
            assert!(db.hidden_ids().unwrap().is_empty());
        }
    }
}
