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
                            SyncEvent::ItemReceived { item_json } => {
                                if let Ok(item) = Self::deserialize_item(&item_json) {
                                    obs.on_item_received(item);
                                }
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

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(_) => return,
            };
            let mut mgr = match NetworkManager::new(command_rx, net_state, local_key) {
                Ok(m) => m,
                Err(_) => return,
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
        let json = Self::serialize_item(item);
        self.send(SyncCommand::BroadcastItem { item_json: json });
    }

    pub fn broadcast_deletion(&self, item_id: &str) {
        self.send(SyncCommand::BroadcastDeletion {
            item_id: item_id.to_string(),
        });
    }

    pub fn broadcast_update(&self, item: &ClipboardItem) {
        let json = Self::serialize_item(item);
        self.send(SyncCommand::BroadcastUpdate { item_json: json });
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

    fn serialize_item(item: &ClipboardItem) -> String {
        let sync_item = manis_pocket_sync::SyncItem {
            id: item.id.clone(),
            application: item.application.clone(),
            first_copied_at: Self::format_timestamp(item.first_copied_at),
            last_copied_at: Self::format_timestamp(item.last_copied_at),
            number_of_copies: item.number_of_copies as i64,
            pin: item.pin.clone(),
            title: item.title.clone(),
            contents: item
                .contents
                .iter()
                .map(|c| manis_pocket_sync::SyncItemContent {
                    content_type: c.content_type.clone(),
                    value: c
                        .value
                        .as_ref()
                        .map(|v| base64::engine::general_purpose::STANDARD.encode(v)),
                })
                .collect(),
            sync_timestamp: Self::format_timestamp(item.sync_timestamp),
            sync_source: item.sync_source.clone().unwrap_or_default(),
        };
        serde_json::to_string(&sync_item).unwrap_or_default()
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
