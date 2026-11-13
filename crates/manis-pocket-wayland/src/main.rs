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

use base64::Engine as _;
use manis_pocket_sync::network::NetworkManager;
use manis_pocket_sync::register::{
    ClipboardValue, MAX_CLIPBOARD_IMAGE_BYTES, MAX_CLIPBOARD_TEXT_BYTES,
};
use manis_pocket_sync::state::{SharedState, SyncCommand, SyncState};
use manis_pocket_sync::{SyncEvent, SyncItemContent};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

const MAX_TEXT_BYTES: usize = MAX_CLIPBOARD_TEXT_BYTES;
const MAX_IMAGE_BYTES: usize = MAX_CLIPBOARD_IMAGE_BYTES;
/// v1 shares PNG screenshots/copies only; other binary flavours are ignored.
const PNG_MIME: &str = manis_pocket_sync::register::SHAREABLE_IMAGE_MIME;

/// What the local Wayland clipboard holds, in the same shape the sync protocol
/// speaks. Keeping one representation lets local reports and remote applies
/// share the change-detection path.
#[derive(Clone, Debug, PartialEq, Eq)]
enum LocalClipboard {
    /// Nothing is copied, or the selection is empty.
    Empty,
    Text(String),
    Image {
        mime: String,
        bytes: Vec<u8>,
    },
}

impl LocalClipboard {
    fn protocol_value(&self) -> ClipboardValue {
        match self {
            Self::Empty => ClipboardValue::Unavailable,
            Self::Text(text) => ClipboardValue::Text(text.clone()),
            Self::Image { mime, bytes } => ClipboardValue::Binary {
                mime: mime.clone(),
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            },
        }
    }
}

/// Outcome of one clipboard poll.
///
/// `Unshareable` is deliberately separate from `Empty`: an image that is too
/// large must not be reported to peers as a cleared clipboard, because that
/// would wipe what they already hold. Callers skip the report entirely.
enum ClipboardRead {
    Shareable(LocalClipboard),
    Unshareable,
}

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
    let initial_clipboard = read_wayland_clipboard().await;
    let mut last_seen = match initial_clipboard.as_ref() {
        Ok(ClipboardRead::Shareable(clipboard)) => Some(clipboard.clone()),
        Ok(ClipboardRead::Unshareable) | Err(_) => None,
    };
    let mut clipboard_initialized = initial_clipboard.is_ok();
    if let Ok(ClipboardRead::Shareable(clipboard)) = initial_clipboard {
        emit_clipboard_gui(gui_protocol, &clipboard);
        commands.send(SyncCommand::ObserveLocalClipboard {
            value: clipboard.protocol_value(),
        })?;
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
                    SyncEvent::CurrentClipboardReceived { event_id, peer_id, content } => {
                        let (reply, permitted) = tokio::sync::oneshot::channel();
                        commands.send(SyncCommand::ShouldApplyCurrentClipboard { event_id: event_id.clone(), reply })?;
                        if !permitted.await.unwrap_or(false) {
                            commands.send(SyncCommand::CurrentClipboardApplied { event_id, success: false })?;
                            continue;
                        }
                        // Decode and validate before touching the selection, so a
                        // malformed payload cannot clear the local clipboard.
                        let target = match decode_remote_clipboard(content.as_ref()) {
                            Ok(target) => target,
                            Err(error) => {
                                eprintln!("Clipboard payload from {peer_id} rejected: {error}");
                                commands.send(SyncCommand::CurrentClipboardApplied { event_id, success: false })?;
                                continue;
                            }
                        };
                        let result: Result<(), Box<dyn Error>> = if last_seen.as_ref() == Some(&target) {
                            Ok(())
                        } else {
                            match &target {
                                LocalClipboard::Empty => clear_wayland_clipboard().await,
                                LocalClipboard::Text(text) => write_wayland_text(text).await,
                                LocalClipboard::Image { mime, bytes } => write_wayland_typed(mime, bytes).await,
                            }
                        };
                        match result {
                            Ok(()) => {
                                emit_clipboard_gui(gui_protocol, &target);
                                last_seen = Some(target);
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
                match read_wayland_clipboard().await {
                    Ok(ClipboardRead::Shareable(current))
                        if !clipboard_initialized || last_seen.as_ref() != Some(&current) =>
                    {
                        clipboard_initialized = true;
                        last_seen = Some(current.clone());
                        emit_clipboard_gui(gui_protocol, &current);
                        commands.send(SyncCommand::ObserveLocalClipboard {
                            value: current.protocol_value(),
                        })?;
                    }
                    Ok(ClipboardRead::Shareable(_)) => {}
                    // Present but not shareable: keep peers on their last value.
                    Ok(ClipboardRead::Unshareable) => {}
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

/// wl-paste reports an empty selection on stderr with a zero-byte stdout.
/// Those phrases all mean "nothing is copied", whatever the requested type.
fn is_empty_selection(stderr: &[u8]) -> bool {
    let error = String::from_utf8_lossy(stderr).to_lowercase();
    error.contains("nothing is copied")
        || error.contains("no suitable type")
        || error.contains("no selection")
}

async fn list_wayland_types() -> io::Result<Option<Vec<String>>> {
    let output = tokio::process::Command::new("wl-paste")
        .arg("--list-types")
        .kill_on_drop(true)
        .output()
        .await?;
    if !output.status.success() {
        if is_empty_selection(&output.stderr) {
            return Ok(None);
        }
        return Err(io::Error::other(format!(
            "wl-paste --list-types: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect(),
    ))
}

/// Reads the current selection, preferring `image/png` when the source offers
/// it and falling back to plain text otherwise.
async fn read_wayland_clipboard() -> io::Result<ClipboardRead> {
    let Some(types) = list_wayland_types().await? else {
        return Ok(ClipboardRead::Shareable(LocalClipboard::Empty));
    };
    if types.is_empty() {
        return Ok(ClipboardRead::Shareable(LocalClipboard::Empty));
    }
    if types.iter().any(|kind| kind == PNG_MIME) {
        return read_wayland_image(PNG_MIME).await;
    }
    read_wayland_text().await
}

async fn read_wayland_text() -> io::Result<ClipboardRead> {
    let output = tokio::process::Command::new("wl-paste")
        .args(["--no-newline", "--type", "text/plain"])
        .kill_on_drop(true)
        .output()
        .await?;
    if !output.status.success() {
        if is_empty_selection(&output.stderr) {
            return Ok(ClipboardRead::Shareable(LocalClipboard::Empty));
        }
        return Err(io::Error::other(format!(
            "wl-paste: {}",
            String::from_utf8_lossy(&output.stderr)
                .to_lowercase()
                .trim()
        )));
    }
    // Oversized text is skipped rather than published as `Unavailable`, matching
    // `ClipboardValue::is_shareable`: copying something we cannot ship must not
    // clear the peer's clipboard. Previously this reported an empty clipboard.
    if output.stdout.len() > MAX_TEXT_BYTES {
        return Ok(ClipboardRead::Unshareable);
    }
    let text = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    if text.is_empty() {
        return Ok(ClipboardRead::Shareable(LocalClipboard::Empty));
    }
    Ok(ClipboardRead::Shareable(LocalClipboard::Text(text)))
}

async fn read_wayland_image(mime: &str) -> io::Result<ClipboardRead> {
    let output = tokio::process::Command::new("wl-paste")
        .args(["--no-newline", "--type", mime])
        .kill_on_drop(true)
        .output()
        .await?;
    if !output.status.success() {
        if is_empty_selection(&output.stderr) {
            return Ok(ClipboardRead::Shareable(LocalClipboard::Empty));
        }
        return Err(io::Error::other(format!(
            "wl-paste --type {mime}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let bytes = output.stdout;
    if bytes.is_empty() {
        return Ok(ClipboardRead::Shareable(LocalClipboard::Empty));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Ok(ClipboardRead::Unshareable);
    }
    Ok(ClipboardRead::Shareable(LocalClipboard::Image {
        mime: mime.to_owned(),
        bytes,
    }))
}

/// Decodes a peer's clipboard content into the local representation.
///
/// `SyncItemContent.value` is base64 for every content type in this protocol,
/// text included (see `portable_text_from_json` in the sync crate). Decoding
/// and validation happen before anything touches the selection, so a malformed
/// or oversized payload is rejected instead of clearing the local clipboard.
fn decode_remote_clipboard(content: Option<&SyncItemContent>) -> Result<LocalClipboard, String> {
    // A cleared peer clipboard is signalled by the absence of content, not by a
    // missing value inside present content.
    let Some(content) = content else {
        return Ok(LocalClipboard::Empty);
    };
    let Some(value) = content.value.as_deref() else {
        return Err(format!(
            "{} payload is missing its value",
            content.content_type.trim()
        ));
    };
    let mime = content.content_type.trim();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| format!("invalid base64 for {mime}: {error}"))?;

    if mime.to_ascii_lowercase().starts_with("image/") {
        if bytes.is_empty() {
            return Ok(LocalClipboard::Empty);
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(format!(
                "{mime} payload is {} bytes; the limit is {MAX_IMAGE_BYTES}",
                bytes.len()
            ));
        }
        return Ok(LocalClipboard::Image {
            mime: mime.to_owned(),
            bytes,
        });
    }

    let is_text_mime = mime.to_ascii_lowercase().starts_with("text/")
        || mime.eq_ignore_ascii_case("public.utf8-plain-text");
    if !is_text_mime {
        return Err(format!("unsupported clipboard content type: {mime}"));
    }
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(format!(
            "text payload is {} bytes; the limit is {MAX_TEXT_BYTES}",
            bytes.len()
        ));
    }
    let text =
        String::from_utf8(bytes).map_err(|error| format!("text payload is not UTF-8: {error}"))?;
    if text.is_empty() {
        return Ok(LocalClipboard::Empty);
    }
    Ok(LocalClipboard::Text(text))
}

/// Emits the GTK shell's clipboard event. Images carry a null `text` so the
/// shell keeps them out of its text history while still seeing the change.
fn emit_clipboard_gui(gui_protocol: bool, clipboard: &LocalClipboard) {
    let event = match clipboard {
        LocalClipboard::Text(text) => serde_json::json!({"type":"clipboard", "text":text}),
        LocalClipboard::Image { mime, bytes } => serde_json::json!({
            "type":"clipboard",
            "text":serde_json::Value::Null,
            "mime":mime,
            "bytes":bytes.len()
        }),
        LocalClipboard::Empty => {
            serde_json::json!({"type":"clipboard", "text":serde_json::Value::Null})
        }
    };
    emit_gui(gui_protocol, event);
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
    write_wayland_typed("text/plain;charset=utf-8", text.as_bytes()).await
}

/// Hands `bytes` to `wl-copy` under an explicit MIME type. Text and images take
/// the same path; only the type and the payload differ.
async fn write_wayland_typed(mime: &str, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut child = tokio::process::Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("wl-copy stdin unavailable")?;
    stdin.write_all(bytes).await?;
    drop(stdin);
    if !child.wait().await?.success() {
        return Err(format!("wl-copy --type {mime} failed").into());
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

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    fn content(content_type: &str, value: Option<&str>) -> SyncItemContent {
        SyncItemContent {
            content_type: content_type.to_owned(),
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn absent_content_clears_the_clipboard() {
        assert_eq!(decode_remote_clipboard(None), Ok(LocalClipboard::Empty));
    }

    #[test]
    fn text_payloads_are_base64_decoded() {
        // `SyncItemContent.value` is base64 for text too; decoding it as plain
        // text would write base64 gibberish into the selection.
        let encoded = STANDARD.encode("héllo\nworld");
        let decoded =
            decode_remote_clipboard(Some(&content("text/plain;charset=utf-8", Some(&encoded))));
        assert_eq!(decoded, Ok(LocalClipboard::Text("héllo\nworld".into())));
    }

    #[test]
    fn png_payloads_keep_their_raw_bytes() {
        let bytes = b"\x89PNG\r\n\x1a\nbody";
        let encoded = STANDARD.encode(bytes);
        let decoded = decode_remote_clipboard(Some(&content("image/png", Some(&encoded))));
        assert_eq!(
            decoded,
            Ok(LocalClipboard::Image {
                mime: "image/png".into(),
                bytes: bytes.to_vec(),
            })
        );
    }

    #[test]
    fn oversized_images_are_rejected_not_applied() {
        let encoded = STANDARD.encode(vec![0u8; MAX_IMAGE_BYTES + 1]);
        let decoded = decode_remote_clipboard(Some(&content("image/png", Some(&encoded))));
        assert!(decoded.is_err(), "expected rejection, got {decoded:?}");
    }

    #[test]
    fn unsupported_content_types_are_rejected() {
        let encoded = STANDARD.encode("payload");
        let decoded = decode_remote_clipboard(Some(&content("application/pdf", Some(&encoded))));
        assert!(decoded.is_err(), "expected rejection, got {decoded:?}");
    }

    #[test]
    fn missing_or_malformed_values_are_rejected() {
        let missing = decode_remote_clipboard(Some(&content("text/plain;charset=utf-8", None)));
        assert!(missing.is_err(), "expected rejection, got {missing:?}");
        let malformed = decode_remote_clipboard(Some(&content("image/png", Some("not base64!"))));
        assert!(malformed.is_err(), "expected rejection, got {malformed:?}");
    }

    #[test]
    fn protocol_values_use_standard_base64() {
        let bytes = vec![0, 1, 2, 250];
        let image = LocalClipboard::Image {
            mime: PNG_MIME.to_owned(),
            bytes: bytes.clone(),
        };
        assert_eq!(
            image.protocol_value(),
            ClipboardValue::Binary {
                mime: PNG_MIME.to_owned(),
                data: STANDARD.encode(&bytes),
            }
        );
        assert_eq!(
            LocalClipboard::Empty.protocol_value(),
            ClipboardValue::Unavailable
        );
        assert_eq!(
            LocalClipboard::Text("a".into()).protocol_value(),
            ClipboardValue::Text("a".into())
        );
    }

    #[test]
    fn empty_selection_messages_are_recognised() {
        assert!(is_empty_selection(b"Nothing is copied\n"));
        assert!(is_empty_selection(b"No suitable type\n"));
        assert!(is_empty_selection(b"No selection\n"));
        assert!(!is_empty_selection(
            b"wl-copy: failed to connect to display\n"
        ));
    }
}
