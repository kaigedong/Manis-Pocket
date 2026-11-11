//! The settings window.
//!
//! The SwiftUI build has seven panes — General, Appearance, Pins, Ignore,
//! Storage, Advanced, Sync — that are all built from the same shape: a section
//! header plus rows of switches and pickers. This mirrors that shell and wires
//! the rows whose values the core already understands; panes whose features have
//! no implementation yet say so instead of showing dead controls.

use gpui::{div, px, AnyElement, App, Context, InteractiveElement, IntoElement, SharedString, Window};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

use manis_pocket_core::history::HistoryManager;
use std::sync::Arc;

use crate::settings::Settings;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    General,
    Appearance,
    Pins,
    Ignore,
    Storage,
    Advanced,
    Sync,
}

impl Pane {
    /// Stable identifier used by the settings file and the pane test hook.
    fn key(self) -> &'static str {
        match self {
            Pane::General => "general",
            Pane::Appearance => "appearance",
            Pane::Pins => "pins",
            Pane::Ignore => "ignore",
            Pane::Storage => "storage",
            Pane::Advanced => "advanced",
            Pane::Sync => "sync",
        }
    }

    const ALL: [Pane; 7] = [
        Pane::General,
        Pane::Appearance,
        Pane::Pins,
        Pane::Ignore,
        Pane::Storage,
        Pane::Advanced,
        Pane::Sync,
    ];

    fn title(self) -> &'static str {
        match self {
            Pane::General => "通用",
            Pane::Appearance => "外观",
            Pane::Pins => "固定",
            Pane::Ignore => "忽略",
            Pane::Storage => "存储",
            Pane::Advanced => "高级",
            Pane::Sync => "同步",
        }
    }
}

pub struct SettingsView {
    settings: Settings,
    /// Where the settings file lives; every change is written immediately, as
    /// the SwiftUI build does with UserDefaults.
    database_path: String,
    /// Used by the devices pane to list paired peers.
    history: Arc<HistoryManager>,
    pane: Pane,
    /// Diagnostic-only field: lets the same input component be tried in a normal
    /// window, which tells apart "the panel window" from "the text element".
    probe: Option<gpui::Entity<InputState>>,
}

impl SettingsView {
    pub fn new(
        database_path: String,
        history: Arc<HistoryManager>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let probe = std::env::var("MANIS_POCKET_IME_PROBE")
            .ok()
            .map(|_| cx.new(|cx| InputState::new(window, cx).placeholder("IME test")));
        Self {
            settings: Settings::load(&database_path),
            database_path,
            history,
            pane: std::env::var("MANIS_POCKET_PANE")
                .ok()
                .and_then(|name| Pane::ALL.into_iter().find(|pane| pane.key() == name))
                .unwrap_or(Pane::General),
            probe,
        }
    }

    fn save(&self) {
        self.settings.save(&self.database_path);
    }

    fn set_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        self.pane = pane;
        cx.notify();
    }
}


/// A labelled row whose control is a row of small segmented buttons.
///
/// `set` is a plain function pointer so the same value can be captured by several
/// click handlers without cloning.
fn pick_row(
    id: &'static str,
    label: &'static str,
    options: &'static [(&'static str, &'static str)],
    current: &str,
    set: fn(&mut Settings, &str),
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let mut picker = h_flex().gap_1();
    for (value, text) in options {
        let chosen = current == *value;
        let mut button = div()
            .id(SharedString::from(format!("{id}-{value}")))
            .px_1p5()
            .py_0p5()
            .rounded(px(4.0))
            .text_xs()
            .cursor_pointer();
        button = if chosen {
            button
                .bg(gpui::rgb(0x0A84FF))
                .text_color(gpui::rgb(0xFF_FFFF))
        } else {
            button
                .bg(gpui::black().opacity(0.05))
                .hover(|style| style.bg(gpui::black().opacity(0.1)))
        };
        let value = *value;
        picker = picker.child(button.child(*text).on_click(cx.listener(
            move |this, _, _, cx| {
                set(&mut this.settings, value);
                this.save();
                cx.notify();
            },
        )));
    }
    h_flex()
        .w_full()
        .justify_between()
        .py_1p5()
        .child(div().text_sm().child(label))
        .child(picker)
        .into_any_element()
}

/// A labelled row whose control is arbitrary.
fn label_row(label: &'static str, control: AnyElement) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .py_1p5()
        .child(div().text_sm().child(label))
        .child(control)
        .into_any_element()
}

/// One labelled row with a switch on the right.
fn switch_row(
    id: &'static str,
    label: &'static str,
    checked: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .py_1p5()
        .child(div().text_sm().child(label))
        .child(Switch::new(id).checked(checked).on_click(on_change))
        .into_any_element()
}

fn section(title: &'static str, rows: Vec<AnyElement>) -> AnyElement {
    v_flex()
        .w_full()
        .gap_1()
        .pb_4()
        .child(
            div()
                .text_xs()
                .text_color(gpui::black().opacity(0.45))
                .child(title),
        )
        .children(rows)
        .into_any_element()
}

fn note(text: impl Into<SharedString>) -> AnyElement {
    let text: SharedString = text.into();
    div()
        .text_sm()
        .text_color(gpui::black().opacity(0.5))
        .child(text)
        .into_any_element()
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = v_flex()
            .w(px(160.0))
            .h_full()
            .items_start()
            .gap_0p5()
            .p_2()
            .children(Pane::ALL.map(|pane| {
                let mut row = div()
                    .id(SharedString::from(pane.title()))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm();
                if self.pane == pane {
                    row = row.bg(gpui::black().opacity(0.08));
                }
                row.hover(|row| row.bg(gpui::black().opacity(0.04)))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_pane(pane, cx)))
                    .child(pane.title())
            }));

        let content = self.render_pane(cx);

        h_flex()
            .size_full()
            .items_start()
            .bg(gpui::white())
            .text_color(gpui::black())
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .px_4()
                    .py_3()
                    .child(content),
            )
    }
}

impl SettingsView {
    fn render_pane(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        match self.pane {
            Pane::General => {
                if let Some(probe) = &self.probe {
                    rows.push(Input::new(probe).appearance(false).into_any_element());
                }
                rows.push(pick_row(
                    "search-mode",
                    "搜索方式",
                    &[
                        ("exact", "精确"),
                        ("fuzzy", "模糊"),
                        ("regexp", "正则"),
                        ("mixed", "混合"),
                    ],
                    &self.settings.search_mode,
                    |settings, value| settings.search_mode = value.to_string(),
                    cx,
                ));
                rows.push(switch_row(
                    "paste-by-default",
                    "选中即粘贴",
                    self.settings.paste_by_default,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.paste_by_default = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                rows.push(switch_row(
                    "show-footer",
                    "显示底部信息栏",
                    self.settings.show_footer,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.show_footer = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                rows.push(switch_row(
                    "show-search",
                    "显示搜索框",
                    self.settings.show_search,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.show_search = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                rows.push(switch_row(
                    "show-app-icons",
                    "显示应用图标",
                    self.settings.show_application_icons,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.show_application_icons = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                rows.push(switch_row(
                    "show-title",
                    "显示窗口标题",
                    self.settings.show_title,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.show_title = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
            }
            Pane::Appearance => {
                rows.push(pick_row(
                    "popup-position",
                    "面板位置",
                    &[
                        ("cursor", "光标"),
                        ("statusItem", "菜单栏"),
                        ("center", "屏幕中央"),
                    ],
                    &self.settings.popup_position,
                    |settings, value| settings.popup_position = value.to_string(),
                    cx,
                ));
                rows.push(pick_row(
                    "pins-position",
                    "固定项位置",
                    &[("top", "顶部"), ("bottom", "底部")],
                    &self.settings.pins_position,
                    |settings, value| settings.pins_position = value.to_string(),
                    cx,
                ));
                rows.push(pick_row(
                    "highlight-match",
                    "匹配高亮",
                    &[("bold", "加粗"), ("background", "底色")],
                    &self.settings.highlight_match,
                    |settings, value| settings.highlight_match = value.to_string(),
                    cx,
                ));
                rows.push(note(format!(
                    "面板尺寸 {}×{}（窗口宽高，单位 pt）",
                    self.settings.window_width, self.settings.window_height
                )));
                rows.push(note("Wayland 下面板位置由合成器决定，需要用 windowrule"));
            }
            Pane::Pins => {
                rows.push(note("⌥P 固定 / 取消固定当前条目".to_string()));
                rows.push(note("固定后的条目显示在列表最前，并带字母标记".to_string()));
                rows.push(pick_row(
                    "pins-position-pins",
                    "固定项位置",
                    &[("top", "顶部"), ("bottom", "底部")],
                    &self.settings.pins_position,
                    |settings, value| settings.pins_position = value.to_string(),
                    cx,
                ));
            }
            Pane::Ignore => {
                rows.push(switch_row(
                    "ignore-allowlist",
                    "仅记录下列应用",
                    self.settings.ignore_all_apps_except_listed,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.ignore_all_apps_except_listed = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                rows.push(note(if self.settings.ignored_apps.is_empty() {
                    "未被忽略的应用".to_string()
                } else {
                    format!("应用清单 {} 项", self.settings.ignored_apps.len())
                }));
                for app in &self.settings.ignored_apps {
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(gpui::black().opacity(0.55))
                            .child(app.clone())
                            .into_any_element(),
                    );
                }
                rows.push(note(if self.settings.ignore_regexp.is_empty() {
                    "未设置忽略正则".to_string()
                } else {
                    format!("忽略正则 {} 条", self.settings.ignore_regexp.len())
                }));
                for pattern in &self.settings.ignore_regexp {
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(gpui::black().opacity(0.55))
                            .child(pattern.clone())
                            .into_any_element(),
                    );
                }
                rows.push(note("在设置文件中编辑 ignoredApps / ignoreRegexp，每行一条"));
            }
            Pane::Storage => {
                rows.push(pick_row(
                    "sort-by",
                    "排序方式",
                    &[
                        ("lastCopiedAt", "上次复制"),
                        ("firstCopiedAt", "首次复制"),
                        ("numberOfCopies", "复制次数"),
                    ],
                    &self.settings.sort_by,
                    |settings, value| settings.sort_by = value.to_string(),
                    cx,
                ));
                rows.push(switch_row(
                    "unlimited-history",
                    "无限历史记录",
                    self.settings.unlimited_history,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.unlimited_history = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                let step = |delta: i64| {
                    move |this: &mut SettingsView, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<SettingsView>| {
                        let next = (this.settings.history_size + delta).clamp(1, 9999);
                        this.settings.history_size = next;
                        this.save();
                        cx.notify();
                    }
                };
                rows.push(label_row(
                    "保留条数",
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .id("size-minus")
                                .px_2()
                                .rounded(px(4.0))
                                .bg(gpui::black().opacity(0.05))
                                .cursor_pointer()
                                .child("-100")
                                .on_click(cx.listener(step(-100))),
                        )
                        .child(div().text_sm().child(format!("{}", self.settings.history_size)))
                        .child(
                            div()
                                .id("size-plus")
                                .px_2()
                                .rounded(px(4.0))
                                .bg(gpui::black().opacity(0.05))
                                .cursor_pointer()
                                .child("+100")
                                .on_click(cx.listener(step(100))),
                        )
                        .into_any_element(),
                ));
                rows.push(switch_row(
                    "save-images",
                    "保存图像",
                    self.settings.save_images,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.save_images = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                rows.push(switch_row(
                    "save-text",
                    "保存文本",
                    self.settings.save_text,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.save_text = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                let size = std::fs::metadata(&self.database_path)
                    .map(|meta| format!("{:.1} MB", meta.len() as f64 / (1024.0 * 1024.0)))
                    .unwrap_or_else(|_| "未知".to_string());
                rows.push(note(format!("数据库占用：{size}")));
            }
            Pane::Advanced => {
                rows.push(note(format!("数据库：{}", self.database_path)));
                rows.push(note(format!(
                    "设备名：{}",
                    self.settings.sync_device_name
                )));
                rows.push(note(format!(
                    "历史上限：{}（当前库共 {} 条）",
                    self.settings.history_size,
                    self.history
                        .load_metadata()
                        .map(|items| items.len())
                        .unwrap_or(0)
                )));
                rows.push(note("文件夹 / 文件类型保存暂未实现"));
            }
            Pane::Sync => {
                rows.push(switch_row(
                    "sync-enabled",
                    "启用同步",
                    self.settings.sync_enabled,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.sync_enabled = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                rows.push(switch_row(
                    "sync-discoverable",
                    "允许被发现",
                    self.settings.sync_discoverable,
                    cx.listener(|this, checked, _, cx| {
                        this.settings.sync_discoverable = *checked;
                        this.save();
                        cx.notify();
                    }),
                ));
                rows.push(note(format!("本机设备名：{}", self.settings.sync_device_name)));
                let peers = self.history.get_paired_peers();
                rows.push(note(format!("已配对设备 {} 台", peers.len())));
                for peer in &peers {
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(gpui::black().opacity(0.55))
                            .child(peer.clone())
                            .into_any_element(),
                    );
                }
                rows.push(note("开关修改后重启应用生效"));
            }
        }

        v_flex()
            .w_full()
            .child(section(self.pane.title(), rows))
            .into_any_element()
    }
}
