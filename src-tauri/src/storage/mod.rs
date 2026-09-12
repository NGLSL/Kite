//! SQLite 历史库：启动频次、最近使用、Query→App 配对。
//! 手写 SQL，无 ORM。连接可被 `Mutex` 串行使用。

use std::path::Path;

use rusqlite::{params, Connection};

pub struct HistoryDb {
    conn: Connection,
}

/// 一条应用的历史加权原料。
#[derive(Debug, Default, Clone)]
pub struct UsageStats {
    pub launch_count: i64,
    pub last_used_at: i64,
}

/// Query→App 配对统计。`last_used_at` 预留调试与后续策略。
#[derive(Debug, Default, Clone)]
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
        Ok(Self { conn })
    }

    /// 记录一次启动；`query_norm` 为空则只记 Usage，不记 Query History。
    pub fn record_launch(&mut self, item_id: &str, query_norm: &str, now: i64) -> rusqlite::Result<()> {
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

    pub fn usage(&self, item_id: &str) -> rusqlite::Result<UsageStats> {
        self.conn
            .query_row(
                "SELECT launch_count, last_used_at FROM usage_history WHERE item_id = ?1",
                params![item_id],
                |row| {
                    Ok(UsageStats {
                        launch_count: row.get(0)?,
                        last_used_at: row.get(1)?,
                    })
                },
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(UsageStats::default()),
                other => Err(other),
            })
    }

    pub fn query_pair(&self, query_norm: &str, item_id: &str) -> rusqlite::Result<QueryPairStats> {
        self.conn
            .query_row(
                "SELECT count, last_used_at FROM query_history
                 WHERE query = ?1 AND item_id = ?2",
                params![query_norm, item_id],
                |row| {
                    Ok(QueryPairStats {
                        count: row.get(0)?,
                        last_used_at: row.get(1)?,
                    })
                },
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(QueryPairStats::default()),
                other => Err(other),
            })
    }
}

/// 秒级时间戳。
pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
