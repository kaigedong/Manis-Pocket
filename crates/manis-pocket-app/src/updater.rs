//! Shared update state and Manis-style update UI components.
//!
//! Manis Pocket is a resident menu-bar app whose history panel, settings window,
//! and about window open and close independently. Keeping the update state in a
//! single GPUI global entity lets a download started from the panel's header
//! pill survive closing the panel and stay in sync with Settings and About.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gpui_kit as gpui;
use gpui_kit::assets::IconName as LucideIcon;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, Global, InteractiveElement,
    IntoElement, MouseButton, ParentElement, RenderImage, SharedString, StatefulInteractiveElement,
    Styled, div, img, prelude::FluentBuilder as _, px, relative, rgb,
};

use crate::app_update::{self, AppUpdateError, AvailableUpdate, DownloadProgress};

const DOWNLOAD_POLL_INTERVAL: Duration = Duration::from_millis(100);
const ACCENT_BLUE: u32 = 0x0A84FF;
const ACCENT_BLUE_HOVER: u32 = 0x0071E3;
const ACCENT_BLUE_ACTIVE: u32 = 0x005EC4;
const STATUS_SUCCESS: u32 = 0x1E8E3E;
const STATUS_ERROR: u32 = 0xD93025;

struct SharedDownloadProgress {
    downloaded_bytes: AtomicU64,
    total_bytes: AtomicU64,
    bytes_per_second: AtomicU64,
    finished: AtomicBool,
}

impl SharedDownloadProgress {
    fn new() -> Self {
        Self {
            downloaded_bytes: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            bytes_per_second: AtomicU64::new(0),
            finished: AtomicBool::new(false),
        }
    }

    fn store(&self, progress: DownloadProgress) {
        self.downloaded_bytes
            .store(progress.downloaded_bytes, Ordering::Relaxed);
        self.total_bytes
            .store(progress.total_bytes.unwrap_or(0), Ordering::Relaxed);
        self.bytes_per_second
            .store(progress.bytes_per_second, Ordering::Relaxed);
    }

    fn load(&self) -> DownloadProgress {
        let total = self.total_bytes.load(Ordering::Relaxed);
        DownloadProgress {
            downloaded_bytes: self.downloaded_bytes.load(Ordering::Relaxed),
            total_bytes: (total > 0).then_some(total),
            bytes_per_second: self.bytes_per_second.load(Ordering::Relaxed),
        }
    }

    fn mark_finished(&self) {
        self.finished.store(true, Ordering::Release);
    }

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AppUpdateState {
    Idle,
    Checking,
    Available(AvailableUpdate),
    Downloading {
        update: AvailableUpdate,
        progress: DownloadProgress,
    },
    Ready {
        update: AvailableUpdate,
        package: PathBuf,
    },
    Current,
    Failed(AppUpdateError),
    InstallFailed(String),
}

pub(crate) struct UpdateStore {
    state: AppUpdateState,
    dismissed_version: Option<String>,
}

struct GlobalUpdater(Entity<UpdateStore>);

impl Global for GlobalUpdater {}

/// Initializes the app-wide update store once at startup.
pub(crate) fn init(cx: &mut App) -> Entity<UpdateStore> {
    let entity = cx.new(|_| UpdateStore::new());
    cx.set_global(GlobalUpdater(entity.clone()));
    entity
}

/// Returns the app-wide update store, if initialized.
pub(crate) fn entity(cx: &App) -> Option<Entity<UpdateStore>> {
    cx.try_global::<GlobalUpdater>()
        .map(|global| global.0.clone())
}

impl UpdateStore {
    pub(crate) fn new() -> Self {
        Self {
            state: AppUpdateState::Idle,
            dismissed_version: None,
        }
    }

    pub(crate) fn state(&self) -> &AppUpdateState {
        &self.state
    }

    /// Version badge shown in Settings navigation when an update is actionable.
    pub(crate) fn badge_version(&self) -> Option<&str> {
        match &self.state {
            AppUpdateState::Available(update) | AppUpdateState::Ready { update, .. } => {
                Some(update.version.as_str())
            }
            AppUpdateState::Idle
            | AppUpdateState::Checking
            | AppUpdateState::Downloading { .. }
            | AppUpdateState::Current
            | AppUpdateState::Failed(_)
            | AppUpdateState::InstallFailed(_) => None,
        }
    }

    /// Whether the panel's top bar currently has a visible update pill.
    #[cfg(test)]
    pub(crate) fn has_visible_header_pill(&self) -> bool {
        match &self.state {
            AppUpdateState::Available(update) | AppUpdateState::Ready { update, .. } => {
                !self.is_banner_dismissed(&update.version)
            }
            AppUpdateState::Downloading { .. } => true,
            AppUpdateState::Idle
            | AppUpdateState::Checking
            | AppUpdateState::Current
            | AppUpdateState::Failed(_)
            | AppUpdateState::InstallFailed(_) => false,
        }
    }

    pub(crate) fn is_banner_dismissed(&self, version: &str) -> bool {
        self.dismissed_version.as_deref() == Some(version)
    }

    /// Triggers an update check automatically when no update is already known.
    pub(crate) fn maybe_auto_check(&mut self, cx: &mut Context<Self>) {
        if std::env::var_os("MANIS_POCKET_DISABLE_UPDATE_CHECK").is_some() {
            return;
        }
        if matches!(
            self.state,
            AppUpdateState::Idle | AppUpdateState::Current | AppUpdateState::Failed(_)
        ) {
            self.check_for_update(cx);
        }
    }

    pub(crate) fn check_for_update(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.state,
            AppUpdateState::Checking | AppUpdateState::Downloading { .. }
        ) {
            return;
        }
        self.dismissed_version = None;
        let previous = std::mem::replace(&mut self.state, AppUpdateState::Checking);
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let checked = executor
                .spawn(async { app_update::check_for_update() })
                .await;
            this.update(cx, |this, cx| {
                this.finish_check(checked, previous);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn dismiss_banner(&mut self, cx: &mut Context<Self>) {
        let version = match &self.state {
            AppUpdateState::Available(update) | AppUpdateState::Ready { update, .. } => {
                Some(update.version.clone())
            }
            AppUpdateState::Idle
            | AppUpdateState::Checking
            | AppUpdateState::Downloading { .. }
            | AppUpdateState::Current
            | AppUpdateState::Failed(_)
            | AppUpdateState::InstallFailed(_) => None,
        };
        if let Some(version) = version {
            self.dismissed_version = Some(version);
            cx.notify();
        }
    }

    pub(crate) fn start_download(&mut self, cx: &mut Context<Self>) {
        let AppUpdateState::Available(update) = self.state.clone() else {
            return;
        };
        if !app_update::supports_self_install() {
            open_url(&update.release_url);
            return;
        }
        self.dismissed_version = None;
        self.state = AppUpdateState::Downloading {
            update: update.clone(),
            progress: DownloadProgress::default(),
        };
        cx.notify();

        let executor = cx.background_executor().clone();
        let timer = executor.clone();
        let shared = Arc::new(SharedDownloadProgress::new());
        let progress_writer = Arc::clone(&shared);
        let download = update.clone();

        cx.spawn(async move |this, cx| {
            let download_task = executor.spawn(async move {
                let result = app_update::download_package_with_progress(&download, |progress| {
                    progress_writer.store(progress);
                });
                progress_writer.mark_finished();
                result
            });

            let mut last_seen = DownloadProgress::default();
            while !shared.is_finished() {
                timer.timer(DOWNLOAD_POLL_INTERVAL).await;
                let latest = shared.load();
                if latest != last_seen {
                    last_seen = latest;
                    let still_alive = this
                        .update(cx, |this, cx| {
                            if let AppUpdateState::Downloading { progress, .. } = &mut this.state {
                                *progress = latest;
                                cx.notify();
                            }
                        })
                        .is_ok();
                    if !still_alive {
                        break;
                    }
                }
            }

            let package = download_task.await;
            this.update(cx, |this, cx| {
                this.finish_download(update, package);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn install_update(&mut self, cx: &mut Context<Self>) {
        let AppUpdateState::Ready { package, .. } = self.state.clone() else {
            return;
        };
        match app_update::install_package(&package) {
            Ok(()) => {
                cx.quit();
            }
            Err(message) => {
                eprintln!("[update] install failed: {message}");
                self.state = AppUpdateState::InstallFailed(message);
                cx.notify();
            }
        }
    }

    fn finish_check(
        &mut self,
        result: Result<Option<AvailableUpdate>, AppUpdateError>,
        previous: AppUpdateState,
    ) {
        self.state = match result {
            Ok(Some(update)) => match previous {
                AppUpdateState::Ready {
                    update: ready,
                    package,
                } if ready.version == update.version => AppUpdateState::Ready {
                    update: ready,
                    package,
                },
                _ => AppUpdateState::Available(update),
            },
            Ok(None) => AppUpdateState::Current,
            Err(error) => {
                eprintln!("[update] check failed: {error}");
                // Keep a previously discovered release visible across transient errors.
                match previous {
                    AppUpdateState::Available(_) | AppUpdateState::Ready { .. } => previous,
                    AppUpdateState::Idle
                    | AppUpdateState::Checking
                    | AppUpdateState::Current
                    | AppUpdateState::Downloading { .. }
                    | AppUpdateState::InstallFailed(_)
                    | AppUpdateState::Failed(_) => AppUpdateState::Failed(error),
                }
            }
        };
    }

    fn finish_download(
        &mut self,
        update: AvailableUpdate,
        result: Result<PathBuf, AppUpdateError>,
    ) {
        self.state = match result {
            Ok(package) => {
                self.dismissed_version = None;
                AppUpdateState::Ready { update, package }
            }
            Err(error) => {
                eprintln!("[update] download failed: {error}");
                AppUpdateState::Failed(error)
            }
        };
    }

    pub(crate) fn status_text(&self) -> String {
        match &self.state {
            AppUpdateState::Idle => String::new(),
            AppUpdateState::Checking => "正在检查新版本…".to_owned(),
            AppUpdateState::Available(update) => {
                if app_update::supports_self_install() {
                    format!("发现新版本 {}，可直接下载", update.version)
                } else {
                    format!("发现新版本 {}，可前往 GitHub 下载", update.version)
                }
            }
            AppUpdateState::Downloading { progress, .. } => progress.label(),
            AppUpdateState::Ready { update, .. } => {
                format!("新版本 {} 已下载，安装后重启即可生效", update.version)
            }
            AppUpdateState::Current => "Manis Pocket 已是最新版本".to_owned(),
            AppUpdateState::Failed(error) => error.user_message().to_owned(),
            AppUpdateState::InstallFailed(message) => format!("安装更新失败：{message}"),
        }
    }
}

/// Opens a URL in the system's default browser.
pub(crate) fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    if let Err(error) = std::process::Command::new(opener).arg(url).spawn() {
        eprintln!("could not open {url}: {error}");
    }
}

fn app_icon_image() -> Option<Arc<RenderImage>> {
    static ICON: OnceLock<Option<Arc<RenderImage>>> = OnceLock::new();
    ICON.get_or_init(|| {
        crate::icons::png_to_render_image(include_bytes!("../assets/app-icon.png"), 72)
            .map(|(image, _, _)| image)
    })
    .clone()
}

/// Renders the Manis-style top bar update pill (Available / Downloading / Ready).
pub(crate) fn header_update_pill(store: &Entity<UpdateStore>, cx: &App) -> Option<AnyElement> {
    let updater = store.read(cx);
    match updater.state() {
        AppUpdateState::Available(update) if !updater.is_banner_dismissed(&update.version) => {
            let self_install = app_update::supports_self_install();
            let tooltip = if self_install {
                format!("点击下载并更新到 {}", update.version)
            } else {
                format!("发现新版本 {}，点击前往 GitHub", update.version)
            };
            Some(header_action_pill(
                store.clone(),
                "更新",
                tooltip,
                false,
                self_install,
                update.release_url.clone(),
            ))
        }
        AppUpdateState::Downloading { update, progress } => {
            Some(header_progress_pill(&update.version, *progress))
        }
        AppUpdateState::Ready { update, .. } if !updater.is_banner_dismissed(&update.version) => {
            let tooltip = format!("重启以更新到 {}", update.version);
            Some(header_action_pill(
                store.clone(),
                "安装更新",
                tooltip,
                true,
                true,
                update.release_url.clone(),
            ))
        }
        AppUpdateState::Idle
        | AppUpdateState::Checking
        | AppUpdateState::Available(_)
        | AppUpdateState::Ready { .. }
        | AppUpdateState::Current
        | AppUpdateState::Failed(_)
        | AppUpdateState::InstallFailed(_) => None,
    }
}

fn header_action_pill(
    store: Entity<UpdateStore>,
    label: &'static str,
    tooltip: String,
    ready_to_install: bool,
    self_install: bool,
    release_url: String,
) -> AnyElement {
    let click_store = store.clone();
    let dismiss_store = store;

    let pill = div()
        .id("header-update-action")
        .cursor_pointer()
        .h(px(24.0))
        .px(px(10.0))
        .rounded_full()
        .bg(rgb(ACCENT_BLUE))
        .hover(|style| style.bg(rgb(ACCENT_BLUE_HOVER)))
        .active(|style| style.bg(rgb(ACCENT_BLUE_ACTIVE)))
        .text_color(gpui::white())
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .flex()
        .items_center()
        .gap(px(4.0))
        .whitespace_nowrap()
        .child(Icon::new(LucideIcon::ArrowUp).size(px(12.0)))
        .child(label)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(move |_, _, cx| {
            if ready_to_install {
                click_store.update(cx, |updater, cx| updater.install_update(cx));
            } else if self_install {
                click_store.update(cx, |updater, cx| updater.start_download(cx));
            } else {
                open_url(&release_url);
            }
        });

    let dismiss = div()
        .id("header-update-dismiss")
        .occlude()
        .cursor_pointer()
        .absolute()
        .top(px(-5.0))
        .right(px(-5.0))
        .size(px(15.0))
        .rounded_full()
        .bg(gpui::white())
        .border_1()
        .border_color(gpui::black().opacity(0.16))
        .shadow_xs()
        .text_color(gpui::black().opacity(0.55))
        .hover(|style| {
            style
                .bg(rgb(0xF2F2F4))
                .text_color(gpui::black().opacity(0.85))
        })
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(LucideIcon::X).size(px(8.0)))
        .tooltip(|window, cx| Tooltip::new("稍后提醒").build(window, cx))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            dismiss_store.update(cx, |updater, cx| updater.dismiss_banner(cx));
        });

    div()
        .relative()
        .flex_shrink_0()
        .child(pill)
        .child(dismiss)
        .into_any_element()
}

fn header_progress_pill(version: &str, progress: DownloadProgress) -> AnyElement {
    let label = progress.label();
    let tooltip = format!("正在下载 Manis Pocket {version}");

    div()
        .id("header-update-progress")
        .relative()
        .overflow_hidden()
        .flex_shrink_0()
        .h(px(24.0))
        .px(px(10.0))
        .rounded_full()
        .bg(rgb(ACCENT_BLUE).opacity(0.10))
        .border_1()
        .border_color(rgb(ACCENT_BLUE).opacity(0.32))
        .text_color(rgb(ACCENT_BLUE))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .flex()
        .items_center()
        .when_some(progress.percent(), |el, percent| {
            el.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(relative(f32::from(percent) / 100.0))
                    .bg(rgb(ACCENT_BLUE).opacity(0.14)),
            )
        })
        .child(
            div()
                .relative()
                .flex()
                .items_center()
                .gap(px(4.0))
                .whitespace_nowrap()
                .child(Icon::new(LucideIcon::ArrowUp).size(px(12.0)))
                .child(label),
        )
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .into_any_element()
}

fn status_color(state: &AppUpdateState) -> gpui::Hsla {
    match state {
        AppUpdateState::Available(_) | AppUpdateState::Downloading { .. } => {
            rgb(ACCENT_BLUE).into()
        }
        AppUpdateState::Ready { .. } | AppUpdateState::Current => rgb(STATUS_SUCCESS).into(),
        AppUpdateState::Failed(_) | AppUpdateState::InstallFailed(_) => rgb(STATUS_ERROR).into(),
        AppUpdateState::Idle | AppUpdateState::Checking => gpui::black().opacity(0.45),
    }
}

fn render_check_trigger(
    id: &'static str,
    store: Entity<UpdateStore>,
    state: &AppUpdateState,
) -> AnyElement {
    let is_checking = matches!(state, AppUpdateState::Checking);
    let is_downloading = matches!(state, AppUpdateState::Downloading { .. });

    div()
        .id(id)
        .size(px(24.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .text_color(gpui::black().opacity(0.55))
        .when(is_checking, |el| {
            el.child(
                Spinner::new()
                    .icon(LucideIcon::RefreshCw)
                    .xsmall()
                    .color(rgb(ACCENT_BLUE).into()),
            )
        })
        .when(is_downloading, |el| {
            el.opacity(0.4)
                .child(Icon::new(LucideIcon::RefreshCw).size(px(13.0)))
        })
        .when(!is_checking && !is_downloading, |el| {
            el.cursor_pointer()
                .hover(|style| {
                    style
                        .bg(gpui::black().opacity(0.06))
                        .text_color(gpui::black().opacity(0.85))
                })
                .tooltip(|window, cx| Tooltip::new("检查更新").build(window, cx))
                .on_click(move |_, _, cx| {
                    store.update(cx, |updater, cx| updater.check_for_update(cx));
                })
                .child(Icon::new(LucideIcon::RefreshCw).size(px(13.0)))
        })
        .into_any_element()
}

fn render_install_action(
    id_prefix: &'static str,
    store: Entity<UpdateStore>,
    state: &AppUpdateState,
) -> Option<AnyElement> {
    match state {
        AppUpdateState::Available(update) => {
            let self_install = app_update::supports_self_install();
            let release_url = update.release_url.clone();
            let (icon, label) = if self_install {
                (LucideIcon::ArrowDown, "下载")
            } else {
                (LucideIcon::ExternalLink, "前往 GitHub")
            };
            Some(
                div()
                    .id(SharedString::from(format!("{id_prefix}-download")))
                    .cursor_pointer()
                    .h(px(28.0))
                    .px(px(12.0))
                    .rounded(px(7.0))
                    .bg(rgb(ACCENT_BLUE))
                    .hover(|style| style.bg(rgb(ACCENT_BLUE_HOVER)))
                    .active(|style| style.bg(rgb(ACCENT_BLUE_ACTIVE)))
                    .text_color(gpui::white())
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .whitespace_nowrap()
                    .child(Icon::new(icon).size(px(12.0)))
                    .child(label)
                    .on_click(move |_, _, cx| {
                        if self_install {
                            store.update(cx, |updater, cx| updater.start_download(cx));
                        } else {
                            open_url(&release_url);
                        }
                    })
                    .into_any_element(),
            )
        }
        AppUpdateState::Downloading { progress, .. } => Some(
            div()
                .id(SharedString::from(format!("{id_prefix}-downloading")))
                .relative()
                .overflow_hidden()
                .h(px(28.0))
                .px(px(12.0))
                .rounded(px(7.0))
                .bg(rgb(ACCENT_BLUE).opacity(0.10))
                .border_1()
                .border_color(rgb(ACCENT_BLUE).opacity(0.28))
                .text_color(rgb(ACCENT_BLUE))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .flex()
                .items_center()
                .when_some(progress.percent(), |el, percent| {
                    el.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(relative(f32::from(percent) / 100.0))
                            .bg(rgb(ACCENT_BLUE).opacity(0.14)),
                    )
                })
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .whitespace_nowrap()
                        .child(Icon::new(LucideIcon::ArrowUp).size(px(12.0)))
                        .child(progress.label()),
                )
                .into_any_element(),
        ),
        AppUpdateState::Ready { .. } => Some(
            div()
                .id(SharedString::from(format!("{id_prefix}-install")))
                .cursor_pointer()
                .h(px(28.0))
                .px(px(12.0))
                .rounded(px(7.0))
                .bg(rgb(ACCENT_BLUE))
                .hover(|style| style.bg(rgb(ACCENT_BLUE_HOVER)))
                .active(|style| style.bg(rgb(ACCENT_BLUE_ACTIVE)))
                .text_color(gpui::white())
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .flex()
                .items_center()
                .gap(px(5.0))
                .whitespace_nowrap()
                .child(Icon::new(LucideIcon::Check).size(px(12.0)))
                .child("安装并重启")
                .on_click(move |_, _, cx| {
                    store.update(cx, |updater, cx| updater.install_update(cx));
                })
                .into_any_element(),
        ),
        AppUpdateState::Idle
        | AppUpdateState::Checking
        | AppUpdateState::Current
        | AppUpdateState::Failed(_)
        | AppUpdateState::InstallFailed(_) => None,
    }
}

/// Renders the Manis-style "关于与更新" card for the Settings window.
pub(crate) fn render_updates_panel(store: &Entity<UpdateStore>, cx: &App) -> AnyElement {
    let updater = store.read(cx);
    let state = updater.state().clone();
    let status = updater.status_text();
    let color = status_color(&state);

    let brand_mark = div()
        .size(px(40.0))
        .flex_shrink_0()
        .rounded(px(10.0))
        .overflow_hidden()
        .bg(rgb(0xF5F5F7))
        .p(px(3.0))
        .flex()
        .items_center()
        .justify_center()
        .when_some(app_icon_image(), |el, icon| el.child(img(icon).size_full()));

    let version_row = h_flex()
        .w_full()
        .min_h(px(72.0))
        .px(px(14.0))
        .py(px(12.0))
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(
            h_flex()
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .gap(px(12.0))
                .child(brand_mark)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(3.0))
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Manis Pocket"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(gpui::black().opacity(0.5))
                                        .child(format!(
                                            "当前版本 · {}",
                                            app_update::current_version()
                                        )),
                                )
                                .child(render_check_trigger(
                                    "settings-update-check",
                                    store.clone(),
                                    &state,
                                )),
                        )
                        .child(
                            div()
                                .min_h(px(16.0))
                                .text_xs()
                                .text_color(color)
                                .child(status),
                        ),
                ),
        )
        .children(render_install_action(
            "settings-update",
            store.clone(),
            &state,
        ));

    let github_row = h_flex()
        .id("settings-update-github")
        .w_full()
        .px(px(14.0))
        .py(px(10.0))
        .items_center()
        .justify_between()
        .cursor_pointer()
        .hover(|style| style.bg(gpui::black().opacity(0.03)))
        .on_click(|_, _, _| open_url(app_update::REPOSITORY_URL))
        .child(
            v_flex()
                .gap(px(2.0))
                .child(div().text_sm().child("前往 GitHub"))
                .child(
                    div()
                        .text_xs()
                        .text_color(gpui::black().opacity(0.45))
                        .child(app_update::REPOSITORY_URL),
                ),
        )
        .child(
            Icon::new(LucideIcon::ExternalLink)
                .size(px(14.0))
                .text_color(gpui::black().opacity(0.45)),
        );

    let metadata_row = h_flex()
        .w_full()
        .px(px(14.0))
        .py(px(10.0))
        .items_center()
        .justify_between()
        .text_xs()
        .text_color(gpui::black().opacity(0.45))
        .child(format!(
            "平台：{} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
        .child("开源协议：MIT");

    let divider = || {
        div()
            .mx(px(14.0))
            .h(px(1.0))
            .bg(gpui::black().opacity(0.08))
    };

    v_flex()
        .w_full()
        .rounded(px(10.0))
        .bg(rgb(0xFAFAFB))
        .border_1()
        .border_color(gpui::black().opacity(0.09))
        .overflow_hidden()
        .child(version_row)
        .child(divider())
        .child(github_row)
        .child(divider())
        .child(metadata_row)
        .into_any_element()
}

/// Renders a compact update status & action strip inside the About window.
pub(crate) fn render_about_update_strip(store: &Entity<UpdateStore>, cx: &App) -> AnyElement {
    let updater = store.read(cx);
    let state = updater.state().clone();
    let status = updater.status_text();
    let color = status_color(&state);

    h_flex()
        .w_full()
        .mt_2()
        .p(px(10.0))
        .rounded(px(8.0))
        .bg(rgb(0xF6F6F8))
        .border_1()
        .border_color(gpui::black().opacity(0.08))
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .child(
            h_flex()
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .gap(px(6.0))
                .child(render_check_trigger(
                    "about-update-check",
                    store.clone(),
                    &state,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(color)
                        .child(if status.is_empty() {
                            "点击检查更新".to_owned()
                        } else {
                            status
                        }),
                ),
        )
        .children(render_install_action("about-update", store.clone(), &state))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_update(version: &str) -> AvailableUpdate {
        AvailableUpdate {
            version: version.to_owned(),
            tag_name: format!("build-{version}"),
            release_url: format!(
                "{}/releases/tag/build-{version}",
                app_update::REPOSITORY_URL
            ),
            asset: None,
        }
    }

    #[test]
    fn check_failure_preserves_previously_discovered_or_ready_update() {
        let mut store = UpdateStore::new();
        let update = sample_update("2.7.150");
        store.finish_check(
            Err(AppUpdateError::NetworkUnavailable),
            AppUpdateState::Available(update.clone()),
        );
        assert_eq!(store.state, AppUpdateState::Available(update.clone()));

        let ready = AppUpdateState::Ready {
            update: update.clone(),
            package: PathBuf::from("/tmp/ManisPocket.app.zip"),
        };
        store.finish_check(Err(AppUpdateError::NetworkUnavailable), ready.clone());
        assert_eq!(store.state, ready);
    }

    #[test]
    fn rechecking_same_version_keeps_downloaded_package_ready() {
        let mut store = UpdateStore::new();
        let update = sample_update("2.7.150");
        let ready = AppUpdateState::Ready {
            update: update.clone(),
            package: PathBuf::from("/tmp/ManisPocket.app.zip"),
        };
        store.finish_check(Ok(Some(update)), ready.clone());
        assert_eq!(store.state, ready);
    }

    #[test]
    fn dismissing_banner_hides_pill_without_clearing_state() {
        let mut store = UpdateStore::new();
        let update = sample_update("2.7.150");
        store.state = AppUpdateState::Available(update.clone());
        assert!(store.has_visible_header_pill());

        store.dismissed_version = Some(update.version.clone());
        assert!(!store.has_visible_header_pill());
        assert_eq!(store.badge_version(), Some("2.7.150"));
        assert_eq!(store.state, AppUpdateState::Available(update));
    }
}
