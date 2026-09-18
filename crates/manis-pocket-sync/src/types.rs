use serde::{Deserialize, Serialize};

/// Information about a discovered or paired peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: String,
    pub display_name: String,
    pub addresses: Vec<String>,
    pub is_connected: bool,
}

/// A syncable clipboard item serialized from Swift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItem {
    pub id: String,
    pub application: Option<String>,
    pub first_copied_at: String,
    pub last_copied_at: String,
    pub number_of_copies: i64,
    pub pin: Option<String>,
    pub title: String,
    pub contents: Vec<SyncItemContent>,
    pub sync_timestamp: String,
    pub sync_source: String,
}

/// One content variant of a clipboard item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItemContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub value: Option<String>,
}

/// Messages sent over gossipsub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncMessage {
    ItemAdded {
        item_json: String,
    },
    ItemDeleted {
        id: String,
        timestamp: String,
    },
    ItemUpdated {
        item_json: String,
    },
    Heartbeat {
        device_id: String,
        timestamp: String,
    },
}

/// Pairing protocol messages sent over request-response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PairingMessage {
    Request {
        session_id: String,
        device_name: String,
        device_id: String,
        public_key: Vec<u8>,
    },
    Accept {
        session_id: String,
    },
    Reject {
        session_id: String,
    },
}

/// Sync request for initial/reconnection bulk sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BulkSyncMessage {
    Request { since_timestamp: String },
    Response { items_json: String },
}

/// Unified event sent from Rust to platform shell via a single callback.
/// All events are JSON-serialized and dispatched to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SyncEvent {
    #[serde(rename = "peer_discovered")]
    PeerDiscovered { peer: PeerInfo },
    #[serde(rename = "peer_lost")]
    PeerLost { peer_id: String },
    #[serde(rename = "pairing_request")]
    PairingRequest {
        peer_id: String,
        display_name: String,
        pin: String,
    },
    #[serde(rename = "pairing_complete")]
    PairingComplete { peer_id: String, success: bool },
    #[serde(rename = "item_received")]
    ItemReceived { item_json: String },
    #[serde(rename = "item_deleted")]
    ItemDeleted { item_id: String },
    #[serde(rename = "item_updated")]
    ItemUpdated { item_json: String },
    #[serde(rename = "error")]
    Error { code: i32, message: String },
    #[serde(rename = "listening")]
    Listening { address: String },
    #[serde(rename = "file_request")]
    FileRequestReceived {
        peer_id: String,
        request_id: String,
        file_path: String,
        offset: u64,
    },
    #[serde(rename = "file_chunk")]
    FileChunkReceived {
        request_id: String,
        file_name: String,
        file_size: u64,
        chunk_index: u32,
        total_chunks: u32,
        data: Vec<u8>,
    },
    #[serde(rename = "file_complete")]
    FileDownloadComplete {
        request_id: String,
        file_path: String,
        success: bool,
    },
}

/// Gossipsub topic name.
pub const TOPIC_NAME: &str = "manis-pocket-sync-v1";

/// Protocol name for pairing request-response.
pub const PAIRING_PROTOCOL: &str = "/manis-pocket-sync/pairing/1";

/// Protocol name for bulk sync request-response.
pub const BULK_SYNC_PROTOCOL: &str = "/manis-pocket-sync/bulk/1";

/// Fixed listen port for reliable reconnection.
pub const LISTEN_PORT: u16 = 31774;

// ── File transfer ─────────────────────────────────────────────────

/// Protocol name for file transfer request-response.
pub const FILE_TRANSFER_PROTOCOL: &str = "/manis-pocket-sync/file/1";

/// Metadata extracted from a file clipboard item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    pub size: u64,
}

/// Request to download a file chunk from a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRequest {
    pub request_id: String,
    pub file_path: String,
    pub offset: u64,
}

/// Response containing a chunk of file data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunk {
    pub request_id: String,
    pub file_name: String,
    pub file_size: u64,
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub data: Vec<u8>,
}
