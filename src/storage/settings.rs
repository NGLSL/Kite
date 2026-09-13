//! 设置与用户 Alias（SQLite）。
//! 表结构随 HistoryDb 一起演进，避免再引入第二套存储。

use rusqlite::{params, Connection};

use crate::storage::HistoryDb;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub hide_on_blur: bool,
    pub autostart: bool,
    /// 可解析的快捷键，如 Alt+Space。
    pub hotkey: String,
    pub hotkey_label: String,
    /// 搜索时是否带上 Everything 文件结果；默认关闭，选择要记住。
    pub search_files: bool,
    /// 是否记录启动历史（Usage + Query History）；暂停后 record 直接跳过。
    pub history_recording: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hide_on_blur: true,
            autostart: false,
            hotkey: crate::system::hotkey::DEFAULT_HOTKEY.into(),
            hotkey_label: crate::system::hotkey::DEFAULT_HOTKEY.into(),
            search_files: false,
            history_recording: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UserAlias {
    pub alias: String,
    pub target_name: String,
    /// 稳定目标标识（AppItem id）；旧数据仅有 target_name，为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

impl HistoryDb {
    /// 初始化 settings / user_aliases 表。
    pub fn ensure_schema(&mut self) -> rusqlite::Result<()> {
        self.conn_mut().execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS user_aliases (
                alias TEXT PRIMARY KEY,
                target_name TEXT NOT NULL
            );
            "#,
        )?;
        // 旧库没有 target_id 列；已存在时 ALTER 报错属预期，忽略即可
        let _ = self.conn_mut().execute(
            "ALTER TABLE user_aliases ADD COLUMN target_id TEXT",
            [],
        );
        Ok(())
    }

    pub fn load_settings(&self) -> Settings {
        let mut s = Settings::default();
        if let Ok(v) = self.get_setting("hide_on_blur") {
            s.hide_on_blur = v != "0";
        }
        if let Ok(v) = self.get_setting("autostart") {
            s.autostart = v == "1";
        }
        if let Ok(v) = self.get_setting("hotkey") {
            if crate::system::hotkey::parse_raw(&v).is_some() {
                s.hotkey_label = crate::system::hotkey::display_label(&v);
                s.hotkey = v;
            }
        }
        if let Ok(v) = self.get_setting("search_files") {
            s.search_files = v == "1";
        }
        if let Ok(v) = self.get_setting("history_recording") {
            s.history_recording = v != "0";
        }
        s
    }

    pub fn save_setting(&mut self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn_mut().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> rusqlite::Result<String> {
        self.conn().query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
    }

    /// 保存用户 Alias。`target_id` 为新数据必填（由 commands 层先校验存在），
    /// None 仅用于兼容旧调用路径；target_name 始终保存规范显示名。
    pub fn set_alias(
        &mut self,
        alias: &str,
        target_id: Option<&str>,
        target_name: &str,
    ) -> rusqlite::Result<()> {
        let a = alias.trim().to_lowercase();
        if a.is_empty() || target_name.trim().is_empty() {
            return Ok(());
        }
        self.conn_mut().execute(
            "INSERT INTO user_aliases (alias, target_name, target_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(alias) DO UPDATE SET
               target_name = excluded.target_name,
               target_id = excluded.target_id",
            params![a, target_name.trim(), target_id],
        )?;
        Ok(())
    }

    pub fn remove_alias(&mut self, alias: &str) -> rusqlite::Result<()> {
        self.conn_mut().execute(
            "DELETE FROM user_aliases WHERE alias = ?1",
            params![alias.trim().to_lowercase()],
        )?;
        Ok(())
    }

    pub fn list_aliases(&self) -> rusqlite::Result<Vec<UserAlias>> {
        let mut stmt = self.conn().prepare(
            "SELECT alias, target_name, target_id FROM user_aliases ORDER BY alias",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(UserAlias {
                alias: r.get(0)?,
                target_name: r.get(1)?,
                target_id: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Query（已 normalize）→ 匹配到的用户 Alias 行（通常 0 或 1 条）。
    pub fn alias_matches(&self, query_norm: &str) -> Vec<UserAlias> {
        self.conn()
            .query_row(
                "SELECT alias, target_name, target_id FROM user_aliases WHERE alias = ?1",
                params![query_norm],
                |r| {
                    Ok(UserAlias {
                        alias: r.get(0)?,
                        target_name: r.get(1)?,
                        target_id: r.get(2)?,
                    })
                },
            )
            .into_iter()
            .collect()
    }

    /// 是否记录启动历史；未配置视为开启。
    pub fn history_recording_enabled(&self) -> bool {
        self.get_setting("history_recording")
            .map(|v| v != "0")
            .unwrap_or(true)
    }

    /// 偏好浏览器 id（chrome/edge/…）；未设置为 None。
    pub fn preferred_browser(&self) -> Option<String> {
        self.get_setting("preferred_browser")
            .ok()
            .filter(|s| !s.is_empty())
    }

    /// 记住用户选过的浏览器，下次优先展示。
    pub fn set_preferred_browser(&mut self, browser_id: &str) -> rusqlite::Result<()> {
        if browser_id.is_empty() {
            return Ok(());
        }
        self.save_setting("preferred_browser", browser_id)
    }

    /// 已缓存的搜索引擎 URL 模板（含 {searchTerms}）。
    pub fn search_url_template(&self) -> Option<String> {
        self.get_setting("search_url_template")
            .ok()
            .filter(|s| !s.is_empty())
    }

    pub fn set_search_url_template(&mut self, template: &str) -> rusqlite::Result<()> {
        if template.is_empty() {
            return Ok(());
        }
        self.save_setting("search_url_template", template)
    }

    pub(crate) fn conn(&self) -> &Connection {
        self.raw_conn()
    }

    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        self.raw_conn_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> HistoryDb {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("kite-settings-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let nanos = std::time::SystemTime::now().elapsed().unwrap().as_nanos();
        let path = dir.join(format!("{nanos}-{n}.db"));
        HistoryDb::open(&path).expect("open temp db")
    }

    #[test]
    fn new_settings_defaults() {
        let db = temp_db();
        let s = db.load_settings();
        assert!(!s.search_files, "搜文件默认关闭");
        assert!(s.history_recording, "历史记录默认开启");
    }

    #[test]
    fn settings_roundtrip_for_new_fields() {
        let mut db = temp_db();
        db.save_setting("search_files", "1").unwrap();
        db.save_setting("history_recording", "0").unwrap();
        let s = db.load_settings();
        assert!(s.search_files);
        assert!(!s.history_recording);
    }

    #[test]
    fn alias_saves_and_reads_target_id() {
        let mut db = temp_db();
        db.set_alias("code", Some("app-123"), "Visual Studio Code").unwrap();
        let rows = db.list_aliases().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target_id.as_deref(), Some("app-123"));
        assert_eq!(rows[0].target_name, "Visual Studio Code");

        let hit = db.alias_matches("code");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].target_id.as_deref(), Some("app-123"));
        assert!(db.alias_matches("other").is_empty());
    }

    #[test]
    fn legacy_alias_without_target_id_still_listed() {
        let mut db = temp_db();
        db.set_alias("wx", None, "微信").unwrap();
        let rows = db.list_aliases().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target_id, None);
        // 旧数据按名称仍可命中
        assert_eq!(db.alias_matches("wx")[0].target_name, "微信");
    }

    #[test]
    fn alias_upsert_replaces_target() {
        let mut db = temp_db();
        db.set_alias("wx", Some("a1"), "微信").unwrap();
        db.set_alias("wx", Some("a2"), "企业微信").unwrap();
        let rows = db.list_aliases().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target_id.as_deref(), Some("a2"));
        assert_eq!(rows[0].target_name, "企业微信");
    }

    #[test]
    fn migration_survives_reopen() {
        // 模拟旧库（无 target_id）→ 打开后应补列且数据保留
        let dir = std::env::temp_dir().join(format!("kite-migrate-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let nanos = std::time::SystemTime::now().elapsed().unwrap().as_nanos();
        let path = dir.join(format!("migrate-{nanos}.db"));
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE user_aliases (alias TEXT PRIMARY KEY, target_name TEXT NOT NULL);
                 INSERT INTO user_aliases VALUES ('old', '旧应用');",
            )
            .unwrap();
        }
        let db = HistoryDb::open(&path).expect("reopen legacy db");
        let rows = db.list_aliases().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].alias, "old");
        assert_eq!(rows[0].target_id, None);
    }
}
