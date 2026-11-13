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

/// A device seen on the local network, paired or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveredPeer {
    pub(crate) peer_id: String,
    pub(crate) display_name: String,
    pub(crate) is_connected: bool,
}

/// A pairing request waiting for the user to compare the code and answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingPairing {
    pub(crate) peer_id: String,
    pub(crate) display_name: String,
    pub(crate) pin: String,
}

/// What the engine last reported, so the settings window can say whether sync is
/// actually running instead of leaving it to the log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SyncStatus {
    pub(crate) listening: Option<String>,
    pub(crate) last_error: Option<String>,
}

static STATUS: Mutex<SyncStatus> = Mutex::new(SyncStatus {
    listening: None,
    last_error: None,
});

/// The engine's current state, as the settings window shows it.
pub(crate) fn status() -> SyncStatus {
    STATUS
        .lock()
        .map(|status| status.clone())
        .unwrap_or_default()
}

fn remember_listening(address: &str) {
    if let Ok(mut status) = STATUS.lock() {
        status.listening = Some(address.to_string());
        status.last_error = None;
    }
}

fn remember_error(message: &str) {
    if let Ok(mut status) = STATUS.lock() {
        status.last_error = Some(message.to_string());
    }
}

/// What mDNS has reported since launch. Replaced wholesale on every update so the
/// UI always sees a consistent snapshot.
static DISCOVERED: Mutex<Vec<DiscoveredPeer>> = Mutex::new(Vec::new());
/// Pairing requests nobody has answered yet.
static PENDING: Mutex<Vec<PendingPairing>> = Mutex::new(Vec::new());

/// The devices currently visible on the network.
pub(crate) fn discovered_peers() -> Vec<DiscoveredPeer> {
    DISCOVERED
        .lock()
        .map(|peers| peers.clone())
        .unwrap_or_default()
}

/// The pairing requests waiting for an answer.
pub(crate) fn pending_pairings() -> Vec<PendingPairing> {
    PENDING.lock().map(|list| list.clone()).unwrap_or_default()
}

/// Drops a pairing request once it has been accepted or rejected.
pub(crate) fn forget_pairing(peer_id: &str) {
    if let Ok(mut list) = PENDING.lock() {
        list.retain(|pending| pending.peer_id != peer_id);
    }
}

/// Records a discovered peer. `upsert` keeps the display name and connection state
/// of a device that mDNS reports repeatedly.
fn remember_discovered(peer: DiscoveredPeer) {
    if let Ok(mut peers) = DISCOVERED.lock() {
        match peers.iter_mut().find(|known| known.peer_id == peer.peer_id) {
            Some(known) => {
                if !peer.display_name.is_empty() {
                    known.display_name = peer.display_name;
                }
                known.is_connected = peer.is_connected;
            }
            None => peers.push(peer),
        }
    }
}

fn forget_discovered(peer_id: &str) {
    if let Ok(mut peers) = DISCOVERED.lock() {
        peers.retain(|peer| peer.peer_id != peer_id);
    }
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
        // Keep the last address and error so the settings window can report the
        // engine's state without a log.
        match &notice {
            SyncNotice::Listening(address) => remember_listening(address),
            SyncNotice::Error(message) => remember_error(message),
            _ => {}
        }
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
        // to see, so it is printed as well as shown in the settings window.
        eprintln!(
            "[sync] peer {display_name} ({peer_id}) connected={is_connected} addrs={}",
            addresses.join(",")
        );
        remember_discovered(DiscoveredPeer {
            peer_id,
            display_name,
            is_connected,
        });
        self.notify(SyncNotice::PeersChanged);
    }

    fn on_peer_lost(&self, peer_id: String) {
        eprintln!("[sync] peer lost: {peer_id}");
        forget_discovered(&peer_id);
        self.notify(SyncNotice::PeersChanged);
    }

    fn on_pairing_request(&self, peer_id: String, display_name: String, pin: String) {
        eprintln!("[sync] pairing request from {display_name} ({peer_id}) pin={pin}");
        if take_request(&peer_id) {
            // We started this pairing, so confirm our own side immediately; the
            // peer still has to accept on its own.
            self.history
                .sync_accept_pairing(peer_id.clone(), pin.clone());
            eprintln!("[sync] auto-confirmed our side of the pairing with {peer_id}");
        }
        if let Ok(mut list) = PENDING.lock()
            && !list.iter().any(|pending| pending.peer_id == peer_id)
        {
            list.push(PendingPairing {
                peer_id: peer_id.clone(),
                display_name: display_name.clone(),
                pin: pin.clone(),
            });
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
        assert!(matches!(rx.try_recv(), Ok(SyncNotice::HistoryChanged)));
    }

    #[test]
    fn peer_deletions_remove_the_entry() {
        let (history, observer, mut rx) = observer();
        observer.on_item_received(item("peer-1", "temporary"));
        let _ = rx.try_recv();

        observer.on_item_deleted("peer-1".to_string());
        assert!(history.load_metadata().unwrap().is_empty());
        assert!(matches!(rx.try_recv(), Ok(SyncNotice::HistoryChanged)));
    }

    #[test]
    fn pairing_requests_reach_the_ui_with_their_pin() {
        let (_history, observer, mut rx) = observer();
        observer.on_pairing_request("peer-9".into(), "Pixel".into(), "123456".into());
        match rx.try_recv() {
            Ok(SyncNotice::PairingRequest {
                peer_id,
                display_name,
                pin,
            }) => {
                assert_eq!(peer_id, "peer-9");
                assert_eq!(display_name, "Pixel");
                assert_eq!(pin, "123456");
            }
            other => panic!("unexpected notice: {other:?}"),
        }
    }

    #[test]
    fn a_discovered_peer_is_listed_once_and_updates_its_state() {
        let peer_id = "test-peer-once";
        remember_discovered(DiscoveredPeer {
            peer_id: peer_id.into(),
            display_name: String::new(),
            is_connected: false,
        });
        remember_discovered(DiscoveredPeer {
            peer_id: peer_id.into(),
            display_name: "Pixel".into(),
            is_connected: true,
        });
        let listed: Vec<DiscoveredPeer> = discovered_peers()
            .into_iter()
            .filter(|peer| peer.peer_id == peer_id)
            .collect();
        assert_eq!(listed.len(), 1, "mDNS reports a peer repeatedly");
        assert_eq!(listed[0].display_name, "Pixel");
        assert!(listed[0].is_connected);

        forget_discovered(peer_id);
        assert!(
            discovered_peers()
                .iter()
                .all(|peer| peer.peer_id != peer_id)
        );
    }

    #[test]
    fn the_status_reports_the_last_error_until_the_engine_listens_again() {
        remember_error("connection refused");
        assert_eq!(
            status().last_error.as_deref(),
            Some("connection refused"),
            "a failure is kept for the settings window to show"
        );

        remember_listening("/ip4/0.0.0.0/tcp/31774");
        let status = status();
        assert_eq!(status.listening.as_deref(), Some("/ip4/0.0.0.0/tcp/31774"));
        assert!(
            status.last_error.is_none(),
            "a working listen address means the old error is stale"
        );
    }

    #[test]
    fn a_pairing_request_stays_until_it_is_answered() {
        let peer_id = "test-peer-pairing";
        if let Ok(mut list) = PENDING.lock() {
            list.push(PendingPairing {
                peer_id: peer_id.into(),
                display_name: "Phone".into(),
                pin: "123456".into(),
            });
        }
        forget_pairing(peer_id);
        assert!(
            pending_pairings()
                .iter()
                .all(|pending| pending.peer_id != peer_id)
        );
    }
}
