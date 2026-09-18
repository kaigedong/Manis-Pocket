use crate::model::ClipboardItem;

/// Callback interface that platforms implement to receive sync events.
/// Platforms (Swift/Kotlin) implement this via UniFFI foreign trait.
#[uniffi::export(with_foreign)]
pub trait ClipboardObserver: Send + Sync + std::fmt::Debug {
    // ── Clipboard events ──────────────────────────────────────────

    /// A new clipboard item was received from a synced peer.
    fn on_item_received(&self, item: ClipboardItem);

    /// An item was deleted by a synced peer.
    fn on_item_deleted(&self, item_id: String);

    /// An item was updated (e.g., appended) by a synced peer.
    fn on_item_updated(&self, item: ClipboardItem);

    // ── Peer discovery events ─────────────────────────────────────

    /// A peer was discovered on the network.
    fn on_peer_discovered(
        &self,
        peer_id: String,
        display_name: String,
        addresses: Vec<String>,
        is_connected: bool,
    );

    /// A peer disconnected or went offline.
    fn on_peer_lost(&self, peer_id: String);

    // ── Pairing events ────────────────────────────────────────────

    /// A pairing request was received from a peer.
    fn on_pairing_request(&self, peer_id: String, display_name: String, pin: String);

    /// A pairing request completed (accepted or rejected).
    fn on_pairing_complete(&self, peer_id: String, success: bool);

    // ── Status events ─────────────────────────────────────────────

    /// Sync is now listening on the given multiaddress.
    fn on_listening(&self, address: String);

    /// An error occurred in the sync engine.
    fn on_error(&self, code: i32, message: String);

    // ── File transfer ─────────────────────────────────────────

    /// A file chunk was received (append to local temp file).
    fn on_file_chunk(
        &self,
        request_id: String,
        file_name: String,
        file_size: i64,
        chunk_index: i32,
        total_chunks: i32,
        data: Vec<u8>,
    );

    /// File download completed (or failed).
    fn on_file_download_complete(&self, request_id: String, file_path: String, success: bool);
}
