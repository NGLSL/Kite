//! Durable live-overlay state, committed together with the journal cursors.

use super::{invalid, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;

const SHADOW_PAGE_BYTES: usize = 4096;

pub(super) struct Store {
    pub conn: Connection,
    previous_shadow: Vec<u8>,
}

impl Store {
    pub fn open(path: &Path, binding: &Value, shadow_bytes: usize) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA cache_size = -8192;
             PRAGMA mmap_size = 0;
             PRAGMA temp_store = FILE;
             PRAGMA auto_vacuum = INCREMENTAL;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS objects (
                 volume INTEGER NOT NULL,
                 frn BLOB NOT NULL,
                 is_dir INTEGER NOT NULL,
                 state TEXT NOT NULL,
                 PRIMARY KEY (volume, frn)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS prefixes (
                 volume INTEGER NOT NULL,
                 base BLOB NOT NULL,
                 current BLOB NOT NULL,
                 PRIMARY KEY (volume, base)
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS staging (
                 ordinal INTEGER PRIMARY KEY,
                 change TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS metadata (
                 key TEXT PRIMARY KEY,
                 value TEXT
             );
             CREATE TABLE IF NOT EXISTS shadow (
                 page INTEGER PRIMARY KEY,
                 data BLOB
             );",
        )?;
        let stored_binding: Option<String> = conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'binding'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(stored_binding) = stored_binding {
            if serde_json::from_str::<Value>(&stored_binding)? != *binding {
                return Err(invalid("live overlay snapshot binding mismatch").into());
            }
            let stored_bytes: String = conn.query_row(
                "SELECT value FROM metadata WHERE key = 'shadow_bytes'",
                [],
                |r| r.get(0),
            )?;
            if stored_bytes.parse::<usize>()? != shadow_bytes {
                return Err(invalid("live overlay shadow length mismatch").into());
            }
        } else {
            // Bind a new database before it can receive any overlay rows.
            conn.execute_batch("BEGIN IMMEDIATE")?;
            let nonempty: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM objects) OR EXISTS(SELECT 1 FROM prefixes)
                 OR EXISTS(SELECT 1 FROM shadow) OR EXISTS(SELECT 1 FROM staging)
                 OR EXISTS(SELECT 1 FROM metadata)",
                [],
                |row| row.get(0),
            )?;
            if nonempty {
                conn.execute_batch("ROLLBACK")?;
                return Err(invalid("unbound live overlay contains existing state").into());
            }
            conn.execute(
                "INSERT INTO metadata (key, value) VALUES ('binding', ?1)",
                [serde_json::to_string(binding)?],
            )?;
            conn.execute(
                "INSERT INTO metadata (key, value) VALUES ('shadow_bytes', ?1)",
                [shadow_bytes.to_string()],
            )?;
            conn.execute_batch("COMMIT")?;
        }
        // A staged round has no effect until its overlay and cursors commit.
        conn.execute("DELETE FROM staging", [])?;
        let mut store = Self {
            conn,
            previous_shadow: Vec::new(),
        };
        store.previous_shadow = store.load_shadow(shadow_bytes)?;
        Ok(store)
    }

    /// A query connection never initializes schema or discards staged rounds.
    pub fn open_reader(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch(
            "PRAGMA query_only = ON;
             PRAGMA cache_size = -4096;
             PRAGMA mmap_size = 0;
             PRAGMA temp_store = FILE;",
        )?;
        Ok(Self {
            conn,
            previous_shadow: Vec::new(),
        })
    }

    pub fn begin(&mut self) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(())
    }

    pub fn commit(&mut self, cursors: &Value, shadow: &[u8]) -> Result<()> {
        if self.conn.is_autocommit() {
            return Err(invalid("live overlay commit without active transaction").into());
        }
        if shadow.len() != self.previous_shadow.len() {
            return Err(invalid("live overlay shadow length mismatch").into());
        }
        {
            let mut save = self.conn.prepare_cached(
                "INSERT INTO shadow (page, data) VALUES (?1, ?2)
                 ON CONFLICT(page) DO UPDATE SET data = excluded.data",
            )?;
            for (page, (current, previous)) in shadow
                .chunks(SHADOW_PAGE_BYTES)
                .zip(self.previous_shadow.chunks(SHADOW_PAGE_BYTES))
                .enumerate()
            {
                if current != previous {
                    save.execute(params![i64::try_from(page)?, current])?;
                }
            }
        }
        self.conn.execute(
            "INSERT INTO metadata (key, value) VALUES ('cursors', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(cursors)?],
        )?;
        self.conn.execute("DELETE FROM staging", [])?;
        self.conn.execute_batch("COMMIT")?;
        self.previous_shadow.copy_from_slice(shadow);
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<()> {
        if !self.conn.is_autocommit() {
            self.conn.execute_batch("ROLLBACK")?;
        }
        Ok(())
    }

    pub fn cursors(&self) -> Result<Option<Value>> {
        let saved: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'cursors'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        saved
            .map(|value| Ok(serde_json::from_str(&value)?))
            .transpose()
    }

    pub fn load_shadow(&self, bytes: usize) -> Result<Vec<u8>> {
        let mut shadow = vec![0; bytes];
        let mut statement = self
            .conn
            .prepare("SELECT page, data FROM shadow ORDER BY page")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let page = usize::try_from(row.get::<_, i64>(0)?)?;
            let offset = page
                .checked_mul(SHADOW_PAGE_BYTES)
                .ok_or_else(|| invalid("live overlay shadow page overflow"))?;
            let data = row.get_ref(1)?.as_blob()?;
            let expected = bytes.saturating_sub(offset).min(SHADOW_PAGE_BYTES);
            if offset >= bytes || data.len() != expected {
                return Err(invalid("invalid live overlay shadow page").into());
            }
            shadow[offset..offset + expected].copy_from_slice(data);
        }
        Ok(shadow)
    }

    pub fn maintenance(&mut self) -> Result<()> {
        if !self.conn.is_autocommit() {
            return Err(invalid("live overlay maintenance during active transaction").into());
        }
        self.conn.execute_batch(
            "PRAGMA incremental_vacuum(64);
             PRAGMA wal_checkpoint(TRUNCATE);",
        )?;
        Ok(())
    }
}
