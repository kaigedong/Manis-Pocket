use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::Engine;
use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic, MessageAuthenticity, MessageId};
use libp2p::identity::Keypair;
use libp2p::mdns;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{PeerId, SwarmBuilder};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::error::ErrorCode;
use crate::register::{self, ClipboardRegister, ClipboardUpdate, ClipboardValue, MergeResult};
use crate::state::{SharedState, SyncCommand};
use crate::types::*;

const PAIRING_TOPIC: &str = "manis-pocket-sync-pairing-v2";
const PAIRING_TIMEOUT: Duration = Duration::from_secs(300);
const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);

struct PendingClipboard {
    peer: PeerId,
    update: ClipboardUpdate,
    channel: Option<request_response::ResponseChannel<ClipboardResponse>>,
    created_at: Instant,
}

struct PendingPairing {
    session_id: String,
    pin: String,
    local_confirmed: bool,
    remote_confirmed: bool,
    created_at: Instant,
}

#[derive(NetworkBehaviour)]
pub struct ManisPocketBehaviour {
    pub mdns: mdns::tokio::Behaviour,
    pub gossipsub: gossipsub::Behaviour,
    pub identify: libp2p::identify::Behaviour,
    pub clipboard_sync: request_response::cbor::Behaviour<ClipboardRequest, ClipboardResponse>,
    pub file_transfer: request_response::cbor::Behaviour<FileRequest, FileChunk>,
}

pub struct NetworkManager {
    swarm: libp2p::Swarm<ManisPocketBehaviour>,
    command_rx: mpsc::UnboundedReceiver<SyncCommand>,
    state: SharedState,
    discovered_peers: HashMap<PeerId, PeerInfo>,
    paired_peers: HashSet<PeerId>,
    pending_pairings: HashMap<PeerId, PendingPairing>,
    clipboard: ClipboardRegister,
    clipboard_store: Option<PathBuf>,
    clipboard_ready: bool,
    pending_persist: bool,
    pending_clipboard: Option<PendingClipboard>,
    listen_port: u16,
    local_peer_id: PeerId,
    /// IP:port strings of our own listen addresses (collected from NewListenAddr).
    /// Used to avoid dialing ourselves when mDNS returns self-reflected addresses.
    local_addrs: HashSet<String>,
}

impl NetworkManager {
    pub fn new(
        command_rx: mpsc::UnboundedReceiver<SyncCommand>,
        state: SharedState,
        local_key: Keypair,
    ) -> Result<Self, ErrorCode> {
        Self::build(command_rx, state, local_key, LISTEN_PORT, None)
    }

    pub fn new_with_storage(
        command_rx: mpsc::UnboundedReceiver<SyncCommand>,
        state: SharedState,
        local_key: Keypair,
        store_path: PathBuf,
    ) -> Result<Self, ErrorCode> {
        Self::build(command_rx, state, local_key, LISTEN_PORT, Some(store_path))
    }

    pub fn new_with_port(
        command_rx: mpsc::UnboundedReceiver<SyncCommand>,
        state: SharedState,
        local_key: Keypair,
        port: u16,
    ) -> Result<Self, ErrorCode> {
        Self::build(command_rx, state, local_key, port, None)
    }

    pub fn new_with_port_and_storage(
        command_rx: mpsc::UnboundedReceiver<SyncCommand>,
        state: SharedState,
        local_key: Keypair,
        port: u16,
        store_path: PathBuf,
    ) -> Result<Self, ErrorCode> {
        Self::build(command_rx, state, local_key, port, Some(store_path))
    }

    fn build(
        command_rx: mpsc::UnboundedReceiver<SyncCommand>,
        state: SharedState,
        local_key: Keypair,
        listen_port: u16,
        clipboard_store: Option<PathBuf>,
    ) -> Result<Self, ErrorCode> {
        let local_peer_id = PeerId::from(local_key.public());
        let clipboard = match clipboard_store.as_deref() {
            Some(path) => {
                register::load_register(path, &local_peer_id.to_string()).map_err(|error| {
                    log::error!("Cannot load clipboard state: {error}");
                    ErrorCode::Init
                })?
            }
            None => ClipboardRegister::default(),
        };

        let mdns_config = mdns::Config {
            query_interval: Duration::from_secs(5),
            ttl: Duration::from_secs(120),
            ..mdns::Config::default()
        };
        let mdns_behaviour =
            mdns::tokio::Behaviour::new(mdns_config, local_peer_id).map_err(|_| ErrorCode::Init)?;

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(1))
            .validation_mode(gossipsub::ValidationMode::Strict)
            .message_id_fn(|msg: &gossipsub::Message| {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&msg.data, &mut hasher);
                MessageId::from(std::hash::Hasher::finish(&hasher).to_string())
            })
            .build()
            .map_err(|_| ErrorCode::Init)?;

        let gossipsub_behaviour = gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(local_key.clone()),
            gossipsub_config,
        )
        .map_err(|_| ErrorCode::Init)?;

        let identify = libp2p::identify::Behaviour::new(
            libp2p::identify::Config::new(PAIRING_PROTOCOL.to_string(), local_key.public())
                .with_agent_version(format!(
                    "manis-pocket-sync/0.1.0/{}",
                    state.lock().unwrap().device_name,
                )),
        );

        let file_transfer = request_response::cbor::Behaviour::new(
            [(
                libp2p::StreamProtocol::new(FILE_TRANSFER_PROTOCOL),
                ProtocolSupport::Full,
            )],
            request_response::Config::default(),
        );

        let clipboard_sync = request_response::cbor::Behaviour::new(
            [(
                libp2p::StreamProtocol::new(CLIPBOARD_PROTOCOL),
                ProtocolSupport::Full,
            )],
            request_response::Config::default(),
        );

        let behaviour = ManisPocketBehaviour {
            mdns: mdns_behaviour,
            gossipsub: gossipsub_behaviour,
            identify,
            clipboard_sync,
            file_transfer,
        };

        let swarm = SwarmBuilder::with_existing_identity(local_key)
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default(),
                libp2p::noise::Config::new,
                libp2p::yamux::Config::default,
            )
            .map_err(|_| ErrorCode::Init)?
            .with_behaviour(|_| behaviour)
            .map_err(|_| ErrorCode::Init)?
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        Ok(Self {
            swarm,
            command_rx,
            state,
            discovered_peers: HashMap::new(),
            paired_peers: HashSet::new(),
            pending_pairings: HashMap::new(),
            clipboard,
            clipboard_ready: clipboard_store.is_none(),
            pending_persist: false,
            clipboard_store,
            pending_clipboard: None,
            listen_port,
            local_peer_id,
            local_addrs: HashSet::new(),
        })
    }

    /// Restore the in-memory paired-peer set from the persisted DB on startup.
    /// Without this, the `paired_peers` gate (which controls whether incoming
    /// sync messages are handled) is empty after a restart, so already-paired
    /// peers silently can't sync until they pair again.
    pub fn set_initial_paired_peers(&mut self, peer_ids: Vec<String>) {
        for id in peer_ids {
            if let Ok(peer) = id.parse::<PeerId>() {
                self.paired_peers.insert(peer);
            }
        }
        log::info!(
            "Restored {} paired peer(s) from DB",
            self.paired_peers.len()
        );
    }

    pub async fn run(&mut self) {
        let pairing_topic = IdentTopic::new(PAIRING_TOPIC);
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&pairing_topic)
        {
            log::error!("Failed to subscribe to pairing topic: {:?}", e);
        }

        let listen_addr: libp2p::Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", self.listen_port)
            .parse()
            .unwrap();
        if self.swarm.listen_on(listen_addr).is_err() {
            self.emit_error(ErrorCode::Network, "Failed to listen on port".into());
            return;
        }

        let mut reconcile = tokio::time::interval(RECONCILE_INTERVAL);
        reconcile.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => {
                    self.handle_swarm_event(event).await;
                }
                Some(command) = self.command_rx.recv() => {
                    if matches!(command, SyncCommand::Shutdown) {
                        break;
                    }
                    self.handle_command(command);
                }
                _ = reconcile.tick() => self.reconcile_all(),
            }
        }
    }

    fn state_emit(&self, event: SyncEvent) {
        let state = self.state.lock().unwrap();
        state.emit(event);
    }

    fn emit_error(&self, code: ErrorCode, msg: String) {
        let state = self.state.lock().unwrap();
        state.emit_error(code, msg);
    }

    // ── Swarm events ──────────────────────────────────────────────

    async fn handle_swarm_event(&mut self, event: SwarmEvent<ManisPocketBehaviourEvent>) {
        match event {
            // mDNS: peer found on LAN — store but don't emit until Identify gives us a name
            SwarmEvent::Behaviour(ManisPocketBehaviourEvent::Mdns(mdns::Event::Discovered(
                peers,
            ))) => {
                for (peer_id, addr) in peers {
                    if peer_id == self.local_peer_id {
                        continue;
                    }
                    // Skip self-dial only for NEW peers — known peers may have
                    // correct addresses from Identify that differ from the mDNS addr.
                    let is_known = self.discovered_peers.contains_key(&peer_id);
                    if !is_known && is_own_addr(&addr, &self.local_addrs) {
                        log::debug!("Skipping self-dial for new peer {}: {}", peer_id, addr);
                        continue;
                    }
                    let info = PeerInfo {
                        peer_id: peer_id.to_string(),
                        display_name: String::new(),
                        addresses: vec![addr.to_string()],
                        is_connected: false,
                    };
                    self.discovered_peers.insert(peer_id, info);
                    if let Err(e) = self.swarm.dial(peer_id) {
                        log::debug!("Dial failed for {}: {:?}", peer_id, e);
                    }
                }
            }
            SwarmEvent::Behaviour(ManisPocketBehaviourEvent::Mdns(mdns::Event::Expired(peers))) => {
                for (peer_id, _) in peers {
                    if let Some(info) = self.discovered_peers.remove(&peer_id) {
                        self.state_emit(SyncEvent::PeerLost {
                            peer_id: info.peer_id,
                        });
                    }
                }
            }

            // Identify: now we know the device name — emit to UI
            SwarmEvent::Behaviour(ManisPocketBehaviourEvent::Identify(
                libp2p::identify::Event::Received { peer_id, info, .. },
            )) => {
                if peer_id == self.local_peer_id {
                    return; // Don't show ourselves
                }
                let device_name = info
                    .agent_version
                    .split('/')
                    .last()
                    .unwrap_or("Unknown")
                    .to_string();
                log::info!("Identified {} as {}", peer_id, device_name);
                let observed_addr = info.observed_addr.to_string();
                let listen_addrs: Vec<String> =
                    info.listen_addrs.iter().map(|a| a.to_string()).collect();

                if let Some(peer_info) = self.discovered_peers.get_mut(&peer_id) {
                    peer_info.display_name = device_name;
                    if !listen_addrs.is_empty() {
                        peer_info.addresses = listen_addrs;
                    } else if !observed_addr.is_empty() {
                        peer_info.addresses = vec![observed_addr];
                    }
                    let updated = peer_info.clone();
                    self.state_emit(SyncEvent::PeerDiscovered { peer: updated });
                } else {
                    let peer_info = PeerInfo {
                        peer_id: peer_id.to_string(),
                        display_name: device_name,
                        addresses: if !listen_addrs.is_empty() {
                            listen_addrs
                        } else {
                            vec![observed_addr]
                        },
                        is_connected: true,
                    };
                    self.discovered_peers.insert(peer_id, peer_info.clone());
                    self.state_emit(SyncEvent::PeerDiscovered { peer: peer_info });
                }
            }

            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                log::info!("Connection established with {}", peer_id);
                if self.paired_peers.contains(&peer_id) {
                    self.request_head(peer_id);
                }
                if let Some(peer_info) = self.discovered_peers.get_mut(&peer_id) {
                    peer_info.is_connected = true;
                    // Only emit if we already have a display_name from Identify
                    if !peer_info.display_name.is_empty() {
                        let info = peer_info.clone();
                        self.state_emit(SyncEvent::PeerDiscovered { peer: info });
                    }
                }
                // If not in discovered_peers yet, Identify will handle it
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                log::warn!("Connection closed with {}", peer_id);
                if let Some(peer_info) = self.discovered_peers.get_mut(&peer_id) {
                    peer_info.is_connected = false;
                    if !peer_info.display_name.is_empty() {
                        let info = peer_info.clone();
                        self.state_emit(SyncEvent::PeerDiscovered { peer: info });
                    }
                }
            }

            // Gossipsub messages
            SwarmEvent::Behaviour(ManisPocketBehaviourEvent::Gossipsub(
                gossipsub::Event::Message { message, .. },
            )) => {
                if message.topic.as_str() == PAIRING_TOPIC {
                    if let (Some(source), Ok(pairing_msg)) = (
                        message.source,
                        serde_json::from_slice::<PairingMessage>(&message.data),
                    ) {
                        self.handle_pairing_message(source, pairing_msg);
                    }
                }
            }

            SwarmEvent::Behaviour(ManisPocketBehaviourEvent::ClipboardSync(
                request_response::Event::Message { peer, message, .. },
            )) => match message {
                request_response::Message::Request {
                    request, channel, ..
                } => {
                    self.handle_clipboard_request(peer, request, channel);
                }
                request_response::Message::Response { response, .. } => {
                    self.handle_clipboard_response(peer, response);
                }
            },
            SwarmEvent::Behaviour(ManisPocketBehaviourEvent::ClipboardSync(
                request_response::Event::OutboundFailure { peer, error, .. },
            )) => {
                log::warn!(
                    "Clipboard delivery to {peer} failed: {error}; reconciliation will retry"
                );
            }

            SwarmEvent::NewListenAddr { address, .. } => {
                log::info!("Listening on {}", address);
                self.local_addrs.insert(address.to_string());
                self.state_emit(SyncEvent::Listening {
                    address: address.to_string(),
                });
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                log::error!("Outgoing connection error: {:?} ({:?})", peer_id, error);
                self.emit_error(
                    ErrorCode::Network,
                    format!("Connection failed: {:?}", error),
                );
            }
            SwarmEvent::IncomingConnectionError { error, .. } => {
                log::error!("Incoming connection error: {:?}", error);
            }
            SwarmEvent::ListenerError { error, .. } => {
                log::error!("Listener error: {:?}", error);
            }

            // ── File transfer ────────────────────────────────
            SwarmEvent::Behaviour(ManisPocketBehaviourEvent::FileTransfer(
                request_response::Event::Message { peer, message, .. },
            )) => {
                self.handle_file_transfer_message(peer, message);
            }
            SwarmEvent::Behaviour(ManisPocketBehaviourEvent::FileTransfer(
                request_response::Event::OutboundFailure {
                    peer,
                    request_id,
                    error,
                    ..
                },
            )) => {
                log::error!("File transfer outbound failure to {}: {:?}", peer, error);
                self.state_emit(SyncEvent::FileDownloadComplete {
                    request_id: request_id.to_string(),
                    file_path: String::new(),
                    success: false,
                });
            }

            _ => {}
        }
    }

    // ── Message handlers ──────────────────────────────────────────

    fn handle_clipboard_request(
        &mut self,
        peer: PeerId,
        request: ClipboardRequest,
        channel: request_response::ResponseChannel<ClipboardResponse>,
    ) {
        if !self.paired_peers.contains(&peer) || !self.clipboard_ready {
            let response = match request {
                ClipboardRequest::Head { .. } => ClipboardResponse::Head {
                    head: None,
                    update: None,
                },
                ClipboardRequest::Apply { .. } => ClipboardResponse::Apply {
                    status: ApplyStatus::Retry,
                },
            };
            self.respond_clipboard(channel, response);
            return;
        }
        match request {
            ClipboardRequest::Head { known } => {
                let current = self.clipboard.head.as_ref();
                let head = current.map(|update| update.revision.clone());
                let update = current
                    .filter(|update| update.revision.peer_id == self.local_peer_id.to_string())
                    .filter(|update| known.as_ref().is_none_or(|known| update.revision > *known))
                    .cloned();
                self.respond_clipboard(channel, ClipboardResponse::Head { head, update });
            }
            ClipboardRequest::Apply { update } => {
                self.receive_clipboard_update(peer, update, Some(channel));
            }
        }
    }

    fn handle_clipboard_response(&mut self, peer: PeerId, response: ClipboardResponse) {
        if !self.paired_peers.contains(&peer) || !self.clipboard_ready {
            return;
        }
        match response {
            ClipboardResponse::Head { head, update } => {
                if let Some(update) = update {
                    self.receive_clipboard_update(peer, update, None);
                }
                if let Some(local) = self.clipboard.head.as_ref() {
                    if local.revision.peer_id == self.local_peer_id.to_string()
                        && head.as_ref().is_none_or(|remote| local.revision > *remote)
                    {
                        self.send_current(peer, local.clone());
                    }
                }
            }
            ClipboardResponse::Apply { status } => {
                if matches!(status, ApplyStatus::Retry | ApplyStatus::Invalid) {
                    log::warn!("Clipboard update to {peer} was not applied: {status:?}");
                }
            }
        }
    }

    fn receive_clipboard_update(
        &mut self,
        peer: PeerId,
        update: ClipboardUpdate,
        channel: Option<request_response::ResponseChannel<ClipboardResponse>>,
    ) {
        let result = if update.revision.peer_id != peer.to_string() {
            MergeResult::Invalid
        } else {
            self.clipboard.compare(&update)
        };
        let status = match result {
            MergeResult::AlreadyCurrent => Some(ApplyStatus::Applied),
            MergeResult::Stale => Some(ApplyStatus::Stale),
            MergeResult::Invalid => Some(ApplyStatus::Invalid),
            MergeResult::Applied if self.pending_clipboard.is_some() => Some(ApplyStatus::Retry),
            MergeResult::Applied => None,
        };
        if let Some(status) = status {
            if let Some(channel) = channel {
                self.respond_clipboard(channel, ClipboardResponse::Apply { status });
            }
            return;
        }
        let text = match &update.value {
            ClipboardValue::Text(text) => Some(text.clone()),
            ClipboardValue::Unavailable => None,
        };
        let event_id = update.event_id.clone();
        self.pending_clipboard = Some(PendingClipboard {
            peer,
            update,
            channel,
            created_at: Instant::now(),
        });
        self.state_emit(SyncEvent::CurrentClipboardReceived {
            event_id,
            peer_id: peer.to_string(),
            text,
        });
    }

    fn handle_pairing_message(&mut self, peer: PeerId, msg: PairingMessage) {
        match msg {
            PairingMessage::Request {
                session_id,
                target_peer_id,
                device_name,
            } => {
                if target_peer_id != self.local_peer_id.to_string()
                    || peer == self.local_peer_id
                    || self.paired_peers.contains(&peer)
                {
                    return;
                }
                if self
                    .pending_pairings
                    .get(&peer)
                    .is_some_and(|pending| pending.session_id == session_id)
                {
                    return;
                }
                let pin = pairing_pin(&session_id, &self.local_peer_id, &peer);
                self.pending_pairings.insert(
                    peer,
                    PendingPairing {
                        session_id,
                        pin: pin.clone(),
                        local_confirmed: false,
                        remote_confirmed: false,
                        created_at: Instant::now(),
                    },
                );
                log::info!("Pairing request from {} ({})", device_name, peer);
                self.state_emit(SyncEvent::PairingRequest {
                    peer_id: peer.to_string(),
                    display_name: device_name,
                    pin,
                });
            }
            PairingMessage::Confirm {
                session_id,
                target_peer_id,
            } => {
                if target_peer_id != self.local_peer_id.to_string() {
                    return;
                }
                if let Some(pending) = self.pending_pairings.get_mut(&peer) {
                    if pending.session_id == session_id
                        && pending.created_at.elapsed() <= PAIRING_TIMEOUT
                    {
                        pending.remote_confirmed = true;
                        self.finish_pairing_if_confirmed(peer);
                    }
                }
            }
            PairingMessage::Reject {
                session_id,
                target_peer_id,
            } => {
                if target_peer_id == self.local_peer_id.to_string()
                    && self
                        .pending_pairings
                        .get(&peer)
                        .is_some_and(|pending| pending.session_id == session_id)
                {
                    self.pending_pairings.remove(&peer);
                    self.state_emit(SyncEvent::PairingComplete {
                        peer_id: peer.to_string(),
                        success: false,
                    });
                }
            }
        }
    }

    fn finish_pairing_if_confirmed(&mut self, peer: PeerId) {
        if self
            .pending_pairings
            .get(&peer)
            .is_some_and(|pending| pending.local_confirmed && pending.remote_confirmed)
        {
            self.pending_pairings.remove(&peer);
            self.paired_peers.insert(peer);
            self.state_emit(SyncEvent::PairingComplete {
                peer_id: peer.to_string(),
                success: true,
            });
            self.request_head(peer);
        }
    }

    fn publish_pairing_message(&mut self, message: PairingMessage) {
        if let Ok(data) = serde_json::to_vec(&message) {
            let topic = IdentTopic::new(PAIRING_TOPIC);
            if let Err(error) = self.swarm.behaviour_mut().gossipsub.publish(topic, data) {
                self.emit_error(
                    ErrorCode::Network,
                    format!("Pairing message failed: {error}"),
                );
            }
        }
    }

    // ── File transfer ────────────────────────────────────────────

    fn handle_file_transfer_message(
        &mut self,
        peer: PeerId,
        msg: request_response::Message<FileRequest, FileChunk>,
    ) {
        match msg {
            request_response::Message::Request {
                request, channel, ..
            } => {
                log::warn!("Rejected file request from {}", peer);
                let _ = self.swarm.behaviour_mut().file_transfer.send_response(
                    channel,
                    FileChunk {
                        request_id: request.request_id,
                        file_name: String::new(),
                        file_size: 0,
                        chunk_index: 0,
                        total_chunks: 0,
                        data: vec![],
                    },
                );
            }
            request_response::Message::Response {
                request_id,
                response,
            } => {
                self.state_emit(SyncEvent::FileChunkReceived {
                    request_id: request_id.to_string(),
                    file_name: response.file_name,
                    file_size: response.file_size,
                    chunk_index: response.chunk_index,
                    total_chunks: response.total_chunks,
                    data: response.data,
                });
            }
        }
    }

    // ── Command handlers ──────────────────────────────────────────

    fn handle_command(&mut self, command: SyncCommand) {
        match command {
            SyncCommand::BroadcastItem { item_json } => {
                if let Some(text) = portable_text_from_json(&item_json) {
                    self.observe_local_clipboard(Some(text));
                } else {
                    self.observe_local_clipboard(None);
                }
            }
            SyncCommand::ObserveLocalClipboard { text } => {
                self.observe_local_clipboard(text);
            }
            SyncCommand::CurrentClipboardApplied { event_id, success } => {
                self.finish_clipboard_apply(&event_id, success);
            }
            SyncCommand::ShouldApplyCurrentClipboard { event_id, reply } => {
                let allowed = self.pending_clipboard.as_ref().is_some_and(|pending| {
                    pending.update.event_id == event_id
                        && self.paired_peers.contains(&pending.peer)
                        && self.clipboard.compare(&pending.update) == MergeResult::Applied
                });
                let _ = reply.send(allowed);
            }
            // History records are local. Clipboard state uses its own versioned protocol.
            SyncCommand::BroadcastDeletion { .. } | SyncCommand::BroadcastUpdate { .. } => {}
            SyncCommand::StartDiscovery | SyncCommand::StopDiscovery => {}

            SyncCommand::RequestPairing { peer_id } => {
                if let Ok(peer) = peer_id.parse::<PeerId>() {
                    if peer == self.local_peer_id || self.paired_peers.contains(&peer) {
                        return;
                    }
                    let device_name = {
                        let state = self.state.lock().unwrap();
                        state.device_name.clone()
                    };
                    let session_id = uuid::Uuid::new_v4().to_string();
                    let pin = pairing_pin(&session_id, &self.local_peer_id, &peer);
                    self.pending_pairings.insert(
                        peer,
                        PendingPairing {
                            session_id: session_id.clone(),
                            pin: pin.clone(),
                            local_confirmed: false,
                            remote_confirmed: false,
                            created_at: Instant::now(),
                        },
                    );
                    self.publish_pairing_message(PairingMessage::Request {
                        session_id,
                        target_peer_id: peer.to_string(),
                        device_name,
                    });
                    let display_name = self
                        .discovered_peers
                        .get(&peer)
                        .map(|info| info.display_name.clone())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| peer.to_string());
                    self.state_emit(SyncEvent::PairingRequest {
                        peer_id: peer.to_string(),
                        display_name,
                        pin,
                    });
                }
            }
            SyncCommand::AcceptPairing { peer_id, pin } => {
                if let Ok(peer) = peer_id.parse::<PeerId>() {
                    let Some(pending) = self.pending_pairings.get_mut(&peer) else {
                        return;
                    };
                    if pending.pin != pin || pending.created_at.elapsed() > PAIRING_TIMEOUT {
                        self.emit_error(
                            ErrorCode::InvalidArg,
                            "Invalid or expired pairing code".into(),
                        );
                        return;
                    }
                    pending.local_confirmed = true;
                    let session_id = pending.session_id.clone();
                    self.publish_pairing_message(PairingMessage::Confirm {
                        session_id,
                        target_peer_id: peer.to_string(),
                    });
                    self.finish_pairing_if_confirmed(peer);
                }
            }
            SyncCommand::RejectPairing { peer_id } => {
                if let Ok(peer) = peer_id.parse::<PeerId>() {
                    if let Some(pending) = self.pending_pairings.remove(&peer) {
                        self.publish_pairing_message(PairingMessage::Reject {
                            session_id: pending.session_id,
                            target_peer_id: peer.to_string(),
                        });
                    }
                }
            }
            SyncCommand::AddPeerAddress { address } => {
                let multiaddr = if address.starts_with('/') {
                    address.clone()
                } else {
                    parse_host_port_to_multiaddr(&address)
                };
                log::info!("Dialing {} (from {})", multiaddr, address);
                if let Ok(addr) = multiaddr.parse::<libp2p::Multiaddr>() {
                    match self.swarm.dial(addr.clone()) {
                        Ok(()) => log::info!("Dialing {}", addr),
                        Err(e) => {
                            log::error!("Failed to dial {}: {:?}", addr, e);
                            self.emit_error(
                                ErrorCode::Network,
                                format!("Failed to dial {}: {:?}", addr, e),
                            );
                        }
                    }
                } else {
                    log::error!("Invalid multiaddr: {}", multiaddr);
                    self.emit_error(
                        ErrorCode::InvalidArg,
                        format!("Invalid address: {}", address),
                    );
                }
            }
            SyncCommand::Unpair { peer_id } => {
                if let Ok(peer) = peer_id.parse::<PeerId>() {
                    self.paired_peers.remove(&peer);
                    if self
                        .pending_clipboard
                        .as_ref()
                        .is_some_and(|pending| pending.peer == peer)
                    {
                        if let Some(pending) = self.pending_clipboard.take() {
                            if let Some(channel) = pending.channel {
                                self.respond_clipboard(
                                    channel,
                                    ClipboardResponse::Apply {
                                        status: ApplyStatus::Retry,
                                    },
                                );
                            }
                        }
                    }
                }
            }
            SyncCommand::SendFileChunk {
                peer_id,
                request_id,
                file_path,
                offset,
            } => {
                let _ = (peer_id, request_id, file_path, offset);
                self.emit_error(
                    ErrorCode::InvalidArg,
                    "File transfer is disabled until files can be shared by explicit ID".into(),
                );
            }
            _ => {}
        }
    }

    fn respond_clipboard(
        &mut self,
        channel: request_response::ResponseChannel<ClipboardResponse>,
        response: ClipboardResponse,
    ) {
        if self
            .swarm
            .behaviour_mut()
            .clipboard_sync
            .send_response(channel, response)
            .is_err()
        {
            log::warn!("Clipboard response channel closed");
        }
    }

    fn request_head(&mut self, peer: PeerId) {
        if !self.clipboard_ready {
            return;
        }
        let known = self
            .clipboard
            .head
            .as_ref()
            .map(|update| update.revision.clone());
        self.swarm
            .behaviour_mut()
            .clipboard_sync
            .send_request(&peer, ClipboardRequest::Head { known });
    }

    fn send_current(&mut self, peer: PeerId, update: ClipboardUpdate) {
        self.swarm
            .behaviour_mut()
            .clipboard_sync
            .send_request(&peer, ClipboardRequest::Apply { update });
    }

    fn reconcile_all(&mut self) {
        if self
            .pending_clipboard
            .as_ref()
            .is_some_and(|pending| pending.created_at.elapsed() > Duration::from_secs(10))
        {
            if let Some(pending) = self.pending_clipboard.take() {
                if let Some(channel) = pending.channel {
                    self.respond_clipboard(
                        channel,
                        ClipboardResponse::Apply {
                            status: ApplyStatus::Retry,
                        },
                    );
                }
            }
        }
        if self.pending_persist {
            if !self.persist_clipboard(&self.clipboard) {
                return;
            }
            self.pending_persist = false;
            self.clipboard_ready = true;
        }
        if !self.clipboard_ready {
            return;
        }
        let peers: Vec<_> = self
            .paired_peers
            .iter()
            .copied()
            .filter(|peer| self.swarm.is_connected(peer))
            .collect();
        for peer in peers {
            self.request_head(peer);
        }
    }

    fn persist_clipboard(&self, register: &ClipboardRegister) -> bool {
        let Some(path) = self.clipboard_store.as_deref() else {
            return true;
        };
        match register::save_register(path, &self.local_peer_id.to_string(), register) {
            Ok(()) => true,
            Err(error) => {
                self.emit_error(
                    ErrorCode::Network,
                    format!("Cannot save clipboard state: {error}"),
                );
                false
            }
        }
    }

    fn observe_local_clipboard(&mut self, text: Option<String>) {
        let was_ready = self.clipboard_ready;
        let value = match text {
            Some(text) => ClipboardValue::Text(text),
            None => ClipboardValue::Unavailable,
        };
        let mut next = self.clipboard.clone();
        let Some(update) = next.local_change(&self.local_peer_id.to_string(), value) else {
            if self.pending_persist {
                if !self.persist_clipboard(&self.clipboard) {
                    return;
                }
                self.pending_persist = false;
            }
            self.clipboard_ready = true;
            if !was_ready {
                self.reconcile_all();
            }
            return;
        };
        self.clipboard = next;
        if !self.persist_clipboard(&self.clipboard) {
            self.pending_persist = true;
            self.clipboard_ready = false;
            return;
        }
        self.pending_persist = false;
        self.clipboard_ready = true;
        for peer in self.paired_peers.iter().copied().collect::<Vec<_>>() {
            self.send_current(peer, update.clone());
        }
        if !was_ready {
            self.reconcile_all();
        }
    }

    fn finish_clipboard_apply(&mut self, event_id: &str, success: bool) {
        let Some(pending) = self.pending_clipboard.take() else {
            return;
        };
        if pending.update.event_id != event_id {
            self.pending_clipboard = Some(pending);
            return;
        }
        let status = if !success || !self.paired_peers.contains(&pending.peer) {
            ApplyStatus::Retry
        } else {
            let mut next = self.clipboard.clone();
            match next.apply(pending.update) {
                MergeResult::Applied => {
                    self.clipboard = next;
                    if self.persist_clipboard(&self.clipboard) {
                        self.pending_persist = false;
                        ApplyStatus::Applied
                    } else {
                        self.pending_persist = true;
                        self.clipboard_ready = false;
                        ApplyStatus::Retry
                    }
                }
                MergeResult::AlreadyCurrent => ApplyStatus::Applied,
                MergeResult::Stale => ApplyStatus::Stale,
                MergeResult::Invalid => ApplyStatus::Invalid,
            }
        };
        if let Some(channel) = pending.channel {
            self.respond_clipboard(channel, ClipboardResponse::Apply { status });
        }
        if status == ApplyStatus::Applied {
            self.request_head(pending.peer);
        }
    }
}

fn portable_text_from_json(json: &str) -> Option<String> {
    let item: SyncItem = serde_json::from_str(json).ok()?;
    let content = item.contents.iter().find(|content| {
        matches!(
            content.content_type.as_str(),
            "text/plain" | "text/plain;charset=utf-8" | "public.utf8-plain-text"
        )
    })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content.value.as_ref()?)
        .ok()?;
    String::from_utf8(bytes).ok()
}

fn pairing_pin(session_id: &str, local: &PeerId, remote: &PeerId) -> String {
    let mut peers = [local.to_string(), remote.to_string()];
    peers.sort();
    let mut hash = Sha256::new();
    hash.update(b"manis-pocket-pairing-v2");
    hash.update(session_id.as_bytes());
    hash.update(peers[0].as_bytes());
    hash.update(peers[1].as_bytes());
    let bytes = hash.finalize();
    let value = u32::from_be_bytes(bytes[..4].try_into().unwrap()) % 1_000_000;
    format!("{value:06}")
}

/// Check whether `addr` matches one of our own listen addresses.
/// Strips `/p2p/PEERID` suffix before comparing, since mDNS may append it.
fn is_own_addr(addr: &libp2p::Multiaddr, local_addrs: &HashSet<String>) -> bool {
    let addr_str = addr.to_string();
    let transport_addr = match addr_str.find("/p2p/") {
        Some(pos) => &addr_str[..pos],
        None => &addr_str,
    };
    local_addrs.contains(transport_addr)
}

fn parse_host_port_to_multiaddr(input: &str) -> String {
    let input = input.trim();
    if let Some(rest) = input.strip_prefix('[') {
        if let Some(bracket_end) = rest.find("]:") {
            let host = &rest[..bracket_end];
            let port = &rest[bracket_end + 2..];
            return format!("/ip6/{}/tcp/{}", host, port);
        }
    }
    if let Some(colon) = input.rfind(':') {
        let host = &input[..colon];
        let port = &input[colon + 1..];
        return format!("/ip4/{}/tcp/{}", host, port);
    }
    input.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn failed_persistence_keeps_new_head_and_retries_before_reconciliation() {
        let directory =
            std::env::temp_dir().join(format!("clipboard-network-state-{}", uuid::Uuid::new_v4()));
        let path = directory.join("state.json");
        let state: SharedState = Arc::new(std::sync::Mutex::new(
            crate::state::SyncState::new("test", "test").unwrap(),
        ));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let key = Keypair::generate_ed25519();
            let (_, commands) = mpsc::unbounded_channel();
            let mut manager = NetworkManager::new_with_port_and_storage(
                commands,
                state.clone(),
                key,
                0,
                path.clone(),
            )
            .unwrap();
            manager.observe_local_clipboard(Some("new clipboard".into()));
            assert_eq!(
                manager.clipboard.head.as_ref().unwrap().value,
                ClipboardValue::Text("new clipboard".into())
            );
            assert!(!manager.clipboard_ready);
            assert!(manager.pending_persist);

            std::fs::create_dir(&directory).unwrap();
            manager.reconcile_all();
            assert!(manager.clipboard_ready);
            assert!(!manager.pending_persist);
            assert_eq!(
                register::load_register(&path, &manager.local_peer_id.to_string()).unwrap(),
                manager.clipboard
            );
        });
        drop(runtime);
        drop(state);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
