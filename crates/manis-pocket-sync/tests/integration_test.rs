use std::sync::{Arc, Mutex};
use std::time::Duration;

use manis_pocket_sync::state::{SharedState, SyncCommand, SyncState};

/// Build a NetworkManager and run it on a dedicated thread (same as FFI does).
fn spawn_node(
    device_name: &str,
    device_id: &str,
    port: u16,
    events: Arc<Mutex<Vec<String>>>,
) -> tokio::sync::mpsc::UnboundedSender<SyncCommand> {
    let state = SyncState::new(device_name, device_id).unwrap();
    let shared: SharedState = Arc::new(std::sync::Mutex::new(state));

    // Register the unified event callback
    {
        let mut guard = shared.lock().unwrap();
        let ev = events.clone();
        *guard.on_event.lock() = Some(Box::new(move |json: &str| {
            ev.lock().unwrap().push(json.to_string());
        }));
    }

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let key = libp2p::identity::Keypair::generate_ed25519();
    let peer_id = key.public().to_peer_id();
    let shared_clone = shared.clone();

    let rt = shared.lock().unwrap().runtime.handle().clone();

    std::thread::spawn(move || {
        let mut nm = if port == 31774 {
            manis_pocket_sync::network::NetworkManager::new(cmd_rx, shared_clone.clone(), key)
                .unwrap()
        } else {
            manis_pocket_sync::network::NetworkManager::new_with_port(
                cmd_rx,
                shared_clone.clone(),
                key,
                port,
            )
            .unwrap()
        };
        rt.block_on(async move { nm.run().await });
    });

    println!(
        "Spawned node '{}' on port {} with peer_id {}",
        device_name, port, peer_id
    );
    cmd_tx
}

fn wait_and_report(events: Arc<Mutex<Vec<String>>>, label: &str, secs: u64) {
    println!("\n--- Waiting {}s for {} ---", secs, label);
    std::thread::sleep(Duration::from_secs(secs));

    let evts = events.lock().unwrap();
    println!("\n=== Captured events ({}) ===", label);
    for e in evts.iter() {
        println!("  {}", e);
    }
    println!("=== Total: {} events ===\n", evts.len());
}

/// Test: manually dial from NodeA to NodeB
#[test]
fn test_manual_dial_two_nodes() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .try_init();

    let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let cmd_tx_a = spawn_node("NodeA", "device-a", 31774, events.clone());
    let cmd_tx_b = spawn_node("NodeB", "device-b", 31775, events.clone());

    std::thread::sleep(Duration::from_secs(2));

    println!("\n--- Dialing NodeB at /ip4/127.0.0.1/tcp/31775 ---");
    let _ = cmd_tx_a.send(SyncCommand::AddPeerAddress {
        address: "/ip4/127.0.0.1/tcp/31775".to_string(),
    });

    wait_and_report(events.clone(), "manual dial", 10);

    let evts = events.lock().unwrap();
    let discovered = evts.iter().any(|e| e.contains("peer_discovered"));
    let errors: Vec<_> = evts
        .iter()
        .filter(|e| e.contains("error"))
        .cloned()
        .collect();

    println!("has_discovered={}, errors={}", discovered, errors.len());
    for e in &errors {
        eprintln!("  ERROR: {}", e);
    }

    assert!(
        discovered,
        "Should discover peer after manual dial. Errors: {:?}",
        errors
    );

    let _ = cmd_tx_a.send(SyncCommand::Shutdown);
    let _ = cmd_tx_b.send(SyncCommand::Shutdown);
}

/// Test: "IP:Port" format is parsed correctly
#[test]
fn test_manual_dial_ip_port_format() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .try_init();

    let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let cmd_tx_a = spawn_node("NodeA", "device-a", 31774, events.clone());
    let cmd_tx_b = spawn_node("NodeB", "device-b", 31775, events.clone());

    std::thread::sleep(Duration::from_secs(2));

    // Use plain "IP:Port" format — Rust should parse it
    println!("\n--- Dialing NodeB with 127.0.0.1:31775 ---");
    let _ = cmd_tx_a.send(SyncCommand::AddPeerAddress {
        address: "127.0.0.1:31775".to_string(),
    });

    wait_and_report(events.clone(), "IP:Port format dial", 10);

    let evts = events.lock().unwrap();
    let discovered = evts.iter().any(|e| e.contains("peer_discovered"));

    assert!(discovered, "Should discover peer with IP:Port format");

    let _ = cmd_tx_a.send(SyncCommand::Shutdown);
    let _ = cmd_tx_b.send(SyncCommand::Shutdown);
}
