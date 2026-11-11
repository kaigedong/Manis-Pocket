//! Manis Pocket — GPUI desktop app.
//!
//! One Rust UI for macOS and Linux, replacing the SwiftUI and GTK front ends.
//! The clipboard engine (`manis-pocket-core`) is used directly as a library;
//! there is no FFI boundary any more.

mod clipboard;
mod icons;
mod ignore;
#[cfg(target_os = "linux")]
mod apps_linux;
mod ipc;
mod rows;
mod settings;
mod settings_ui;
mod sync_runtime;
mod tray;

use chrono::Utc;
use futures::StreamExt;
use gpui::{
    TitlebarOptions, div, point, px, size, App, Bounds, Context, Entity, IntoElement, Point, SharedString,
    Size,
    Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};
use gpui_kit::base::{IndexPath, StyledExt, h_flex, v_flex};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::*;
use gpui_kit::img;
use manis_pocket_core::history::HistoryManager;
use manis_pocket_core::model::{ClipboardContent, ClipboardItem, SearchMode};
use settings::Settings;
use std::sync::{Arc, OnceLock};

/// One history manager for the whole process: the panel and the settings window
/// both read it, and pairing commands act on it while no window is open.
static HISTORY: OnceLock<Arc<HistoryManager>> = OnceLock::new();

fn history() -> Arc<HistoryManager> {
    HISTORY.get().expect("history is initialised at startup").clone()
}

/// Row metrics from the macOS build (`Popup.itemHeight`, `Popup.cornerRadius`).
const ROW_MIN_HEIGHT: f32 = 22.0;
const ROW_CORNER: f32 = 4.0;
const ICON_SIZE: f32 = 15.0;
/// Selection is the system accent at 80% (`ListItemView.swift:115`); GPUI has no
/// system-accent API, so this is the default macOS blue.
const SELECTION_ACCENT: u32 = 0x0A84FF;
/// Measured chrome around the list: the search row and the footer.
const HEADER_HEIGHT: f32 = 44.0;
const FOOTER_HEIGHT: f32 = 122.0;

/// Character budget standing in for `truncationMode(.middle)`.
const TITLE_BUDGET: usize = 34;
/// Row width inside the card: panel width minus the list's horizontal padding.
const ROW_WIDTH: f32 = 420.0;
/// Space the title may use before the shortcut hint starts.
const ROW_TITLE_WIDTH: f32 = 350.0;

/// Transparent margin around the card, so its shadow and rounded corners have
/// somewhere to draw. The window is larger than the card by twice this.
const PANEL_MARGIN: f32 = 8.0;

/// Virtualised history list. Holds metadata only; payloads are fetched from the
/// core when an entry is copied or previewed.
struct HistoryList {
    /// Arranged: pinned entries first, then history.
    items: Vec<ClipboardItem>,
    /// How many leading entries belong to the pinned section.
    pinned_count: usize,
    selected: Option<IndexPath>,
    history: Arc<HistoryManager>,
    show_application_icons: bool,
    /// Mirrors `pasteByDefault`: pressing enter also pastes into the front app.
    paste_by_default: bool,
    /// Mirrors `showSpecialSymbols`: render whitespace markers in titles.
    show_special_symbols: bool,
    /// Whether matches use a background colour instead of bold text.
    highlight_background: bool,
    /// Match ranges per item id, straight from the core's search results.
    highlights: std::collections::HashMap<String, Vec<(usize, usize)>>,
    /// Row under the pointer: only that row reveals its action buttons.
    hovered: Option<String>,
    /// The panel that owns this list, so a row action can refresh it in place.
    panel: Option<gpui::WeakEntity<HistoryPanel>>,
}

impl HistoryList {
    fn replace_items(&mut self, items: Vec<ClipboardItem>, pins_position: &str, sort_by: &str) {
        let arranged = rows::arrange(items, pins_position, sort_by);
        self.pinned_count = arranged.pinned_count;
        self.items = arranged.items;
    }

    /// A single flat section: the list reserves header space for every section,
    /// which left a large gap under the pinned block. Pinned rows are already
    /// marked by their pin chip.
    fn sections(&self) -> Vec<(usize, usize)> {
        vec![(0, self.items.len())]
    }

    fn item_at(&self, ix: IndexPath) -> Option<&ClipboardItem> {
        let (start, len) = *self.sections().get(ix.section)?;
        if ix.row >= len {
            return None;
        }
        self.items.get(start + ix.row)
    }
}

impl ListDelegate for HistoryList {
    type Item = ListItem;

    fn sections_count(&self, _cx: &App) -> usize {
        self.sections().len()
    }

    fn items_count(&self, section: usize, _cx: &App) -> usize {
        self.sections().get(section).map(|(_, len)| *len).unwrap_or(0)
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let item = self.item_at(ix)?;
        let selected = self.selected == Some(ix);
        let title = rows::truncate_middle(&rows::row_title(item), TITLE_BUDGET);
        let foreground = if selected {
            gpui::white()
        } else {
            gpui::black()
        };

        let mut row = h_flex()
            .w(px(ROW_WIDTH))
            .min_h(px(ROW_MIN_HEIGHT))
            .text_size(px(13.0))
            .text_color(foreground);

        // Application icon (15×15) then a 5pt gap, or a 10pt gap without it.
        if self.show_application_icons {
            if let Some(bundle) = item.application.as_deref() {
                // The real bundle icon when AppKit can supply one, otherwise a
                // letter badge so the row still says where it came from.
                let glyph = match icons::icon_for_bundle(bundle) {
                    Some(render) => img(render)
                        .size(px(ICON_SIZE))
                        .rounded(px(3.0))
                        .into_any_element(),
                    None => {
                        let (initial, (red, green, blue)) = rows::app_badge(bundle);
                        div()
                            .size(px(ICON_SIZE))
                            .rounded(px(3.0))
                            .bg(gpui::rgb(
                                (u32::from(red) << 16)
                                    | (u32::from(green) << 8)
                                    | u32::from(blue),
                            ))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(9.0))
                            .text_color(gpui::white())
                            .child(initial)
                            .into_any_element()
                    }
                };
                row = row
                    .child(div().pl_1().child(glyph))
                    .child(div().w(px(5.0)));
            } else {
                row = row.child(div().w(px(10.0)));
            }
        } else {
            row = row.child(div().w(px(10.0)));
        }

        // Pin letter at the start of the row: the list item is content-sized, so a
        // trailing hint would overlap the title instead of sitting at the edge.
        if let Some(pin) = item.pin.clone() {
            row = row
                .child(
                    div()
                        .px_1()
                        .rounded(px(3.0))
                        .bg(if selected {
                            gpui::rgb(0xFF_FFFF).opacity(0.22)
                        } else {
                            gpui::rgb(0x0A84FF).opacity(0.14)
                        })
                        .text_size(px(10.0))
                        .text_color(if selected {
                            gpui::rgb(0xFF_FFFF)
                        } else {
                            gpui::rgb(0x0A84FF)
                        })
                        .child(SharedString::from(pin)),
                )
                .child(div().w(px(5.0)));
        }

        // A colour swatch before the title when the copied text is a colour.
        if let Some((red, green, blue)) = rows::hex_color(&title) {
            row = row
                .child(
                    div()
                        .size(px(12.0))
                        .rounded(px(2.0))
                        .bg(gpui::rgb(
                            (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue),
                        )),
                )
                .child(div().w(px(5.0)));
        }

        // Highlights only apply to titles that were not shortened, since the
        // ranges index the original string.
        let ranges = self
            .highlights
            .get(&item.id)
            .filter(|_| rows::row_title(item).chars().count() <= TITLE_BUDGET);
        let spans = ranges
            .map(|ranges| rows::highlight_spans(&title, ranges))
            .unwrap_or_else(|| vec![(title.clone(), false)]);
        let highlight_background = self.highlight_background;

        row = row.child(
            h_flex()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .children(spans.into_iter().map(|(text, matched)| {
                    let mut span = div().child(text);
                    if matched {
                        span = if highlight_background {
                            span.bg(gpui::rgb(0xFFF1_76))
                        } else {
                            span.font_weight(gpui::FontWeight::BOLD)
                        };
                    }
                    span
                })),
        );

        let click_id = item.id.clone();
        let click_history = self.history.clone();
        let hover_id = item.id.clone();

        Some(
            ListItem::new(("row", ix.row))
                .rounded(px(ROW_CORNER))
                .bg(gpui::rgb(SELECTION_ACCENT).opacity(if selected { 0.8 } else { 0.0 }))
                .child(div().w_full().child(row))
                .on_hover(cx.listener(
                    move |state: &mut ListState<HistoryList>,
                          hovered: &bool,
                          _window: &mut Window,
                          cx: &mut Context<ListState<HistoryList>>| {
                        let target = hovered.then(|| hover_id.clone());
                        let delegate = state.delegate_mut();
                        if delegate.hovered != target {
                            delegate.hovered = target;
                            cx.notify();
                        }
                    },
                ))
                .on_click(move |_, window, _cx| {
                    if copy_entry_to_clipboard(&click_history, &click_id) {
                        window.remove_window();
                    }
                }),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected = ix;
        // Decoding belongs here, not in `render`: doing it while painting kept
        // the window from ever appearing.
        if let Some(panel) = self.panel.as_ref().and_then(|panel| panel.upgrade()) {
            panel.update(cx, |panel, cx| {
                panel.load_preview_image(cx);
                cx.notify();
            });
        }
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        let Some(ix) = self.selected else {
            return;
        };
        // Rows are section-relative: the pinned block shifts every index.
        let Some(item) = self.item_at(ix) else {
            return;
        };
        let id = item.id.clone();
        let paste = self.paste_by_default;
        if copy_entry_to_clipboard(&self.history, &id) {
            // The panel's job is done once the entry is on the clipboard.
            _window.remove_window();
            if paste {
                // Focus has to return to the previous application before the
                // shortcut is delivered, or the paste lands in a dying window.
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    if !crate::clipboard::paste_into_frontmost() {
                        eprintln!("[clip] auto-paste failed (missing accessibility permission?)");
                    }
                });
            }
        }
    }
}

struct HistoryPanel {
    history: Arc<HistoryManager>,
    /// Every entry's metadata, used as the search corpus.
    all_items: Vec<ClipboardItem>,
    input: Entity<InputState>,
    list: Entity<ListState<HistoryList>>,
    settings: settings::Settings,
    /// The preview sidebar, toggled with ⌃Space like the macOS build.
    preview_open: bool,
    /// Decoded preview images, keyed by entry id: decoding a PNG on every frame
    /// would make the panel stutter on large screenshots.
    preview_images: std::collections::HashMap<String, Option<(std::sync::Arc<gpui_kit::RenderImage>, f32, f32)>>,
}

impl HistoryPanel {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let database = storage_path();
        let history = history();
        let mut settings = settings::Settings::load(&database);
        // Metadata only: payloads are fetched on demand so startup does not
        // depend on how much content the history holds.
        let all_items = history.load_metadata().unwrap_or_default();

        // A default smaller than what is already stored would delete entries the
        // first time something new is copied, so never load below the real size.
        let stored = all_items.len() as i64;
        if !settings.unlimited_history && settings.history_size < stored {
            settings.history_size = stored;
            settings.save(&database);
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(SharedString::from("搜索"))
        });
        let pins_position = settings.pins_position.clone();
        let arranged = rows::arrange(all_items.clone(), &pins_position, &settings.sort_by);
        let panel_handle = cx.entity().downgrade();
        let list = cx.new(|cx| {
            ListState::new(
                HistoryList {
                    items: arranged.items,
                    pinned_count: arranged.pinned_count,
                    // The macOS panel highlights the newest entry on open.
                    selected: Some(IndexPath {
                        section: 0,
                        row: 0,
                        column: 0,
                    }),
                    history: history.clone(),
                    show_application_icons: settings.show_application_icons,
                    paste_by_default: settings.paste_by_default,
                    show_special_symbols: settings.show_special_symbols,
                    highlight_background: settings.highlight_match != "bold",
                    highlights: std::collections::HashMap::new(),
                    hovered: None,
                    panel: Some(panel_handle),
                },
                window,
                cx,
            )
        });

        // The macOS panel disappears as soon as it stops being the active
        // window. Guard on a first activation so startup cannot dismiss it.
        let mut seen_active = false;
        cx.observe_window_activation(window, move |_this, window, _cx| {
            if window.is_window_active() {
                seen_active = true;
            } else if seen_active {
                window.remove_window();
            }
        })
        .detach();

        // The search box is the panel's primary target: without focus no input
        // handler is registered, and macOS routes keystrokes straight into the
        // buffer instead of letting the IME compose.
        let focus_handle = input.read(cx).focus_handle(cx);
        window.focus(&focus_handle, cx);

        // Sync runs only when the user enabled it; the observer lives in the
        // core's threads and talks to us through a channel.
        if settings.sync_enabled {
            let (sync_tx, mut sync_rx) = futures::channel::mpsc::unbounded();
            sync_runtime::start(history.clone(), &settings, sync_tx);
            cx.spawn(async move |this, cx| {
                while let Some(notice) = sync_rx.next().await {
                    if this
                        .update(cx, |this, cx| this.on_sync_notice(notice, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }

        cx.subscribe(&input, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh(cx);
            }
        })
        .detach();

        Self {
            history,
            all_items,
            input,
            list,
            settings,
            preview_open: std::env::var("MANIS_POCKET_PREVIEW").is_ok(),
            preview_images: std::collections::HashMap::new(),
        }
    }


    /// A peer changed the history (or the network state): reload and redraw.
    fn on_sync_notice(&mut self, notice: sync_runtime::SyncNotice, cx: &mut Context<Self>) {
        match notice {
            sync_runtime::SyncNotice::HistoryChanged => {
                self.all_items = self.history.load_metadata().unwrap_or_default();
                self.refresh(cx);
            }
            sync_runtime::SyncNotice::Listening(address) => {
                eprintln!("sync listening on {address}");
            }
            sync_runtime::SyncNotice::Error(message) => {
                eprintln!("sync error: {message}");
            }
            // Peer and pairing events are surfaced in the settings window.
            _ => {}
        }
    }

    /// The entry the list currently has selected.
    fn selected_item(&self, cx: &App) -> Option<ClipboardItem> {
        let list = self.list.read(cx);
        let delegate = list.delegate();
        delegate.item_at(delegate.selected?).cloned()
    }

    /// Pins or unpins the selected entry. Pins take the next free letter, which
    /// is what the row shows on its right.
    fn toggle_pin_selected(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item(cx) else {
            return;
        };
        let used: Vec<String> = self
            .all_items
            .iter()
            .filter_map(|entry| entry.pin.clone())
            .collect();
        let pins = rows::available_pins(&used);
        let _ = self.history.toggle_pin(item.id.clone(), pins);
        self.reload(cx);
    }

    /// Deletes the selected entry and tells paired devices, like the macOS build.
    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item(cx) else {
            return;
        };
        if self.history.delete(item.id.clone()).is_ok() {
            self.history.sync_broadcast_deletion(item.id.clone());
        }
        self.reload(cx);
    }

    /// Opens or closes the preview sidebar, widening the window to match.
    fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_open = !self.preview_open;
        if self.preview_open {
            self.load_preview_image(cx);
        }
        // The next render resizes to fit the new width along with the content.
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.all_items = self.history.load_metadata().unwrap_or_default();
        self.refresh(cx);
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        // The image cache is filled from the current selection, so refresh it
        // whenever the list content changes.
        let query = self.input.read(cx).value().to_string();
        let results = self
            .history
            .search(&query, self.all_items.clone(), search_mode(&self.settings.search_mode));

        // Search results carry the matched ranges, which the rows draw as
        // highlights; keep them keyed by entry so the list can look them up.
        let mut highlights = std::collections::HashMap::new();
        let items: Vec<ClipboardItem> = results
            .into_iter()
            .map(|result| {
                if !result.ranges.is_empty() {
                    highlights.insert(
                        result.item.id.clone(),
                        result
                            .ranges
                            .iter()
                            .map(|range| (range.start.max(0) as usize, range.end.max(0) as usize))
                            .collect(),
                    );
                }
                result.item
            })
            .collect();

        let pins_position = self.settings.pins_position.clone();
        let sort_by = self.settings.sort_by.clone();
        self.list.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            delegate.highlights = highlights;
            delegate.replace_items(items, &pins_position, &sort_by);
            delegate.selected = Some(IndexPath {
                section: 0,
                row: 0,
                column: 0,
            });
            cx.notify();
        });
    }
}

impl HistoryPanel {
    /// The right-hand preview sidebar: full text plus where it came from.
    /// The selected entry's picture, if it has one.
    fn preview_image(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let item = self.selected_item(cx)?;
        if !matches!(rows::row_kind(&item), rows::RowKind::Image) {
            return None;
        }
        let (render, w, h) = self.preview_images.get(&item.id)?.clone()?;
        let max_width = (self.settings.preview_width as f32 - 24.0).max(80.0);
        let scale = (max_width / w).min(1.0);
        Some(
            img(render)
                .w(px(w * scale))
                .h(px(h * scale))
                .rounded(px(4.0))
                .into_any_element(),
        )
    }

    /// Decodes the selected entry's picture once and keeps it for repaints.
    fn load_preview_image(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item(cx).or_else(|| {
            self.list
                .read(cx)
                .delegate()
                .item_at(gpui_kit::base::IndexPath {
                    section: 0,
                    row: 0,
                    column: 0,
                })
                .cloned()
        }) else {
            return;
        };
        if !matches!(rows::row_kind(&item), rows::RowKind::Image) {
            return;
        }
        if self.preview_images.contains_key(&item.id) {
            return;
        }
        // Keep the cache from growing without bound while scrolling a long list.
        if self.preview_images.len() > 32 {
            self.preview_images.clear();
        }
        let decoded = self
            .history
            .contents(item.id.clone())
            .ok()
            .and_then(|contents| {
                contents
                    .into_iter()
                    .find(|content| content.content_type == "public.png")
            })
            .and_then(|content| content.value)
            .and_then(|bytes| icons::png_to_render_image(&bytes, 900));
        self.preview_images.insert(item.id.clone(), decoded);
    }

    fn render_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let width = self.settings.preview_width as f32;
        let selected = self.selected_item(cx);
        let now = Utc::now().timestamp_millis();
        let lines = selected
            .as_ref()
            .map(|item| rows::preview_lines(item, now, 600))
            .unwrap_or_default();

        let image = self.preview_image(cx);
        v_flex()
            .w(px(width))
            .h_full()
            .gap_2()
            .px_3()
            .py_2()
            .border_l_1()
            .border_color(gpui::black().opacity(0.08))
            .children(image)
            .children(lines.is_empty().then(|| {
                div()
                    .text_sm()
                    .text_color(gpui::black().opacity(0.4))
                    .child("选中一条以预览")
                    .into_any_element()
            }))
            .children(
                lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, line)| {
                        let mut row = div().child(line);
                        // The first line is the content, the second its source.
                        if index == 0 {
                            row = row.text_sm();
                        } else {
                            row = row.text_xs().text_color(gpui::black().opacity(0.5));
                        }
                        row.into_any_element()
                    })
                    .collect::<Vec<_>>(),
            )
            .into_any_element()
    }
}

impl HistoryPanel {
    /// The action footer: the same five rows the macOS build puts under the list.
    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let row = |id: &'static str, label: &'static str, shortcut: &'static str| {
            h_flex()
                .id(id)
                .w_full()
                .min_h(px(ROW_MIN_HEIGHT))
                .px_2()
                .justify_between()
                .text_size(px(13.0))
                .rounded(px(ROW_CORNER))
                .hover(|style| style.bg(gpui::black().opacity(0.06)))
                .cursor_pointer()
                .child(div().child(label))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(gpui::black().opacity(0.4))
                        .child(shortcut),
                )
        };

        v_flex()
            .w_full()
            .child(
                div()
                    .mx_1p5()
                    .mb_1p5()
                    .h(px(1.0))
                    .bg(gpui::black().opacity(0.08)),
            )
            .child(
                row("footer-clear", "清除", "⌥⌘Del").on_click(cx.listener(|this, _, _, cx| {
                    let _ = this.history.clear_unpinned();
                    this.reload(cx);
                })),
            )
            .child(
                row("footer-clear-all", "全部清除", "⇧⌥⌘Del")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let _ = this.history.clear_all();
                        this.reload(cx);
                    })),
            )
            .child(
                row("footer-preferences", "偏好设置…", "⌘,")
                    .on_click(cx.listener(|_, _, window, cx| {
                        let _ = window;
                        open_settings(cx);
                    })),
            )
            .child(
                row("footer-about", "关于", "").on_click(cx.listener(|_, _, _, _cx| {
                    #[cfg(target_os = "macos")]
                    icons::show_about_panel();
                })),
            )
            .child(
                row("footer-quit", "退出", "⌘Q").on_click(cx.listener(|_, _, _, cx| cx.quit())),
            )
            .into_any_element()
    }
}

impl HistoryPanel {
    /// Panel size for the current content, following the macOS rule: the window
    /// grows with the list but never past the configured height.
    fn desired_size(&self) -> gpui::Size<gpui::Pixels> {
        let rows = self.visible_rows() as f32;
        let footer = if self.settings.show_footer {
            FOOTER_HEIGHT
        } else {
            0.0
        };
        let content = rows * ROW_MIN_HEIGHT + HEADER_HEIGHT + footer;
        let min_height = HEADER_HEIGHT + 5.0;
        let max_height = self.settings.window_height.max(min_height as f64) as f32;
        let height = content.clamp(min_height, max_height);
        let width = if self.preview_open {
            self.settings.window_width + self.settings.preview_width
        } else {
            self.settings.window_width
        };
        size(
            px(width as f32 + PANEL_MARGIN * 2.0),
            px(height + PANEL_MARGIN * 2.0),
        )
    }

    fn visible_rows(&self) -> usize {
        self.all_items.len().min(self.settings.window_height.max(1.0) as usize)
    }
}

impl Render for HistoryPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The panel hugs its content: an empty history is a small card, a full
        // one grows up to the configured height.
        let desired = self.desired_size();
        let current = window.viewport_size();
        if std::env::var("MANIS_POCKET_SIZE_LOG").is_ok() {
            eprintln!(
                "[size] current={:.0}x{:.0} desired={:.0}x{:.0} preview={} rows={}",
                current.width.as_f32(),
                current.height.as_f32(),
                desired.width.as_f32(),
                desired.height.as_f32(),
                self.preview_open,
                self.visible_rows()
            );
        }
        if (current.height.as_f32() - desired.height.as_f32()).abs() > 0.5
            || (current.width.as_f32() - desired.width.as_f32()).abs() > 0.5
        {
            window.resize(desired);
        }


        let count = self.list.read(cx).delegate().items.len();

        div()
            .flex()
            .size_full()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let modifiers = event.keystroke.modifiers;
                if std::env::var("MANIS_POCKET_KEY_LOG").is_ok() {
                    let focused = this.input.read(cx).focus_handle(cx).is_focused(window);
                    eprintln!(
                        "[key] key={key:?} char={:?} alt={} ctrl={} shift={} cmd={} input_focused={focused}",
                        event.keystroke.key_char,
                        modifiers.alt,
                        modifiers.control,
                        modifiers.shift,
                        modifiers.platform,
                    );
                }
                if key == "escape" {
                    window.remove_window();
                } else if modifiers.alt && key == "p" {
                    // ⌥P toggles the pin (`KeyboardShortcuts_pin` = option + P).
                    this.toggle_pin_selected(cx);
                } else if modifiers.control && key == "space" {
                    // ⌃Space toggles the preview (`KeyboardShortcuts_togglePreview`).
                    this.toggle_preview(cx);
                } else if key == "backspace" && modifiers.platform && modifiers.alt {
                    // ⌥⌘⌫ clears unpinned, ⇧⌥⌘⌫ clears everything.
                    if modifiers.shift {
                        let _ = this.history.clear_all();
                    } else {
                        let _ = this.history.clear_unpinned();
                    }
                    this.reload(cx);
                } else if modifiers.platform && key == "," {
                    open_settings(cx);
                } else if modifiers.platform && key == "q" {
                    cx.quit();
                } else if modifiers.alt && (key == "backspace" || key == "delete") {
                    // ⌥⌫ deletes (`KeyboardShortcuts_delete` = option + delete).
                    this.delete_selected(cx);
                }
            }))
            // Room for the card's shadow; the window itself is transparent so
            // the rounded corners actually show the desktop behind them.
            .p_2()
            .child(
                div()
                    .v_flex()
                    .size_full()
                    .corner_radii(gpui::Corners::all(px(9.0)))
                    .bg(gpui::white())
                    .shadow_lg()
                    .overflow_hidden()
                    .text_color(gpui::black())
                    // The preview is a *right-hand column*, so the card splits
                    // horizontally: the list column keeps the search box, the
                    // rows, the footer and the status line.
                    .child(
                        h_flex()
                            .size_full()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .h_full()
                                    .children(
                                        (self.settings.show_search || self.settings.show_title)
                                            .then(|| {
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .px_3()
                                                    .pt_2p5()
                                                    .pb_2()
                                                    .children(self.settings.show_title.then(|| {
                                                        div()
                                                            .text_sm()
                                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                                            .flex_shrink_0()
                                                            .child(SharedString::from("Manis Pocket"))
                                                    }))
                                                    .children(self.settings.show_search.then(|| {
                                                        div()
                                                            .flex_1()
                                                            .rounded(px(6.0))
                                                            .bg(gpui::rgb(0xF2F2F4))
                                                            .px_2()
                                                            .py_0p5()
                                                            .child(
                                                                Input::new(&self.input)
                                                                    .appearance(false),
                                                            )
                                                    }))
                                                    .into_any_element()
                                            }),
                                    )
                                    .child(div().flex_1().px_2().child(List::new(&self.list)))
                                    .children(
                                        self.settings.show_footer.then(|| self.render_footer(cx)),
                                    )
                                    .children(self.settings.show_footer.then(|| {
                                        div()
                                            .px_3()
                                            .py_2()
                                            .border_t_1()
                                            .border_color(gpui::black().opacity(0.08))
                                            .text_xs()
                                            .text_color(gpui::black().opacity(0.45))
                                            .child(SharedString::from(format!(
                                                "{count} 条 · 上下选择 · 回车复制 · ⌘1-9 快速取用 · esc 关闭"
                                            )))
                                            .into_any_element()
                                    })),
                            )
                            .children(self.preview_open.then(|| self.render_preview(cx))),
                    ),
            )
    }
}


/// Maps the `searchMode` setting onto the core's search mode.
fn search_mode(value: &str) -> SearchMode {
    match value {
        "fuzzy" => SearchMode::Fuzzy,
        "regexp" => SearchMode::Regexp,
        "mixed" => SearchMode::Mixed,
        _ => SearchMode::Exact,
    }
}

/// Writes an entry's payload back to the system clipboard.
///
/// Text first: it is what almost every entry carries. Image copy needs a PNG
/// decode and arrives with the preview work.
fn copy_entry_to_clipboard(history: &HistoryManager, id: &str) -> bool {
    let Ok(contents) = history.contents(id.to_string()) else {
        return false;
    };
    // File lists go back as file URLs; without this branch a copied file would
    // come back as the plain path text.
    if let Some(files) = contents
        .iter()
        .find(|content| content.content_type == "public.file-url")
    {
        return crate::clipboard::apply_remote(files);
    }

    // Images come back as PNG; text is the common case and is tried first.
    if let Some(png) = contents
        .iter()
        .find(|content| content.content_type == "public.png")
        .and_then(|content| content.value.as_ref())
    {
        return icons::write_png_to_pasteboard(png);
    }

    let Some(text) = contents
        .iter()
        .find(|content| {
            matches!(
                content.content_type.as_str(),
                "public.utf8-plain-text" | "text/plain" | "text/plain;charset=utf-8"
            )
        })
        .and_then(|content| content.value.as_ref())
        .and_then(|value| String::from_utf8(value.clone()).ok())
    else {
        return false;
    };
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .is_ok()
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
fn panel_bounds(cx: &App, panel: Size<gpui::Pixels>, settings: &Settings) -> Bounds<gpui::Pixels> {
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

    let origin = match settings.popup_position.as_str() {
        // Centred on the screen, like the macOS `.center` mode.
        "center" => point(
            screen.origin.x + (screen.size.width - panel.width) / 2.0,
            screen.origin.y + (screen.size.height - panel.height) / 2.0,
        ),
        // At the pointer: the panel's top-left corner follows the cursor
        // (macOS `.cursor` default). The pointer can be on another display, and
        // the Y flip for the main screen does not apply there, so a point outside
        // this display falls back to the corner instead of going off-screen.
        "cursor" => match mouse_location(&screen) {
            Some(mouse) if within(&screen, mouse) => {
                clamp(mouse.x.as_f32(), mouse.y.as_f32())
            }
            _ => default_origin(&screen, panel),
        },
        // `statusItem`, `window` and `lastPosition` need the menu-bar button or
        // the frontmost window rectangle, which are not wired yet; the top-right
        // anchor is their closest honest approximation.
        _ => default_origin(&screen, panel),
    };
    let _ = margin;
    Bounds::new(origin, panel)
}

/// Whether a point lies inside a display's bounds.
fn within(screen: &Bounds<gpui::Pixels>, point: Point<gpui::Pixels>) -> bool {
    point.x >= screen.origin.x
        && point.y >= screen.origin.y
        && point.x <= screen.origin.x + screen.size.width
        && point.y <= screen.origin.y + screen.size.height
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

/// Current pointer position in window coordinates (origin top-left).
///
/// macOS reports the mouse with a bottom-left origin, Hyprland with a top-left
/// one; both are normalised here so the caller only sees screen space.
#[cfg(target_os = "macos")]
fn mouse_location(screen: &Bounds<gpui::Pixels>) -> Option<Point<gpui::Pixels>> {
    let location = unsafe { objc2_app_kit::NSEvent::mouseLocation() };
    let x = location.x as f32;
    let y_from_bottom = location.y as f32;
    // Flip into the top-left origin the rest of the layout uses, relative to the
    // display (multi-monitor safe: the display origin is subtracted first).
    let screen_bottom = screen.origin.y.as_f32() + screen.size.height.as_f32();
    let y = screen_bottom - y_from_bottom;
    Some(point(px(x), px(y)))
}

#[cfg(target_os = "linux")]
fn mouse_location(_screen: &Bounds<gpui::Pixels>) -> Option<Point<gpui::Pixels>> {
    let output = std::process::Command::new("hyprctl")
        .arg("cursorpos")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (x, y) = text.trim().split_once(',')?;
    Some(point(
        px(x.trim().parse::<f32>().ok()?),
        px(y.trim().parse::<f32>().ok()?),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn mouse_location(_screen: &Bounds<gpui::Pixels>) -> Option<Point<gpui::Pixels>> {
    None
}

/// Reports whether the command reached a running instance.
fn exit_after(delivered: bool) -> () {
    if !delivered {
        eprintln!("no running Manis Pocket instance to command");
        std::process::exit(1);
    }
}

/// Opens the settings window.
fn open_settings(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(720.0), px(520.0)), cx);
    let database = storage_path();
    gpui_kit::open_window(
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
    )
    .expect("open settings window");
}

/// Opens the history panel. Called at launch and by the global shortcut.
fn open_panel(cx: &mut App) {
    let settings = settings::Settings::load(&storage_path());
    let panel = size(
        px(settings.window_width as f32 + PANEL_MARGIN * 2.0),
        px(settings.window_height as f32 + PANEL_MARGIN * 2.0),
    );
    let bounds = panel_bounds(cx, panel, &settings);
    gpui_kit::open_window(
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
                handle.update(cx, |panel, cx| panel.load_preview_image(cx));
            });
            panel
        },
    )
    .expect("open history panel");
}

/// Opens the panel unless it is already on screen.
fn open_panel_if_needed(cx: &mut App) {
    if cx.windows().is_empty() {
        open_panel(cx);
    }
}

/// Shows the panel if it is closed, closes it if it is open.
fn toggle_panel(cx: &mut App) {
    let windows = cx.windows();
    if windows.is_empty() {
        open_panel(cx);
        return;
    }
    for handle in windows {
        let _ = handle.update(cx, |_view, window, _cx| window.remove_window());
    }
}

/// The SwiftUI build opens the panel with ⇧⌘C (modifiers 768 + key code 8).
fn register_popup_hotkey() {
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::GlobalHotKeyManager;

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
        if arg == "--database" {
            if let Some(path) = args.next() {
                let _ = DATABASE_OVERRIDE.set(path);
            }
        }
    }

    // `--copy <id>` puts one entry back on the clipboard: useful for scripting,
    // and it exercises the same path the panel uses.
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--copy" {
            if let Some(id) = args.next() {
                let ok = HistoryManager::new(storage_path())
                    .map(|history| copy_entry_to_clipboard(&history, &id))
                    .unwrap_or(false);
                println!("{ok}");
                return;
            }
        }
    }

    // Headless pairing helpers: `manis-pocket --pair <peer>`,
    // `--accept <peer> <pin>`, `--connect <multiaddr>`.
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|arg| arg == name);
    if let Some(index) = flag("--pair") {
        if let Some(peer_id) = args.get(index + 1) {
            return exit_after(ipc::send(&storage_path(), &format!("pair {peer_id}")));
        }
    }
    if let Some(index) = flag("--accept") {
        if let (Some(peer_id), Some(pin)) = (args.get(index + 1), args.get(index + 2)) {
            return exit_after(ipc::send(&storage_path(), &format!("accept {peer_id} {pin}")));
        }
    }
    if let Some(index) = flag("--connect") {
        if let Some(address) = args.get(index + 1) {
            return exit_after(ipc::send(&storage_path(), &format!("connect {address}")));
        }
    }

    let shared_history =
        Arc::new(HistoryManager::new(storage_path()).expect("open history database"));
    let _ = HISTORY.set(shared_history);

    gpui_kit::application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        register_symbol_font(cx);
        // Linux defaults to quitting when the last window closes, which would
        // end the clipboard watcher every time the panel is dismissed. A
        // resident app only quits when asked to.
        cx.set_quit_mode(gpui::QuitMode::Explicit);
        register_popup_hotkey();

        // The tray only reports intent; the window handling stays here.
        let (tray_tx, mut tray_rx) = futures::channel::mpsc::unbounded();
        // `showInStatusBar == false` means the app lives only behind its shortcut.
        if settings::Settings::load(&storage_path()).show_in_status_bar {
            tray::install(tray_tx);
        }

        // A second launch asks this instance to toggle instead of starting up.
        let (ipc_tx, mut ipc_rx) = futures::channel::mpsc::unbounded();
        ipc::listen(&storage_path(), ipc_tx);

        start_clipboard_capture(cx, history(), settings::Settings::load(&storage_path()));

        if open_settings_at_launch {
            open_settings(cx);
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

    if matches!(copy, clipboard::Copy::Image(_)) && !settings.save_images {
        eprintln!("[clip] image storage disabled");
        return;
    }
    if matches!(copy, clipboard::Copy::File(_)) && !settings.save_text {
        eprintln!("[clip] text storage disabled");
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

    let _ = history.add(item, settings.history_size as i32, settings.unlimited_history);

    // Tell the sync engine what is on the clipboard now, so paired peers receive
    // it. Without this the local copy stays local.
    let broadcast = history.sync_observe_local_clipboard(Some(ClipboardContent {
        content_type: content_type.to_string(),
        value: Some(bytes),
    }));
    eprintln!("[clip] broadcast to peers: {broadcast:?}");
}

/// Starts clipboard capture for the lifetime of the process.
fn start_clipboard_capture(cx: &mut App, history: Arc<HistoryManager>, settings: Settings) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    clipboard::spawn(tx);
    cx.spawn(async move |_cx| {
        while let Some(copy) = rx.next().await {
            record_clipboard_change(&history, &settings, copy);
        }
    })
    .detach();
}
