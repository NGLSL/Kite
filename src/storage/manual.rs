//! Persistent records for explicitly registered application entries.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualApp {
    pub id: i64,
    pub path: String,
    pub display_name: String,
    pub item_id: String,
}

use rusqlite::params;

use crate::storage::HistoryDb;

fn normalize_path_key(path: &str) -> String {
    path.trim().replace('/', "\\").to_lowercase()
}

fn invalid_parameter(name: &'static str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(name.to_string())
}

impl HistoryDb {
    pub fn list_manual_apps(&self) -> rusqlite::Result<Vec<ManualApp>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, path, display_name, item_id
             FROM manual_apps
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ManualApp {
                id: row.get(0)?,
                path: row.get(1)?,
                display_name: row.get(2)?,
                item_id: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Save a manual entry without doing filesystem validation. The scanner
    /// owns validation; storage only persists the already verified launch
    /// identity and user-facing name.
    pub fn save_manual_app(
        &mut self,
        path: &str,
        display_name: &str,
        item_id: &str,
    ) -> rusqlite::Result<i64> {
        let path = path.trim();
        let display_name = display_name.trim();
        let item_id = item_id.trim();
        if path.is_empty() {
            return Err(invalid_parameter("path"));
        }
        if display_name.is_empty() {
            return Err(invalid_parameter("display_name"));
        }
        if item_id.is_empty() {
            return Err(invalid_parameter("item_id"));
        }
        let path_key = normalize_path_key(path);

        // A stable launch identity is the primary duplicate key. Path is also
        // checked so aliases differing only in slash/case cannot duplicate a
        // record when a caller supplies a stale identity.
        let existing: Option<i64> = match self.conn().query_row(
            "SELECT id FROM manual_apps WHERE item_id = ?1 ORDER BY id ASC LIMIT 1",
            params![item_id],
            |row| row.get(0),
        ) {
            Ok(value) => Some(value),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(error) => return Err(error),
        };
        if let Some(id) = existing {
            return Ok(id);
        }
        let existing: Option<i64> = match self.conn().query_row(
            "SELECT id FROM manual_apps WHERE path_key = ?1 ORDER BY id ASC LIMIT 1",
            params![path_key],
            |row| row.get(0),
        ) {
            Ok(value) => Some(value),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(error) => return Err(error),
        };
        if let Some(id) = existing {
            return Ok(id);
        }

        self.conn_mut().execute(
            "INSERT INTO manual_apps (path, path_key, display_name, item_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![path, path_key, display_name, item_id],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Refresh identities after validating the current targets of registered
    /// entries. This deliberately updates only `manual_apps.item_id`; launch
    /// history and all other preferences stay keyed to their original id.
    /// An empty identity represents an entry that is currently invalid.
    pub fn refresh_manual_app_identities(
        &mut self,
        identities: &[(i64, String)],
    ) -> rusqlite::Result<()> {
        if identities.is_empty() {
            return Ok(());
        }
        let tx = self.conn_mut().transaction()?;
        for (id, item_id) in identities {
            tx.execute(
                "UPDATE manual_apps SET item_id = ?1 WHERE id = ?2",
                params![item_id.trim(), id],
            )?;
        }
        tx.commit()
    }

    pub fn rename_manual_app(&mut self, id: i64, display_name: &str) -> rusqlite::Result<()> {
        let display_name = display_name.trim();
        if display_name.is_empty() {
            return Err(invalid_parameter("display_name"));
        }
        self.conn_mut().execute(
            "UPDATE manual_apps SET display_name = ?1 WHERE id = ?2",
            params![display_name, id],
        )?;
        Ok(())
    }

    pub fn remove_manual_app(&mut self, id: i64) -> rusqlite::Result<()> {
        self.conn_mut()
            .execute("DELETE FROM manual_apps WHERE id = ?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::storage::HistoryDb;

    fn temp_path() -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!("kite-manual-{}", std::process::id()))
            .join(format!("{n}.db"))
    }

    #[test]
    fn manual_app_roundtrip_deduplicates_path_and_identity() {
        let path = temp_path();
        let mut db = HistoryDb::open(&path).unwrap();
        let id = db
            .save_manual_app(r"C:\Apps\Demo.exe", "Demo", "stable-a")
            .unwrap();
        assert_eq!(
            db.save_manual_app(r"c:/apps/demo.exe", "Changed", "stable-b")
                .unwrap(),
            id
        );
        assert_eq!(
            db.save_manual_app(r"D:\Other\demo.exe", "Other", "stable-a")
                .unwrap(),
            id
        );
        let rows = db.list_manual_apps().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, r"C:\Apps\Demo.exe");
        assert_eq!(rows[0].display_name, "Demo");
        assert_eq!(rows[0].item_id, "stable-a");

        db.rename_manual_app(id, "Renamed").unwrap();
        assert_eq!(db.list_manual_apps().unwrap()[0].display_name, "Renamed");
        db.remove_manual_app(id).unwrap();
        assert!(db.list_manual_apps().unwrap().is_empty());
    }

    #[test]
    fn refresh_identity_allows_old_registration_and_keeps_preferences_on_old_id() {
        let path = temp_path();
        let mut db = HistoryDb::open(&path).unwrap();
        let old_id = db
            .save_manual_app(r"C:\Apps\Demo.lnk", "Demo", "stable-old")
            .unwrap();
        db.record_launch("stable-old", "demo", 10).unwrap();
        db.pin_item("stable-old", 11).unwrap();
        db.demote_item("stable-old", 12).unwrap();
        db.set_alias("demo alias", Some("stable-old"), "Demo")
            .unwrap();
        db.hide_item(
            &crate::model::AppItem::scanned(
                "stable-old".into(),
                "Demo".into(),
                r"C:\Apps\Demo.lnk".into(),
                None,
                None,
                "manual",
            ),
            13,
        )
        .unwrap();

        db.refresh_manual_app_identities(&[(old_id, "stable-new".into())])
            .unwrap();
        let new_old_registration = db
            .save_manual_app(r"C:\Apps\OldTarget.exe", "Old target", "stable-old")
            .unwrap();
        assert_ne!(new_old_registration, old_id);
        assert_eq!(
            db.save_manual_app(r"C:/Apps/Demo.lnk", "Current target", "stable-new")
                .unwrap(),
            old_id
        );

        let rows = db.list_manual_apps().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, r"C:\Apps\Demo.lnk");
        assert_eq!(rows[0].display_name, "Demo");
        assert_eq!(rows[0].item_id, "stable-new");
        assert_eq!(rows[1].path, r"C:\Apps\OldTarget.exe");
        assert_eq!(rows[1].item_id, "stable-old");
        assert_eq!(
            db.usage_snapshot(&["stable-old".into()])["stable-old"].launch_count,
            1
        );
        assert!(db.usage_snapshot(&["stable-new".into()]).is_empty());
        assert_eq!(db.pinned_ids(), vec!["stable-old"]);
        assert!(db.is_demoted("stable-old"));
        assert!(!db.is_demoted("stable-new"));
        assert!(db.hidden_ids().unwrap().contains("stable-old"));
        assert!(!db.hidden_ids().unwrap().contains("stable-new"));
        assert_eq!(
            db.list_aliases().unwrap()[0].target_id.as_deref(),
            Some("stable-old")
        );
    }

    #[test]
    fn refresh_empty_identity_does_not_block_new_registration() {
        let path = temp_path();
        let mut db = HistoryDb::open(&path).unwrap();
        let id = db
            .save_manual_app(r"C:\Apps\Invalid.lnk", "Invalid", "stable-old")
            .unwrap();
        db.refresh_manual_app_identities(&[(id, String::new())])
            .unwrap();
        let replacement = db
            .save_manual_app(r"C:\Apps\Recovered.exe", "Recovered", "stable-new")
            .unwrap();
        assert_ne!(replacement, id);
        assert_eq!(db.list_manual_apps().unwrap()[0].item_id, "");
    }

    #[test]
    fn refresh_identity_failure_rolls_back_all_rows() {
        let path = temp_path();
        let mut db = HistoryDb::open(&path).unwrap();
        let first = db
            .save_manual_app(r"C:\Apps\First.lnk", "First", "old-first")
            .unwrap();
        let second = db
            .save_manual_app(r"C:\Apps\Second.lnk", "Second", "old-second")
            .unwrap();
        db.raw_conn_mut()
            .execute(
                &format!(
                    "CREATE TRIGGER fail_manual_identity_refresh
                     BEFORE UPDATE OF item_id ON manual_apps
                     WHEN NEW.id = {second}
                     BEGIN SELECT RAISE(ABORT, 'injected refresh failure'); END;"
                ),
                [],
            )
            .unwrap();

        assert!(db
            .refresh_manual_app_identities(&[
                (first, "new-first".into()),
                (second, "new-second".into()),
            ])
            .is_err());
        let rows = db.list_manual_apps().unwrap();
        assert_eq!(rows[0].item_id, "old-first");
        assert_eq!(rows[1].item_id, "old-second");
    }
}
