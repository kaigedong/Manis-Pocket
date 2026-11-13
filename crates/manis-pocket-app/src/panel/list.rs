//! The list delegate: it draws one row and turns pointer events into panel
//! actions. Everything it needs from the panel goes through `HistoryPanel`.

use super::*;

pub(crate) struct HistoryList {
    /// Arranged: pinned entries first, then history.
    pub(crate) items: Vec<ClipboardItem>,
    /// How many leading entries belong to the pinned section.
    pub(crate) pinned_count: usize,
    pub(crate) selected: Option<IndexPath>,
    pub(crate) history: Arc<HistoryManager>,
    pub(crate) show_application_icons: bool,
    /// Mirrors `pasteByDefault`: pressing enter also pastes into the front app.
    pub(crate) paste_by_default: bool,
    /// Mirrors `showSpecialSymbols`: render whitespace markers in titles.
    pub(crate) show_special_symbols: bool,
    /// Entries picked with ⌘-click, in pick order. Empty means "no multi-select".
    pub(crate) checked: Vec<String>,
    /// Whether matches use a background colour instead of bold text.
    pub(crate) highlight_background: bool,
    /// Match ranges per item id, straight from the core's search results.
    pub(crate) highlights: std::collections::HashMap<String, Vec<(usize, usize)>>,
    /// Row under the pointer: only that row reveals its action buttons.
    pub(crate) hovered: Option<String>,
    /// The panel that owns this list, so a row action can refresh it in place.
    pub(crate) panel: Option<gpui::WeakEntity<HistoryPanel>>,
}

impl HistoryList {
    pub(crate) fn replace_items(
        &mut self,
        items: Vec<ClipboardItem>,
        pins_position: &str,
        sort_by: &str,
    ) {
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

/// A small icon button for the row's trailing actions.
fn action_icon_button(
    id: SharedString,
    icon: gpui_kit::assets::IconName,
    on_accent: bool,
    handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let tint = if on_accent {
        gpui::white().opacity(0.9)
    } else {
        gpui::black().opacity(0.55)
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(18.0))
        .rounded(px(3.0))
        .cursor_pointer()
        .hover(move |style| {
            style.bg(if on_accent {
                gpui::white().opacity(0.18)
            } else {
                gpui::black().opacity(0.08)
            })
        })
        .child(
            gpui_kit::component::Icon::new(icon)
                .xsmall()
                .text_color(tint),
        )
        .on_click(handler)
        .into_any_element()
}
