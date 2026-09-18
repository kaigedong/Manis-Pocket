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
}

#[uniffi::export]
impl HistoryManager {
    #[uniffi::constructor]
    pub fn new(db_path: String) -> Result<Self, CoreError> {
        let storage = Storage::open(&db_path)?;
        Ok(HistoryManager {
            storage: Mutex::new(storage),
            sync_engine: Mutex::new(None),
        })
    }

    // ── Persistence ───────────────────────────────────────────────

    /// Load all items from storage.
    pub fn load(&self) -> Result<Vec<ClipboardItem>, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .get_all_items()
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

        let existing_items = storage.get_all_items()?;
        if let Some(similar) = find_similar(&item, &existing_items) {
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
                let all = storage.get_all_items()?;
                let unpinned: Vec<&ClipboardItem> =
                    all.iter().filter(|i| i.pin.is_none()).collect();
                let excess = (count - max_size as i64 + 1) as usize;
                for item in unpinned.iter().rev().take(excess) {
                    storage.delete_item(&item.id)?;
                }
            }
        }

        storage.insert_item(&item)?;
        Ok(item)
    }

    pub fn delete(&self, id: String) -> Result<(), CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .delete_item(&id)
    }

    pub fn clear_unpinned(&self) -> Result<u64, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .delete_unpinned()
    }

    pub fn clear_all(&self) -> Result<u64, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .delete_all()
    }

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

    pub fn search(
        &self,
        query: &str,
        items: Vec<ClipboardItem>,
        mode: SearchMode,
    ) -> Vec<SearchResult> {
        SearchEngine::search(query, &items, mode)
    }

    pub fn sort(
        &self,
        items: Vec<ClipboardItem>,
        sort_by: SortBy,
        pin_to_top: bool,
    ) -> Vec<ClipboardItem> {
        sort_items(items, sort_by, pin_to_top)
    }

    pub fn storage_size_bytes(&self, db_path: String) -> i64 {
        self.storage
            .lock()
            .map(|s| s.db_size_bytes(&db_path))
            .unwrap_or(0)
    }

    pub fn count(&self) -> Result<i64, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .count_items()
    }

    pub fn migrate_from_swiftdata(&self, swiftdata_path: String) -> Result<u64, CoreError> {
        self.storage
            .lock()
            .map_err(|e| CoreError::Storage { msg: e.to_string() })?
            .migrate_from_swiftdata(&swiftdata_path)
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

    pub fn sync_add_peer_address(&self, address: String) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.add_peer_address(&address);
            }
        }
    }

    pub fn sync_start_discovery(&self) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.start_discovery();
            }
        }
    }

    pub fn sync_stop_discovery(&self) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.stop_discovery();
            }
        }
    }

    /// Restart discovery to find new peers.
    pub fn sync_refresh_discovery(&self) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.stop_discovery();
                e.start_discovery();
            }
        }
    }

    pub fn sync_request_pairing(&self, peer_id: String) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.request_pairing(&peer_id);
            }
        }
    }

    pub fn sync_accept_pairing(&self, peer_id: String, pin: String) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.accept_pairing(&peer_id, &pin);
            }
        }
    }

    pub fn sync_reject_pairing(&self, peer_id: String) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.reject_pairing(&peer_id);
            }
        }
    }

    pub fn sync_unpair(&self, peer_id: String) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.unpair(&peer_id);
            }
        }
    }

    /// Broadcast a newly copied item to synced peers.
    pub fn sync_broadcast_item(&self, item: ClipboardItem) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.broadcast_item(&item);
            }
        }
    }

    /// Broadcast a deletion to synced peers.
    pub fn sync_broadcast_deletion(&self, item_id: String) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.broadcast_deletion(&item_id);
            }
        }
    }

    /// Broadcast an update to synced peers.
    pub fn sync_broadcast_update(&self, item: ClipboardItem) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.broadcast_update(&item);
            }
        }
    }

    // ── File transfer ─────────────────────────────────────────

    /// Request a file download from a peer.
    pub fn request_file(&self, peer_id: String, file_path: String) {
        if let Ok(guard) = self.sync_engine.lock() {
            if let Some(ref e) = *guard {
                e.request_file(&peer_id, &file_path);
            }
        }
    }

    // ── Peer management (persisted in Rust, UI just displays) ─

    /// Get all paired peers with their metadata (ID, name, admin status, online status).
    /// Returns JSON strings — each is `{"peerId":..., "displayName":..., "isAdmin":..., "isOnline":...}`.
    pub fn get_paired_peers(&self) -> Vec<String> {
        let storage = self.storage.lock().unwrap();
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

fn find_similar<'a>(
    new_item: &ClipboardItem,
    existing: &'a [ClipboardItem],
) -> Option<&'a ClipboardItem> {
    let transient_types = [
        "com.apple.modified",
        "com.manispocket.from-manispocket",
        "com.apple.linkpresentation.metadata",
        "com.apple.custom-webkit-pasteboard-data",
        "com.apple.pasteboard.promised-file-url",
        "org.chromium.source-url",
        "org.chromium.source-web-custom-data",
        "com.apple.NSItemProvider.Providers",
    ];

    existing.iter().find(|existing| {
        let new_non_transient: Vec<&ClipboardContent> = new_item
            .contents
            .iter()
            .filter(|c| !transient_types.contains(&c.content_type.as_str()))
            .collect();
        if new_non_transient.is_empty() {
            return false;
        }
        new_non_transient.iter().all(|new_content| {
            existing.contents.iter().any(|existing_content| {
                existing_content.content_type == new_content.content_type
                    && existing_content.value == new_content.value
            })
        })
    })
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
