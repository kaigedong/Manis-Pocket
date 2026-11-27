//! Standalone floating image preview window.
//!
//! Hovering an image thumbnail opens this non-activating popup window beside
//! the history panel instead of overlaying neighboring rows inside the panel.
//! That keeps vertical pointer movement across the clipboard list completely
//! unobstructed and lets screenshots render much larger than the 450 pt panel.

use std::sync::Arc;

use gpui_kit as gpui;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, DisplayId, Entity, Global,
    InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, RenderImage,
    SharedString, StatefulInteractiveElement, Styled, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowId, WindowKind, WindowOptions, div, point, px,
    size,
};
use manis_pocket_core::history::HistoryManager;

use super::{copy_entry_to_clipboard, zoom};

pub(crate) struct PreviewRequest {
    pub id: String,
    pub fit: Arc<RenderImage>,
    pub detail: Option<Arc<RenderImage>>,
    pub native_size: (u32, u32),
    pub layout: zoom::PreviewWindowLayout,
    pub display_id: Option<DisplayId>,
    pub history: Arc<HistoryManager>,
    pub on_leave: Arc<dyn Fn(&mut App)>,
}

struct ActivePreview {
    handle: AnyWindowHandle,
    view: Entity<ImagePreviewView>,
    id: String,
    hovered: bool,
    bounds: Bounds<Pixels>,
}

#[derive(Default)]
struct PreviewWindowState {
    active: Option<ActivePreview>,
}

impl Global for PreviewWindowState {}

fn ensure_state(cx: &mut App) {
    if !cx.has_global::<PreviewWindowState>() {
        cx.set_global(PreviewWindowState::default());
    }
}

pub(crate) fn is_open(cx: &App) -> bool {
    cx.try_global::<PreviewWindowState>()
        .and_then(|state| state.active.as_ref())
        .is_some()
}

pub(crate) fn is_hovered(cx: &App) -> bool {
    cx.try_global::<PreviewWindowState>()
        .and_then(|state| state.active.as_ref())
        .is_some_and(|active| active.hovered)
}

pub(crate) fn current_id(cx: &App) -> Option<String> {
    cx.try_global::<PreviewWindowState>()
        .and_then(|state| state.active.as_ref())
        .map(|active| active.id.clone())
}

fn set_hovered(window_id: WindowId, hovered: bool, cx: &mut App) {
    if !cx.has_global::<PreviewWindowState>() {
        return;
    }
    if let Some(active) = cx.global_mut::<PreviewWindowState>().active.as_mut()
        && active.handle.window_id() == window_id
    {
        active.hovered = hovered;
    }
}

/// Clears tracked state if the active preview window closed externally.
pub(crate) fn on_window_closed(cx: &mut App, closed_id: WindowId) {
    if !cx.has_global::<PreviewWindowState>() {
        return;
    }
    let state = cx.global_mut::<PreviewWindowState>();
    if state
        .active
        .as_ref()
        .is_some_and(|active| active.handle.window_id() == closed_id)
    {
        state.active = None;
    }
}

/// Closes the floating image preview window if it is open.
pub(crate) fn close(cx: &mut App) {
    if !cx.has_global::<PreviewWindowState>() {
        return;
    }
    let previous = cx.global_mut::<PreviewWindowState>().active.take();
    if let Some(active) = previous {
        let _ = active.handle.update(cx, |_, window, _| {
            window.remove_window();
        });
    }
}

fn bounds_close(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    (a.origin.x.as_f32() - b.origin.x.as_f32()).abs() < 2.0
        && (a.origin.y.as_f32() - b.origin.y.as_f32()).abs() < 2.0
        && (a.size.width.as_f32() - b.size.width.as_f32()).abs() < 2.0
        && (a.size.height.as_f32() - b.size.height.as_f32()).abs() < 2.0
}

/// Opens or updates the standalone floating image preview window.
pub(crate) fn show(request: PreviewRequest, cx: &mut App) {
    ensure_state(cx);

    // If the same entry is already shown at the same geometry (e.g. upgrading
    // from the fit texture to the sharp detail decode), update in place.
    let existing = cx
        .try_global::<PreviewWindowState>()
        .and_then(|state| state.active.as_ref())
        .filter(|active| {
            active.id == request.id && bounds_close(active.bounds, request.layout.window_bounds)
        })
        .map(|active| active.view.clone());

    if let Some(view) = existing {
        let fit = request.fit.clone();
        let detail = request.detail.clone();
        let native_size = request.native_size;
        let layout = request.layout;
        view.update(cx, |view, cx| {
            view.fit = fit;
            if detail.is_some() {
                view.detail = detail;
            }
            view.native_size = native_size;
            view.layout = layout;
            cx.notify();
        });
        return;
    }

    let previous = cx.global_mut::<PreviewWindowState>().active.take();

    let id = request.id.clone();
    let window_bounds = request.layout.window_bounds;
    let opened = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(window_bounds)),
            display_id: request.display_id,
            // Non-activating popup: stays above the panel without stealing key
            // focus so search typing, arrow navigation, and Esc keep working.
            kind: WindowKind::PopUp,
            focus: false,
            show: true,
            window_background: WindowBackgroundAppearance::Transparent,
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("图片预览")),
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
        |_window, cx| {
            cx.new(|_| ImagePreviewView {
                id: request.id,
                fit: request.fit,
                detail: request.detail,
                native_size: request.native_size,
                layout: request.layout,
                history: request.history,
                zoom_factor: zoom::PREVIEW_MIN_FACTOR,
                pointer: None,
                probe: zoom::BoundsCell::default(),
                on_leave: request.on_leave,
            })
        },
    );

    if let Ok((handle, view)) = opened {
        cx.global_mut::<PreviewWindowState>().active = Some(ActivePreview {
            handle,
            view,
            id,
            hovered: false,
            bounds: window_bounds,
        });
    }

    if let Some(old) = previous {
        let _ = old.handle.update(cx, |_, window, _| {
            window.remove_window();
        });
    }
}

pub(crate) struct ImagePreviewView {
    id: String,
    fit: Arc<RenderImage>,
    detail: Option<Arc<RenderImage>>,
    native_size: (u32, u32),
    layout: zoom::PreviewWindowLayout,
    history: Arc<HistoryManager>,
    zoom_factor: f32,
    pointer: Option<Point<Pixels>>,
    probe: zoom::BoundsCell,
    on_leave: Arc<dyn Fn(&mut App)>,
}

impl Render for ImagePreviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let wanted = self.layout.window_bounds.size;
        let current = window.viewport_size();
        if (current.width.as_f32() - wanted.width.as_f32()).abs() > 0.5
            || (current.height.as_f32() - wanted.height.as_f32()).abs() > 0.5
        {
            window.resize(size(wanted.width, wanted.height));
        }

        let zoom_state = (self.zoom_factor > 1.01).then(|| {
            let center = point(
                px(self.layout.image_width / 2.0),
                px(self.layout.image_height / 2.0),
            );
            (self.pointer.unwrap_or(center), self.zoom_factor)
        });

        let picture = zoom::picture_box(
            self.fit.clone(),
            self.detail.clone(),
            self.layout.image_width,
            self.layout.image_height,
            zoom_state,
            self.probe.clone(),
        )
        .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
            let Some(bounds) = this.probe.get() else {
                return;
            };
            let next = zoom::local_point(bounds, event.position);
            if this.pointer != Some(next) {
                this.pointer = Some(next);
                if this.zoom_factor > 1.01 {
                    cx.notify();
                }
            }
        }))
        .on_scroll(cx.listener(|this, event: &gpui::ScrollEvent, _, cx| {
            let delta = event.delta.pixel_delta(px(16.0)).y.as_f32();
            if let Some(bounds) = this.probe.get() {
                this.pointer = Some(zoom::local_point(bounds, event.position));
            }
            this.zoom_factor = zoom::adjust_preview_factor(this.zoom_factor, delta * 0.006);
            cx.stop_propagation();
            cx.notify();
        }))
        .on_pinch(cx.listener(|this, event: &gpui::PinchEvent, _, cx| {
            if let Some(bounds) = this.probe.get() {
                this.pointer = Some(zoom::local_point(bounds, event.position));
            }
            this.zoom_factor = zoom::adjust_preview_factor(this.zoom_factor, event.delta);
            cx.stop_propagation();
            cx.notify();
        }));

        let (native_w, native_h) = self.native_size;
        let caption = if self.zoom_factor > 1.01 {
            format!(
                "{:.1}× · {native_w} × {native_h} · 滚轮缩放 · 点击复制",
                self.zoom_factor
            )
        } else {
            format!("{native_w} × {native_h} · 滚轮缩放 · 点击复制")
        };

        let copy_id = self.id.clone();
        let history = self.history.clone();
        let on_leave = self.on_leave.clone();

        div().size_full().p(px(zoom::PREVIEW_WINDOW_MARGIN)).child(
            v_flex()
                .id("floating-image-preview")
                .w(px(self.layout.card_width))
                .h(px(self.layout.card_height))
                .p(px(zoom::PREVIEW_CARD_PADDING))
                .rounded(px(10.0))
                .bg(gpui::white())
                .border_1()
                .border_color(gpui::black().opacity(0.12))
                .shadow_lg()
                .overflow_hidden()
                .cursor_pointer()
                .items_center()
                .justify_between()
                .on_hover(move |hovered, window, cx| {
                    let hovered = *hovered;
                    set_hovered(window.window_handle().window_id(), hovered, cx);
                    if !hovered {
                        on_leave(cx);
                    }
                })
                .on_click(move |_, _window, cx| {
                    if copy_entry_to_clipboard(&history, &copy_id) {
                        close(cx);
                        crate::close_panel_if_open(cx);
                    }
                })
                .child(picture)
                .child(
                    h_flex()
                        .w_full()
                        .h(px(zoom::PREVIEW_FOOTER_HEIGHT))
                        .pt(px(4.0))
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(gpui::black().opacity(0.45))
                        .child(caption),
                ),
        )
    }
}
