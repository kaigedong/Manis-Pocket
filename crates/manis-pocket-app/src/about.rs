//! The About window.
//!
//! macOS used to answer this with `orderFrontStandardAboutPanel`, which reads the
//! version out of the app bundle's `Info.plist`. Off macOS there is no bundle, so
//! the footer item pointed at the project page instead: it answered "where is the
//! source?" but not "which build am I looking at?", which is what people open
//! About for.
//!
//! One self-drawn window serves both platforms, like the rest of the UI. It is the
//! same shape as the history panel — a transparent window around a white card —
//! so a tiling compositor that stretches the window still shows a small floating
//! card instead of a half-screen white slab.

use gpui::{AnyElement, Context, IntoElement, Render, Styled, Window, div, px, rgb};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;

/// What this binary is, as resolved by `build.rs`: the package version, and for
/// builds that are not attached to a release the git commit count and revision
/// — `0.1.0.r1136.gf85a27d`, the same string the Linux package reports.
const VERSION: &str = env!("MANIS_POCKET_BUILD_VERSION");

const PROJECT: &str = "https://github.com/kaigedong/Manis-Pocket";

/// Card width, matching the default panel width so the two windows look related.
const CARD_WIDTH: f32 = 396.0;

pub(crate) struct AboutView;

/// The window the card is drawn in: a little larger than the card, which leaves
/// room for its shadow.
const WINDOW_SIZE: (f32, f32) = (440.0, 300.0);

impl Render for AboutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The panel does the same: asking for the card's size on every frame is
        // what keeps a tiling compositor's oversize tile from becoming the window.
        let (width, height) = WINDOW_SIZE;
        let current = window.viewport_size();
        if (current.width.as_f32() - width).abs() > 0.5
            || (current.height.as_f32() - height).abs() > 0.5
        {
            window.resize(size(px(width), px(height)));
        }

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(card(cx))
    }
}

/// The white card: everything the window shows.
fn card(cx: &mut Context<AboutView>) -> AnyElement {
    v_flex()
        .w(px(CARD_WIDTH))
        .rounded(px(9.0))
        .bg(gpui::white())
        .text_color(gpui::black())
        .shadow_lg()
        .overflow_hidden()
        .p_4()
        .gap_1()
        .child(
            div()
                .text_2xl()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Manis Pocket"),
        )
        .child(
            div()
                .text_sm()
                .text_color(gpui::black().opacity(0.55))
                .child("剪贴板历史 · 设备间同步"),
        )
        .child(
            v_flex()
                .py_3()
                .child(info_row("版本", div().child(VERSION).into_any_element()))
                .child(info_row("平台", div().child(platform()).into_any_element()))
                .child(info_row(
                    "项目主页",
                    div()
                        .id("about-project")
                        .text_color(rgb(0x0A84FF))
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        // A browser can be missing or refuse to start; the URL
                        // stays readable above either way, so this is a shortcut
                        // rather than the only way to see it.
                        .on_click(|_, _, _| open_project_page())
                        .child(PROJECT)
                        .into_any_element(),
                )),
        )
        .child(
            h_flex()
                .items_center()
                .justify_end()
                .child(close_button(cx)),
        )
        .into_any_element()
}

/// One label/value line: the label keeps its column so the values line up.
fn info_row(label: &'static str, value: AnyElement) -> AnyElement {
    h_flex()
        .items_start()
        .gap_3()
        .py_0p5()
        .child(
            div()
                .w(px(64.0))
                .flex_shrink_0()
                .text_sm()
                .text_color(gpui::black().opacity(0.45))
                .child(label),
        )
        .child(div().flex_1().text_sm().child(value))
        .into_any_element()
}

fn close_button(cx: &mut Context<AboutView>) -> AnyElement {
    div()
        .id("about-close")
        .px_4()
        .py_1()
        .rounded(px(6.0))
        .bg(rgb(0x0A84FF))
        .text_color(gpui::white())
        .text_sm()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x0071E3)))
        .child("关闭")
        .on_click(cx.listener(|_, _, window, _cx| {
            window.remove_window();
        }))
        .into_any_element()
}

/// Opens the project page in the desktop's browser.
fn open_project_page() {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    if let Err(error) = std::process::Command::new(opener).arg(PROJECT).spawn() {
        eprintln!("could not open {PROJECT}: {error}");
    }
}

/// The platform line about the build, e.g. `linux x86_64`.
fn platform() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}
