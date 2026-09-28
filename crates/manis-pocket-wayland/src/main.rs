use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::io::{self, BufRead, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use manis_pocket_sync::SyncEvent;
use manis_pocket_sync::network::NetworkManager;
use manis_pocket_sync::state::{SharedState, SyncCommand, SyncState};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

const MAX_TEXT_BYTES: usize = manis_pocket_sync::register::MAX_CLIPBOARD_TEXT_BYTES;

struct Options {
    name: String,
    connect: Option<String>,
    gui_protocol: bool,
}

fn emit_gui(enabled: bool, event: serde_json::Value) {
    if enabled {
        println!("@@MANIS_POCKET_EVENT {event}");
        let _ = io::stdout().flush();
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err("WAYLAND_DISPLAY is missing; run inside a Wayland session".into());
    }
    for command in ["wl-paste", "wl-copy"] {
        std::process::Command::new(command)
            .arg("--version")
            .output()
            .map_err(|_| format!("{command} is required; install wl-clipboard"))?;
    }

    let config = config_dir()?;
    fs::create_dir_all(&config)?;
    fs::set_permissions(&config, fs::Permissions::from_mode(0o700))?;
    let key = load_or_create_key(&config.join("identity.bin"))?;
    let peer_id = key.public().to_peer_id();
    let pairs_path = config.join("paired-peers.json");
    let paired = load_pairs(&pairs_path)?;

    // SyncState owns an unused Tokio runtime; retain it outside block_on so it
    // is dropped after the active runtime has stopped.
    let state: SharedState = Arc::new(Mutex::new(
        SyncState::new(&options.name, &peer_id.to_string())
            .map_err(|error| format!("Cannot initialize sync state: {error:?}"))?,
    ));
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    {
        let state = state.lock().unwrap();
        *state.on_event.lock() = Some(Box::new(move |json: &str| {
            if let Ok(event) = serde_json::from_str::<SyncEvent>(json) {
                let _ = events_tx.send(event);
            }
        }));
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(run(
        options,
        pairs_path,
        key,
        state.clone(),
        paired,
        events_rx,
    ));
    drop(runtime);
    drop(state);
    result
}

async fn run(
    options: Options,
    pairs_path: PathBuf,
    key: libp2p::identity::Keypair,
    state: SharedState,
    mut paired: HashSet<String>,
    mut events_rx: mpsc::UnboundedReceiver<SyncEvent>,
) -> Result<(), Box<dyn Error>> {
    let (commands, command_rx) = mpsc::unbounded_channel();
    let clipboard_store = pairs_path.with_file_name("clipboard-state.json");
    let mut manager = NetworkManager::new_with_storage(command_rx, state, key, clipboard_store)
        .map_err(|error| format!("Cannot initialize network: {error:?}"))?;
    manager.set_initial_paired_peers(paired.iter().cloned().collect());
    let network = tokio::spawn(async move { manager.run().await });
    let gui_protocol = options.gui_protocol;
    if let Some(address) = options.connect {
        commands.send(SyncCommand::AddPeerAddress { address })?;
    }

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            match line {
                Ok(line) => {
                    if input_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut discovered = HashMap::<String, String>::new();
    for peer_id in &paired {
        emit_gui(
            gui_protocol,
            serde_json::json!({"type":"peer", "id":peer_id, "name":peer_id, "connected":false, "paired":true}),
        );
    }
    let initial_clipboard = read_wayland_text().await;
    let mut last_seen = initial_clipboard.as_ref().ok().cloned().flatten();
    let mut clipboard_initialized = initial_clipboard.is_ok();
    if let Ok(text) = initial_clipboard {
        emit_gui(
            gui_protocol,
            serde_json::json!({"type":"clipboard", "text":text}),
        );
        commands.send(SyncCommand::ObserveLocalClipboard { text })?;
    }
    let mut suppress_poll_until = None::<Instant>;
    let mut poll = tokio::time::interval(Duration::from_millis(350));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    println!("Manis Pocket Wayland is running. Type 'help' for commands.");

    tokio::pin!(network);
    loop {
        tokio::select! {
            Some(event) = events_rx.recv() => {
                match event {
                    SyncEvent::PeerDiscovered { peer } => {
                        emit_gui(gui_protocol, serde_json::json!({"type":"peer", "id":peer.peer_id, "name":peer.display_name, "connected":peer.is_connected, "paired":paired.contains(&peer.peer_id)}));
                        discovered.insert(peer.peer_id.clone(), peer.display_name.clone());
                        println!("peer {} {} ({})", peer.peer_id, peer.display_name,
                            if peer.is_connected { "connected" } else { "offline" });
                    }
                    SyncEvent::PeerLost { peer_id } => {
                        emit_gui(gui_protocol, serde_json::json!({"type":"peer_lost", "id":peer_id}));
                        discovered.remove(&peer_id);
                        println!("peer {peer_id} left");
                    }
                    SyncEvent::PairingRequest { peer_id, display_name, pin } => {
                        emit_gui(gui_protocol, serde_json::json!({"type":"pairing_request", "id":peer_id, "name":display_name, "pin":pin}));
                        println!("Pair with {display_name} ({peer_id})? Compare code {pin} on both devices.");
                        println!("Type: confirm {peer_id} {pin}  or  reject {peer_id}");
                    }
                    SyncEvent::PairingComplete { peer_id, success } => {
                        emit_gui(gui_protocol, serde_json::json!({"type":"pairing_complete", "id":peer_id, "success":success}));
                        if success {
                            paired.insert(peer_id.clone());
                            save_pairs(&pairs_path, &paired)?;
                            println!("Paired with {peer_id}");
                        } else {
                            println!("Pairing rejected by {peer_id}");
                        }
                    }
                    SyncEvent::CurrentClipboardReceived { event_id, peer_id, text } => {
                        let (reply, permitted) = tokio::sync::oneshot::channel();
                        commands.send(SyncCommand::ShouldApplyCurrentClipboard { event_id: event_id.clone(), reply })?;
                        if !permitted.await.unwrap_or(false) {
                            commands.send(SyncCommand::CurrentClipboardApplied { event_id, success: false })?;
                            continue;
                        }
                        let result = if last_seen == text {
                            Ok(())
                        } else if let Some(ref text) = text {
                            write_wayland_text(text).await
                        } else {
                            clear_wayland_clipboard().await
                        };
                        match result {
                            Ok(()) => {
                                emit_gui(gui_protocol, serde_json::json!({"type":"clipboard", "text":text}));
                                last_seen = text;
                                suppress_poll_until = Some(Instant::now() + Duration::from_secs(1));
                                println!("Clipboard updated from {peer_id}");
                                commands.send(SyncCommand::CurrentClipboardApplied { event_id, success: true })?;
                            }
                            Err(error) => {
                                eprintln!("Clipboard write failed: {error}");
                                commands.send(SyncCommand::CurrentClipboardApplied { event_id, success: false })?;
                            }
                        }
                    }
                    SyncEvent::Error { message, .. } => {
                        emit_gui(gui_protocol, serde_json::json!({"type":"error", "message":message}));
                        eprintln!("Sync error: {message}");
                    }
                    SyncEvent::Listening { address } => {
                        emit_gui(gui_protocol, serde_json::json!({"type":"listening", "address":address}));
                        println!("Listening on {address}");
                    }
                    _ => {}
                }
            }
            Some(line) = input_rx.recv() => {
                if !handle_input(&line, &commands, &mut paired, &pairs_path, &discovered, gui_protocol)? {
                    commands.send(SyncCommand::Shutdown)?;
                    break;
                }
            }
            _ = poll.tick() => {
                if suppress_poll_until.is_some_and(|until| Instant::now() < until) {
                    continue;
                }
                suppress_poll_until = None;
                match read_wayland_text().await {
                    Ok(current) if !clipboard_initialized || last_seen != current => {
                        clipboard_initialized = true;
                        emit_gui(gui_protocol, serde_json::json!({"type":"clipboard", "text":current}));
                        last_seen = current.clone();
                        commands.send(SyncCommand::ObserveLocalClipboard { text: current })?;
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("Clipboard read failed: {error}"),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                commands.send(SyncCommand::Shutdown)?;
                break;
            }
            result = &mut network => {
                result?;
                return Err("network worker stopped".into());
            }
        }
    }
    Ok(())
}

fn handle_input(
    line: &str,
    commands: &mpsc::UnboundedSender<SyncCommand>,
    paired: &mut HashSet<String>,
    pairs_path: &Path,
    discovered: &HashMap<String, String>,
    gui_protocol: bool,
) -> Result<bool, Box<dyn Error>> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["help"] => println!(
            "Commands: peers, connect IP:PORT, pair PEER_ID, confirm PEER_ID CODE, reject PEER_ID, unpair PEER_ID, quit"
        ),
        ["peers"] => {
            for (id, name) in discovered {
                println!(
                    "{id} {name}{}",
                    if paired.contains(id) { " [paired]" } else { "" }
                );
            }
        }
        ["connect", address] => commands.send(SyncCommand::AddPeerAddress {
            address: (*address).into(),
        })?,
        ["pair", peer_id] => commands.send(SyncCommand::RequestPairing {
            peer_id: (*peer_id).into(),
        })?,
        ["confirm", peer_id, pin] => commands.send(SyncCommand::AcceptPairing {
            peer_id: (*peer_id).into(),
            pin: (*pin).into(),
        })?,
        ["reject", peer_id] => commands.send(SyncCommand::RejectPairing {
            peer_id: (*peer_id).into(),
        })?,
        ["unpair", peer_id] => {
            commands.send(SyncCommand::Unpair {
                peer_id: (*peer_id).into(),
            })?;
            paired.remove(*peer_id);
            save_pairs(pairs_path, paired)?;
            emit_gui(
                gui_protocol,
                serde_json::json!({"type":"unpaired", "id":peer_id}),
            );
            println!("Unpaired {peer_id}");
        }
        ["quit"] | ["exit"] => return Ok(false),
        [] => {}
        _ => println!("Unknown command. Type 'help'."),
    }
    Ok(true)
}

async fn read_wayland_text() -> io::Result<Option<String>> {
    let output = tokio::process::Command::new("wl-paste")
        .args(["--no-newline", "--type", "text/plain"])
        .kill_on_drop(true)
        .output()
        .await?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).to_lowercase();
        if error.contains("nothing is copied")
            || error.contains("no suitable type")
            || error.contains("no selection")
        {
            return Ok(None);
        }
        return Err(io::Error::other(format!("wl-paste: {}", error.trim())));
    }
    if output.stdout.len() > MAX_TEXT_BYTES {
        return Ok(None);
    }
    let text = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    Ok(Some(text).filter(|text| !text.is_empty()))
}

async fn clear_wayland_clipboard() -> Result<(), Box<dyn Error>> {
    if tokio::process::Command::new("wl-copy")
        .arg("--clear")
        .status()
        .await?
        .success()
    {
        Ok(())
    } else {
        Err("wl-copy --clear failed".into())
    }
}

async fn write_wayland_text(text: &str) -> Result<(), Box<dyn Error>> {
    let mut child = tokio::process::Command::new("wl-copy")
        .args(["--type", "text/plain;charset=utf-8"])
        .stdin(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("wl-copy stdin unavailable")?;
    stdin.write_all(text.as_bytes()).await?;
    drop(stdin);
    if !child.wait().await?.success() {
        return Err("wl-copy failed".into());
    }
    Ok(())
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut name = std::env::var("HOSTNAME").unwrap_or_else(|_| "Linux Wayland".into());
    let mut connect = None;
    let mut gui_protocol = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = args.next().ok_or("--name needs a value")?,
            "--connect" => connect = Some(args.next().ok_or("--connect needs IP:PORT")?),
            "--gui-protocol" => gui_protocol = true,
            "--help" | "-h" => {
                println!("Usage: manis-pocket-wayland [--name NAME] [--connect IP:PORT]");
                std::process::exit(0);
            }
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    Ok(Options {
        name,
        connect,
        gui_protocol,
    })
}

fn config_dir() -> Result<PathBuf, Box<dyn Error>> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("HOME is missing")?;
    Ok(base.join("manis-pocket"))
}

fn load_or_create_key(path: &Path) -> Result<libp2p::identity::Keypair, Box<dyn Error>> {
    if path.exists() {
        return Ok(libp2p::identity::Keypair::from_protobuf_encoding(
            &fs::read(path)?,
        )?);
    }
    let key = libp2p::identity::Keypair::generate_ed25519();
    write_private(path, &key.to_protobuf_encoding()?)?;
    Ok(key)
}

fn load_pairs(path: &Path) -> Result<HashSet<String>, Box<dyn Error>> {
    if !path.exists() {
        return Ok(HashSet::new());
    }
    Ok(serde_json::from_slice::<Vec<String>>(&fs::read(path)?)?
        .into_iter()
        .collect())
}

fn save_pairs(path: &Path, pairs: &HashSet<String>) -> Result<(), Box<dyn Error>> {
    let mut sorted: Vec<_> = pairs.iter().collect();
    sorted.sort();
    write_private(path, &serde_json::to_vec_pretty(&sorted)?)?;
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}
