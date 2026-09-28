use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use manis_pocket_sync::{NetworkManager, SharedState, SyncCommand, SyncEvent, SyncState};
use tokio::sync::mpsc;

use crate::model::{ClipboardContent, ClipboardItem, CoreError};
use crate::platform::ClipboardObserver;

/// P2P sync engine wrapping manis-pocket-sync's NetworkManager.
/// Routes all events through the platform's `ClipboardObserver` implementation.
pub struct SyncEngine {
    state: SharedState,
    command_tx: mpsc::UnboundedSender<SyncCommand>,
    #[allow(dead_code)]
    observer: Arc<dyn ClipboardObserver>,
    /// Live set of currently connected peer IDs (updated from events).
    connected_peers: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

#[cfg(test)]
mod tests {
    use super::SyncEngine;
    use crate::model::{ClipboardContent, ClipboardItem};

    #[test]
    fn wire_item_contains_only_portable_text() {
        let item = ClipboardItem {
            id: "test-id".into(),
            application: None,
            first_copied_at: 1_700_000_000_000,
            last_copied_at: 1_700_000_000_000,
            number_of_copies: 1,
            pin: None,
            title: "hello".into(),
            contents: vec![
                ClipboardContent {
                    content_type: "public.file-url".into(),
                    value: Some(b"file:///private/path".to_vec()),
                },
                ClipboardContent {
                    content_type: "public.utf8-plain-text".into(),
                    value: Some(b"hello".to_vec()),
                },
            ],
            sync_timestamp: 1_700_000_000_000,
            sync_source: None,
            sync_deleted: false,
        };
        let json = SyncEngine::serialize_item(&item).unwrap();
        let wire: manis_pocket_sync::SyncItem = serde_json::from_str(&json).unwrap();
        assert_eq!(wire.contents.len(), 1);
        assert_eq!(wire.contents[0].content_type, "text/plain;charset=utf-8");
        assert!(!json.contains("private/path"));
    }
}

impl SyncEngine {
    /// Create and start the sync engine.
    /// Spawns a background thread with its own tokio runtime for the libp2p network.
    /// `keypair_bytes` is persisted between restarts for stable peer ID.
    pub fn start(
        device_name: &str,
        device_id: &str,
        observer: Arc<dyn ClipboardObserver>,
        stored_keypair: Option<Vec<u8>>,
        initial_paired_peer_ids: Vec<String>,
        clipboard_store: Option<PathBuf>,
    ) -> Result<(Self, Vec<u8>), CoreError> {
        let sync_state = SyncState::new(device_name, device_id).map_err(|e| CoreError::Sync {
            msg: format!("Failed to create sync state: {:?}", e),
        })?;
        let state = Arc::new(std::sync::Mutex::new(sync_state));
        let obs = observer.clone();

        let connected = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
        let connected_clone = connected.clone();

        // Register the unified event callback — dispatches to observer trait methods
        {
            let obs = observer.clone();
            state
                .lock()
                .unwrap()
                .on_event
                .lock()
                .replace(Box::new(move |json: &str| {
                    if let Ok(event) = serde_json::from_str::<SyncEvent>(json) {
                        match event {
                            SyncEvent::ItemReceived { item_json, peer_id } => {
                                if let Ok(mut item) = Self::deserialize_item(&item_json) {
                                    item.sync_source = Some(peer_id);
                                    obs.on_item_received(item);
                                }
                            }
                            SyncEvent::CurrentClipboardReceived {
                                event_id,
                                peer_id,
                                text,
                            } => {
                                let now = chrono::Utc::now().timestamp_millis();
                                let contents = text
                                    .clone()
                                    .map(|text| {
                                        vec![ClipboardContent {
                                            content_type: "text/plain;charset=utf-8".into(),
                                            value: Some(text.into_bytes()),
                                        }]
                                    })
                                    .unwrap_or_default();
                                obs.on_item_received(ClipboardItem {
                                    id: event_id,
                                    application: None,
                                    first_copied_at: now,
                                    last_copied_at: now,
                                    number_of_copies: 1,
                                    pin: None,
                                    title: text.unwrap_or_default().chars().take(120).collect(),
                                    contents,
                                    sync_timestamp: now,
                                    sync_source: Some(peer_id),
                                    sync_deleted: false,
                                });
                            }
                            SyncEvent::ItemDeleted { item_id } => {
                                obs.on_item_deleted(item_id);
                            }
                            SyncEvent::ItemUpdated { item_json } => {
                                if let Ok(item) = Self::deserialize_item(&item_json) {
                                    obs.on_item_updated(item);
                                }
                            }
                            SyncEvent::PeerDiscovered { peer } => {
                                if peer.is_connected {
                                    connected_clone.lock().unwrap().insert(peer.peer_id.clone());
                                } else {
                                    connected_clone.lock().unwrap().remove(&peer.peer_id);
                                }
                                obs.on_peer_discovered(
                                    peer.peer_id,
                                    peer.display_name,
                                    peer.addresses,
                                    peer.is_connected,
                                );
                            }
                            SyncEvent::PeerLost { peer_id } => {
                                connected_clone.lock().unwrap().remove(&peer_id);
                                obs.on_peer_lost(peer_id);
                            }
                            SyncEvent::PairingRequest {
                                peer_id,
                                display_name,
                                pin,
                            } => {
                                obs.on_pairing_request(peer_id, display_name, pin);
                            }
                            SyncEvent::PairingComplete { peer_id, success } => {
                                obs.on_pairing_complete(peer_id, success);
                            }
                            SyncEvent::Listening { address } => {
                                obs.on_listening(address);
                            }
                            SyncEvent::Error { code, message } => {
                                obs.on_error(code, message);
                            }
                            SyncEvent::FileRequestReceived { .. } => {
                                // Handled by NetworkManager internally
                            }
                            SyncEvent::FileChunkReceived {
                                request_id,
                                file_name,
                                file_size,
                                chunk_index,
                                total_chunks,
                                data,
                            } => {
                                obs.on_file_chunk(
                                    request_id,
                                    file_name,
                                    file_size as i64,
                                    chunk_index as i32,
                                    total_chunks as i32,
                                    data,
                                );
                            }
                            SyncEvent::FileDownloadComplete {
                                request_id,
                                file_path,
                                success,
                            } => {
                                obs.on_file_download_complete(request_id, file_path, success);
                            }
                        }
                    }
                }));
        }

        // Spawn the network manager in a background thread
        let local_key = if let Some(ref bytes) = stored_keypair {
            libp2p::identity::Keypair::from_protobuf_encoding(bytes).map_err(|e| {
                CoreError::Sync {
                    msg: format!("Invalid stored keypair: {}", e),
                }
            })?
        } else {
            libp2p::identity::Keypair::generate_ed25519()
        };
        let keypair_bytes = local_key
            .to_protobuf_encoding()
            .map_err(|e| CoreError::Sync {
                msg: format!("Failed to encode keypair: {}", e),
            })?;
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let net_state = state.clone();
        let startup_observer = obs.clone();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(_) => return,
            };
            let manager = match clipboard_store {
                Some(path) => {
                    NetworkManager::new_with_storage(command_rx, net_state, local_key, path)
                }
                None => NetworkManager::new(command_rx, net_state, local_key),
            };
            let mut mgr = match manager {
                Ok(m) => m,
                Err(error) => {
                    startup_observer.on_error(
                        error as i32,
                        "Cannot initialize clipboard network or state".into(),
                    );
                    return;
                }
            };
            // Restore paired peers so incoming sync messages aren't dropped after restart.
            mgr.set_initial_paired_peers(initial_paired_peer_ids);
            rt.block_on(mgr.run());
        });

        // Send StartDiscovery so peers can find us
        if let Err(e) = command_tx.send(SyncCommand::StartDiscovery) {
            log::error!("SyncEngine: failed to send StartDiscovery: {:?}", e);
        }

        Ok((
            SyncEngine {
                state,
                command_tx,
                observer: obs,
                connected_peers: connected,
            },
            keypair_bytes,
        ))
    }

    /// Stop the sync engine.
    pub fn stop(&self) {
        let _ = self.command_tx.send(SyncCommand::Shutdown);
    }

    fn send(&self, cmd: SyncCommand) {
        if let Err(e) = self.command_tx.send(cmd) {
            log::error!(
                "SyncEngine: failed to send command (network thread may have crashed): {:?}",
                e
            );
        }
    }

    // ── Peer management ───────────────────────────────────────────

    pub fn add_peer_address(&self, address: &str) {
        self.send(SyncCommand::AddPeerAddress {
            address: address.to_string(),
        });
    }

    pub fn start_discovery(&self) {
        self.send(SyncCommand::StartDiscovery);
    }

    pub fn stop_discovery(&self) {
        self.send(SyncCommand::StopDiscovery);
    }

    // ── Pairing ───────────────────────────────────────────────────

    pub fn request_pairing(&self, peer_id: &str) {
        self.send(SyncCommand::RequestPairing {
            peer_id: peer_id.to_string(),
        });
    }

    pub fn accept_pairing(&self, peer_id: &str, pin: &str) {
        self.send(SyncCommand::AcceptPairing {
            peer_id: peer_id.to_string(),
            pin: pin.to_string(),
        });
    }

    pub fn reject_pairing(&self, peer_id: &str) {
        self.send(SyncCommand::RejectPairing {
            peer_id: peer_id.to_string(),
        });
    }

    pub fn unpair(&self, peer_id: &str) {
        self.send(SyncCommand::Unpair {
            peer_id: peer_id.to_string(),
        });
    }

    // ── Broadcast ─────────────────────────────────────────────────

    pub fn broadcast_item(&self, item: &ClipboardItem) {
        let text = Self::portable_text(item);
        self.observe_local_clipboard(text);
    }

    pub fn observe_local_clipboard(&self, text: Option<String>) {
        self.send(SyncCommand::ObserveLocalClipboard { text });
    }

    pub fn confirm_current_clipboard(&self, event_id: &str, success: bool) {
        self.send(SyncCommand::CurrentClipboardApplied {
            event_id: event_id.to_owned(),
            success,
        });
    }

    pub fn should_apply_current_clipboard(&self, event_id: &str) -> bool {
        let (reply, result) = tokio::sync::oneshot::channel();
        if self
            .command_tx
            .send(SyncCommand::ShouldApplyCurrentClipboard {
                event_id: event_id.to_owned(),
                reply,
            })
            .is_err()
        {
            return false;
        }
        result.blocking_recv().unwrap_or(false)
    }

    pub fn broadcast_deletion(&self, item_id: &str) {
        self.send(SyncCommand::BroadcastDeletion {
            item_id: item_id.to_string(),
        });
    }

    pub fn broadcast_update(&self, item: &ClipboardItem) {
        if let Some(json) = Self::serialize_item(item) {
            self.send(SyncCommand::BroadcastUpdate { item_json: json });
        }
    }

    // ── File transfer ────────────────────────────────────────────

    pub fn request_file(&self, peer_id: &str, file_path: &str) {
        let request_id = uuid::Uuid::new_v4().to_string();
        self.send(SyncCommand::SendFileChunk {
            peer_id: peer_id.to_string(),
            request_id,
            file_path: file_path.to_string(),
            offset: 0,
        });
    }

    /// Check if a peer is currently connected.
    pub fn is_peer_connected(&self, peer_id: &str) -> bool {
        self.connected_peers.lock().unwrap().contains(peer_id)
    }

    // ── Serialization ────────────────────────────────────────────

    fn portable_text(item: &ClipboardItem) -> Option<String> {
        // Clipboard protocol v3 transfers plain UTF-8 text only. Native pasteboard
        // formats can be large, platform-specific, or contain file paths.
        let text = item
            .contents
            .iter()
            .find(|content| {
                matches!(
                    content.content_type.as_str(),
                    "public.utf8-plain-text" | "text/plain" | "text/plain;charset=utf-8"
                )
            })?
            .value
            .as_ref()?;
        if text.is_empty() || text.len() > manis_pocket_sync::register::MAX_CLIPBOARD_TEXT_BYTES {
            return None;
        }
        String::from_utf8(text.clone()).ok()
    }

    fn serialize_item(item: &ClipboardItem) -> Option<String> {
        let text = Self::portable_text(item)?;
        let sync_item = manis_pocket_sync::SyncItem {
            id: item.id.clone(),
            application: item.application.clone(),
            first_copied_at: Self::format_timestamp(item.first_copied_at),
            last_copied_at: Self::format_timestamp(item.last_copied_at),
            number_of_copies: item.number_of_copies as i64,
            pin: item.pin.clone(),
            title: item.title.clone(),
            contents: vec![manis_pocket_sync::SyncItemContent {
                content_type: "text/plain;charset=utf-8".into(),
                value: Some(base64::engine::general_purpose::STANDARD.encode(text.as_bytes())),
            }],
            sync_timestamp: Self::format_timestamp(item.sync_timestamp),
            sync_source: item.sync_source.clone().unwrap_or_default(),
        };
        serde_json::to_string(&sync_item).ok()
    }

    fn deserialize_item(json: &str) -> Result<ClipboardItem, ()> {
        let sync_item: manis_pocket_sync::SyncItem = serde_json::from_str(json).map_err(|_| ())?;
        Ok(ClipboardItem {
            id: sync_item.id,
            application: sync_item.application,
            first_copied_at: Self::parse_timestamp(&sync_item.first_copied_at),
            last_copied_at: Self::parse_timestamp(&sync_item.last_copied_at),
            number_of_copies: sync_item.number_of_copies as i32,
            pin: sync_item.pin,
            title: sync_item.title,
            contents: sync_item
                .contents
                .into_iter()
                .map(|c| ClipboardContent {
                    content_type: c.content_type,
                    value: c.value.map(|v| {
                        base64::engine::general_purpose::STANDARD
                            .decode(v)
                            .unwrap_or_default()
                    }),
                })
                .collect(),
            sync_timestamp: Self::parse_timestamp(&sync_item.sync_timestamp),
            sync_source: if sync_item.sync_source.is_empty() {
                None
            } else {
                Some(sync_item.sync_source)
            },
            sync_deleted: false,
        })
    }

    fn format_timestamp(millis: i64) -> String {
        chrono::DateTime::from_timestamp_millis(millis)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    }

    fn parse_timestamp(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0)
    }
}
