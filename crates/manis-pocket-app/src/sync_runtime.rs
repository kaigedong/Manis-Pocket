//! Runs the P2P sync engine inside the app.
//!
//! The core owns the network protocol; this module bridges it to the two things
//! the app needs: history that reflects what peers send, and a stream of events
//! the UI can render (discovered peers, pairing requests, errors).
//!
//! The observer runs on libp2p's threads, so it never touches GPUI state — it
//! pushes into a channel that the UI polls on its own tick.

use futures::channel::mpsc::UnboundedSender;
use manis_pocket_core::history::HistoryManager;
use manis_pocket_core::model::ClipboardItem;
use manis_pocket_core::platform::ClipboardObserver;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;

use crate::settings::Settings;

/// Something that happened in sync, surfaced to the UI.
#[derive(Clone, Debug)]
#[allow(dead_code)] // pairing fields are shown by the settings window next
pub enum SyncNotice {
    /// Items changed; the list should reload.
    HistoryChanged,
    PeersChanged,
    PairingRequest {
        peer_id: String,
        display_name: String,
        pin: String,
    },
    Listening(String),
    Error(String),
}

/// Peers this instance asked to pair with.
///
/// The protocol needs an explicit confirmation from *both* sides. A user who
/// pressed "pair" has already consented, so the request is remembered here and
/// confirmed automatically when its own pairing event comes back.
static REQUESTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Marks a peer as "we asked to pair", so its event is auto-confirmed.
pub fn remember_pairing(peer_id: &str) {
    remember_request(peer_id);
}

fn remember_request(peer_id: &str) {
    if let Ok(mut slot) = REQUESTED.lock() {
        slot.get_or_insert_with(HashSet::new)
            .insert(peer_id.to_string());
    }
}

fn take_request(peer_id: &str) -> bool {
    REQUESTED
        .lock()
        .ok()
        .and_then(|mut slot| slot.as_mut().map(|set| set.remove(peer_id)))
        .unwrap_or(false)
}

/// Applies peer traffic to the local history and forwards events.
pub struct AppObserver {
    history: Arc<HistoryManager>,
    max_size: i32,
    unlimited: bool,
    tx: UnboundedSender<SyncNotice>,
}

impl std::fmt::Debug for AppObserver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AppObserver")
    }
}

impl AppObserver {
    fn notify(&self, notice: SyncNotice) {
        let _ = self.tx.unbounded_send(notice);
    }
}

impl ClipboardObserver for AppObserver {
    fn on_item_received(&self, item: ClipboardItem) {
        let from_peer = item.sync_source.is_some();
        let event_id = item.id.clone();
        let content = item.contents.first().cloned();

        // Peers send entries the same way the local clipboard produces them, so
        // the normal add path (dedup, size limit) applies.
        if self
            .history
            .add(item, self.max_size, self.unlimited)
            .is_ok()
        {
            self.notify(SyncNotice::HistoryChanged);
        }

        if from_peer {
            // The engine holds the update until the platform says it landed on
            // the clipboard; without this confirmation every later update is
            // answered with Retry and sync jams after the first entry.
            let applied = content
                .as_ref()
                .map(crate::clipboard::apply_remote)
                .unwrap_or(false);
            self.history
                .sync_confirm_current_clipboard(event_id, applied);
        }
    }

    fn on_item_deleted(&self, item_id: String) {
        if self.history.delete(item_id).is_ok() {
            self.notify(SyncNotice::HistoryChanged);
        }
    }

    fn on_item_updated(&self, item: ClipboardItem) {
        // Updates replace the stored text of an existing entry.
        let text = item
            .contents
            .iter()
            .find(|content| content.content_type.starts_with("text/"))
            .and_then(|content| content.value.as_ref())
            .and_then(|value| String::from_utf8(value.clone()).ok());
        if let Some(text) = text {
            let _ = self.history.update_item_text(item.id, text);
            self.notify(SyncNotice::HistoryChanged);
        }
    }

    fn on_peer_discovered(
        &self,
        peer_id: String,
        display_name: String,
        addresses: Vec<String>,
        is_connected: bool,
    ) {
        // Pairing is driven by peer id, which is the one thing a terminal needs
        // to see, so it is printed rather than only shown in the UI.
        eprintln!(
            "[sync] peer {display_name} ({peer_id}) connected={is_connected} addrs={}",
            addresses.join(",")
        );
        self.notify(SyncNotice::PeersChanged);
    }

    fn on_peer_lost(&self, peer_id: String) {
        eprintln!("[sync] peer lost: {peer_id}");
        self.notify(SyncNotice::PeersChanged);
    }

    fn on_pairing_request(&self, peer_id: String, display_name: String, pin: String) {
        eprintln!("[sync] pairing request from {display_name} ({peer_id}) pin={pin}");
        if take_request(&peer_id) {
            // We started this pairing, so confirm our own side immediately; the
            // peer still has to accept on its own.
            let _ = self
                .history
                .sync_accept_pairing(peer_id.clone(), pin.clone());
            eprintln!("[sync] auto-confirmed our side of the pairing with {peer_id}");
        }
        self.notify(SyncNotice::PairingRequest {
            peer_id,
            display_name,
            pin,
        });
    }

    fn on_pairing_complete(&self, peer_id: String, success: bool) {
        eprintln!("[sync] pairing with {peer_id} complete: {success}");
        if success {
            // The engine holds pairs in memory only; without persisting it a
            // restart would silently drop the peer.
            let name = peer_id.clone();
            self.history.save_paired_peer(peer_id, name, false);
        }
        self.notify(SyncNotice::PeersChanged);
    }

    fn on_listening(&self, address: String) {
        eprintln!("[sync] listening on {address}");
        self.notify(SyncNotice::Listening(address));
    }

    fn on_error(&self, _code: i32, message: String) {
        eprintln!("[sync] error: {message}");
        self.notify(SyncNotice::Error(message));
    }

    fn on_file_chunk(
        &self,
        _request_id: String,
        _file_name: String,
        _file_size: i64,
        _chunk_index: i32,
        _total_chunks: i32,
        _data: Vec<u8>,
    ) {
        // File transfer is not wired into the UI yet.
    }

    fn on_file_download_complete(&self, _request_id: String, _file_path: String, _success: bool) {
        // File transfer is not wired into the UI yet.
    }
}

/// Starts sync when the user has it enabled. Returns whether it is running.
pub fn start(
    history: Arc<HistoryManager>,
    settings: &Settings,
    tx: UnboundedSender<SyncNotice>,
) -> bool {
    let observer = Arc::new(AppObserver {
        history: history.clone(),
        max_size: settings.history_size as i32,
        unlimited: settings.unlimited_history,
        tx,
    });

    match history.start_sync(
        settings.sync_device_name.clone(),
        settings.sync_device_id.clone(),
        observer,
    ) {
        Ok(()) => true,
        Err(error) => {
            // Sync is optional; the clipboard manager works without it.
            eprintln!("sync disabled: could not start engine: {error:?}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use manis_pocket_core::model::ClipboardContent;

    fn observer() -> (
        Arc<HistoryManager>,
        AppObserver,
        futures::channel::mpsc::UnboundedReceiver<SyncNotice>,
    ) {
        let history = Arc::new(HistoryManager::new(":memory:".to_string()).unwrap());
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let observer = AppObserver {
            history: history.clone(),
            max_size: 1000,
            unlimited: false,
            tx,
        };
        (history, observer, rx)
    }

    fn item(id: &str, text: &str) -> ClipboardItem {
        ClipboardItem {
            id: id.into(),
            application: None,
            first_copied_at: 1,
            last_copied_at: 1,
            number_of_copies: 1,
            pin: None,
            title: text.into(),
            contents: vec![ClipboardContent {
                content_type: "public.utf8-plain-text".into(),
                value: Some(text.as_bytes().to_vec()),
            }],
            sync_timestamp: 1,
            sync_source: None,
            sync_deleted: false,
        }
    }

    #[test]
    fn peer_items_land_in_history_and_notify_the_ui() {
        let (history, observer, mut rx) = observer();
        observer.on_item_received(item("peer-1", "from the phone"));

        let stored = history.load_metadata().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].title, "from the phone");
        assert!(matches!(
            rx.try_next(),
            Ok(Some(SyncNotice::HistoryChanged))
        ));
    }

    #[test]
    fn peer_deletions_remove_the_entry() {
        let (history, observer, mut rx) = observer();
        observer.on_item_received(item("peer-1", "temporary"));
        let _ = rx.try_next();

        observer.on_item_deleted("peer-1".to_string());
        assert!(history.load_metadata().unwrap().is_empty());
        assert!(matches!(
            rx.try_next(),
            Ok(Some(SyncNotice::HistoryChanged))
        ));
    }

    #[test]
    fn pairing_requests_reach_the_ui_with_their_pin() {
        let (_history, observer, mut rx) = observer();
        observer.on_pairing_request("peer-9".into(), "Pixel".into(), "123456".into());
        match rx.try_next() {
            Ok(Some(SyncNotice::PairingRequest {
                peer_id,
                display_name,
                pin,
            })) => {
                assert_eq!(peer_id, "peer-9");
                assert_eq!(display_name, "Pixel");
                assert_eq!(pin, "123456");
            }
            other => panic!("unexpected notice: {other:?}"),
        }
    }
}
