//! 设置与用户 Alias（SQLite）。
//! 表结构随 HistoryDb 一起演进，避免再引入第二套存储。

use rusqlite::{params, Connection};

use crate::storage::HistoryDb;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub hide_on_blur: bool,
    pub autostart: bool,
    pub max_results: i64,
    /// 可解析的快捷键，如 Alt+Space。
    pub hotkey: String,
    pub hotkey_label: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hide_on_blur: true,
            autostart: false,
            max_results: 10,
            hotkey: crate::system::hotkey::DEFAULT_HOTKEY.into(),
            hotkey_label: crate::system::hotkey::DEFAULT_HOTKEY.into(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UserAlias {
    pub alias: String,
    pub target_name: String,
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
        if let Ok(v) = self.get_setting("max_results") {
            if let Ok(n) = v.parse() {
                s.max_results = n;
            }
        }
        if let Ok(v) = self.get_setting("hotkey") {
            if crate::system::hotkey::parse_hotkey(&v).is_some() {
                s.hotkey_label = crate::system::hotkey::display_label(&v);
                s.hotkey = v;
            }
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

    pub fn set_alias(&mut self, alias: &str, target_name: &str) -> rusqlite::Result<()> {
        let a = alias.trim().to_lowercase();
        if a.is_empty() || target_name.trim().is_empty() {
            return Ok(());
        }
        self.conn_mut().execute(
            "INSERT INTO user_aliases (alias, target_name) VALUES (?1, ?2)
             ON CONFLICT(alias) DO UPDATE SET target_name = excluded.target_name",
            params![a, target_name.trim()],
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
        let mut stmt = self
            .conn()
            .prepare("SELECT alias, target_name FROM user_aliases ORDER BY alias")?;
        let rows = stmt.query_map([], |r| {
            Ok(UserAlias {
                alias: r.get(0)?,
                target_name: r.get(1)?,
            })
        })?;
        rows.collect()
    }

    /// Query（已 normalize）→ 目标应用名列表。
    pub fn alias_targets(&self, query_norm: &str) -> Vec<String> {
        self.conn()
            .query_row(
                "SELECT target_name FROM user_aliases WHERE alias = ?1",
                params![query_norm],
                |r| r.get::<_, String>(0),
            )
            .into_iter()
            .collect()
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
