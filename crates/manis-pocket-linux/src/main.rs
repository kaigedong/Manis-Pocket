use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

const EVENT_PREFIX: &str = "@@MANIS_POCKET_EVENT ";
const HISTORY_LIMIT: usize = 100;

#[derive(Clone)]
struct Peer {
    name: String,
    connected: bool,
    paired: bool,
}

struct Ui {
    history: Vec<String>,
    peers: HashMap<String, Peer>,
    history_path: PathBuf,
    history_list: gtk::ListBox,
    peer_list: gtk::ListBox,
    search: gtk::SearchEntry,
    status: gtk::Label,
    window: ApplicationWindow,
    input: Rc<RefCell<ChildStdin>>,
    events: mpsc::Sender<serde_json::Value>,
}

fn main() {
    let app = Application::builder()
        .application_id("io.github.kaigedong.ManisPocket")
        .build();
    app.connect_activate(build_window);
    app.run();
}

fn build_window(app: &Application) {
    if let Some(window) = app.windows().first() {
        window.present();
        return;
    }
    let hold_guard = app.hold();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Manis Pocket")
        .default_width(720)
        .default_height(540)
        .build();
    let header = gtk::HeaderBar::new();
    header.set_title_widget(Some(&gtk::Label::new(Some("Manis Pocket"))));
    let quit = gtk::Button::with_label("Quit");
    let quit_app = app.clone();
    quit.connect_clicked(move |_| quit_app.quit());
    header.pack_end(&quit);
    window.set_titlebar(Some(&header));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let notebook = gtk::Notebook::new();
    notebook.set_hexpand(true);
    notebook.set_vexpand(true);
    root.append(&notebook);

    let history_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    history_page.set_margin_top(16);
    history_page.set_margin_bottom(16);
    history_page.set_margin_start(20);
    history_page.set_margin_end(20);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search clipboard history"));
    history_page.append(&search);
    let history_list = gtk::ListBox::new();
    history_list.add_css_class("boxed-list");
    history_list.set_selection_mode(gtk::SelectionMode::None);
    let empty_history = gtk::Label::new(Some(
        "Copy text to see it here. Select Copy to use it again.",
    ));
    empty_history.set_wrap(true);
    empty_history.set_margin_top(32);
    history_list.set_placeholder(Some(&empty_history));
    let history_scroll = gtk::ScrolledWindow::new();
    history_scroll.set_vexpand(true);
    history_scroll.set_child(Some(&history_list));
    history_page.append(&history_scroll);
    notebook.append_page(&history_page, Some(&gtk::Label::new(Some("History"))));

    let devices_page = gtk::Box::new(gtk::Orientation::Vertical, 16);
    devices_page.set_margin_top(20);
    devices_page.set_margin_bottom(20);
    devices_page.set_margin_start(20);
    devices_page.set_margin_end(20);
    let instruction = gtk::Label::new(Some(
        "Keep Manis Pocket open on both devices. Pair once to sync text clipboard changes.",
    ));
    instruction.set_wrap(true);
    instruction.set_xalign(0.0);
    devices_page.append(&instruction);
    let peer_list = gtk::ListBox::new();
    peer_list.add_css_class("boxed-list");
    peer_list.set_selection_mode(gtk::SelectionMode::None);
    let empty_peers = gtk::Label::new(Some(
        "No devices found yet. Check that both devices are on the same network, or connect by IP below.",
    ));
    empty_peers.set_wrap(true);
    empty_peers.set_margin_top(24);
    peer_list.set_placeholder(Some(&empty_peers));
    let peer_scroll = gtk::ScrolledWindow::new();
    peer_scroll.set_vexpand(true);
    peer_scroll.set_child(Some(&peer_list));
    devices_page.append(&peer_scroll);
    let address_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let address = gtk::Entry::new();
    address.set_placeholder_text(Some("IP address:31774"));
    address.set_hexpand(true);
    let connect = gtk::Button::with_label("Connect");
    address_row.append(&address);
    address_row.append(&connect);
    devices_page.append(&address_row);
    notebook.append_page(&devices_page, Some(&gtk::Label::new(Some("Devices"))));

    let status = gtk::Label::new(Some(
        "Starting clipboard sync… Closing the window keeps sync running.",
    ));
    status.set_xalign(0.0);
    status.set_margin_start(20);
    status.set_margin_end(20);
    status.set_margin_top(8);
    status.set_margin_bottom(10);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&status);
    window.set_child(Some(&root));

    let backend = std::env::current_exe()
        .expect("Cannot locate Manis Pocket")
        .with_file_name("manis-pocket-wayland");
    let mut child = match Command::new(&backend)
        .arg("--gui-protocol")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            status.set_text(&format!("Cannot start clipboard sync: {error}"));
            window.present();
            return;
        }
    };
    let input = Rc::new(RefCell::new(child.stdin.take().expect("backend stdin")));
    let stdout = child.stdout.take().expect("backend stdout");
    let stderr = child.stderr.take().expect("backend stderr");
    let (events_tx, events_rx) = mpsc::channel::<serde_json::Value>();
    let output_tx = events_tx.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(json) = line.strip_prefix(EVENT_PREFIX) {
                if let Ok(event) = serde_json::from_str(json) {
                    if output_tx.send(event).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = output_tx.send(serde_json::json!({"type":"stopped"}));
    });
    let error_tx = events_tx.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if error_tx
                .send(serde_json::json!({"type":"error", "message":line}))
                .is_err()
            {
                break;
            }
        }
    });

    let history_path = history_path();
    let ui = Rc::new(RefCell::new(Ui {
        history: load_history(&history_path),
        peers: HashMap::new(),
        history_path,
        history_list,
        peer_list,
        search,
        status,
        window: window.clone(),
        input: input.clone(),
        events: events_tx,
    }));
    render_history(&ui.borrow());

    let search_ui = ui.clone();
    ui.borrow().search.connect_search_changed(move |_| {
        render_history(&search_ui.borrow());
    });
    let connect_ui = ui.clone();
    connect.connect_clicked(move |_| {
        let value = address.text().trim().to_string();
        if value.parse::<std::net::SocketAddr>().is_err() {
            connect_ui
                .borrow()
                .status
                .set_text("Enter an IP address and port, such as 192.168.1.2:31774.");
            return;
        }
        if send_command(&connect_ui.borrow().input, &format!("connect {value}")).is_ok() {
            connect_ui.borrow().status.set_text("Connecting…");
        }
    });
    let events_ui = ui.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        while let Ok(event) = events_rx.try_recv() {
            handle_event(&events_ui, event);
        }
        glib::ControlFlow::Continue
    });
    let child = Rc::new(RefCell::new(child));
    app.connect_shutdown(move |_| {
        let _ = &hold_guard;
        let mut child = child.borrow_mut();
        let _ = child.kill();
        let _ = child.wait();
    });
    window.connect_close_request(|window| {
        window.hide();
        glib::Propagation::Stop
    });
    window.present();
}

fn handle_event(ui: &Rc<RefCell<Ui>>, event: serde_json::Value) {
    let kind = event
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    match kind {
        "clipboard" => {
            if let Some(text) = event.get("text").and_then(|value| value.as_str()) {
                if !text.is_empty() {
                    let mut state = ui.borrow_mut();
                    state.history.retain(|item| item != text);
                    state.history.insert(0, text.to_owned());
                    state.history.truncate(HISTORY_LIMIT);
                    if let Err(error) = save_history(&state.history_path, &state.history) {
                        state
                            .status
                            .set_text(&format!("Could not save history: {error}"));
                    }
                    render_history(&state);
                }
            }
        }
        "peer" => {
            let Some(id) = event.get("id").and_then(|value| value.as_str()) else {
                return;
            };
            let name = event
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or(id);
            let mut state = ui.borrow_mut();
            state.peers.insert(
                id.to_owned(),
                Peer {
                    name: name.to_owned(),
                    connected: event
                        .get("connected")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                    paired: event
                        .get("paired")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                },
            );
            render_peers(&state);
        }
        "peer_lost" => {
            if let Some(id) = event.get("id").and_then(|value| value.as_str()) {
                let mut state = ui.borrow_mut();
                if state.peers.get(id).is_some_and(|peer| !peer.paired) {
                    state.peers.remove(id);
                } else if let Some(peer) = state.peers.get_mut(id) {
                    peer.connected = false;
                }
                render_peers(&state);
            }
        }
        "pairing_request" => {
            let id = event
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let name = event
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or(id);
            let pin = event
                .get("pin")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if id.is_empty() || pin.is_empty() {
                return;
            }
            show_pairing(ui, id, name, pin);
        }
        "pairing_complete" => {
            let id = event
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let success = event
                .get("success")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let mut state = ui.borrow_mut();
            state
                .peers
                .entry(id.to_owned())
                .or_insert_with(|| Peer {
                    name: id.to_owned(),
                    connected: true,
                    paired: false,
                })
                .paired = success;
            state.status.set_text(if success {
                "Device paired. Clipboard sync is active."
            } else {
                "Pairing was declined."
            });
            render_peers(&state);
        }
        "unpaired" => {
            if let Some(id) = event.get("id").and_then(|value| value.as_str()) {
                let mut state = ui.borrow_mut();
                if let Some(peer) = state.peers.get_mut(id) {
                    peer.paired = false;
                }
                state.status.set_text("Device unpaired.");
                render_peers(&state);
            }
        }
        "copied" => ui.borrow().status.set_text("Copied to clipboard"),
        "copy_error" => ui
            .borrow()
            .status
            .set_text("Could not copy this item. Check wl-clipboard."),
        "listening" => ui
            .borrow()
            .status
            .set_text("Watching clipboard · Waiting for devices"),
        "error" => {
            let message = event
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("Unknown sync error");
            ui.borrow()
                .status
                .set_text(&format!("Sync error: {message}"));
        }
        "stopped" => ui
            .borrow()
            .status
            .set_text("Clipboard sync stopped. Reopen Manis Pocket to retry."),
        _ => {}
    }
}

fn render_history(ui: &Ui) {
    clear_list(&ui.history_list);
    let query = ui.search.text().to_lowercase();
    let placeholder = if query.is_empty() {
        "Copy text to see it here. Select Copy to use it again."
    } else {
        "No history items match this search."
    };
    let empty = gtk::Label::new(Some(placeholder));
    empty.set_wrap(true);
    empty.set_margin_top(32);
    ui.history_list.set_placeholder(Some(&empty));
    for text in ui
        .history
        .iter()
        .filter(|text| text.to_lowercase().contains(&query))
    {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(10);
        row.set_margin_bottom(10);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let preview = text.replace(['\n', '\r'], " ");
        let label = gtk::Label::new(Some(&preview));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(70);
        let copy = gtk::Button::with_label("Copy");
        let value = text.clone();
        let status = ui.status.clone();
        let events = ui.events.clone();
        copy.connect_clicked(move |_| {
            let value = value.clone();
            let events = events.clone();
            std::thread::spawn(move || {
                let kind = if copy_to_clipboard(&value).is_ok() {
                    "copied"
                } else {
                    "copy_error"
                };
                let _ = events.send(serde_json::json!({"type":kind}));
            });
            status.set_text("Copying…");
        });
        row.append(&label);
        row.append(&copy);
        ui.history_list.append(&row);
    }
}

fn render_peers(ui: &Ui) {
    clear_list(&ui.peer_list);
    let mut peers: Vec<_> = ui.peers.iter().collect();
    peers.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    for (id, peer) in peers {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(10);
        row.set_margin_bottom(10);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let label = gtk::Label::new(Some(&format!(
            "{} · {}",
            peer.name,
            if peer.connected {
                if peer.paired {
                    "Connected"
                } else {
                    "Available"
                }
            } else {
                "Offline"
            }
        )));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let action = gtk::Button::with_label(if peer.paired { "Unpair" } else { "Pair" });
        action.set_sensitive(peer.connected || peer.paired);
        let command = format!("{} {id}", if peer.paired { "unpair" } else { "pair" });
        let input = ui.input.clone();
        action.connect_clicked(move |_| {
            let _ = send_command(&input, &command);
        });
        row.append(&label);
        row.append(&action);
        ui.peer_list.append(&row);
    }
}

fn show_pairing(ui: &Rc<RefCell<Ui>>, id: &str, name: &str, pin: &str) {
    let state = ui.borrow();
    state.window.present();
    let dialog = gtk::Dialog::builder()
        .transient_for(&state.window)
        .modal(true)
        .title("Pair device")
        .build();
    let content = dialog.content_area();
    content.set_spacing(14);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);
    let message = gtk::Label::new(Some(&format!(
        "Compare this code with {name}. Confirm on both devices if it matches."
    )));
    message.set_wrap(true);
    content.append(&message);
    let code = gtk::Label::new(Some(pin));
    code.add_css_class("title-1");
    content.append(&code);
    dialog.add_button("Reject", gtk::ResponseType::Reject);
    dialog.add_button("Confirm", gtk::ResponseType::Accept);
    let input = state.input.clone();
    let id = id.to_owned();
    let pin = pin.to_owned();
    dialog.connect_response(move |dialog, response| {
        let command = if response == gtk::ResponseType::Accept {
            format!("confirm {id} {pin}")
        } else {
            format!("reject {id}")
        };
        let _ = send_command(&input, &command);
        dialog.close();
    });
    dialog.present();
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn send_command(input: &Rc<RefCell<ChildStdin>>, command: &str) -> io::Result<()> {
    let mut input = input.borrow_mut();
    writeln!(input, "{command}")?;
    input.flush()
}

fn copy_to_clipboard(text: &str) -> io::Result<()> {
    let mut child = Command::new("wl-copy")
        .args(["--type", "text/plain;charset=utf-8"])
        .stdin(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("wl-copy stdin")
        .write_all(text.as_bytes())?;
    if child.wait()?.success() {
        Ok(())
    } else {
        Err(io::Error::other("wl-copy failed"))
    }
}

fn history_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .expect("HOME or XDG_DATA_HOME is required");
    base.join("manis-pocket/history.json")
}

fn load_history(path: &PathBuf) -> Vec<String> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .unwrap_or_default()
}

fn save_history(path: &PathBuf, history: &[String]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid history path"))?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let temporary = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut file, history)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}
