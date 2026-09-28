use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use manis_pocket_sync::SyncEvent;
use manis_pocket_sync::state::{SharedState, SyncCommand, SyncState};

struct Node {
    peer_id: String,
    commands: tokio::sync::mpsc::UnboundedSender<SyncCommand>,
    events: Arc<Mutex<Vec<SyncEvent>>>,
}

fn available_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn spawn_node(name: &str, port: u16) -> Node {
    let key = libp2p::identity::Keypair::generate_ed25519();
    let peer_id = key.public().to_peer_id().to_string();
    let state: SharedState = Arc::new(Mutex::new(SyncState::new(name, name).unwrap()));
    let events = Arc::new(Mutex::new(Vec::new()));
    {
        let events = events.clone();
        let guard = state.lock().unwrap();
        *guard.on_event.lock() = Some(Box::new(move |json: &str| {
            if let Ok(event) = serde_json::from_str::<SyncEvent>(json) {
                events.lock().unwrap().push(event);
            }
        }));
    }
    let (commands, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut manager =
            manis_pocket_sync::network::NetworkManager::new_with_port(rx, state, key, port)
                .unwrap();
        rt.block_on(manager.run());
    });
    Node {
        peer_id,
        commands,
        events,
    }
}

fn wait_for(node: &Node, predicate: impl Fn(&SyncEvent) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if node.events.lock().unwrap().iter().any(&predicate) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for sync event"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn code_for(node: &Node, peer: &str) -> String {
    node.events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            SyncEvent::PairingRequest { peer_id, pin, .. } if peer_id == peer => Some(pin.clone()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn synthetic_clipboard_message_reaches_second_node_after_pairing() {
    let port_a = available_port();
    let mut port_b = available_port();
    while port_b == port_a {
        port_b = available_port();
    }
    let mut port_c = available_port();
    while port_c == port_a || port_c == port_b {
        port_c = available_port();
    }
    let a = spawn_node("sync-review-a", port_a);
    let b = spawn_node("sync-review-b", port_b);
    let c = spawn_node("sync-review-unpaired", port_c);
    wait_for(&a, |event| matches!(event, SyncEvent::Listening { .. }));
    wait_for(&b, |event| matches!(event, SyncEvent::Listening { .. }));
    wait_for(&c, |event| matches!(event, SyncEvent::Listening { .. }));

    a.commands
        .send(SyncCommand::AddPeerAddress {
            address: format!("127.0.0.1:{port_b}"),
        })
        .unwrap();
    wait_for(
        &a,
        |event| matches!(event, SyncEvent::PeerDiscovered { peer } if peer.peer_id == b.peer_id),
    );
    wait_for(
        &b,
        |event| matches!(event, SyncEvent::PeerDiscovered { peer } if peer.peer_id == a.peer_id),
    );
    a.commands
        .send(SyncCommand::AddPeerAddress {
            address: format!("127.0.0.1:{port_c}"),
        })
        .unwrap();
    wait_for(
        &a,
        |event| matches!(event, SyncEvent::PeerDiscovered { peer } if peer.peer_id == c.peer_id),
    );

    a.commands
        .send(SyncCommand::RequestPairing {
            peer_id: b.peer_id.clone(),
        })
        .unwrap();
    wait_for(
        &b,
        |event| matches!(event, SyncEvent::PairingRequest { peer_id, .. } if peer_id == &a.peer_id),
    );
    wait_for(
        &a,
        |event| matches!(event, SyncEvent::PairingRequest { peer_id, .. } if peer_id == &b.peer_id),
    );
    let pin = code_for(&b, &a.peer_id);
    assert_eq!(pin, code_for(&a, &b.peer_id));

    a.commands
        .send(SyncCommand::ObserveLocalClipboard {
            text: Some("before-approval".into()),
        })
        .unwrap();
    a.commands
        .send(SyncCommand::AcceptPairing {
            peer_id: b.peer_id.clone(),
            pin: "wrong-code".into(),
        })
        .unwrap();
    wait_for(
        &a,
        |event| matches!(event, SyncEvent::Error { message, .. } if message.contains("Invalid or expired pairing code")),
    );

    b.commands
        .send(SyncCommand::AcceptPairing {
            peer_id: a.peer_id.clone(),
            pin: pin.clone(),
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!b.events.lock().unwrap().iter().any(|event| matches!(
        event,
        SyncEvent::PairingComplete { success: true, .. }
            | SyncEvent::CurrentClipboardReceived { .. }
    )));
    a.commands
        .send(SyncCommand::AcceptPairing {
            peer_id: b.peer_id.clone(),
            pin,
        })
        .unwrap();
    wait_for(
        &a,
        |event| matches!(event, SyncEvent::PairingComplete { peer_id, success: true } if peer_id == &b.peer_id),
    );
    wait_for(
        &b,
        |event| matches!(event, SyncEvent::PairingComplete { peer_id, success: true } if peer_id == &a.peer_id),
    );

    wait_for(
        &b,
        |event| matches!(event, SyncEvent::CurrentClipboardReceived { text: Some(text), .. } if text == "before-approval"),
    );
    let first_event = b
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            SyncEvent::CurrentClipboardReceived {
                event_id,
                text: Some(text),
                ..
            } if text == "before-approval" => Some(event_id.clone()),
            _ => None,
        })
        .unwrap();
    b.commands
        .send(SyncCommand::CurrentClipboardApplied {
            event_id: first_event.clone(),
            success: false,
        })
        .unwrap();
    let retry_deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let count = b
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(event,
                    SyncEvent::CurrentClipboardReceived { event_id, text: Some(text), .. }
                        if event_id == &first_event && text == "before-approval"
                )
            })
            .count();
        if count >= 2 {
            break;
        }
        assert!(
            Instant::now() < retry_deadline,
            "failed clipboard write was not retried"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    b.commands
        .send(SyncCommand::CurrentClipboardApplied {
            event_id: first_event,
            success: true,
        })
        .unwrap();

    a.commands
        .send(SyncCommand::ObserveLocalClipboard {
            text: Some("synthetic-clipboard-payload".into()),
        })
        .unwrap();
    wait_for(
        &b,
        |event| matches!(event, SyncEvent::CurrentClipboardReceived { text: Some(text), peer_id, .. } if text == "synthetic-clipboard-payload" && peer_id == &a.peer_id),
    );
    let second_event = b
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            SyncEvent::CurrentClipboardReceived {
                event_id,
                text: Some(text),
                ..
            } if text == "synthetic-clipboard-payload" => Some(event_id.clone()),
            _ => None,
        })
        .unwrap();
    b.commands
        .send(SyncCommand::CurrentClipboardApplied {
            event_id: second_event,
            success: true,
        })
        .unwrap();

    a.commands
        .send(SyncCommand::ObserveLocalClipboard { text: None })
        .unwrap();
    wait_for(
        &b,
        |event| matches!(event, SyncEvent::CurrentClipboardReceived { text: None, peer_id, .. } if peer_id == &a.peer_id),
    );
    let clear_event = b
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            SyncEvent::CurrentClipboardReceived {
                event_id,
                text: None,
                ..
            } => Some(event_id.clone()),
            _ => None,
        })
        .unwrap();
    b.commands
        .send(SyncCommand::CurrentClipboardApplied {
            event_id: clear_event,
            success: true,
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !c.events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, SyncEvent::CurrentClipboardReceived { .. }))
    );

    a.commands.send(SyncCommand::Shutdown).unwrap();
    b.commands.send(SyncCommand::Shutdown).unwrap();
    c.commands.send(SyncCommand::Shutdown).unwrap();
}
