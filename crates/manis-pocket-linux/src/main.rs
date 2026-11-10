use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, cairo, gdk, glib};
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

const STYLE: &str = include_str!("style.css");
const PALETTE_LIGHT: &str = include_str!("palette-light.css");
const PALETTE_DARK: &str = include_str!("palette-dark.css");

#[derive(Clone)]
struct Peer {
    name: String,
    connected: bool,
    paired: bool,
}

#[derive(Clone, Copy)]
enum StatusKind {
    Idle,
    Ok,
    Warn,
    Error,
}

struct Ui {
    history: Vec<String>,
    peers: HashMap<String, Peer>,
    history_path: PathBuf,
    history_list: gtk::ListBox,
    peer_list: gtk::ListBox,
    search: gtk::Entry,
    status: gtk::Label,
    dot: gtk::Box,
    window: ApplicationWindow,
    history_tab: gtk::ToggleButton,
    devices_tab: gtk::ToggleButton,
    /// Text of each rendered history row, indexed like `history_list`.
    rows: RefCell<Vec<String>>,
    input: Rc<RefCell<ChildStdin>>,
    events: mpsc::Sender<serde_json::Value>,
}

thread_local! {
    static CSS_PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

fn main() {
    let app = Application::builder()
        .application_id("io.github.kaigedong.ManisPocket")
        .build();
    app.connect_activate(build_window);
    app.run();
}

// ── Appearance ───────────────────────────────────────────────────────────────

fn prefers_dark() -> bool {
    if let Ok(value) = std::env::var("MP_THEME") {
        return value.eq_ignore_ascii_case("dark");
    }
    if std::env::var("GTK_THEME").is_ok_and(|theme| theme.to_lowercase().contains("dark")) {
        return true;
    }
    gtk::Settings::default().is_some_and(|settings| {
        settings.is_gtk_application_prefer_dark_theme()
            || settings
                .gtk_theme_name()
                .is_some_and(|name| name.to_lowercase().contains("dark"))
    })
}

/// Installs the stylesheet for the active appearance. The palette tokens are
/// prepended to the rules so `@mp_*` names resolve before the rules are parsed.
fn apply_theme(dark: bool) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let palette = if dark { PALETTE_DARK } else { PALETTE_LIGHT };
    let css = format!("{palette}\n{STYLE}");
    CSS_PROVIDER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(previous) = slot.take() {
            gtk::style_context_remove_provider_for_display(&display, &previous);
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_data(&css);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        *slot = Some(provider);
    });
}

fn watch_appearance() {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    settings.connect_gtk_application_prefer_dark_theme_notify(|_| apply_theme(prefers_dark()));
    settings.connect_notify_local(Some("gtk-theme-name"), |_, _| apply_theme(prefers_dark()));
}

// ── Text helpers ─────────────────────────────────────────────────────────────

/// The macOS list shows one visual line: newlines become `⏎`, tabs become `⇥`.
fn display_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\r' => {}
            '\n' => out.push_str(" ⏎ "),
            '\t' => out.push_str(" ⇥ "),
            _ => out.push(ch),
        }
    }
    out.trim().to_owned()
}

/// Bold the search match, mirroring the macOS `HighlightMatches` treatment.
fn row_markup(text: &str, query: &str) -> String {
    let plain = glib::markup_escape_text(text).to_string();
    if query.is_empty() {
        return plain;
    }
    let lower = text.to_lowercase();
    let needle = query.to_lowercase();
    let Some(lower_start) = lower.find(&needle) else {
        return plain;
    };
    let chars_before = lower[..lower_start].chars().count();
    let matched_chars = lower[lower_start..lower_start + needle.len()]
        .chars()
        .count();
    let indices: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    let start_byte = indices.get(chars_before).copied().unwrap_or(0);
    let end_byte = if chars_before + matched_chars >= indices.len() {
        text.len()
    } else {
        indices[chars_before + matched_chars]
    };
    if !text.is_char_boundary(start_byte) || !text.is_char_boundary(end_byte) {
        return plain;
    }
    format!(
        "{}<span weight=\"bold\">{}</span>{}",
        glib::markup_escape_text(&text[..start_byte]),
        glib::markup_escape_text(&text[start_byte..end_byte]),
        glib::markup_escape_text(&text[end_byte..])
    )
}

/// macOS shows a 12×12 swatch only when the item text is a hex colour
/// (`ColorImage.from(_:)`); ordinary text rows have no leading accessory.
fn parse_hex_color(text: &str) -> Option<(f64, f64, f64)> {
    let digits: Option<Vec<u8>> = text
        .trim()
        .strip_prefix('#')?
        .chars()
        .map(|ch| ch.to_digit(16).map(|digit| digit as u8))
        .collect();
    let (red, green, blue) = match digits?.as_slice() {
        [r, g, b] => (r * 17, g * 17, b * 17),
        [r1, r2, g1, g2, b1, b2] => (r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2),
        _ => return None,
    };
    Some((
        f64::from(red) / 255.0,
        f64::from(green) / 255.0,
        f64::from(blue) / 255.0,
    ))
}

fn hex_swatch(text: &str) -> Option<gtk::DrawingArea> {
    let (red, green, blue) = parse_hex_color(text)?;
    let area = gtk::DrawingArea::new();
    area.set_content_width(12);
    area.set_content_height(12);
    area.set_valign(gtk::Align::Center);
    area.set_draw_func(move |_, cr: &cairo::Context, width, height| {
        let (width, height) = (f64::from(width), f64::from(height));
        let radius = 2.5_f64.min(width / 2.0).min(height / 2.0);
        cr.set_source_rgb(red, green, blue);
        cr.new_sub_path();
        cr.arc(
            width - radius,
            radius,
            radius,
            -std::f64::consts::FRAC_PI_2,
            0.0,
        );
        cr.arc(
            width - radius,
            height - radius,
            radius,
            0.0,
            std::f64::consts::FRAC_PI_2,
        );
        cr.arc(
            radius,
            height - radius,
            radius,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        );
        cr.arc(
            radius,
            radius,
            radius,
            std::f64::consts::PI,
            1.5 * std::f64::consts::PI,
        );
        cr.close_path();
        let _ = cr.fill();
    });
    Some(area)
}

fn separator() -> gtk::Separator {
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.add_css_class("mp-separator");
    separator
}

fn hint(key: &str, action: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.set_valign(gtk::Align::Center);
    let key = gtk::Label::new(Some(key));
    key.add_css_class("mp-key");
    let action = gtk::Label::new(Some(action));
    action.add_css_class("mp-hint");
    row.append(&key);
    row.append(&action);
    row
}

fn spacer() -> gtk::Box {
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer
}

// ── Status + clipboard actions ───────────────────────────────────────────────

fn set_status(ui: &Ui, kind: StatusKind, text: &str) {
    ui.status.set_text(text);
    ui.status.set_tooltip_text(Some(text));
    for class in ["mp-dot-ok", "mp-dot-warn", "mp-dot-error"] {
        ui.dot.remove_css_class(class);
    }
    match kind {
        StatusKind::Ok => ui.dot.add_css_class("mp-dot-ok"),
        StatusKind::Warn => ui.dot.add_css_class("mp-dot-warn"),
        StatusKind::Error => ui.dot.add_css_class("mp-dot-error"),
        StatusKind::Idle => {}
    }
}

fn copy_text(status: &gtk::Label, events: &mpsc::Sender<serde_json::Value>, text: String) {
    let events = events.clone();
    let status = status.clone();
    status.set_text("Copying…");
    std::thread::spawn(move || {
        let kind = if copy_to_clipboard(&text).is_ok() {
            "copied"
        } else {
            "copy_error"
        };
        let _ = events.send(serde_json::json!({ "type": kind }));
    });
}

fn copy_selected(ui: &Rc<RefCell<Ui>>) {
    let (text, status, events) = {
        let state = ui.borrow();
        let index = state
            .history_list
            .selected_row()
            .map(|row| row.index())
            .unwrap_or(0) as usize;
        let text = state.rows.borrow().get(index).cloned();
        (text, state.status.clone(), state.events.clone())
    };
    if let Some(text) = text {
        copy_text(&status, &events, text);
    }
}

fn move_selection(ui: &Rc<RefCell<Ui>>, delta: i32) {
    let state = ui.borrow();
    let count = state.rows.borrow().len() as i32;
    if count == 0 {
        return;
    }
    let current = state
        .history_list
        .selected_row()
        .map_or(if delta > 0 { -1 } else { count }, |row| row.index());
    let next = (current + delta).clamp(0, count - 1);
    let Some(row) = state.history_list.row_at_index(next) else {
        return;
    };
    state.history_list.select_row(Some(&row));
    if let Some(adjustment) = state.history_list.adjustment() {
        let (top, bottom) = (
            adjustment.value(),
            adjustment.value() + adjustment.page_size(),
        );
        let allocation = row.allocation();
        let row_top = f64::from(allocation.y());
        let row_bottom = row_top + f64::from(allocation.height());
        if row_top < top {
            adjustment.set_value(row_top);
        } else if row_bottom > bottom {
            adjustment.set_value(row_bottom - adjustment.page_size());
        }
    }
}

// ── Window ───────────────────────────────────────────────────────────────────

fn build_window(app: &Application) {
    if let Some(window) = app.windows().first() {
        window.present();
        return;
    }
    let hold_guard = app.hold();
    apply_theme(prefers_dark());
    watch_appearance();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Manis Pocket")
        .default_width(480)
        .default_height(680)
        .build();
    window.add_css_class("mp-window");

    // Titlebar: title, segmented page switch, sync status, hide button.
    let titlebar = gtk::WindowHandle::new();
    titlebar.add_css_class("mp-titlebar");
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.add_css_class("mp-titlebar-content");

    let title = gtk::Label::new(Some("Manis Pocket"));
    title.add_css_class("mp-title");
    title.set_margin_end(6);

    let segment = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    segment.add_css_class("mp-segment");
    segment.set_valign(gtk::Align::Center);
    let history_tab = gtk::ToggleButton::with_label("History");
    let devices_tab = gtk::ToggleButton::with_label("Devices");
    devices_tab.set_group(Some(&history_tab));
    history_tab.set_active(true);
    for tab in [&history_tab, &devices_tab] {
        tab.add_css_class("mp-segment-item");
        segment.append(tab);
    }

    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("mp-dot");
    dot.set_valign(gtk::Align::Center);
    dot.set_size_request(7, 7);

    let status = gtk::Label::new(Some("Starting clipboard sync…"));
    status.add_css_class("mp-status");
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    status.set_max_width_chars(34);
    status.set_valign(gtk::Align::Center);

    let close = gtk::Button::new();
    close.set_icon_name("window-close-symbolic");
    close.add_css_class("mp-close");
    close.set_valign(gtk::Align::Center);
    close.set_tooltip_text(Some("Hide Manis Pocket"));
    let close_window = window.clone();
    close.connect_clicked(move |_| close_window.hide());

    bar.append(&title);
    bar.append(&segment);
    bar.append(&spacer());
    bar.append(&dot);
    bar.append(&status);
    bar.append(&close);
    titlebar.set_child(Some(&bar));
    window.set_titlebar(Some(&titlebar));

    // ── History page ────────────────────────────────────────────────────────
    let history_page = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let search_wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    search_wrap.set_margin_top(2);
    search_wrap.set_margin_bottom(6);
    search_wrap.set_margin_start(10);
    search_wrap.set_margin_end(10);

    let search_box = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    search_box.add_css_class("mp-search");
    search_box.set_hexpand(true);

    let search_icon = gtk::Image::from_icon_name("edit-find-symbolic");
    search_icon.add_css_class("mp-search-icon");

    let search = gtk::Entry::new();
    search.add_css_class("mp-search-entry");
    search.set_placeholder_text(Some("Search clipboard history"));
    search.set_hexpand(true);

    let clear = gtk::Button::new();
    clear.set_icon_name("edit-clear-symbolic");
    clear.add_css_class("mp-search-clear");
    clear.set_valign(gtk::Align::Center);
    clear.set_visible(false);
    clear.set_tooltip_text(Some("Clear search"));
    let clear_search = search.clone();
    clear.connect_clicked(move |_| clear_search.set_text(""));

    search_box.append(&search_icon);
    search_box.append(&search);
    search_box.append(&clear);
    search_wrap.append(&search_box);
    history_page.append(&search_wrap);

    let history_list = gtk::ListBox::new();
    history_list.add_css_class("mp-list");
    history_list.set_selection_mode(gtk::SelectionMode::Single);
    history_list.set_activate_on_single_click(true);
    history_list.set_valign(gtk::Align::Start);
    history_list.set_margin_start(5);
    history_list.set_margin_end(5);
    history_list.set_margin_top(4);
    history_list.set_margin_bottom(4);

    let history_scroll = gtk::ScrolledWindow::new();
    history_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    history_scroll.set_vexpand(true);
    history_scroll.set_child(Some(&history_list));
    history_scroll.set_margin_top(2);
    history_page.append(&history_scroll);

    history_page.append(&separator());

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    footer.add_css_class("mp-footer");
    let hints = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    hints.append(&hint("↵", "Copy"));
    hints.append(&hint("↑↓", "Navigate"));
    hints.append(&hint("esc", "Clear"));
    let quit = gtk::Button::with_label("Quit");
    quit.add_css_class("mp-quit");
    quit.set_valign(gtk::Align::Center);
    let quit_app = app.clone();
    quit.connect_clicked(move |_| quit_app.quit());
    footer.append(&hints);
    footer.append(&spacer());
    footer.append(&quit);
    history_page.append(&footer);

    // ── Devices page ────────────────────────────────────────────────────────
    let devices_page = gtk::Box::new(gtk::Orientation::Vertical, 10);
    devices_page.set_margin_top(12);
    devices_page.set_margin_bottom(12);
    devices_page.set_margin_start(14);
    devices_page.set_margin_end(14);

    let instruction = gtk::Label::new(Some(
        "Keep Manis Pocket open on both devices. Pair once to sync text clipboard changes.",
    ));
    instruction.add_css_class("mp-instruction");
    instruction.set_wrap(true);
    instruction.set_xalign(0.0);
    devices_page.append(&instruction);

    let devices_title = gtk::Label::new(Some("DISCOVERED DEVICES"));
    devices_title.add_css_class("mp-section");
    devices_title.set_xalign(0.0);
    devices_title.set_margin_top(4);
    devices_page.append(&devices_title);

    let peer_list = gtk::ListBox::new();
    peer_list.add_css_class("mp-list");
    peer_list.set_selection_mode(gtk::SelectionMode::None);

    let peer_scroll = gtk::ScrolledWindow::new();
    peer_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    peer_scroll.set_vexpand(false);
    peer_scroll.set_min_content_height(104);
    peer_scroll.set_child(Some(&peer_list));
    devices_page.append(&peer_scroll);

    let address_title = gtk::Label::new(Some("CONNECT BY ADDRESS"));
    address_title.add_css_class("mp-section");
    address_title.set_xalign(0.0);
    devices_page.append(&address_title);

    let address_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let address = gtk::Entry::new();
    address.add_css_class("mp-field");
    address.set_placeholder_text(Some("192.168.1.2:31774"));
    address.set_hexpand(true);
    let connect = gtk::Button::with_label("Connect");
    connect.add_css_class("mp-btn");
    connect.add_css_class("mp-btn-primary");
    connect.set_valign(gtk::Align::Center);
    address_row.append(&address);
    address_row.append(&connect);
    devices_page.append(&address_row);
    devices_page.append(&spacer());

    // ── Page stack ──────────────────────────────────────────────────────────
    let stack = gtk::Stack::new();
    stack.add_named(&history_page, Some("history"));
    stack.add_named(&devices_page, Some("devices"));
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(160);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("mp-content");
    content.append(&stack);
    window.set_child(Some(&content));

    let history_stack = stack.clone();
    history_tab.connect_toggled(move |tab| {
        if tab.is_active() {
            history_stack.set_visible_child_name("history");
        }
    });
    let devices_stack = stack.clone();
    devices_tab.connect_toggled(move |tab| {
        if tab.is_active() {
            devices_stack.set_visible_child_name("devices");
        }
    });

    // ── Backend ─────────────────────────────────────────────────────────────
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
            let message = format!("Cannot start clipboard sync: {error}");
            status.set_text(&message);
            status.set_tooltip_text(Some(&message));
            dot.add_css_class("mp-dot-error");
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
            if let Some(json) = line.strip_prefix(EVENT_PREFIX)
                && let Ok(event) = serde_json::from_str(json)
                && output_tx.send(event).is_err()
            {
                break;
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
        dot,
        window: window.clone(),
        history_tab,
        devices_tab,
        rows: RefCell::new(Vec::new()),
        input: input.clone(),
        events: events_tx,
    }));
    render_history(&ui.borrow());
    render_peers(&ui.borrow());

    let search_ui = ui.clone();
    ui.borrow().search.connect_changed(move |entry| {
        clear.set_visible(!entry.text().is_empty());
        let state = search_ui.borrow();
        render_history(&state);
    });
    let activate_ui = ui.clone();
    ui.borrow().history_list.connect_row_activated(move |_, _| {
        copy_selected(&activate_ui);
        activate_ui.borrow().search.grab_focus();
    });
    let connect_ui = ui.clone();
    connect.connect_clicked(move |_| {
        let value = address.text().trim().to_owned();
        if value.parse::<std::net::SocketAddr>().is_err() {
            set_status(
                &connect_ui.borrow(),
                StatusKind::Error,
                "Enter an IP address and port, such as 192.168.1.2:31774.",
            );
            return;
        }
        let state = connect_ui.borrow();
        if send_command(&state.input, &format!("connect {value}")).is_ok() {
            set_status(&state, StatusKind::Warn, "Connecting…");
        }
    });

    // Keyboard: arrows navigate while the search field keeps focus.
    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    let key_ui = ui.clone();
    let key_app = app.clone();
    key.connect_key_pressed(move |_, keyval, _, modifiers| {
        let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        if control {
            let state = key_ui.borrow();
            return match keyval {
                gdk::Key::q => {
                    drop(state);
                    key_app.quit();
                    glib::Propagation::Stop
                }
                gdk::Key::f | gdk::Key::k => {
                    state.search.grab_focus();
                    glib::Propagation::Stop
                }
                gdk::Key::_1 => {
                    state.history_tab.set_active(true);
                    glib::Propagation::Stop
                }
                gdk::Key::_2 => {
                    state.devices_tab.set_active(true);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            };
        }
        match keyval {
            gdk::Key::Escape => {
                let state = key_ui.borrow();
                if state.search.text().is_empty() {
                    state.window.hide();
                } else {
                    state.search.set_text("");
                }
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                copy_selected(&key_ui);
                let state = key_ui.borrow();
                state.search.grab_focus();
                glib::Propagation::Stop
            }
            gdk::Key::Up => {
                move_selection(&key_ui, -1);
                glib::Propagation::Stop
            }
            gdk::Key::Down => {
                move_selection(&key_ui, 1);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    window.add_controller(key);

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
    ui.borrow().search.grab_focus();
}

// ── Events ───────────────────────────────────────────────────────────────────

fn handle_event(ui: &Rc<RefCell<Ui>>, event: serde_json::Value) {
    let kind = event
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    match kind {
        "clipboard" => {
            if let Some(text) = event.get("text").and_then(|value| value.as_str())
                && !text.is_empty()
            {
                let mut state = ui.borrow_mut();
                state.history.retain(|item| item != text);
                state.history.insert(0, text.to_owned());
                state.history.truncate(HISTORY_LIMIT);
                if let Err(error) = save_history(&state.history_path, &state.history) {
                    set_status(
                        &state,
                        StatusKind::Error,
                        &format!("Could not save history: {error}"),
                    );
                }
                render_history(&state);
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
            render_peers(&state);
            set_status(
                &state,
                if success {
                    StatusKind::Ok
                } else {
                    StatusKind::Warn
                },
                if success {
                    "Device paired. Clipboard sync is active."
                } else {
                    "Pairing was declined."
                },
            );
        }
        "unpaired" => {
            if let Some(id) = event.get("id").and_then(|value| value.as_str()) {
                let mut state = ui.borrow_mut();
                if let Some(peer) = state.peers.get_mut(id) {
                    peer.paired = false;
                }
                render_peers(&state);
                set_status(&state, StatusKind::Idle, "Device unpaired.");
            }
        }
        "copied" => set_status(&ui.borrow(), StatusKind::Ok, "Copied to clipboard"),
        "copy_error" => set_status(
            &ui.borrow(),
            StatusKind::Error,
            "Could not copy this item. Check wl-clipboard.",
        ),
        "listening" => set_status(
            &ui.borrow(),
            StatusKind::Ok,
            "Watching clipboard · Waiting for devices",
        ),
        "error" => {
            let message = event
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("Unknown sync error");
            let text = if message.starts_with("Sync error") {
                message.to_owned()
            } else {
                format!("Sync error: {message}")
            };
            set_status(&ui.borrow(), StatusKind::Error, &text);
        }
        "stopped" => set_status(
            &ui.borrow(),
            StatusKind::Error,
            "Clipboard sync stopped. Reopen Manis Pocket to retry.",
        ),
        _ => {}
    }
}

// ── Rendering ────────────────────────────────────────────────────────────────

fn render_history(ui: &Ui) {
    let query = ui.search.text();
    let needle = query.to_lowercase();
    let matches: Vec<String> = ui
        .history
        .iter()
        .filter(|text| needle.is_empty() || text.to_lowercase().contains(&needle))
        .cloned()
        .collect();

    clear_list(&ui.history_list);
    let empty = gtk::Label::new(Some(if needle.is_empty() {
        "Copy text to see it here. Click an item to copy it again."
    } else {
        "No history items match this search."
    }));
    empty.add_css_class("mp-empty");
    empty.set_wrap(true);
    empty.set_width_chars(38);
    empty.set_max_width_chars(40);
    empty.set_justify(gtk::Justification::Center);
    empty.set_halign(gtk::Align::Center);
    empty.set_margin_top(40);
    empty.set_margin_start(24);
    empty.set_margin_end(24);
    ui.history_list.set_placeholder(Some(&empty));

    for text in &matches {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("mp-row");
        row.set_activatable(true);

        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        inner.add_css_class("mp-row-inner");
        if let Some(swatch) = hex_swatch(text) {
            inner.add_css_class("mp-row-inner-icon");
            inner.append(&swatch);
        }

        let title = gtk::Label::new(None);
        title.add_css_class("mp-row-title");
        title.set_markup(&row_markup(&display_text(text), &query));
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_single_line_mode(true);
        inner.append(&title);

        let action = gtk::Button::new();
        action.set_icon_name("edit-copy-symbolic");
        action.add_css_class("mp-row-action");
        action.set_valign(gtk::Align::Center);
        action.set_tooltip_text(Some("Copy"));
        let value = text.clone();
        let status = ui.status.clone();
        let events = ui.events.clone();
        action.connect_clicked(move |_| copy_text(&status, &events, value.clone()));
        inner.append(&action);

        row.set_child(Some(&inner));

        // macOS: hovering a row promotes it to the selection.
        let motion = gtk::EventControllerMotion::new();
        let hover = row.clone();
        let hover_list = ui.history_list.clone();
        motion.connect_enter(move |_, _, _| {
            hover.add_css_class("mp-row-hover");
            hover_list.select_row(Some(&hover));
        });
        let hover = row.clone();
        motion.connect_leave(move |_| hover.remove_css_class("mp-row-hover"));
        row.add_controller(motion);

        ui.history_list.append(&row);
    }

    *ui.rows.borrow_mut() = matches;
    if let Some(first) = ui.history_list.row_at_index(0) {
        ui.history_list.select_row(Some(&first));
    }
}

fn render_peers(ui: &Ui) {
    clear_list(&ui.peer_list);
    let mut peers: Vec<_> = ui.peers.iter().collect();
    peers.sort_by(|a, b| a.1.name.cmp(&b.1.name));

    let empty = gtk::Label::new(Some(
        "No devices found yet. Check that both devices are on the same network, or connect by address below.",
    ));
    empty.add_css_class("mp-empty");
    empty.set_wrap(true);
    empty.set_width_chars(34);
    empty.set_max_width_chars(36);
    empty.set_justify(gtk::Justification::Center);
    empty.set_halign(gtk::Align::Center);
    empty.set_margin_top(18);
    empty.set_margin_bottom(18);
    empty.set_margin_start(20);
    empty.set_margin_end(20);
    ui.peer_list.set_placeholder(Some(&empty));

    for (id, peer) in peers {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("mp-row");
        row.set_activatable(false);

        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        inner.add_css_class("mp-row-inner");

        let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
        texts.set_hexpand(true);
        texts.set_valign(gtk::Align::Center);
        let name = gtk::Label::new(Some(&peer.name));
        name.add_css_class("mp-device-name");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let state = gtk::Label::new(Some(if peer.connected {
            if peer.paired {
                "Connected"
            } else {
                "Available"
            }
        } else {
            "Offline"
        }));
        state.add_css_class("mp-device-state");
        state.set_xalign(0.0);
        texts.append(&name);
        texts.append(&state);
        inner.append(&texts);

        let action = gtk::Button::with_label(if peer.paired { "Unpair" } else { "Pair" });
        action.add_css_class("mp-btn");
        action.set_valign(gtk::Align::Center);
        action.set_sensitive(peer.connected || peer.paired);
        let command = format!("{} {id}", if peer.paired { "unpair" } else { "pair" });
        let input = ui.input.clone();
        action.connect_clicked(move |_| {
            let _ = send_command(&input, &command);
        });
        inner.append(&action);

        row.set_child(Some(&inner));
        ui.peer_list.append(&row);
    }
}

fn show_pairing(ui: &Rc<RefCell<Ui>>, id: &str, name: &str, pin: &str) {
    let state = ui.borrow();
    state.window.present();
    let dialog = gtk::Dialog::builder()
        .transient_for(&state.window)
        .modal(true)
        .title("Pair Device")
        .build();
    dialog.add_css_class("mp-dialog");
    let content = dialog.content_area();
    content.set_spacing(14);
    content.set_margin_top(22);
    content.set_margin_bottom(22);
    content.set_margin_start(24);
    content.set_margin_end(24);

    let message = gtk::Label::new(Some(&format!(
        "Compare this code with {name}. Confirm on both devices if it matches."
    )));
    message.add_css_class("mp-instruction");
    message.set_wrap(true);
    message.set_justify(gtk::Justification::Center);
    content.append(&message);

    let code_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    code_row.set_halign(gtk::Align::Center);
    for digit in pin.chars() {
        let cell = gtk::Label::new(Some(&digit.to_string()));
        cell.add_css_class("mp-pin-cell");
        cell.set_justify(gtk::Justification::Center);
        code_row.append(&cell);
    }
    content.append(&code_row);

    dialog.add_button("Reject", gtk::ResponseType::Reject);
    dialog.add_button("Confirm", gtk::ResponseType::Accept);
    for (response, primary) in [
        (gtk::ResponseType::Reject, false),
        (gtk::ResponseType::Accept, true),
    ] {
        if let Some(button) = dialog.widget_for_response(response) {
            button.add_css_class("mp-btn");
            if primary {
                button.add_css_class("mp-btn-primary");
            }
        }
    }

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
