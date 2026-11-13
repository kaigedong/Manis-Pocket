use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::model::{ClipboardContent, ClipboardItem, CoreError, SearchMode, SearchResult, SortBy};
use crate::platform::ClipboardObserver;
use crate::search::SearchEngine;
use crate::sort::sort_items;
use crate::storage::Storage;
use crate::sync::SyncEngine;

/// Central coordinator for clipboard history management.
/// Owns persistence (Storage), sync (SyncEngine), and search/sort.
///
/// # Sync Group & Permissions
///
/// Devices that pair form a **group**. Rules:
/// - Only one admin per group. The first device to pair becomes admin.
/// - Admin can transfer admin role to any member (promote_to_admin).
/// - Admin can remove members; regular members cannot remove anyone.
/// - A device belongs to at most one group; must leave before joining another.
#[derive(uniffi::Object)]
pub struct HistoryManager {
    storage: Mutex<Storage>,
    sync_engine: Mutex<Option<SyncEngine>>,
    clipboard_state_path: Option<PathBuf>,
}

#[uniffi::export]
impl HistoryManager {
    #[uniffi::constructor]
    /// Opens the history at `db_path`, creating and migrating it as needed.
    ///
    /// # Errors
    /// [`CoreError::Storage`] when the database cannot be opened or migrated.
    pub fn new(db_path: String) -> Result<Self, CoreError> {
        let storage = Storage::open(&db_path)?;
        let clipboard_state_path = if db_path.is_empty() || db_path == ":memory:" {
            None
        } else {
            Some(PathBuf::from(&db_path).with_extension("clipboard-state.json"))
        };
        Ok(HistoryManager {
            storage: Mutex::new(storage),
            sync_engine: Mutex::new(None),
            clipboard_state_path,
        })
    }

    // ── Persistence ───────────────────────────────────────────────

    /// Load all items from storage, payloads included.
    pub fn load(&self) -> Result<Vec<ClipboardItem>, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .get_all_items()
    }

    /// Load every item without payloads.
    ///
    /// UIs list the whole history but only need payloads for what they render
    /// or copy, so this keeps startup independent of how much content is stored.
    pub fn load_metadata(&self) -> Result<Vec<ClipboardItem>, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .get_all_metadata()
    }

    /// Fetch one item's payloads on demand.
    pub fn contents(&self, id: String) -> Result<Vec<ClipboardContent>, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .get_contents(&id)
    }

    /// Add a new clipboard item. Handles deduplication and size limiting.
    /// Rejects items that have no meaningful content.
    pub fn add(
        &self,
        item: ClipboardItem,
        max_size: i32,
        is_unlimited: bool,
    ) -> Result<ClipboardItem, CoreError> {
        // Reject items with no content
        if item.title.is_empty() && item.contents.is_empty() {
            return Err(CoreError::InvalidArg {
                msg: "empty item".into(),
            });
        }
        let storage = self
            .storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;

        // Deduplication goes through the content fingerprint index: only
        // entries sharing a payload are loaded, never the whole history.
        let shareable: Vec<&ClipboardContent> = item
            .contents
            .iter()
            .filter(|content| !is_transient(&content.content_type))
            .collect();
        let similar = if shareable.is_empty() {
            None
        } else {
            storage.find_by_contents(&shareable)?
        };
        if let Some(similar) = similar {
            let mut updated = similar.clone();
            updated.last_copied_at = item.last_copied_at;
            updated.number_of_copies += 1;
            for new_content in &item.contents {
                let has_type = updated
                    .contents
                    .iter()
                    .any(|c| c.content_type == new_content.content_type);
                if !has_type {
                    updated.contents.push(new_content.clone());
                }
            }
            for new_content in &item.contents {
                if let Some(existing) = updated
                    .contents
                    .iter_mut()
                    .find(|c| c.content_type == new_content.content_type)
                {
                    existing.value = new_content.value.clone();
                }
            }
            storage.update_item(&updated)?;
            return Ok(updated);
        }

        if !is_unlimited && max_size > 0 {
            let count = storage.count_items()?;
            if count >= max_size as i64 {
                let excess = (count - max_size as i64 + 1) as usize;
                for id in storage.oldest_unpinned_ids(excess)? {
                    storage.delete_item(&id)?;
                }
            }
        }

        storage.insert_item(&item)?;
        Ok(item)
    }

    /// Deletes one entry and its payloads.
    ///
    /// # Errors
    /// [`CoreError::Storage`] when the database rejects the delete.
    pub fn delete(&self, id: String) -> Result<(), CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .delete_item(&id)
    }

    /// Deletes every unpinned entry and reports how many went.
    ///
    /// # Errors
    /// [`CoreError::Storage`] when the database rejects the delete.
    pub fn clear_unpinned(&self) -> Result<u64, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .delete_unpinned()
    }

    /// Deletes everything, pins included, and reports how many went.
    ///
    /// # Errors
    /// [`CoreError::Storage`] when the database rejects the delete.
    pub fn clear_all(&self) -> Result<u64, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .delete_all()
    }

    /// Pins an entry with the next free letter, or unpins it.
    ///
    /// # Errors
    /// [`CoreError::NotFound`] when the id is unknown, and [`CoreError::Storage`]
    /// when the update fails.
    pub fn toggle_pin(
        &self,
        id: String,
        available_pins: Vec<String>,
    ) -> Result<ClipboardItem, CoreError> {
        let storage = self
            .storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        let mut item = storage
            .get_item(&id)?
            .ok_or(CoreError::NotFound { id: id.clone() })?;
        if item.pin.is_some() {
            item.pin = None;
        } else {
            item.pin = available_pins.first().cloned();
        }
        storage.update_item(&item)?;
        Ok(item)
    }

    /// Replaces the text of an entry with what a peer sent.
    ///
    /// # Errors
    /// [`CoreError::Storage`] when the update fails.
    pub fn update_item_text(
        &self,
        id: String,
        new_text: String,
    ) -> Result<ClipboardItem, CoreError> {
        let storage = self
            .storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?;
        let mut item = storage
            .get_item(&id)?
            .ok_or(CoreError::NotFound { id: id.clone() })?;
        let text_type = "public.utf8-plain-text";
        if let Some(content) = item
            .contents
            .iter_mut()
            .find(|c| c.content_type == text_type)
        {
            content.value = Some(new_text.as_bytes().to_vec());
        }
        item.title = new_text;
        storage.update_item(&item)?;
        Ok(item)
    }

    /// Matches `query` against `pool`, in the order the caller asked for.
    ///
    /// # Errors
    /// Never fails today; the signature keeps the search engine swappable.
    pub fn search(
        &self,
        query: &str,
        items: Vec<ClipboardItem>,
        mode: SearchMode,
    ) -> Vec<SearchResult> {
        SearchEngine::search(query, &items, mode)
    }

    /// Orders entries for display, honouring pins and the chosen sort.
    ///
    /// # Errors
    /// Never fails today.
    pub fn sort(
        &self,
        items: Vec<ClipboardItem>,
        sort_by: SortBy,
        pin_to_top: bool,
    ) -> Vec<ClipboardItem> {
        sort_items(items, sort_by, pin_to_top)
    }

    /// Size of the database file in bytes, or zero when it cannot be read.
    pub fn storage_size_bytes(&self, db_path: String) -> i64 {
        self.storage
            .lock()
            .map(|s| s.db_size_bytes(&db_path))
            .unwrap_or(0)
    }

    /// How many entries the history holds.
    ///
    /// # Errors
    /// [`CoreError::Storage`] when the database cannot be queried.
    pub fn count(&self) -> Result<i64, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .count_items()
    }

    /// Imports the history of the SwiftUI build, which kept it in a SwiftData
    /// store. Callers should treat a failure as advisory: a store written by a
    /// different schema must not stop the app from starting.
    ///
    /// # Errors
    /// [`CoreError::Storage`] when the store cannot be opened, or when its tables
    /// are not the ones the SwiftUI build wrote.
    pub fn migrate_from_swiftdata(&self, swiftdata_path: String) -> Result<u64, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .migrate_from_swiftdata(&swiftdata_path)
    }

    /// Imports the history the GTK build kept as a JSON array of strings, newest
    /// first.
    ///
    /// Entries are spaced a second apart so the newest stays newest; an empty or
    /// missing file imports nothing.
    ///
    /// # Errors
    /// [`CoreError::Storage`] when the file cannot be read or is not a JSON array
    /// of strings.
    pub fn import_json_history(&self, path: &str) -> Result<u64, CoreError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(CoreError::Storage {
                    msg: format!("could not read {path}: {error}"),
                });
            }
        };
        let entries: Vec<String> =
            serde_json::from_str(&text).map_err(|error| CoreError::Storage {
                msg: format!("{path} is not a history file: {error}"),
            })?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or_default();
        let mut imported = 0;
        for (index, entry) in entries.iter().enumerate() {
            if entry.trim().is_empty() {
                continue;
            }
            let timestamp = now - (index as i64 * 1000);
            let item = ClipboardItem {
                id: uuid::Uuid::new_v4().to_string(),
                application: None,
                first_copied_at: timestamp,
                last_copied_at: timestamp,
                number_of_copies: 1,
                pin: None,
                title: entry.clone(),
                contents: vec![ClipboardContent {
                    content_type: "public.utf8-plain-text".into(),
                    value: Some(entry.as_bytes().to_vec()),
                }],
                sync_timestamp: timestamp,
                sync_source: None,
                sync_deleted: false,
            };
            if self.add(item, i32::MAX, true).is_ok() {
                imported += 1;
            }
        }
        Ok(imported)
    }

    // ── Sync ──────────────────────────────────────────────────────

    /// Start the P2P sync engine. The observer receives all sync events.
    pub fn start_sync(
        &self,
        device_name: String,
        device_id: String,
        observer: Arc<dyn ClipboardObserver>,
    ) -> Result<(), CoreError> {
        let stored_keypair = self
            .storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .get_config("libp2p_keypair");
        // Load persisted paired peer IDs so the engine can restore the in-memory
        // paired set (otherwise sync messages from them get dropped after restart).
        let initial_paired_peer_ids: Vec<String> = self
            .storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })
            .ok()
            .and_then(|s| s.get_paired_peers().ok())
            .map(|peers| peers.into_iter().map(|(id, _, _)| id).collect())
            .unwrap_or_default();
        let (engine, keypair_bytes) = SyncEngine::start(
            &device_name,
            &device_id,
            observer,
            stored_keypair,
            initial_paired_peer_ids,
            self.clipboard_state_path.clone(),
        )?;
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .set_config("libp2p_keypair", &keypair_bytes)?;
        let mut guard = self
            .sync_engine
            .lock()
            .map_err(|e| CoreError::Sync { msg: e.to_string() })?;
        *guard = Some(engine);
        Ok(())
    }

    /// Stop the sync engine.
    pub fn stop_sync(&self) -> Result<(), CoreError> {
        let mut guard = self
            .sync_engine
            .lock()
            .map_err(|e| CoreError::Sync { msg: e.to_string() })?;
        if let Some(engine) = guard.take() {
            engine.stop();
        }
        Ok(())
    }

    /// Dials an address entered by hand, for networks where mDNS is silent.
    pub fn sync_add_peer_address(&self, address: String) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.add_peer_address(&address);
        }
    }

    /// Starts local discovery. The current engine reports readiness at build time,
    /// so this is a no-op kept for the platform layer.
    pub fn sync_start_discovery(&self) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.start_discovery();
        }
    }

    /// Stops local discovery; the counterpart of [`Self::sync_start_discovery`].
    pub fn sync_stop_discovery(&self) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.stop_discovery();
        }
    }

    /// Restart discovery to find new peers.
    pub fn sync_refresh_discovery(&self) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.stop_discovery();
            e.start_discovery();
        }
    }

    /// Asks a discovered peer to pair; the peer answers with a six-digit code.
    pub fn sync_request_pairing(&self, peer_id: String) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.request_pairing(&peer_id);
        }
    }

    /// Accepts a pairing request after the user compared the code.
    pub fn sync_accept_pairing(&self, peer_id: String, pin: String) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.accept_pairing(&peer_id, &pin);
        }
    }

    /// Declines a pairing request.
    pub fn sync_reject_pairing(&self, peer_id: String) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.reject_pairing(&peer_id);
        }
    }

    /// Drops a paired peer from the running engine and from the register.
    pub fn sync_unpair(&self, peer_id: String) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.unpair(&peer_id);
        }
    }

    /// Broadcast a newly copied item to synced peers.
    pub fn sync_broadcast_item(&self, item: ClipboardItem) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.broadcast_item(&item);
        }
    }

    /// Report the actual system clipboard.
    ///
    /// `None` means the clipboard is empty. Callers must not call this for
    /// content the protocol cannot share: that would clear the peer.
    pub fn sync_observe_local_clipboard(&self, content: Option<ClipboardContent>) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref engine) = *guard
        {
            engine.observe_local_clipboard(content);
        }
    }

    /// Complete a remote clipboard update only after the platform clipboard write.
    pub fn sync_confirm_current_clipboard(&self, event_id: String, success: bool) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref engine) = *guard
        {
            engine.confirm_current_clipboard(&event_id, success);
        }
    }

    /// Whether a clipboard event came from this device, in which case applying it
    /// would overwrite what the user just copied.
    pub fn sync_should_apply_current_clipboard(&self, event_id: String) -> bool {
        self.sync_engine
            .lock()
            .ok()
            .and_then(|guard| {
                guard
                    .as_ref()
                    .map(|engine| engine.should_apply_current_clipboard(&event_id))
            })
            .unwrap_or(false)
    }

    /// Broadcast a deletion to synced peers.
    pub fn sync_broadcast_deletion(&self, item_id: String) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.broadcast_deletion(&item_id);
        }
    }

    /// Broadcast an update to synced peers.
    pub fn sync_broadcast_update(&self, item: ClipboardItem) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.broadcast_update(&item);
        }
    }

    // ── File transfer ─────────────────────────────────────────

    /// Request a file download from a peer.
    pub fn request_file(&self, peer_id: String, file_path: String) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(ref e) = *guard
        {
            e.request_file(&peer_id, &file_path);
        }
    }

    // ── Peer management (persisted in Rust, UI just displays) ─

    /// Get all paired peers with their metadata (ID, name, admin status, online status).
    /// Returns JSON strings — each is `{"peerId":..., "displayName":..., "isAdmin":..., "isOnline":...}`.
    pub fn get_paired_peers(&self) -> Vec<String> {
        // The storage behind the lock is plain SQLite state, so a poisoned lock is
        // still safe to read; taking the app down over it would lose the session.
        let storage = self
            .storage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let db_peers = storage.get_paired_peers().unwrap_or_default();
        // Online status comes from live NetworkManager state, not cached DB column
        drop(storage);
        db_peers
            .into_iter()
            .map(|(peer_id, display_name, is_admin)| {
                let is_online = if let Ok(guard) = self.sync_engine.lock() {
                    guard
                        .as_ref()
                        .map(|e| e.is_peer_connected(&peer_id))
                        .unwrap_or(false)
                } else {
                    false
                };
                serde_json::to_string(&serde_json::json!({
                    "peerId": peer_id,
                    "displayName": display_name,
                    "isAdmin": is_admin,
                    "isOnline": is_online
                }))
                .unwrap_or_default()
            })
            .collect()
    }
    /// Records a peer that has just finished pairing.
    pub fn save_paired_peer(&self, peer_id: String, display_name: String, is_admin: bool) {
        match self.storage.lock() {
            Ok(s) => {
                if let Err(e) = s.save_paired_peer(&peer_id, &display_name, is_admin) {
                    log::error!("Failed to save paired peer {}: {}", peer_id, e);
                }
            }
            Err(e) => log::error!("Failed to lock storage for save_paired_peer: {}", e),
        }
    }

    /// Forget a device: tell the running engine to drop it and delete the record,
    /// so a later start does not resurrect it from the database.
    pub fn unpair(&self, peer_id: String) {
        if let Ok(guard) = self.sync_engine.lock()
            && let Some(engine) = guard.as_ref()
        {
            engine.unpair(&peer_id);
        }
        self.remove_paired_peer(peer_id);
    }

    /// Remove a paired peer.
    pub fn remove_paired_peer(&self, peer_id: String) {
        match self.storage.lock() {
            Ok(s) => {
                if let Err(e) = s.remove_paired_peer(&peer_id) {
                    log::error!("Failed to remove paired peer {}: {}", peer_id, e);
                }
            }
            Err(e) => log::error!("Failed to lock storage for remove_paired_peer: {}", e),
        }
    }
}

// ── Deduplication ─────────────────────────────────────────────────

/// Pasteboard types that change on every copy without changing what the user
/// actually copied (browser metadata, our own marker, ...). They must not
/// defeat deduplication.
fn is_transient(content_type: &str) -> bool {
    matches!(
        content_type,
        "com.apple.modified"
            | "com.manispocket.from-manispocket"
            | "com.apple.linkpresentation.metadata"
            | "com.apple.custom-webkit-pasteboard-data"
            | "com.apple.pasteboard.promised-file-url"
            | "org.chromium.source-url"
            | "org.chromium.source-web-custom-data"
            | "com.apple.NSItemProvider.Providers"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(id: &str, title: &str, types: &[(&str, &[u8])]) -> ClipboardItem {
        ClipboardItem {
            id: id.to_string(),
            application: None,
            first_copied_at: 1000,
            last_copied_at: 1000,
            number_of_copies: 1,
            pin: None,
            title: title.to_string(),
            contents: types
                .iter()
                .map(|(t, v)| ClipboardContent {
                    content_type: t.to_string(),
                    value: Some(v.to_vec()),
                })
                .collect(),
            sync_timestamp: 1000,
            sync_source: None,
            sync_deleted: false,
        }
    }

    #[test]
    fn a_json_history_is_imported_newest_first() {
        let directory = std::env::temp_dir().join(format!("manis-json-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("history.json");
        std::fs::write(&path, r#"["newest","older","with \"quotes\"",""]"#).expect("write");

        let manager = HistoryManager::new(":memory:".to_string()).expect("open");
        let imported = manager
            .import_json_history(path.display().to_string().as_str())
            .expect("import");
        // The empty entry is skipped.
        assert_eq!(imported, 3);

        let items = manager.load_metadata().expect("metadata");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title, "newest");
        assert_eq!(items[1].title, "older");
        assert_eq!(items[2].title, "with \"quotes\"");
        assert!(
            items[0].last_copied_at > items[2].last_copied_at,
            "the newest entry must stay the newest"
        );

        let contents = manager.contents(items[0].id.clone()).expect("contents");
        assert_eq!(contents[0].value.as_deref(), Some(&b"newest"[..]));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_or_broken_json_history_is_not_fatal() {
        let manager = HistoryManager::new(":memory:".to_string()).expect("open");
        assert_eq!(
            manager
                .import_json_history("/nonexistent/history.json")
                .expect("missing file is fine"),
            0
        );

        let directory = std::env::temp_dir().join(format!("manis-json-bad-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("history.json");
        std::fs::write(&path, "not json at all").expect("write");
        assert!(
            manager
                .import_json_history(path.display().to_string().as_str())
                .is_err(),
            "a broken file must be reported"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn test_add_and_load() {
        let mgr = HistoryManager::new(":memory:".to_string()).unwrap();
        mgr.add(
            make_item("1", "Hello", &[("public.utf8-plain-text", b"Hello")]),
            100,
            false,
        )
        .unwrap();
        assert_eq!(mgr.load().unwrap().len(), 1);
    }

    #[test]
    fn dedup_ignores_transient_types_and_requires_all_contents() {
        let mgr = HistoryManager::new(":memory:".to_string()).unwrap();
        mgr.add(
            make_item("a", "Hello", &[("public.utf8-plain-text", b"Hello")]),
            1000,
            false,
        )
        .unwrap();

        // A transient marker alone must not defeat deduplication.
        let mut with_marker = make_item(
            "b",
            "Hello",
            &[
                ("public.utf8-plain-text", b"Hello"),
                ("com.apple.modified", b"123"),
            ],
        );
        with_marker.last_copied_at = 2000;
        let merged = mgr.add(with_marker, 1000, false).unwrap();
        assert_eq!(merged.id, "a");
        assert_eq!(merged.number_of_copies, 2);
        assert_eq!(mgr.load_metadata().unwrap().len(), 1);

        // An extra non-transient payload is a different item.
        let with_image = make_item(
            "c",
            "Hello",
            &[
                ("public.utf8-plain-text", b"Hello"),
                ("public.png", b"\x89PNG"),
            ],
        );
        let added = mgr.add(with_image, 1000, false).unwrap();
        assert_eq!(added.id, "c");
        assert_eq!(mgr.load_metadata().unwrap().len(), 2);
    }

    #[test]
    fn text_from_a_peer_merges_with_the_same_local_text() {
        let mgr = HistoryManager::new(":memory:".to_string()).unwrap();
        // Captured locally on macOS.
        mgr.add(
            make_item("local", "hi", &[("public.utf8-plain-text", b"hi")]),
            1000,
            false,
        )
        .unwrap();

        // The same text arriving from a peer, spelled the way the wire does.
        let mut from_peer = make_item("peer", "", &[("text/plain;charset=utf-8", b"hi")]);
        from_peer.sync_source = Some("12D3KooPeer".into());
        let merged = mgr.add(from_peer, 1000, false).unwrap();

        assert_eq!(merged.id, "local", "peer copy should merge, not duplicate");
        assert_eq!(merged.number_of_copies, 2);
        assert_eq!(mgr.load_metadata().unwrap().len(), 1);
    }

    #[test]
    fn trimming_drops_only_the_oldest_unpinned_entries() {
        let mgr = HistoryManager::new(":memory:".to_string()).unwrap();
        for (index, text) in ["one", "two", "three"].iter().enumerate() {
            let mut item = make_item(text, text, &[("public.utf8-plain-text", text.as_bytes())]);
            item.last_copied_at = 1000 + index as i64;
            mgr.add(item, 2, false).unwrap();
        }
        let titles: Vec<String> = mgr
            .load_metadata()
            .unwrap()
            .into_iter()
            .map(|item| item.title)
            .collect();
        assert_eq!(titles, vec!["three".to_string(), "two".to_string()]);
    }

    #[test]
    fn metadata_load_skips_payloads_and_contents_load_on_demand() {
        let mgr = HistoryManager::new(":memory:".to_string()).unwrap();
        mgr.add(
            make_item("1", "Hello", &[("public.utf8-plain-text", b"Hello")]),
            1000,
            false,
        )
        .unwrap();
        let blob = vec![7u8; 4096];
        mgr.add(
            make_item("2", "Blob", &[("public.png", &blob)]),
            1000,
            false,
        )
        .unwrap();

        let metadata = mgr.load_metadata().unwrap();
        assert_eq!(metadata.len(), 2);
        assert!(
            metadata
                .iter()
                .flat_map(|item| item.contents.iter())
                .all(|content| content.value.is_none()),
            "metadata must not carry payloads"
        );
        // The types still come along: the list needs them to tell an image row
        // from a text row without loading the payload.
        assert!(
            metadata
                .iter()
                .any(|item| item.contents.iter().any(|c| c.content_type == "public.png"))
        );
        // Listing and search only need titles, which metadata still provides.
        assert!(metadata.iter().any(|item| item.title == "Blob"));

        let contents = mgr.contents("2".to_string()).unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].value.as_deref(), Some(blob.as_slice()));
    }

    #[test]
    fn test_delete() {
        let mgr = HistoryManager::new(":memory:".to_string()).unwrap();
        mgr.add(
            make_item("1", "Hello", &[("public.utf8-plain-text", b"Hello")]),
            100,
            false,
        )
        .unwrap();
        mgr.delete("1".to_string()).unwrap();
        assert!(mgr.load().unwrap().is_empty());
    }

    #[test]
    fn test_deduplication() {
        let mgr = HistoryManager::new(":memory:".to_string()).unwrap();
        mgr.add(
            make_item("1", "Hello", &[("public.utf8-plain-text", b"Hello")]),
            100,
            false,
        )
        .unwrap();
        let mut item2 = make_item("2", "Hello", &[("public.utf8-plain-text", b"Hello")]);
        item2.last_copied_at = 2000;
        let result = mgr.add(item2, 100, false).unwrap();
        assert_eq!(result.id, "1");
        assert_eq!(result.number_of_copies, 2);
    }

    #[test]
    fn test_toggle_pin() {
        let mgr = HistoryManager::new(":memory:".to_string()).unwrap();
        mgr.add(
            make_item("1", "Hello", &[("public.utf8-plain-text", b"Hello")]),
            100,
            false,
        )
        .unwrap();
        let pinned = mgr
            .toggle_pin("1".to_string(), vec!["b".to_string()])
            .unwrap();
        assert_eq!(pinned.pin, Some("b".to_string()));
    }

    #[test]
    fn test_size_limit() {
        let mgr = HistoryManager::new(":memory:".to_string()).unwrap();
        for i in 0..5 {
            mgr.add(
                make_item(
                    &format!("{}", i),
                    &format!("Item {}", i),
                    &[("public.utf8-plain-text", b"text")],
                ),
                3,
                false,
            )
            .unwrap();
        }
        assert!(mgr.load().unwrap().len() <= 3);
    }
}
