//! Manis Pocket — GPUI desktop app.
//!
//! One Rust UI for macOS and Linux, replacing the SwiftUI and GTK front ends.
//! The clipboard engine (`manis-pocket-core`) is used directly as a library;
//! there is no FFI boundary any more.

// `unsafe` blocks still mark the Objective-C calls that touch AppKit, even where
// objc2 has made the individual call safe; the lint would flag every one of them.
#![allow(unused_unsafe)]

mod about;
#[cfg(target_os = "linux")]
mod apps_linux;
mod clipboard;
mod icons;
mod ignore;
mod ipc;
mod panel;
mod peers;
mod placement;
mod rows;
mod settings;
mod settings_ui;
mod sync_runtime;
mod tray;

use panel::{HistoryPanel, PANEL_MARGIN, copy_entry_to_clipboard};

use chrono::Utc;
use futures::StreamExt;
use gpui::{
    App, Bounds, Context, Entity, IntoElement, Point, SharedString, Size, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div, point, px, size,
};
use gpui_kit::base::{IndexPath, StyledExt, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::img;
use gpui_kit::*;
use manis_pocket_core::history::HistoryManager;
use manis_pocket_core::model::{ClipboardContent, ClipboardItem, SearchMode};
use settings::Settings;
use std::sync::{Arc, OnceLock};

/// One history manager for the whole process: the panel and the settings window
/// both read it, and pairing commands act on it while no window is open.
static HISTORY: OnceLock<Arc<HistoryManager>> = OnceLock::new();

fn history() -> Arc<HistoryManager> {
    HISTORY
        .get()
        .expect("history is initialised at startup")
        .clone()
}

/// `--database` override, set once before anything reads the path.
static DATABASE_OVERRIDE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn storage_path() -> String {
    if let Some(path) = DATABASE_OVERRIDE.get() {
        return path.clone();
    }
    // Dev/test override so the app can run against a scratch database without
    // touching the real history.
    if let Ok(path) = std::env::var("MANIS_POCKET_DB") {
        return path;
    }
    let base = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    // Same directory and file the SwiftUI build uses on macOS, so the existing
    // history is picked up unchanged.
    let directory = if cfg!(target_os = "macos") {
        base.join("ManisPocket")
    } else {
        base.join("manis-pocket")
    };
    let _ = std::fs::create_dir_all(&directory);
    directory
        .join("manispocket.db")
        .to_string_lossy()
        .into_owned()
}

/// Where the panel appears: under the menu bar, against the right edge of the
/// primary display. That approximates the macOS build's default strategy of
/// dropping the panel below its status item.
fn panel_bounds(
    cx: &App,
    panel: Size<gpui::Pixels>,
    settings: &Settings,
    anchor: Option<tray::TrayAnchor>,
) -> Bounds<gpui::Pixels> {
    let Some(display) = cx.primary_display() else {
        return Bounds::centered(None, panel, cx);
    };
    let screen = display.bounds();
    let margin = px(PANEL_MARGIN);

    // Keep the whole card on screen whatever the anchor was.
    let clamp = |origin_x: f32, origin_y: f32| {
        let max_x = screen.origin.x + screen.size.width - panel.width;
        let max_y = screen.origin.y + screen.size.height - panel.height;
        point(
            px(origin_x.clamp(screen.origin.x.as_f32(), max_x.as_f32())),
            px(origin_y.clamp(screen.origin.y.as_f32(), max_y.as_f32())),
        )
    };

    // A click on the status icon pins the panel under the icon, whatever the
    // position setting says — that is what the SwiftUI build does.
    // Every entry point opens under the status icon; the setting can only move
    // the panel to the middle of the screen instead. A click carries the icon
    // rectangle, the shortcut and IPC ask the tray for it.
    if settings.popup_position != "center" {
        let anchor = anchor.or_else(|| {
            settings.status_item_x.map(|x| tray::TrayAnchor {
                x: f64::from(x),
                y: 24.0,
            })
        });
        if let Some(anchor) = anchor {
            let (x, y) = placement::anchored_origin(&screen, panel, anchor);
            return Bounds::new(clamp(x, y), panel);
        }
        // Straight after launch the status item has no frame yet; the top-right
        // corner is where it will be, and it beats following the pointer.
        return Bounds::new(
            clamp(
                default_origin(&screen, panel).x.as_f32(),
                default_origin(&screen, panel).y.as_f32(),
            ),
            panel,
        );
    }

    let origin = match settings.popup_position.as_str() {
        // Centred on the screen, like the macOS `.center` mode.
        "center" => point(
            screen.origin.x + (screen.size.width - panel.width) / 2.0,
            screen.origin.y + (screen.size.height - panel.height) / 2.0,
        ),
        // `statusItem`, `window` and `lastPosition` need the menu-bar button or
        // the frontmost window rectangle, which are not wired yet; the top-right
        // anchor is their closest honest approximation.
        _ => default_origin(&screen, panel),
    };
    let _ = margin;
    Bounds::new(origin, panel)
}

/// Top-right corner, 10px from the edge and 6px under the menu bar.
///
/// The window carries a transparent margin for the card's shadow, so the offset
/// applies to the card rather than the window's invisible border.
fn default_origin(screen: &Bounds<gpui::Pixels>, panel: Size<gpui::Pixels>) -> Point<gpui::Pixels> {
    let margin = px(PANEL_MARGIN);
    let gap = px(10.0);
    let menu_bar = px(28.0);
    point(
        screen.origin.x + screen.size.width - panel.width + margin - gap,
        screen.origin.y + menu_bar + px(6.0) - margin,
    )
}

/// Reports whether the command reached a running instance.
fn exit_after(delivered: bool) {
    if !delivered {
        eprintln!("no running Manis Pocket instance to command");
        std::process::exit(1);
    }
}

/// Opens the settings window.
fn open_settings(cx: &mut App) {
    if let Some(id) = settings_window_id() {
        for handle in cx.windows() {
            if handle.window_id() == id {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                cx.activate(true);
                return;
            }
        }
    }
    let bounds = Bounds::centered(None, size(px(720.0), px(520.0)), cx);
    let database = storage_path();
    let opened = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("Manis Pocket 设置")),
                ..Default::default()
            }),
            app_id: Some("io.github.kaigedong.ManisPocket".to_string()),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            let database = database.clone();
            let history = history();
            cx.new(|cx| settings_ui::SettingsView::new(database, history, window, cx))
        },
    );
    match opened {
        Ok((handle, _)) => {
            set_settings_window(Some(handle.window_id()));
            let closed = cx.on_window_closed(|_cx, id| {
                if settings_window_id() == Some(id) {
                    set_settings_window(None);
                }
            });
            std::mem::forget(closed);
            cx.activate(true);
        }
        Err(error) => eprintln!("could not open the settings window: {error}"),
    }
}

/// Id of the About window while it exists, so clicking the footer item twice
/// focuses the version instead of stacking copies of it.
static ABOUT_WINDOW: std::sync::Mutex<Option<gpui::WindowId>> = std::sync::Mutex::new(None);

fn about_window_id() -> Option<gpui::WindowId> {
    ABOUT_WINDOW.lock().ok().and_then(|guard| *guard)
}

fn set_about_window(id: Option<gpui::WindowId>) {
    if let Ok(mut guard) = ABOUT_WINDOW.lock() {
        *guard = id;
    }
}

/// Opens the About window.
///
/// macOS used to answer the footer item with the system About panel and Linux with
/// a browser pointed at the project page; neither said which version was running,
/// which is the question About exists to answer. The window draws the version
/// `build.rs` baked into this binary, on both platforms.
fn open_about(cx: &mut App) {
    if let Some(id) = about_window_id() {
        for handle in cx.windows() {
            if handle.window_id() == id {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                cx.activate(true);
                return;
            }
        }
        // The window is gone but its id was not cleared; forget it and start over.
        set_about_window(None);
    }
    let bounds = Bounds::centered(None, size(px(440.0), px(300.0)), cx);
    let opened = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // The same shape as the history panel: a dialog-sized floating window
            // whose background is transparent, with the visible content drawn as a
            // card. A tiling compositor that stretches the window then shows a
            // card on the desktop rather than a half-screen white slab.
            kind: WindowKind::Floating,
            window_background: WindowBackgroundAppearance::Transparent,
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("关于 Manis Pocket")),
                appears_transparent: true,
                traffic_light_position: Some(point(px(-100.0), px(-100.0))),
            }),
            app_id: Some("io.github.kaigedong.ManisPocket".to_string()),
            is_resizable: false,
            is_movable: false,
            is_minimizable: false,
            ..Default::default()
        },
        cx,
        |_window, cx| cx.new(|_| about::AboutView),
    );
    match opened {
        Ok((handle, _)) => {
            set_about_window(Some(handle.window_id()));
            let closed = cx.on_window_closed(|_cx, id| {
                if about_window_id() == Some(id) {
                    set_about_window(None);
                }
            });
            std::mem::forget(closed);
            cx.activate(true);
        }
        Err(error) => eprintln!("could not open the about window: {error}"),
    }
}

/// Whether the save switches let this copy into the history.
///
/// "保存文本" covers everything textual, files included: a file entry is the path.
fn store_allowed(copy: &clipboard::Copy, settings: &settings::Settings) -> bool {
    match copy {
        clipboard::Copy::Image(_) => settings.save_images,
        clipboard::Copy::Text(_) | clipboard::Copy::File(_) => settings.save_text,
    }
}

/// Keeps a pre-existing history from being trimmed by the default size limit.
///
/// On the very first run the default of 500 would delete everything older the next
/// time something is copied, so the limit is raised once to whatever is already
/// stored. A size the user has chosen is never overridden: once a settings file
/// exists, it is theirs.
/// The limit to keep on a first run. A history already larger than the limit wins,
/// so the default does not delete it; `unlimited` means no limit at all.
fn first_run_limit(limit: i64, stored: i64, unlimited: bool) -> i64 {
    if unlimited || limit >= stored {
        limit
    } else {
        stored
    }
}

fn protect_existing_history(history: &HistoryManager, database: &str) {
    if settings::settings_path(database).exists() {
        return;
    }
    let stored = history.count().unwrap_or(0);
    if stored == 0 {
        return;
    }
    let mut settings = settings::Settings::load(database);
    let limit = first_run_limit(
        settings.history_size.max(0),
        stored,
        settings.unlimited_history,
    );
    if limit != settings.history_size {
        settings.history_size = limit;
        settings.save(database);
    }
}

/// Imports the SwiftUI build's history the first time this build runs.
///
/// The old app kept its entries in a SwiftData store next to ours. Nobody notices a
/// successful import, and a failed one — a different schema, a half-written file —
/// must not stop the app from starting, so it is reported and ignored.
fn import_legacy_history(history: &HistoryManager) {
    if history.count().unwrap_or(1) > 0 {
        return;
    }
    if let Some(store) = legacy_swiftdata_path() {
        match history.migrate_from_swiftdata(store.clone()) {
            Ok(0) => {}
            Ok(count) => eprintln!("imported {count} entries from {store}"),
            Err(error) => eprintln!("could not import {store}: {error}"),
        }
    }
    // The GTK build kept a plain JSON array next to the database it never shared
    // with this one. Renaming it afterwards keeps a cleared history from refilling.
    let json = std::path::Path::new(&storage_path()).with_file_name("history.json");
    if !json.exists() {
        return;
    }
    match history.import_json_history(json.to_string_lossy().as_ref()) {
        Ok(0) => {}
        Ok(count) => {
            eprintln!("imported {count} entries from {}", json.display());
            let _ = std::fs::rename(&json, json.with_extension("json.imported"));
        }
        Err(error) => eprintln!("could not import {}: {error}", json.display()),
    }
}

/// Where the SwiftUI build kept its SwiftData store, if it is there.
fn legacy_swiftdata_path() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path =
        std::path::Path::new(&home).join("Library/Application Support/ManisPocket/default.store");
    path.exists().then(|| path.display().to_string())
}

/// Opens the history panel. Called at launch and by the global shortcut.
/// Id of the panel window while it exists.
///
/// `App::windows()` still lists a window after `remove_window()`, so asking it
/// whether the panel is up answers "yes" forever after the first close and the
/// panel can never be opened again. The id is cleared by the window-closed
/// observer, which is the only reliable signal.
static PANEL_WINDOW: std::sync::Mutex<Option<gpui::WindowId>> = std::sync::Mutex::new(None);
/// The settings window, so asking for it twice focuses the first one instead of
/// stacking copies — a pairing request can arrive while it is already open.
static SETTINGS_WINDOW: std::sync::Mutex<Option<gpui::WindowId>> = std::sync::Mutex::new(None);

fn settings_window_id() -> Option<gpui::WindowId> {
    SETTINGS_WINDOW.lock().ok().and_then(|guard| *guard)
}

fn set_settings_window(id: Option<gpui::WindowId>) {
    if let Ok(mut guard) = SETTINGS_WINDOW.lock() {
        *guard = id;
    }
}

fn panel_window_id() -> Option<gpui::WindowId> {
    PANEL_WINDOW.lock().ok().and_then(|guard| *guard)
}

fn set_panel_window(id: Option<gpui::WindowId>) {
    if let Ok(mut guard) = PANEL_WINDOW.lock() {
        *guard = id;
    }
}

fn open_panel(cx: &mut App) {
    open_panel_at(cx, None)
}

/// Opens the panel, preferring the status icon when the click came from it.
fn open_panel_at(cx: &mut App, anchor: Option<tray::TrayAnchor>) {
    let settings = settings::Settings::load(&storage_path());
    let panel = size(
        px(settings.window_width as f32 + PANEL_MARGIN * 2.0),
        px(settings.window_height as f32 + PANEL_MARGIN * 2.0),
    );
    // Fill the icon cache before the first frame: loading it from the deferred
    // step made the chips and row icons appear a beat after the text, which read
    // as the panel shifting.
    // Warm only what the first frame shows; the rest is filled from the deferred
    // step, so opening never waits for every icon in the history.
    if let Ok(items) = history().load_metadata() {
        icons::preload(
            items
                .iter()
                .take(20)
                .filter_map(|item| item.application.as_deref()),
        );
    }
    let bounds = panel_bounds(cx, panel, &settings, anchor);
    let opened = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // `Floating`, not `PopUp`: gpui-macos implements `PopUp` as a
            // non-activating panel at window level 101, which sits *above* the
            // IME candidate window (level ~20-26) — the candidate list existed
            // but was hidden behind the panel. Floating uses level 3, still above
            // ordinary windows.
            kind: WindowKind::Floating,
            window_background: WindowBackgroundAppearance::Transparent,
            // The SwiftUI panel has no titlebar and no traffic lights.
            // `titlebar: None` leaves the window unshowable on macOS, so the
            // titlebar stays transparent and the buttons are moved off-window.
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("Manis Pocket")),
                appears_transparent: true,
                traffic_light_position: Some(point(px(-100.0), px(-100.0))),
            }),
            // Matches the GTK build's app id so compositor rules and the
            // window list treat both versions the same.
            app_id: Some("io.github.kaigedong.ManisPocket".to_string()),
            // A fixed-size panel, like the SwiftUI build: no resize, no drag,
            // nothing to minimise.
            is_resizable: false,
            is_movable: false,
            is_minimizable: false,
            ..Default::default()
        },
        cx,
        |window, cx| {
            let panel = cx.new(|cx| HistoryPanel::new(window, cx));
            // The list is built inside `new`, so the preview cache is filled once
            // the entity exists rather than during the first paint.
            let handle = panel.clone();
            window.defer(cx, move |_window, cx| {
                handle.update(cx, |panel, cx| {
                    panel.sync_checked(cx);
                    panel.load_app_icons(cx);
                    panel.load_row_images(cx);
                    panel.load_preview_image(cx);
                });
            });
            panel
        },
    );
    // A window that could not be created leaves the app usable: the next click on
    // the status item tries again instead of the process aborting.
    let Ok((handle, _)) = opened else {
        eprintln!("could not open the history panel");
        return;
    };
    // Remember which window is the panel, so a second click can find it and a
    // closed one is not mistaken for an open one.
    set_panel_window(Some(handle.window_id()));
    // Without a window of its own the app is inactive, and a new window made in
    // that state is not ordered to the front, so the panel would exist unseen.
    cx.activate(true);
    let _ = handle.update(cx, |_, window, _| window.activate_window());
}

/// Opens the panel unless it is already on screen.
fn open_panel_if_needed(cx: &mut App) {
    if panel_window_id().is_none() {
        open_panel(cx);
    }
}

/// Shows the panel if it is closed, closes it if it is open.
fn toggle_panel(cx: &mut App) {
    toggle_panel_at(cx, None)
}

fn toggle_panel_at(cx: &mut App, anchor: Option<tray::TrayAnchor>) {
    let Some(id) = panel_window_id() else {
        open_panel_at(cx, anchor);
        return;
    };
    let ok = close_panel_window(cx, id);
    if !ok {
        // The window was already gone; forget it and open a fresh one.
        set_panel_window(None);
        open_panel_at(cx, anchor);
    }
}

/// Closes the panel window, reporting whether it still existed.
fn close_panel_window(cx: &mut App, id: gpui::WindowId) -> bool {
    for handle in cx.windows() {
        if handle.window_id() == id {
            let closed = handle
                .update(cx, |_, window, _| window.remove_window())
                .is_ok();
            if closed {
                return true;
            }
        }
    }
    false
}

/// The SwiftUI build opens the panel with ⇧⌘C (modifiers 768 + key code 8).
fn register_popup_hotkey() {
    use global_hotkey::GlobalHotKeyManager;
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};

    let Ok(manager) = GlobalHotKeyManager::new() else {
        // No hotkey support (no display server, sandboxed): the panel can still
        // be opened by launching the app.
        return;
    };
    let hotkey = HotKey::new(Some(Modifiers::META | Modifiers::SHIFT), Code::KeyC);
    if manager.register(hotkey).is_ok() {
        // The manager drives the shortcut for the whole process, so it must
        // outlive this function.
        Box::leak(Box::new(manager));
    }
}

/// Registers macOS's symbol font with GPUI.
///
/// The UI font has ⌘ and ⌥ but not ⌫/⌦, which the keyboard hints need. GPUI's
/// text system only knows fonts it has been given, so the system file is loaded
/// explicitly and the hints ask for it by name.
fn register_symbol_font(cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        const PATH: &str = "/System/Library/Fonts/Apple Symbols.ttf";
        if let Ok(bytes) = std::fs::read(PATH) {
            let _ = cx
                .text_system()
                .add_fonts(vec![std::borrow::Cow::Owned(bytes)]);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

fn main() {
    // The core and sync crates log through `log`; without a logger that output
    // is discarded, which makes network problems invisible. `RUST_LOG=debug`
    // turns it on.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // `manis-pocket --toggle` is what a compositor binding should call. When no
    // instance is running it falls through and starts the app normally.
    if std::env::args().any(|arg| arg == "--toggle") && ipc::send_toggle(&storage_path()) {
        return;
    }
    let open_settings_at_launch = std::env::args().any(|arg| arg == "--settings");

    if std::env::args().any(|arg| arg == "--probe-app") {
        println!("{:?}", source_application());
        return;
    }

    // `--database <path>` lets a detached launch use a scratch history instead of
    // the real one; `open --args` cannot pass environment variables.
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--database"
            && let Some(path) = args.next()
        {
            let _ = DATABASE_OVERRIDE.set(path);
        }
    }

    // `--copy <id>` puts one entry back on the clipboard: useful for scripting,
    // and it exercises the same path the panel uses.
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        // Copies several entries in the given order, joined by newlines: the same
        // work the panel's paste stack does, and the only form of it that can be
        // checked without a window.
        if arg == "--copy-stack"
            && let Some(ids) = args.next()
        {
            let history = HistoryManager::new(storage_path()).ok();
            let mut parts: Vec<String> = Vec::new();
            if let Some(history) = history {
                for id in ids.split(',').map(str::trim).filter(|id| !id.is_empty()) {
                    let Ok(contents) = history.contents(id.to_string()) else {
                        continue;
                    };
                    if let Some(text) = contents
                        .iter()
                        .find(|content| {
                            matches!(
                                content.content_type.as_str(),
                                "public.utf8-plain-text"
                                    | "text/plain"
                                    | "text/plain;charset=utf-8"
                            )
                        })
                        .and_then(|content| content.value.as_ref())
                        .map(|value| String::from_utf8_lossy(value).to_string())
                    {
                        parts.push(text);
                    }
                }
            }
            let ok = !parts.is_empty() && crate::clipboard::write_text(&parts.join("\n"));
            println!("{ok}");
            return;
        }

        if arg == "--copy"
            && let Some(id) = args.next()
        {
            let ok = HistoryManager::new(storage_path())
                .map(|history| copy_entry_to_clipboard(&history, &id))
                .unwrap_or(false);
            println!("{ok}");
            return;
        }
    }

    // Headless pairing helpers: `manis-pocket --pair <peer>`,
    // `--accept <peer> <pin>`, `--connect <multiaddr>`.
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|arg| arg == name);
    if let Some(index) = flag("--pair")
        && let Some(peer_id) = args.get(index + 1)
    {
        return exit_after(ipc::send(&storage_path(), &format!("pair {peer_id}")));
    }
    if let Some(index) = flag("--accept")
        && let (Some(peer_id), Some(pin)) = (args.get(index + 1), args.get(index + 2))
    {
        return exit_after(ipc::send(
            &storage_path(),
            &format!("accept {peer_id} {pin}"),
        ));
    }
    if let Some(index) = flag("--connect")
        && let Some(address) = args.get(index + 1)
    {
        return exit_after(ipc::send(&storage_path(), &format!("connect {address}")));
    }

    let shared_history =
        Arc::new(HistoryManager::new(storage_path()).expect("open history database"));
    import_legacy_history(&shared_history);
    protect_existing_history(&shared_history, &storage_path());
    let _ = HISTORY.set(shared_history);

    gpui_kit::application()
        // The rows draw their pin and delete buttons from the bundled SVG set;
        // without an asset source the renderer has nothing to load and the window
        // never comes up.
        .with_assets(gpui_kit::assets::Assets::new(""))
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            register_symbol_font(cx);
            // Linux defaults to quitting when the last window closes, which would
            // end the clipboard watcher every time the panel is dismissed. A
            // resident app only quits when asked to.
            cx.set_quit_mode(gpui::QuitMode::Explicit);
            register_popup_hotkey();

            // The panel is created on demand, so its id has to be forgotten the
            // moment the window really closes.
            let closed = cx.on_window_closed(|_cx, id| {
                if panel_window_id() == Some(id) {
                    set_panel_window(None);
                }
            });
            std::mem::forget(closed);

            // The tray only reports intent; the window handling stays here.
            let (tray_tx, mut tray_rx) = futures::channel::mpsc::unbounded();
            // `showInStatusBar == false` means the app lives only behind its shortcut.
            let tray_settings = settings::Settings::load(&storage_path());
            if tray_settings.show_in_status_bar {
                tray::install(tray_tx, &tray_settings.menu_icon);
            }

            // A second launch asks this instance to toggle instead of starting up.
            let (ipc_tx, mut ipc_rx) = futures::channel::mpsc::unbounded();
            ipc::listen(&storage_path(), ipc_tx);

            start_clipboard_capture(cx, history(), storage_path());

            if open_settings_at_launch {
                open_settings(cx);
            } else if std::env::var_os("MANIS_POCKET_ABOUT").is_some() {
                // Screenshot hook: the About window is otherwise only reachable
                // through the footer, which needs a pointer.
                open_about(cx);
            } else {
                open_panel(cx);
            }

            // A resident app: the panel window comes and goes, the process keeps
            // watching the shortcut, the tray menu and the clipboard.
            cx.spawn(async move |cx| {
                use global_hotkey::GlobalHotKeyEvent;
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(120))
                        .await;

                    while let Ok(command) = ipc_rx.try_recv() {
                        eprintln!("[ipc] {command:?}");
                        match command {
                            ipc::IpcCommand::Toggle => cx.update(toggle_panel),
                            ipc::IpcCommand::Pair(peer_id) => {
                                sync_runtime::remember_pairing(&peer_id);
                                history().sync_request_pairing(peer_id);
                            }
                            ipc::IpcCommand::Accept { peer_id, pin } => {
                                history().sync_accept_pairing(peer_id, pin);
                            }
                            ipc::IpcCommand::Connect(address) => {
                                history().sync_add_peer_address(address);
                            }
                        }
                    }

                    while let Ok(command) = tray_rx.try_recv() {
                        match command {
                            tray::TrayCommand::Toggle(anchor) => cx.update(|cx| {
                                if let Some(anchor) = anchor {
                                    let mut settings = settings::Settings::load(&storage_path());
                                    if settings.status_item_x != Some(anchor.x as f32) {
                                        settings.status_item_x = Some(anchor.x as f32);
                                        settings.save(&storage_path());
                                    }
                                }
                                toggle_panel_at(cx, anchor)
                            }),
                            tray::TrayCommand::Open => cx.update(open_panel_if_needed),
                            tray::TrayCommand::Settings => cx.update(open_settings),
                            tray::TrayCommand::Quit => cx.update(|cx| cx.quit()),
                        }
                    }

                    while GlobalHotKeyEvent::receiver().try_recv().is_ok() {
                        cx.update(open_panel_if_needed);
                    }
                }
            })
            .detach();

            cx.activate(true);
        });
}

/// Which application owns the copy, when the platform can say.
#[cfg(target_os = "macos")]
fn source_application() -> Option<String> {
    icons::frontmost_bundle_id()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn source_application() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn source_application() -> Option<String> {
    apps_linux::active_app()
}

/// Records a new copy observed by the platform watcher.
///
/// This runs at app scope, not per window: the panel is destroyed whenever it
/// loses focus, and capture must keep working while it is gone.
fn record_clipboard_change(
    history: &HistoryManager,
    settings: &Settings,
    copy: Option<clipboard::Copy>,
) {
    let Some(copy) = copy else {
        // A file list or a colour: nothing history can show yet, and writing a
        // placeholder would be worse than staying quiet.
        eprintln!("[clip] change observed: unsupported payload");
        return;
    };

    let (content_type, bytes, title) = match &copy {
        clipboard::Copy::Text(text) => (
            "public.utf8-plain-text",
            text.clone().into_bytes(),
            text.chars().take(120).collect::<String>(),
        ),
        // An image has no text to show: the row draws a thumbnail placeholder
        // and the preview says how large it is.
        clipboard::Copy::Image(png) => ("public.png", png.clone(), String::new()),
        clipboard::Copy::File(files) => (
            "public.file-url",
            files.clone().into_bytes(),
            rows::file_title(files),
        ),
    };
    eprintln!(
        "[clip] change observed: {content_type} {} bytes",
        bytes.len()
    );

    if !store_allowed(&copy, settings) {
        eprintln!("[clip] storage disabled for this entry");
        return;
    }

    let application = source_application();
    let text_for_rules = match &copy {
        clipboard::Copy::Text(text) => text.as_str(),
        clipboard::Copy::File(files) => files.as_str(),
        clipboard::Copy::Image(_) => "",
    };
    if ignore::should_ignore(settings, application.as_deref(), text_for_rules) {
        eprintln!("[clip] ignored by rule");
        return;
    }

    let now = Utc::now().timestamp_millis();
    let item = ClipboardItem {
        id: uuid::Uuid::new_v4().to_string(),
        application,
        first_copied_at: now,
        last_copied_at: now,
        number_of_copies: 1,
        pin: None,
        title,
        contents: vec![ClipboardContent {
            content_type: content_type.to_string(),
            value: Some(bytes.clone()),
        }],
        sync_timestamp: now,
        sync_source: None,
        sync_deleted: false,
    };

    let _ = history.add(
        item,
        settings.history_size as i32,
        settings.unlimited_history,
    );

    // Tell the sync engine what is on the clipboard now, so paired peers receive
    // it. Without this the local copy stays local.
    let broadcast = ();
    history.sync_observe_local_clipboard(Some(ClipboardContent {
        content_type: content_type.to_string(),
        value: Some(bytes),
    }));
    eprintln!("[clip] broadcast to peers: {broadcast:?}");
}

/// Starts clipboard capture for the lifetime of the process.
///
/// The settings are re-read for every copy instead of captured once: the save
/// switches, the ignore rules and the history limit are all meant to apply to the
/// next thing you copy, not to the next launch. The file is small, and a copy
/// happens at human speed.
fn start_clipboard_capture(cx: &mut App, history: Arc<HistoryManager>, database: String) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    clipboard::spawn(tx);
    cx.spawn(async move |_cx| {
        while let Some(copy) = rx.next().await {
            let settings = Settings::load(&database);
            record_clipboard_change(&history, &settings, copy);
        }
    })
    .detach();
}

#[cfg(test)]
mod import_tests {
    // `super::*` would bring in gpui's own `test` attribute and shadow the built-in
    // one, so only the two types these tests need are imported.
    use manis_pocket_core::history::HistoryManager;

    #[test]
    fn the_save_switches_gate_the_right_kinds() {
        use crate::clipboard::Copy;
        use crate::settings::Settings;

        let settings = Settings {
            save_text: false,
            save_images: true,
            ..Settings::default()
        };
        assert!(!crate::store_allowed(&Copy::Text("x".into()), &settings));
        assert!(!crate::store_allowed(
            &Copy::File("file:///a".into()),
            &settings
        ));
        assert!(crate::store_allowed(&Copy::Image(vec![1, 2, 3]), &settings));

        let settings = Settings {
            save_text: true,
            save_images: false,
            ..Settings::default()
        };
        assert!(crate::store_allowed(&Copy::Text("x".into()), &settings));
        assert!(!crate::store_allowed(
            &Copy::Image(vec![1, 2, 3]),
            &settings
        ));
    }

    #[test]
    fn a_first_run_raises_the_limit_but_keeps_one_that_fits() {
        // A history bigger than the limit would be trimmed by the default.
        assert_eq!(crate::first_run_limit(500, 1004, false), 1004);
        // A limit that already fits is left alone, including one the user picked.
        assert_eq!(crate::first_run_limit(2000, 1004, false), 2000);
        assert_eq!(crate::first_run_limit(100, 100, false), 100);
        // Unlimited means the limit is irrelevant.
        assert_eq!(crate::first_run_limit(500, 1004, true), 500);
    }

    #[test]
    fn a_missing_legacy_store_imports_nothing() {
        let history = HistoryManager::new(":memory:".to_string()).expect("open");
        let imported = history
            .migrate_from_swiftdata("/nonexistent/default.store".to_string())
            .expect("a missing store is not an error");
        assert_eq!(imported, 0);
    }

    #[test]
    fn a_store_that_is_not_sqlite_reports_an_error() {
        let directory = std::env::temp_dir().join(format!("manis-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("broken.store");
        std::fs::write(&path, b"this is not a database").expect("write");
        let history = HistoryManager::new(":memory:".to_string()).expect("open");
        let result = history.migrate_from_swiftdata(path.display().to_string());
        assert!(result.is_err(), "a broken store must be reported");
        let _ = std::fs::remove_dir_all(&directory);
    }
}
