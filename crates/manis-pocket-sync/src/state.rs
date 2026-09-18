use std::sync::Arc;

use parking_lot::Mutex;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::error::ErrorCode;
use crate::types::*;

type EventCallback = Arc<Mutex<Option<Box<dyn Fn(&str) + Send + Sync>>>>;

pub type SharedState = Arc<std::sync::Mutex<SyncState>>;

pub struct SyncState {
    pub runtime: Runtime,
    pub command_tx: mpsc::UnboundedSender<SyncCommand>,
    pub device_id: String,
    pub device_name: String,

    /// Single unified callback — receives JSON-serialized SyncEvent
    pub on_event: EventCallback,
}

impl SyncState {
    pub fn new(device_name: &str, device_id: &str) -> Result<Self, ErrorCode> {
        let runtime = Runtime::new().map_err(|_| ErrorCode::Init)?;
        let (command_tx, _command_rx) = mpsc::unbounded_channel();

        Ok(Self {
            runtime,
            command_tx,
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
            on_event: Arc::new(Mutex::new(None)),
        })
    }

    /// Emit a SyncEvent as JSON to the platform shell.
    pub fn emit(&self, event: SyncEvent) {
        if let Ok(json) = serde_json::to_string(&event) {
            log::debug!("emit: {}", json);
            if let Some(cb) = self.on_event.lock().as_ref() {
                cb(&json);
            }
        }
    }

    pub fn emit_error(&self, code: ErrorCode, message: String) {
        self.emit(SyncEvent::Error {
            code: code as i32,
            message,
        });
    }
}

#[derive(Debug)]
pub enum SyncCommand {
    StartDiscovery,
    StopDiscovery,
    RequestPairing {
        peer_id: String,
    },
    AcceptPairing {
        peer_id: String,
        pin: String,
    },
    RejectPairing {
        peer_id: String,
    },
    BroadcastItem {
        item_json: String,
    },
    BroadcastDeletion {
        item_id: String,
    },
    BroadcastUpdate {
        item_json: String,
    },
    AddPeerAddress {
        address: String,
    },
    Unpair {
        peer_id: String,
    },
    /// Send a file chunk to a peer.
    SendFileChunk {
        peer_id: String,
        request_id: String,
        file_path: String,
        offset: u64,
    },
    Shutdown,
}
