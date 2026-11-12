use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::Path;

use crate::model::{ClipboardContent, ClipboardItem, CoreError};

pub struct Storage {
    conn: Connection,
}

#[cfg(test)]
mod tests {
    use super::Storage;

    #[test]
    fn legacy_pairs_require_fresh_confirmation() {
        let storage = Storage::open_in_memory().unwrap();
        storage
            .conn
            .execute(
                "INSERT INTO sync_pairs (peer_id, display_name, is_admin, protocol_version) VALUES ('legacy', 'Old device', 1, 1)",
                [],
            )
            .unwrap();
        assert!(storage.get_paired_peers().unwrap().is_empty());

        storage
            .save_paired_peer("legacy", "New device", true)
            .unwrap();
        let pairs = storage.get_paired_peers().unwrap();
        assert_eq!(pairs, vec![("legacy".into(), "New device".into(), true)]);
    }
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

        if current_version < 5 {
            // Legacy pairs were accepted before remote approval. Keep the rows
            // for migration diagnostics, but require a fresh v2 pairing.
            self.conn
                .execute_batch(
                    "ALTER TABLE sync_pairs ADD COLUMN protocol_version INTEGER NOT NULL DEFAULT 1;
                     INSERT INTO schema_version (version) VALUES (5);",
                )
                .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        }

        if current_version < 6 {
            // Content fingerprints: duplicating a copy used to require loading
            // every stored payload (16 MiB+ on a mature history). The index lets
            // `add()` find the existing entry by (type, sha256(value)) instead.
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS item_content_hashes (
                        item_id TEXT NOT NULL,
                        content_type TEXT NOT NULL,
                        value_hash TEXT NOT NULL,
                        PRIMARY KEY (item_id, content_type, value_hash)
                     );
                     CREATE INDEX IF NOT EXISTS idx_content_hash
                        ON item_content_hashes(content_type, value_hash);
                     INSERT INTO schema_version (version) VALUES (6);",
                )
                .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
            self.rebuild_content_hashes()?;
        }

        if current_version < 7 {
            // Fingerprints now normalise plain-text spellings, so entries stored
            // locally and the same text arriving over sync compare equal.
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (7)", [])
                .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
            self.rebuild_content_hashes()?;
        }

        Ok(())
    }

    // ── Sync pairs ────────────────────────────────────────────

    pub fn get_paired_peers(&self) -> Result<Vec<(String, String, bool)>, CoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT peer_id, display_name, is_admin != 0 FROM sync_pairs WHERE protocol_version >= 2 ORDER BY display_name",
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
                "INSERT OR REPLACE INTO sync_pairs (peer_id, display_name, is_admin, protocol_version) VALUES (?1, ?2, ?3, 2)",
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
        self.record_content_hashes(&item.id, &item.contents)?;

        Ok(())
    }

    pub fn get_all_items(&self) -> Result<Vec<ClipboardItem>, CoreError> {
        let mut items = self.get_all_metadata()?;
        for item in &mut items {
            item.contents = self.get_contents_for_item(&item.id)?;
        }
        Ok(items)
    }

    /// Every visible item **without** its payloads.
    ///
    /// A history list needs titles, timestamps and source apps for thousands of
    /// rows, but payloads only for the handful it renders or copies. Loading
    /// them all would pull hundreds of megabytes of BLOBs into memory, so this
    /// is the query UIs should use for the list.
    pub fn get_all_metadata(&self) -> Result<Vec<ClipboardItem>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, application, first_copied_at, last_copied_at, number_of_copies, pin, title, sync_timestamp, sync_source, sync_deleted
             FROM history_items WHERE sync_deleted = 0
             ORDER BY last_copied_at DESC",
        )?;

        let mut items = stmt
            .query_map([], item_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        // The list needs to know *what kind* of entry each row is (text, image,
        // file) to label and preview it, but not the payload bytes. Attaching the
        // type alone keeps this query cheap and stops image entries from showing
        // up as an unnamed entry.
        let mut types: HashMap<String, Vec<ClipboardContent>> = HashMap::new();
        let mut stmt = self
            .conn
            .prepare("SELECT item_id, content_type FROM history_contents ORDER BY item_id, id")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (item_id, content_type) = row?;
            types.entry(item_id).or_default().push(ClipboardContent {
                content_type,
                value: None,
            });
        }
        for item in &mut items {
            if let Some(found) = types.remove(&item.id) {
                item.contents = found;
            }
        }
        Ok(items)
    }

    /// Payloads of one item, fetched on demand by the UI.
    pub fn get_contents(&self, item_id: &str) -> Result<Vec<ClipboardContent>, CoreError> {
        self.get_contents_for_item(item_id)
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
        self.conn.execute(
            "DELETE FROM item_content_hashes WHERE item_id = ?1",
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
        self.conn.execute(
            "DELETE FROM item_content_hashes WHERE item_id = ?1",
            params![item.id],
        )?;
        for content in &item.contents {
            self.conn.execute(
                "INSERT INTO history_contents (item_id, content_type, value) VALUES (?1, ?2, ?3)",
                params![item.id, content.content_type, content.value.as_deref()],
            )?;
        }
        self.record_content_hashes(&item.id, &item.contents)?;

        Ok(())
    }

    /// Store the (type, hash) fingerprint for each content of an item.
    fn record_content_hashes(
        &self,
        item_id: &str,
        contents: &[ClipboardContent],
    ) -> Result<(), CoreError> {
        for content in contents {
            self.conn.execute(
                "INSERT OR IGNORE INTO item_content_hashes (item_id, content_type, value_hash)
                 VALUES (?1, ?2, ?3)",
                params![
                    item_id,
                    fingerprint_type(&content.content_type),
                    content_hash(content)
                ],
            )?;
        }
        Ok(())
    }

    /// Backfill fingerprints for databases created before they existed.
    fn rebuild_content_hashes(&self) -> Result<(), CoreError> {
        self.conn
            .execute("DELETE FROM item_content_hashes", [])
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        let items = self.get_all_items()?;
        for item in &items {
            self.record_content_hashes(&item.id, &item.contents)?;
        }
        Ok(())
    }

    /// Find an existing item that already contains **all** of `contents`.
    ///
    /// Candidates come from the fingerprint index on the first content, so the
    /// only payload read is the handful of entries sharing that fingerprint.
    pub fn find_by_contents(
        &self,
        contents: &[&ClipboardContent],
    ) -> Result<Option<ClipboardItem>, CoreError> {
        let Some(first) = contents.first() else {
            return Ok(None);
        };
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT h.item_id
             FROM item_content_hashes h
             JOIN history_items i ON i.id = h.item_id
             WHERE h.content_type = ?1 AND h.value_hash = ?2 AND i.sync_deleted = 0",
        )?;
        let candidates = stmt
            .query_map(
                params![fingerprint_type(&first.content_type), content_hash(first)],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;

        for id in candidates {
            let Some(existing) = self.get_item(&id)? else {
                continue;
            };
            let covers_all = contents.iter().all(|wanted| {
                existing.contents.iter().any(|stored| {
                    fingerprint_type(&stored.content_type) == fingerprint_type(&wanted.content_type)
                        && stored.value == wanted.value
                })
            });
            if covers_all {
                return Ok(Some(existing));
            }
        }
        Ok(None)
    }

    /// Ids of the oldest unpinned entries, for trimming to the configured size
    /// without loading the whole history.
    pub fn oldest_unpinned_ids(&self, limit: usize) -> Result<Vec<String>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM history_items
             WHERE pin IS NULL AND sync_deleted = 0
             ORDER BY last_copied_at ASC
             LIMIT ?1",
        )?;
        let ids = stmt
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
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

/// Maps a `history_items` row onto `ClipboardItem` without contents.
fn item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipboardItem> {
    Ok(ClipboardItem {
        id: row.get(0)?,
        application: row.get(1)?,
        first_copied_at: row.get(2)?,
        last_copied_at: row.get(3)?,
        number_of_copies: row.get(4)?,
        pin: row.get(5)?,
        title: row.get(6)?,
        contents: Vec::new(),
        sync_timestamp: row.get(7)?,
        sync_source: row.get(8)?,
        sync_deleted: row.get::<_, i32>(9)? != 0,
    })
}

/// Content types that all mean "plain text".
///
/// macOS says `public.utf8-plain-text`, the sync wire format says
/// `text/plain;charset=utf-8`, and Linux tools say `text/plain`. Deduplication
/// has to see them as the same payload or a synced copy of something already in
/// the history becomes a second entry.
fn fingerprint_type(content_type: &str) -> &str {
    match content_type {
        "public.utf8-plain-text" | "text/plain" | "text/plain;charset=utf-8" => "text/plain",
        other => other,
    }
}

/// Stable fingerprint of one clipboard payload.
fn content_hash(content: &ClipboardContent) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(fingerprint_type(&content.content_type).as_bytes());
    hasher.update([0u8]);
    if let Some(value) = &content.value {
        hasher.update(value);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
