use rusqlite::{Connection, params};
use std::path::Path;

use crate::model::{ClipboardContent, ClipboardItem, CoreError};

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(path: &str) -> Result<Self, CoreError> {
        let conn = if path.is_empty() || path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            let parent = Path::new(path).parent();
            if let Some(dir) = parent {
                std::fs::create_dir_all(dir).map_err(|e| CoreError::Storage {
                    msg: format!("Failed to create database directory: {}", e),
                })?;
            }
            Connection::open(path)?
        };
        let storage = Storage { conn };
        storage.run_migrations()?;
        Ok(storage)
    }

    pub fn open_in_memory() -> Result<Self, CoreError> {
        Self::open(":memory:")
    }

    fn run_migrations(&self) -> Result<(), CoreError> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY);",
            )
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;

        let current_version: u32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 1 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS history_items (
                    id TEXT PRIMARY KEY,
                    application TEXT,
                    first_copied_at INTEGER NOT NULL,
                    last_copied_at INTEGER NOT NULL,
                    number_of_copies INTEGER NOT NULL DEFAULT 1,
                    pin TEXT,
                    title TEXT NOT NULL DEFAULT '',
                    sync_timestamp INTEGER NOT NULL,
                    sync_source TEXT,
                    sync_deleted INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS history_contents (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    item_id TEXT NOT NULL,
                    content_type TEXT NOT NULL,
                    value BLOB,
                    FOREIGN KEY (item_id) REFERENCES history_items(id) ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_contents_item_id ON history_contents(item_id);
                CREATE INDEX IF NOT EXISTS idx_items_last_copied ON history_items(last_copied_at DESC);
                CREATE INDEX IF NOT EXISTS idx_items_pin ON history_items(pin);

                INSERT INTO schema_version (version) VALUES (1);
                ",
                )
                .map_err(|e| CoreError::Storage {
                    msg: e.to_string(),
                })?;
        }

        if current_version < 2 {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS sync_pairs (
                    peer_id TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    is_admin INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO schema_version (version) VALUES (2);",
                )
                .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        }

        if current_version < 3 {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS sync_config (
                    key TEXT PRIMARY KEY,
                    value BLOB
                );
                INSERT INTO schema_version (version) VALUES (3);",
                )
                .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        }

        if current_version < 4 {
            // Fix: older V2 migration created sync_pairs without is_admin column.
            // Add the column for databases that already ran the old V2.
            self.conn
                .execute(
                    "ALTER TABLE sync_pairs ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .or_else(|e| {
                    // Column may already exist (fresh DB with fixed V2) — not an error
                    if e.to_string().contains("duplicate column name") {
                        Ok(0)
                    } else {
                        Err(e)
                    }
                })
                .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (4)", [])
                .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        }

        Ok(())
    }

    // ── Sync pairs ────────────────────────────────────────────

    pub fn get_paired_peers(&self) -> Result<Vec<(String, String, bool)>, CoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT peer_id, display_name, is_admin != 0 FROM sync_pairs ORDER BY display_name",
            )
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| CoreError::Storage { msg: e.to_string() })?);
        }
        Ok(result)
    }

    pub fn save_paired_peer(
        &self,
        peer_id: &str,
        display_name: &str,
        is_admin: bool,
    ) -> Result<(), CoreError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO sync_pairs (peer_id, display_name, is_admin) VALUES (?1, ?2, ?3)",
                rusqlite::params![peer_id, display_name, is_admin as i32],
            )
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        Ok(())
    }

    pub fn promote_to_admin(&self, peer_id: &str) -> Result<(), CoreError> {
        self.conn
            .execute(
                "UPDATE sync_pairs SET is_admin = 1 WHERE peer_id = ?1",
                rusqlite::params![peer_id],
            )
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        Ok(())
    }

    pub fn am_admin(&self) -> bool {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sync_pairs", [], |row| row.get(0))
            .unwrap_or(0);
        count == 0 || true
    }

    pub fn remove_paired_peer(&self, peer_id: &str) -> Result<(), CoreError> {
        self.conn
            .execute(
                "DELETE FROM sync_pairs WHERE peer_id = ?1",
                rusqlite::params![peer_id],
            )
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        Ok(())
    }

    // ── Sync config ───────────────────────────────────────────

    pub fn get_config(&self, key: &str) -> Option<Vec<u8>> {
        self.conn
            .query_row(
                "SELECT value FROM sync_config WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .ok()
    }

    pub fn set_config(&self, key: &str, value: &[u8]) -> Result<(), CoreError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO sync_config (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        Ok(())
    }

    pub fn insert_item(&self, item: &ClipboardItem) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO history_items (id, application, first_copied_at, last_copied_at, number_of_copies, pin, title, sync_timestamp, sync_source, sync_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                item.id,
                item.application,
                item.first_copied_at,
                item.last_copied_at,
                item.number_of_copies,
                item.pin,
                item.title,
                item.sync_timestamp,
                item.sync_source,
                item.sync_deleted as i32,
            ],
        )?;

        for content in &item.contents {
            self.conn.execute(
                "INSERT INTO history_contents (item_id, content_type, value) VALUES (?1, ?2, ?3)",
                params![item.id, content.content_type, content.value.as_deref()],
            )?;
        }

        Ok(())
    }

    pub fn get_all_items(&self) -> Result<Vec<ClipboardItem>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, application, first_copied_at, last_copied_at, number_of_copies, pin, title, sync_timestamp, sync_source, sync_deleted
             FROM history_items WHERE sync_deleted = 0
             ORDER BY last_copied_at DESC",
        )?;

        let items = stmt
            .query_map([], |row| {
                Ok(ClipboardItem {
                    id: row.get(0)?,
                    application: row.get(1)?,
                    first_copied_at: row.get(2)?,
                    last_copied_at: row.get(3)?,
                    number_of_copies: row.get(4)?,
                    pin: row.get(5)?,
                    title: row.get(6)?,
                    contents: vec![], // populated below
                    sync_timestamp: row.get(7)?,
                    sync_source: row.get(8)?,
                    sync_deleted: row.get::<_, i32>(9)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Populate contents for each item
        let mut result = Vec::with_capacity(items.len());
        for mut item in items {
            item.contents = self.get_contents_for_item(&item.id)?;
            result.push(item);
        }

        Ok(result)
    }

    pub fn get_item(&self, id: &str) -> Result<Option<ClipboardItem>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, application, first_copied_at, last_copied_at, number_of_copies, pin, title, sync_timestamp, sync_source, sync_deleted
             FROM history_items WHERE id = ?1",
        )?;

        let item = stmt.query_row(params![id], |row| {
            Ok(ClipboardItem {
                id: row.get(0)?,
                application: row.get(1)?,
                first_copied_at: row.get(2)?,
                last_copied_at: row.get(3)?,
                number_of_copies: row.get(4)?,
                pin: row.get(5)?,
                title: row.get(6)?,
                contents: vec![],
                sync_timestamp: row.get(7)?,
                sync_source: row.get(8)?,
                sync_deleted: row.get::<_, i32>(9)? != 0,
            })
        });

        match item {
            Ok(mut item) => {
                item.contents = self.get_contents_for_item(id)?;
                Ok(Some(item))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Storage { msg: e.to_string() }),
        }
    }

    fn get_contents_for_item(&self, item_id: &str) -> Result<Vec<ClipboardContent>, CoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT content_type, value FROM history_contents WHERE item_id = ?1")?;

        let contents = stmt
            .query_map(params![item_id], |row| {
                Ok(ClipboardContent {
                    content_type: row.get(0)?,
                    value: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(contents)
    }

    pub fn delete_item(&self, id: &str) -> Result<(), CoreError> {
        let rows = self.conn.execute(
            "DELETE FROM history_contents WHERE item_id = ?1",
            params![id],
        )?;
        self.conn
            .execute("DELETE FROM history_items WHERE id = ?1", params![id])?;
        if rows == 0 {
            // contents deleted, check item
        }
        Ok(())
    }

    pub fn delete_unpinned(&self) -> Result<u64, CoreError> {
        // Delete contents first (foreign key cascade should handle this, but be explicit)
        self.conn.execute(
            "DELETE FROM history_contents WHERE item_id IN (SELECT id FROM history_items WHERE pin IS NULL)",
            [],
        )?;
        let rows = self
            .conn
            .execute("DELETE FROM history_items WHERE pin IS NULL", [])?;
        Ok(rows as u64)
    }

    pub fn delete_all(&self) -> Result<u64, CoreError> {
        self.conn.execute("DELETE FROM history_contents", [])?;
        let rows = self.conn.execute("DELETE FROM history_items", [])?;
        Ok(rows as u64)
    }

    pub fn update_item(&self, item: &ClipboardItem) -> Result<(), CoreError> {
        self.conn.execute(
            "UPDATE history_items SET application = ?2, first_copied_at = ?3, last_copied_at = ?4,
             number_of_copies = ?5, pin = ?6, title = ?7, sync_timestamp = ?8, sync_source = ?9, sync_deleted = ?10
             WHERE id = ?1",
            params![
                item.id,
                item.application,
                item.first_copied_at,
                item.last_copied_at,
                item.number_of_copies,
                item.pin,
                item.title,
                item.sync_timestamp,
                item.sync_source,
                item.sync_deleted as i32,
            ],
        )?;

        // Replace contents
        self.conn.execute(
            "DELETE FROM history_contents WHERE item_id = ?1",
            params![item.id],
        )?;
        for content in &item.contents {
            self.conn.execute(
                "INSERT INTO history_contents (item_id, content_type, value) VALUES (?1, ?2, ?3)",
                params![item.id, content.content_type, content.value.as_deref()],
            )?;
        }

        Ok(())
    }

    pub fn count_items(&self) -> Result<i64, CoreError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM history_items WHERE sync_deleted = 0",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn db_size_bytes(&self, path: &str) -> i64 {
        std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0)
    }

    /// Migrate data from SwiftData SQLite (reads Z_HISTORYITEM tables).
    pub fn migrate_from_swiftdata(&mut self, swiftdata_path: &str) -> Result<u64, CoreError> {
        if !Path::new(swiftdata_path).exists() {
            return Ok(0);
        }

        let sd_conn = Connection::open(swiftdata_path).map_err(|e| CoreError::Storage {
            msg: format!("Failed to open SwiftData DB: {}", e),
        })?;

        // Read SwiftData items. SwiftData uses Z_ prefixed table names.
        let mut item_stmt = sd_conn
            .prepare(
                "SELECT Z_PK, ZAPPLICATION, ZFIRSTCOPIEDAT, ZLASTCOPIEDAT, ZNUMBEROFCOPIES,
                    ZPIN, ZTITLE, ZSYNCID, ZSYNCTIMESTAMP, ZSYNCSOURCE, ZSYNCDELETED
             FROM Z_HISTORYITEM",
            )
            .map_err(|e| CoreError::Storage {
                msg: format!("SwiftData query failed: {}", e),
            })?;

        let swiftdata_items = item_stmt
            .query_map([], |row| {
                // SwiftData stores dates as Core Data timestamps (seconds since 2001-01-01)
                // We need to convert to Unix epoch millis
                let first_copied: f64 = row.get::<_, f64>(2)?;
                let last_copied: f64 = row.get::<_, f64>(3)?;
                let sync_ts: f64 = row.get::<_, f64>(8)?;

                // Core Data epoch: 2001-01-01 00:00:00 UTC = 978307200 Unix seconds
                let cd_epoch: f64 = 978307200.0;
                let first_ms = ((first_copied + cd_epoch) * 1000.0) as i64;
                let last_ms = ((last_copied + cd_epoch) * 1000.0) as i64;
                let sync_ms = ((sync_ts + cd_epoch) * 1000.0) as i64;

                let pk: i64 = row.get(0)?;
                let pin_val: Option<String> = row.get(5)?;
                // If pin is empty string, treat as None
                let pin = pin_val.and_then(|p| if p.is_empty() { None } else { Some(p) });

                Ok((
                    pk,
                    ClipboardItem {
                        id: row.get::<_, String>(7).unwrap_or_default(),
                        application: row.get(1)?,
                        first_copied_at: first_ms,
                        last_copied_at: last_ms,
                        number_of_copies: row.get::<_, i64>(4).unwrap_or(1) as i32,
                        pin,
                        title: row.get::<_, String>(6).unwrap_or_default(),
                        contents: vec![],
                        sync_timestamp: sync_ms,
                        sync_source: row.get(9)?,
                        sync_deleted: row.get::<_, i64>(10).unwrap_or(0) != 0,
                    },
                ))
            })
            .map_err(|e| CoreError::Storage {
                msg: format!("SwiftData row parse failed: {}", e),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CoreError::Storage {
                msg: format!("SwiftData collect failed: {}", e),
            })?;

        drop(item_stmt);

        // Read contents
        let mut content_stmt = sd_conn
            .prepare("SELECT ZITEM, ZTYPE, ZVALUE FROM Z_HISTORYITEMCONTENT")
            .map_err(|e| CoreError::Storage {
                msg: format!("SwiftData content query failed: {}", e),
            })?;

        let contents_by_pk: std::collections::HashMap<i64, Vec<ClipboardContent>> = {
            let mut map: std::collections::HashMap<i64, Vec<ClipboardContent>> =
                std::collections::HashMap::new();
            let rows = content_stmt
                .query_map([], |row| {
                    let item_pk: i64 = row.get(0)?;
                    let content_type: String = row.get(1)?;
                    let value: Option<Vec<u8>> = row.get(2)?;
                    Ok((
                        item_pk,
                        ClipboardContent {
                            content_type,
                            value,
                        },
                    ))
                })
                .map_err(|e| CoreError::Storage {
                    msg: format!("SwiftData content parse failed: {}", e),
                })?;

            for row in rows {
                let (pk, content) = row.map_err(|e| CoreError::Storage {
                    msg: format!("SwiftData content row failed: {}", e),
                })?;
                map.entry(pk).or_default().push(content);
            }
            map
        };

        drop(content_stmt);
        drop(sd_conn);

        // Insert into our storage
        let mut count = 0u64;
        for (pk, mut item) in swiftdata_items {
            item.contents = contents_by_pk.get(&pk).cloned().unwrap_or_default();
            if item.id.is_empty() {
                item.id = uuid::Uuid::new_v4().to_string();
            }
            if let Err(e) = self.insert_item(&item) {
                log::warn!("Skipping item {} during migration: {}", item.id, e);
                continue;
            }
            count += 1;
        }

        Ok(count)
    }
}
