use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic, MessageAuthenticity, MessageId};
use libp2p::identity::Keypair;
use libp2p::mdns;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{PeerId, SwarmBuilder};
use tokio::sync::mpsc;

use crate::error::ErrorCode;
use crate::state::{SharedState, SyncCommand};
use crate::types::*;

const PAIRING_TOPIC: &str = "manis-pocket-sync-pairing-v1";
const CHUNK_SIZE: u64 = 1024 * 1024; // 1 MiB chunks

#[derive(NetworkBehaviour)]
pub struct ManisPocketBehaviour {
    pub mdns: mdns::tokio::Behaviour,
    pub gossipsub: gossipsub::Behaviour,
    pub identify: libp2p::identify::Behaviour,
    pub file_transfer: request_response::cbor::Behaviour<FileRequest, FileChunk>,
}

pub struct NetworkManager {
    swarm: libp2p::Swarm<ManisPocketBehaviour>,
    command_rx: mpsc::UnboundedReceiver<SyncCommand>,
    state: SharedState,
    discovered_peers: HashMap<PeerId, PeerInfo>,
    paired_peers: HashSet<PeerId>,
    /// Pairing session IDs we've already shown to the user (avoid duplicate dialogs)
    seen_pairing_sessions: HashSet<String>,
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
        Self::build(command_rx, state, local_key, LISTEN_PORT)
    }

    pub fn new_with_port(
        command_rx: mpsc::UnboundedReceiver<SyncCommand>,
        state: SharedState,
        local_key: Keypair,
        port: u16,
    ) -> Result<Self, ErrorCode> {
        Self::build(command_rx, state, local_key, port)
    }

    fn build(
        command_rx: mpsc::UnboundedReceiver<SyncCommand>,
        state: SharedState,
        local_key: Keypair,
        listen_port: u16,
    ) -> Result<Self, ErrorCode> {
        let local_peer_id = PeerId::from(local_key.public());

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

        let behaviour = ManisPocketBehaviour {
            mdns: mdns_behaviour,
            gossipsub: gossipsub_behaviour,
            identify,
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
            seen_pairing_sessions: HashSet::new(),
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
        let sync_topic = IdentTopic::new(TOPIC_NAME);
        if let Err(e) = self.swarm.behaviour_mut().gossipsub.subscribe(&sync_topic) {
            log::error!("Failed to subscribe to sync topic: {:?}", e);
        }

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
                gossipsub::Event::Message {
                    message,
                    propagation_source,
                    ..
                },
            )) => {
                let topic = message.topic.as_str();
                if topic == PAIRING_TOPIC {
                    if let Ok(pairing_msg) = serde_json::from_slice::<PairingMessage>(&message.data)
                    {
                        self.handle_pairing_message(propagation_source, pairing_msg);
                    }
                } else if topic == TOPIC_NAME {
                    if self.paired_peers.contains(&propagation_source) {
                        if let Ok(sync_msg) = serde_json::from_slice::<SyncMessage>(&message.data) {
                            self.handle_sync_message(sync_msg);
                        }
                    } else {
                        // NOTE: no `log` backend is installed on macOS/Android (the C FFI's
                        // StderrLogger isn't used via UniFFI), so use eprintln! to guarantee
                        // this surfaces in the terminal when diagnosing dropped messages.
                        eprintln!(
                            "[sync] DROPPED message from unpaired peer {} (have {} paired: {:?})",
                            propagation_source,
                            self.paired_peers.len(),
                            self.paired_peers
                        );
                    }
                }
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

    fn handle_sync_message(&self, msg: SyncMessage) {
        match msg {
            SyncMessage::ItemAdded { item_json } => {
                self.state_emit(SyncEvent::ItemReceived { item_json });
            }
            SyncMessage::ItemDeleted { id, .. } => {
                self.state_emit(SyncEvent::ItemDeleted { item_id: id });
            }
            SyncMessage::ItemUpdated { item_json } => {
                self.state_emit(SyncEvent::ItemUpdated { item_json });
            }
            SyncMessage::Heartbeat { .. } => {}
        }
    }

    fn handle_pairing_message(&mut self, peer: PeerId, msg: PairingMessage) {
        match msg {
            PairingMessage::Request {
                session_id,
                device_name,
                ..
            } => {
                // Deduplicate: only show one dialog per session
                if self.seen_pairing_sessions.contains(&session_id) {
                    return;
                }
                self.seen_pairing_sessions.insert(session_id);
                let pin = format!("{:06}", rand::random::<u32>() % 1_000_000);
                log::info!("Pairing request from {} ({})", device_name, peer);
                self.state_emit(SyncEvent::PairingRequest {
                    peer_id: peer.to_string(),
                    display_name: device_name,
                    pin,
                });
            }
            PairingMessage::Accept { .. } => {
                log::info!("Pairing accepted by {}", peer);
                self.paired_peers.insert(peer);
                self.state_emit(SyncEvent::PairingComplete {
                    peer_id: peer.to_string(),
                    success: true,
                });
            }
            PairingMessage::Reject { .. } => {
                log::info!("Pairing rejected by {}", peer);
                self.state_emit(SyncEvent::PairingComplete {
                    peer_id: peer.to_string(),
                    success: false,
                });
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
                request_id,
                request,
                channel,
            } => {
                log::info!("File request from {}: {}", peer, request.file_path);
                self.send_file_chunks(channel, &request);
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

    fn send_file_chunks(
        &mut self,
        channel: request_response::ResponseChannel<FileChunk>,
        req: &FileRequest,
    ) {
        let path = &req.file_path;
        match std::fs::read(path) {
            Ok(data) => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let size = data.len() as u64;
                log::info!("Sending file '{}' ({:.1} KiB)", name, size as f64 / 1024.0);
                let chunk = FileChunk {
                    request_id: req.request_id.clone(),
                    file_name: name,
                    file_size: size,
                    chunk_index: 0,
                    total_chunks: 1,
                    data,
                };
                if let Err(e) = self
                    .swarm
                    .behaviour_mut()
                    .file_transfer
                    .send_response(channel, chunk)
                {
                    log::warn!("Failed to send file chunk: {:?}", e);
                }
            }
            Err(e) => {
                log::error!("File not found {}: {}", path, e);
                let chunk = FileChunk {
                    request_id: req.request_id.clone(),
                    file_name: String::new(),
                    file_size: 0,
                    chunk_index: 0,
                    total_chunks: 0,
                    data: vec![],
                };
                if let Err(e) = self
                    .swarm
                    .behaviour_mut()
                    .file_transfer
                    .send_response(channel, chunk)
                {
                    log::warn!("Failed to send error response: {:?}", e);
                }
            }
        }
    }

    // ── Command handlers ──────────────────────────────────────────

    fn handle_command(&mut self, command: SyncCommand) {
        match command {
            SyncCommand::BroadcastItem { item_json } => {
                let msg = SyncMessage::ItemAdded { item_json };
                self.broadcast_sync_message(msg);
            }
            SyncCommand::BroadcastDeletion { item_id } => {
                let msg = SyncMessage::ItemDeleted {
                    id: item_id,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                self.broadcast_sync_message(msg);
            }
            SyncCommand::BroadcastUpdate { item_json } => {
                let msg = SyncMessage::ItemUpdated { item_json };
                self.broadcast_sync_message(msg);
            }
            SyncCommand::StartDiscovery | SyncCommand::StopDiscovery => {}

            SyncCommand::RequestPairing { peer_id } => {
                if let Ok(peer) = peer_id.parse::<PeerId>() {
                    let (device_name, device_id) = {
                        let state = self.state.lock().unwrap();
                        (state.device_name.clone(), state.device_id.clone())
                    };
                    let session_id = uuid::Uuid::new_v4().to_string();
                    let request = PairingMessage::Request {
                        session_id: session_id.clone(),
                        device_name,
                        device_id,
                        public_key: vec![],
                    };
                    if let Ok(data) = serde_json::to_vec(&request) {
                        let topic = IdentTopic::new(PAIRING_TOPIC);
                        if let Err(e) = self.swarm.behaviour_mut().gossipsub.publish(topic, data) {
                            log::warn!("Failed to publish pairing request: {:?}", e);
                        }
                    }
                    // Optimistically add — the remote will respond with Accept/Reject.
                    // Emit PairingComplete so the platform persists the peer to DB now,
                    // otherwise if the gossipsub Accept is lost we'd never save it.
                    self.paired_peers.insert(peer);
                    self.seen_pairing_sessions.insert(session_id);
                    self.state_emit(SyncEvent::PairingComplete {
                        peer_id: peer.to_string(),
                        success: true,
                    });
                    log::info!("Sent pairing request to {}", peer);
                }
            }
            SyncCommand::AcceptPairing { peer_id, .. } => {
                match peer_id.parse::<PeerId>() {
                    Ok(peer) => {
                        log::info!("Accepting pairing with {}", peer);
                        self.paired_peers.insert(peer);
                        // Notify the requester via gossipsub
                        let accept = PairingMessage::Accept {
                            session_id: String::new(),
                        };
                        if let Ok(data) = serde_json::to_vec(&accept) {
                            let topic = IdentTopic::new(PAIRING_TOPIC);
                            if let Err(e) =
                                self.swarm.behaviour_mut().gossipsub.publish(topic, data)
                            {
                                log::warn!("Failed to publish pairing accept: {:?}", e);
                            }
                        }
                        self.state_emit(SyncEvent::PairingComplete {
                            peer_id: peer.to_string(),
                            success: true,
                        });
                    }
                    Err(e) => log::error!(
                        "Failed to parse peer_id '{}' in AcceptPairing: {:?}",
                        peer_id,
                        e
                    ),
                }
            }
            SyncCommand::RejectPairing { peer_id } => {
                if let Ok(peer) = peer_id.parse::<PeerId>() {
                    log::info!("Rejecting pairing with {}", peer);
                    let reject = PairingMessage::Reject {
                        session_id: String::new(),
                    };
                    if let Ok(data) = serde_json::to_vec(&reject) {
                        let topic = IdentTopic::new(PAIRING_TOPIC);
                        if let Err(e) = self.swarm.behaviour_mut().gossipsub.publish(topic, data) {
                            log::warn!("Failed to publish pairing reject: {:?}", e);
                        }
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
                }
            }
            SyncCommand::SendFileChunk {
                peer_id,
                request_id,
                file_path,
                offset,
            } => {
                if let Ok(peer) = peer_id.parse::<PeerId>() {
                    let request = FileRequest {
                        request_id,
                        file_path,
                        offset,
                    };
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .file_transfer
                        .send_request(&peer, request);
                }
            }
            _ => {}
        }
    }

    fn broadcast_sync_message(&mut self, msg: SyncMessage) {
        if let Ok(data) = serde_json::to_vec(&msg) {
            let topic = IdentTopic::new(TOPIC_NAME);
            if let Err(e) = self.swarm.behaviour_mut().gossipsub.publish(topic, data) {
                log::warn!("Failed to publish sync message: {:?}", e);
            }
        }
    }
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
