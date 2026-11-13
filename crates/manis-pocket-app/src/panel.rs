//! The history panel: the list delegate that draws rows and the view that owns
//! the search field, the checked stack and the preview sidebar.
//!
//! Both live here rather than in `main` because the rendering rules — row
//! geometry, which entry is highlighted, and what a hover shows — belong together.

use crate::*;

/// Row metrics from the macOS build (`Popup.itemHeight`, `Popup.cornerRadius`).
/// Content height of one row. `ListItem` adds 4 px of padding above and below;
/// the 13 px text then makes one row about 28 px tall. The SwiftUI build fits a
/// 22 px row because SwiftUI's default leading is tighter than GPUI's, and GPUI
/// exposes no line-height setter, so the pitch stays a little looser.
const ROW_MIN_HEIGHT: f32 = 20.0;
/// Thumbnail height for image rows. The list takes one measurement for every
/// row, so this has to fit inside the normal row pitch.
const ROW_THUMB_HEIGHT: f32 = 22.0;
/// What one row actually occupies, used for the window's height.
const ROW_PITCH: f32 = 30.0;
/// Height of the application filter bar.
const APP_FILTER_HEIGHT: f32 = 28.0;
/// Height of the status line under the footer.
const STATUS_HEIGHT: f32 = 30.0;
const ROW_CORNER: f32 = 4.0;
const ICON_SIZE: f32 = 15.0;
/// Selection is the system accent at 80% (`ListItemView.swift:115`); GPUI has no
/// system-accent API, so this is the default macOS blue.
const SELECTION_ACCENT: u32 = 0x0A84FF;
/// Measured chrome around the list: the search row and the footer.
const HEADER_HEIGHT: f32 = 44.0;
const FOOTER_HEIGHT: f32 = 122.0;

/// Character budget standing in for `truncationMode(.middle)`.
const TITLE_BUDGET: usize = 28;

/// Transparent margin around the card, so its shadow and rounded corners have
/// somewhere to draw. The window is larger than the card by twice this.
pub(crate) const PANEL_MARGIN: f32 = 8.0;

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
    /// Entries picked with ⌘-click, in pick order. Empty means "no multi-select".
    checked: Vec<String>,
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
        self.sections()
            .get(section)
            .map(|(_, len)| *len)
            .unwrap_or(0)
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let item = self.item_at(ix)?;
        let selected = self.selected == Some(ix);
        let full_title = rows::row_title_with_symbols(item, self.show_special_symbols);
        let title = rows::truncate_middle(&full_title, TITLE_BUDGET);
        let foreground = if selected {
            gpui::white()
        } else {
            gpui::black()
        };

        let mut row = h_flex()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(ROW_MIN_HEIGHT))
            .text_size(px(13.0))
            .text_color(foreground);

        // Application icon (15×15) then a 5pt gap, or a 10pt gap without it.
        if self.show_application_icons {
            if let Some(bundle) = item.application.as_deref() {
                // The real bundle icon when AppKit can supply one, otherwise a
                // letter badge so the row still says where it came from.
                let glyph = match icons::cached_icon(bundle) {
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
                                (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue),
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
                row = row.child(div().pl_1().child(glyph)).child(div().w(px(5.0)));
            } else {
                row = row.child(div().w(px(10.0)));
            }
        } else {
            row = row.child(div().w(px(10.0)));
        }

        // A pinned row is marked with the dot at the right; the letter that used
        // to sit here only existed for its shortcut, which is gone.

        // A colour swatch before the title when the copied text is a colour.
        if let Some((red, green, blue)) = rows::hex_color(&title) {
            row = row
                .child(div().size(px(12.0)).rounded(px(2.0)).bg(gpui::rgb(
                    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue),
                )))
                .child(div().w(px(5.0)));
        }

        // Highlights only apply to titles that were not shortened, since the
        // ranges index the original string.
        let ranges = self
            .highlights
            .get(&item.id)
            .filter(|_| full_title.chars().count() <= TITLE_BUDGET);
        let spans = ranges
            .map(|ranges| rows::highlight_spans(&title, ranges))
            .unwrap_or_else(|| vec![(title.clone(), false)]);
        let highlight_background = self.highlight_background;

        // An image entry shows its picture rather than the word "图片": the
        // SwiftUI row draws a thumbnail up to 40 high in place of the title.
        let image_thumb = match rows::row_kind(item) {
            rows::RowKind::Image => icons::cached_thumb(&item.id),
            _ => None,
        };
        row = match image_thumb {
            Some((render, width, height)) => {
                // The list measures one row and applies it to all of them, so a
                // tall thumbnail would stretch every text row with it.
                let scale = (ROW_THUMB_HEIGHT / height).min(340.0 / width).min(1.0);
                row.child(
                    img(render)
                        .w(px(width * scale))
                        .h(px(height * scale))
                        .rounded(px(3.0)),
                )
            }
            None => row.child(h_flex().flex_1().min_w(px(0.0)).overflow_hidden().children(
                spans.into_iter().map(|(text, matched)| {
                    let mut span = div().child(text);
                    if matched {
                        span = if highlight_background {
                            span.bg(gpui::rgb(0xFF_F176))
                        } else {
                            span.font_weight(gpui::FontWeight::BOLD)
                        };
                    }
                    span
                }),
            )),
        };

        let click_id = item.id.clone();
        let click_history = self.history.clone();
        let panel_handle = self.panel.clone();
        let hover_id = item.id.clone();

        // Trailing slot: the copy and delete buttons while hovered, otherwise the
        // ⌘N hint the SwiftUI build shows.
        let hovering = self.hovered.as_deref() == Some(item.id.as_str());
        let trailing_inner = if hovering {
            // A row is copied by clicking it, so the only actions left are pin
            // and delete. Pinned rows show the unpin glyph.
            let pin_id = item.id.clone();
            let pin_history = self.history.clone();
            let pin_icon = if item.pin.is_some() {
                gpui_kit::assets::IconName::PinOff
            } else {
                gpui_kit::assets::IconName::Pin
            };
            let used: Vec<String> = self
                .items
                .iter()
                .filter_map(|entry| entry.pin.clone())
                .collect();
            let delete_id = item.id.clone();
            let delete_history = self.history.clone();
            h_flex()
                .gap_1()
                .flex_shrink_0()
                .child(action_icon_button(
                    SharedString::from(format!("act-{}-pin", item.id)),
                    pin_icon,
                    selected,
                    cx.listener(move |state, _, window, cx| {
                        let history = pin_history.clone();
                        let id = pin_id.clone();
                        let pins = rows::available_pins(&used);
                        let panel = state.delegate().panel.clone();
                        // Toggling reloads the list, which cannot happen while the
                        // list is dispatching this click.
                        cx.on_next_frame(window, move |_state, _window, cx| {
                            let _ = history.toggle_pin(id.clone(), pins.clone());
                            if let Some(panel) = panel.as_ref().and_then(|panel| panel.upgrade()) {
                                panel.update(cx, |panel, cx| panel.reload(cx));
                            }
                        });
                    }),
                ))
                .child(action_icon_button(
                    SharedString::from(format!("act-{}-delete", item.id)),
                    gpui_kit::assets::IconName::Trash,
                    selected,
                    cx.listener(move |state, _, window, cx| {
                        let history = delete_history.clone();
                        let id = delete_id.clone();
                        let panel = state.delegate().panel.clone();
                        cx.on_next_frame(window, move |_state, _window, cx| {
                            let _ = history.delete(id.clone());
                            if let Some(panel) = panel.as_ref().and_then(|panel| panel.upgrade()) {
                                panel.update(cx, |panel, cx| panel.reload(cx));
                            }
                        });
                    }),
                ))
                .into_any_element()
        } else if let Some(order) = self
            .checked
            .iter()
            .position(|id| id == &item.id)
            .map(|index| index + 1)
        {
            div()
                .flex_shrink_0()
                .px_1()
                .rounded(px(3.0))
                .bg(if selected {
                    gpui::rgb(0xFF_FFFF).opacity(0.35)
                } else {
                    gpui::rgb(0x0A84FF)
                })
                .text_size(px(10.0))
                .text_color(gpui::white())
                .child(SharedString::from(format!("{order}")))
                .into_any_element()
        } else {
            div().flex_shrink_0().into_any_element()
        };

        // Hovering swaps buttons in for the hint; a fixed box keeps the row from
        // changing size under the pointer.
        let trailing = div()
            .flex_shrink_0()
            .h(px(18.0))
            .flex()
            .items_center()
            .justify_end()
            .child(trailing_inner)
            .into_any_element();

        // The pinned entries are sorted to the top; a rule under the last one
        // separates them from the history without labelling anything.
        let ends_pinned_block = self.pinned_count > 0
            && ix.row + 1 == self.pinned_count
            && self.pinned_count < self.items.len();
        // `selected` also tells the list item it is the active row, which is what
        // keeps the library's hover background from covering our blue.
        // The list item paints its own selection colour from the theme, which is a
        // pale blue. The row body carries our accent instead and covers it, so the
        // highlight is one colour wherever it lands. `selected` is still passed so
        // the library leaves the active row out of its hover styling.
        let body = div()
            .w_full()
            .rounded(px(ROW_CORNER))
            .px_3()
            .py_1()
            .bg(gpui::rgb(SELECTION_ACCENT).opacity(if selected { 0.8 } else { 0.0 }))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(row)
                    .child(trailing),
            );

        let item_element = if ends_pinned_block {
            ListItem::new(("row", ix.row))
                .selected(selected)
                .w_full()
                .px(px(0.0))
                .py(px(0.0))
                .border_b_1()
                .border_color(gpui::rgb(0x0A84FF).opacity(0.25))
        } else {
            ListItem::new(("row", ix.row))
                .selected(selected)
                .w_full()
                .px(px(0.0))
                .py(px(0.0))
        };

        Some(
            item_element
                .child(body)
                .on_hover(cx.listener(
                    move |state: &mut ListState<HistoryList>,
                          hovered: &bool,
                          _window: &mut Window,
                          cx: &mut Context<ListState<HistoryList>>| {
                        let delegate = state.delegate_mut();
                        // Enter and leave arrive in pointer order, so sweeping
                        // down delivered the old row's leave after the new row's
                        // enter and wiped the hover. Only clear our own.
                        let changed = if *hovered {
                            delegate.hovered = Some(hover_id.clone());
                            true
                        } else if delegate.hovered.as_deref() == Some(hover_id.as_str()) {
                            delegate.hovered = None;
                            true
                        } else {
                            false
                        };
                        if changed {
                            cx.notify();
                        }
                    },
                ))
                .on_click(move |event, window, _cx| {
                    // ⌘-click builds a paste stack instead of copying at once.
                    let command = matches!(
                        event,
                        gpui::ClickEvent::Mouse(mouse) if mouse.down.modifiers.platform
                    );
                    if command {
                        if let Some(panel) = panel_handle.as_ref().and_then(|panel| panel.upgrade())
                        {
                            // Updating the panel touches the list, which cannot
                            // happen while the list is being updated; wait a beat.
                            let id = click_id.clone();
                            window.on_next_frame(move |_window, cx| {
                                panel.update(cx, |panel, cx| {
                                    panel.toggle_checked(id.clone(), cx);
                                });
                            });
                        }
                        return;
                    }
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
        // This runs inside the list's own update, so the panel is only told later:
        // a spawned task runs between frames, unlike `defer`, which still runs
        // while the list is on the update stack.
        let id = ix.and_then(|ix| self.item_at(ix).map(|item| item.id.clone()));
        if let Some(panel) = self.panel.as_ref().and_then(|panel| panel.upgrade()) {
            cx.spawn(async move |_state, cx| {
                panel.update(cx, |panel, cx| panel.select_entry(id, cx));
            })
            .detach();
        }
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        // With entries picked by ⌘-click, enter pastes them together in pick
        // order instead of copying the highlighted row.
        if !self.checked.is_empty() {
            // "选中即粘贴" turns the picked stack into a real paste stack: each
            // entry is placed on the clipboard and pasted before the next one,
            // which is what the SwiftUI build does with multi-selection.
            if self.paste_by_default {
                let history = self.history.clone();
                let stack = self.checked.clone();
                self.checked.clear();
                _window.remove_window();
                std::thread::spawn(move || {
                    for id in stack {
                        if !copy_entry_to_clipboard(&history, &id) {
                            continue;
                        }
                        // The target app reads the clipboard on paste; a short
                        // pause keeps the next write from overtaking it.
                        std::thread::sleep(std::time::Duration::from_millis(80));
                        crate::clipboard::paste_into_frontmost();
                        std::thread::sleep(std::time::Duration::from_millis(260));
                    }
                });
                return;
            }

            let mut parts: Vec<String> = Vec::new();
            for id in self.checked.clone() {
                let Ok(contents) = self.history.contents(id) else {
                    continue;
                };
                if let Some(text) = contents
                    .iter()
                    .find(|content| {
                        matches!(
                            content.content_type.as_str(),
                            "public.utf8-plain-text" | "text/plain" | "text/plain;charset=utf-8"
                        )
                    })
                    .and_then(|content| content.value.as_ref())
                    .map(|value| String::from_utf8_lossy(value).to_string())
                {
                    parts.push(text);
                }
            }
            if !parts.is_empty() && crate::clipboard::write_text(&parts.join("\n")) {
                self.checked.clear();
                _window.remove_window();
            }
            return;
        }

        // Enter copies what is highlighted; with nothing highlighted (a filter can
        // clear it) it falls back to the first row, so the key never goes dead.
        let ix = match self.selected {
            Some(ix) => ix,
            None if !self.items.is_empty() => gpui_kit::base::IndexPath {
                section: 0,
                row: 0,
                column: 0,
            },
            None => return,
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

pub(crate) struct HistoryPanel {
    history: Arc<HistoryManager>,
    /// Every entry's metadata, used as the search corpus.
    all_items: Vec<ClipboardItem>,
    input: Entity<InputState>,
    list: Entity<ListState<HistoryList>>,
    settings: settings::Settings,
    /// The preview sidebar, toggled with ⌃Space like the macOS build.
    preview_open: bool,
    /// When set, the list only shows entries copied from this application.
    active_app: Option<String>,
    /// Entries picked with ⌘-click, in pick order; the order is the paste order.
    checked: Vec<String>,
    /// Decoded preview images, keyed by entry id: decoding a PNG on every frame
    /// would make the panel stutter on large screenshots.
    /// Set while the footer's "clear everything" is waiting for a second click.
    confirm_clear_all: bool,
    /// How many rows the list is showing, mirrored so the render never has to
    /// read the list entity.
    visible_count: usize,
    /// The highlighted entry by id, for the same reason.
    selected_id: Option<String>,
    preview_images: std::collections::HashMap<
        String,
        Option<(std::sync::Arc<gpui_kit::RenderImage>, f32, f32)>,
    >,
}

impl HistoryPanel {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let database = storage_path();
        let history = history();
        let mut settings = settings::Settings::load(&database);
        // Metadata only: payloads are fetched on demand so startup does not
        // depend on how much content the history holds.
        let all_metadata = history.load_metadata().unwrap_or_default();
        let all_items = all_metadata.clone();
        // A default smaller than what is already stored would delete entries the
        // first time something new is copied, so never load below the real size.
        let stored = all_items.len() as i64;
        if !settings.unlimited_history && settings.history_size < stored {
            settings.history_size = stored;
            settings.save(&database);
        }

        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(SharedString::from("搜索")));
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
                    checked: Vec::new(),
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
            active_app: None,
            // Screenshot/test hook: pre-check entries by id.
            checked: std::env::var("MANIS_POCKET_CHECKED")
                .ok()
                .map(|ids| {
                    ids.split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            confirm_clear_all: false,
            visible_count: 0,
            selected_id: None,
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

    /// Records the highlighted entry and warms its preview.
    pub fn select_entry(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.selected_id == id {
            return;
        }
        self.selected_id = id;
        self.load_preview_image(cx);
        cx.notify();
    }

    /// The highlighted entry, resolved from the panel's own copy of the history.
    fn selected_item(&self) -> Option<ClipboardItem> {
        let id = self.selected_id.as_deref()?;
        self.all_items.iter().find(|item| item.id == id).cloned()
    }

    /// Moves the highlighted row and keeps it in view.
    fn move_selection(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        // Read outside the update so the panel's own state can move with it.
        // The delegate owns the highlighted row: `refresh` sets it directly, so
        // ListState::selected_index() stays None and using it made the first
        // press re-select the row that was already highlighted.
        let (count, current, ids) = {
            let state = self.list.read(cx);
            let delegate = state.delegate();
            (
                delegate.items.len(),
                delegate.selected.map(|ix| ix.row),
                delegate
                    .items
                    .iter()
                    .map(|item| item.id.clone())
                    .collect::<Vec<_>>(),
            )
        };
        if count == 0 {
            return;
        }
        let Some(next) = rows::next_row(current, delta, count) else {
            return;
        };
        self.selected_id = ids.get(next).cloned();
        self.load_preview_image(cx);
        let list = self.list.clone();
        list.update(cx, |state, cx| {
            state.set_selected_index(
                Some(gpui_kit::base::IndexPath {
                    section: 0,
                    row: next,
                    column: 0,
                }),
                window,
                cx,
            );
            state.scroll_to_selected_item(window, cx);
        });
        cx.notify();
    }

    /// Copies what the user is pointing at: the highlighted row, or the first one.
    fn confirm_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list = self.list.clone();
        list.update(cx, |state, cx| {
            state.delegate_mut().confirm(false, window, cx);
        });
    }

    /// Pins or unpins the selected entry. Pins take the next free letter, which
    /// is what the row shows on its right.
    fn toggle_pin_selected(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item() else {
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
        let Some(item) = self.selected_item() else {
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

    /// Applications present in the history, in the order they appear in the list
    /// (most recent first). The empty string is the "unknown source" bucket the
    /// SwiftUI bar shows separately, so it is kept.
    fn source_apps(&self) -> Vec<String> {
        let mut apps: Vec<String> = Vec::new();
        for item in &self.all_items {
            let Some(app) = item.application.clone() else {
                continue;
            };
            if app.is_empty() {
                continue;
            }
            if !apps.contains(&app) {
                apps.push(app);
            }
        }
        apps
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.confirm_clear_all = false;
        // The image cache is filled from the current selection, so refresh it
        // whenever the list content changes.
        let query = self.input.read(cx).value().to_string();
        let pool = match &self.active_app {
            Some(app) => self
                .all_items
                .iter()
                .filter(|item| item.application.as_deref() == Some(app.as_str()))
                .cloned()
                .collect(),
            None => self.all_items.clone(),
        };
        let results = self
            .history
            .search(&query, pool, search_mode(&self.settings.search_mode));

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
        // Keep the panel's own view of the list: the render and the selection
        // resolve against these instead of reading the list entity.
        self.visible_count = items.len();
        self.selected_id = items.first().map(|item| item.id.clone());
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
    fn preview_image(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        let item = self.selected_item()?;
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
    /// Adds or removes one entry from the ⌘-clicked set.
    fn toggle_checked(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(index) = self.checked.iter().position(|existing| existing == &id) {
            self.checked.remove(index);
        } else {
            self.checked.push(id);
        }
        self.sync_checked(cx);
        cx.notify();
    }

    /// Mirrors the panel's picked set into the list delegate, which draws the
    /// order badges.
    pub(crate) fn sync_checked(&mut self, cx: &mut Context<Self>) {
        let checked = self.checked.clone();
        self.list.update(cx, |state, cx| {
            state.delegate_mut().checked = checked;
            cx.notify();
        });
    }

    /// Warms the app-icon cache off the paint path: a paint that asks AppKit for
    /// an icon re-enters the list and aborts.
    pub(crate) fn load_app_icons(&mut self, cx: &mut Context<Self>) {
        let apps: Vec<String> = self
            .all_items
            .iter()
            .filter_map(|item| item.application.clone())
            .filter(|app| !app.is_empty())
            .collect();
        icons::preload(apps.iter().map(String::as_str));
        cx.notify();
    }

    /// Decodes row thumbnails for image entries, off the paint path.
    pub(crate) fn load_row_images(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self
            .all_items
            .iter()
            .filter(|item| matches!(rows::row_kind(item), rows::RowKind::Image))
            .map(|item| item.id.clone())
            .take(60)
            .collect();
        let mut loaded_any = false;
        for id in ids {
            if icons::has_thumb(&id) {
                continue;
            }
            let thumb = self
                .history
                .contents(id.clone())
                .ok()
                .and_then(|contents| {
                    contents
                        .into_iter()
                        .find(|content| content.content_type == "public.png")
                })
                .and_then(|content| content.value)
                .and_then(|bytes| icons::png_to_render_image(&bytes, 340));
            icons::store_thumb(&id, thumb);
            loaded_any = true;
        }
        if loaded_any {
            cx.notify();
        }
    }

    pub(crate) fn load_preview_image(&mut self, _cx: &mut Context<Self>) {
        let Some(item) = self
            .selected_item()
            .or_else(|| self.all_items.first().cloned())
        else {
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
        let selected = self.selected_item();
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
    /// One icon per application that appears in the history; clicking filters the
    /// list and clicking the same icon again clears the filter.
    fn render_app_filter(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let apps = self.source_apps();
        if apps.len() < 2 {
            return None;
        }
        let active = self.active_app.clone();
        let mut bar = h_flex().w_full().gap_1().px_3().pb_2().items_center();

        for app in apps.into_iter().take(8) {
            // The same cached bundle icon the rows draw, with the letter badge as
            // the fallback when AppKit cannot supply one.
            let icon = match icons::cached_icon(&app) {
                Some(render) => img(render)
                    .size(px(18.0))
                    .rounded(px(4.0))
                    .into_any_element(),
                None => {
                    let (initial, (red, green, blue)) = rows::app_badge(&app);
                    div()
                        .size(px(18.0))
                        .rounded(px(4.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(10.0))
                        .text_color(gpui::white())
                        .bg(gpui::rgb(
                            (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue),
                        ))
                        .child(initial)
                        .into_any_element()
                }
            };

            let selected = active.as_deref() == Some(app.as_str());
            let dimmed = active.is_some() && !selected;
            let target = app.clone();
            let app_name = icons::app_name(&app);
            let mut button = div()
                .id(SharedString::from(format!("app-filter-{app}")))
                .p(px(2.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(app_name.clone()).build(window, cx)
                })
                .child(icon);
            button = if selected {
                button
                    .bg(gpui::rgb(0x0A84FF).opacity(0.20))
                    .border_1()
                    .border_color(gpui::rgb(0x0A84FF).opacity(0.80))
            } else if dimmed {
                // The SwiftUI bar fades the apps that are not filtering.
                button.opacity(0.45)
            } else {
                button.hover(|style| style.bg(gpui::black().opacity(0.08)))
            };
            bar = bar.child(button.on_click(cx.listener(move |this, _, _, cx| {
                this.active_app = if this.active_app.as_deref() == Some(target.as_str()) {
                    None
                } else {
                    Some(target.clone())
                };
                this.refresh(cx);
                cx.notify();
            })));
        }

        Some(bar.into_any_element())
    }

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
                row(
                    "footer-clear-all",
                    if self.confirm_clear_all {
                        "再点一次确认"
                    } else {
                        "全部清除"
                    },
                    "⇧⌥⌘Del",
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if !clear_all_armed(&mut this.confirm_clear_all) {
                        // Arming only lasts while the panel stays as it is; anything
                        // that reloads it disarms again.
                        cx.notify();
                        return;
                    }
                    let _ = this.history.clear_all();
                    this.reload(cx);
                })),
            )
            .child(
                row("footer-preferences", "偏好设置…", "⌘,").on_click(cx.listener(
                    |_, _, window, cx| {
                        let _ = window;
                        open_settings(cx);
                    },
                )),
            )
            .child(
                row("footer-about", "关于", "").on_click(cx.listener(|_, _, _, _cx| {
                    show_about();
                })),
            )
            .child(row("footer-quit", "退出", "⌘Q").on_click(cx.listener(|_, _, _, cx| cx.quit())))
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
        // Everything that is not the list: search row, filter bar, footer and the
        // status line. Under-counting these clipped the last row.
        let chrome = HEADER_HEIGHT
            + if self.source_apps().len() >= 2 {
                APP_FILTER_HEIGHT
            } else {
                0.0
            }
            + footer
            + if self.settings.show_footer {
                STATUS_HEIGHT
            } else {
                0.0
            };
        let content = rows * ROW_PITCH + chrome;
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
        self.all_items
            .len()
            .min(self.settings.window_height.max(1.0) as usize)
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

        // Reading the list here used to abort: a key press notifies the list,
        // the panel re-renders, and this render ran while the list was still
        // being updated. The counts are kept instead.
        let count = self.visible_count;
        let checked = self.checked.len();

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
                } else if key == "down" {
                    this.move_selection(1, window, cx);
                } else if key == "up" {
                    this.move_selection(-1, window, cx);
                } else if key == "pageup" {
                    this.move_selection(-8, window, cx);
                } else if key == "pagedown" {
                    this.move_selection(8, window, cx);
                } else if key == "home" {
                    this.move_selection(isize::MIN / 2, window, cx);
                } else if key == "end" {
                    this.move_selection(isize::MAX / 2, window, cx);
                } else if key == "enter" {
                    this.confirm_selection(window, cx);
                } else if modifiers.alt && key == "p" {
                    // ⌥P toggles the pin (`KeyboardShortcuts_pin` = option + P).
                    this.toggle_pin_selected(cx);
                } else if modifiers.control && key == "space" {
                    // ⌃Space toggles the preview (`KeyboardShortcuts_togglePreview`).
                    this.toggle_preview(cx);
                } else if key == "backspace" && modifiers.platform && modifiers.alt {
                    // ⌥⌘⌫ clears unpinned, ⇧⌥⌘⌫ clears everything — and the latter
                    // asks once, exactly like the footer item does.
                    if modifiers.shift {
                        if clear_all_armed(&mut this.confirm_clear_all) {
                            let _ = this.history.clear_all();
                            this.reload(cx);
                        } else {
                            cx.notify();
                        }
                    } else {
                        let _ = this.history.clear_unpinned();
                        this.reload(cx);
                    }
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
                                    .children(self.render_app_filter(cx))
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
                                            .child(SharedString::from(if checked > 0 {
                                                format!("已选 {checked} 项 · 回车按顺序粘贴 · esc 关闭")
                                            } else {
                                                format!("{count} 条 · 上下选择 · 回车复制 · ⌘ 点击多选 · esc 关闭")
                                            }))
                                            .into_any_element()
                                    })),
                            )
                            .children(self.preview_open.then(|| self.render_preview(cx))),
                    ),
            )
    }
}

/// Maps the `searchMode` setting onto the core's search mode.
/// Two-step confirmation for the one action that destroys everything: the first
/// click arms it, the second clears. Returns whether this click should clear.
fn clear_all_armed(armed: &mut bool) -> bool {
    let was_armed = *armed;
    *armed = !was_armed;
    was_armed
}

/// Shows what the app is: macOS has a standard about panel, Linux gets the project
/// page, which is more useful than a menu item that does nothing.
fn show_about() {
    #[cfg(target_os = "macos")]
    icons::show_about_panel();
    #[cfg(not(target_os = "macos"))]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg("https://github.com/kaigedong/Manis-Pocket")
            .spawn();
    }
}

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
pub(crate) fn copy_entry_to_clipboard(history: &HistoryManager, id: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::clear_all_armed;

    #[test]
    fn clearing_everything_needs_two_clicks() {
        let mut armed = false;
        assert!(!clear_all_armed(&mut armed), "the first click only arms");
        assert!(armed);
        assert!(clear_all_armed(&mut armed), "the second click clears");
        assert!(!armed, "and the confirmation is spent");
    }

    #[test]
    fn a_spent_confirmation_arms_again() {
        let mut armed = false;
        let _ = clear_all_armed(&mut armed);
        let _ = clear_all_armed(&mut armed);
        assert!(!clear_all_armed(&mut armed), "a third click must not clear");
    }
}
