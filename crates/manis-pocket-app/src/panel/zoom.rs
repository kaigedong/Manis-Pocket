//! Image preview window geometry and pointer-anchored magnification.
//!
//! A row thumbnail is 22 pt tall, too small to tell two screenshots apart. Two
//! surfaces show the full picture without obstructing the list:
//!
//! * hovering a thumbnail opens a separate floating preview window beside the
//!   panel (never over neighboring rows), sized generously to the free screen
//!   space;
//! * inside that preview window (with scroll/pinch) and on the `⌃Space` preview
//!   sidebar, the picture can be magnified so the pixel under the pointer stays
//!   under the pointer.
//!
//! The geometry lives here as plain functions so it can be tested without a
//! window; the panel and preview window own the state and event wiring.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{Bounds, ParentElement, Pixels, Point, Styled, div, point, px, size};
use gpui_kit::{RenderImage, img};

/// A decoded picture and the size, in points, it is meant to be drawn at.
pub(crate) type Picture = (Arc<RenderImage>, f32, f32);

/// Where a measured element landed in the window, filled in during prepaint.
pub(crate) type BoundsCell = Rc<Cell<Option<Bounds<Pixels>>>>;

/// Widest decode kept for magnification. A 5K screenshot stays sharp at native
/// size, and anything wider is downscaled rather than uploaded whole.
pub(crate) const DETAIL_MAX_WIDTH: u32 = 4096;
/// Widest fit decode used for the standalone preview window.
pub(crate) const PREVIEW_FIT_MAX_WIDTH: u32 = 1400;
/// Zoom range for the sidebar magnifier. Below 1.5× the magnifier barely moves.
pub(crate) const MIN_FACTOR: f32 = 1.5;
/// Zoom floor for the standalone preview window (`1.0` = full sharp fit).
pub(crate) const PREVIEW_MIN_FACTOR: f32 = 1.0;
pub(crate) const MAX_FACTOR: f32 = 8.0;

/// Transparent outer margin around the floating preview card for its shadow.
pub(crate) const PREVIEW_WINDOW_MARGIN: f32 = 8.0;
/// Inner padding of the floating preview card around the picture.
pub(crate) const PREVIEW_CARD_PADDING: f32 = 8.0;
/// Subtle hint footer height inside the floating preview card.
pub(crate) const PREVIEW_FOOTER_HEIGHT: f32 = 24.0;
/// Separation kept between the history panel card and the preview card.
pub(crate) const PREVIEW_WINDOW_GAP: f32 = 10.0;
/// Minimum width of the preview card so the footer hint remains legible.
const PREVIEW_MIN_CARD_WIDTH: f32 = 200.0;
/// Safe margin from the display's usable edges (menu bar / Dock / screen edge).
const SCREEN_EDGE_GAP: f32 = 12.0;
/// Generous cap so huge screenshots stay comfortable without swallowing the display.
const PREVIEW_MAX_IMAGE_WIDTH: f32 = 960.0;
const PREVIEW_MAX_IMAGE_HEIGHT: f32 = 760.0;

/// Screen-space geometry for the standalone floating image preview window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreviewWindowLayout {
    pub window_bounds: Bounds<Pixels>,
    pub card_width: f32,
    pub card_height: f32,
    pub image_width: f32,
    pub image_height: f32,
}

/// Computes bounds for the floating preview window strictly beside `panel_window`.
///
/// The preview window is placed on whichever side of the history panel (left or
/// right) has more usable room, separated by `PREVIEW_WINDOW_GAP` so vertical
/// pointer movement across the clipboard list never touches it.
pub(crate) fn preview_window_layout(
    panel_window: Bounds<Pixels>,
    panel_margin: f32,
    anchor_in_panel: Bounds<Pixels>,
    picture: (f32, f32),
    visible: Bounds<Pixels>,
) -> PreviewWindowLayout {
    let (pic_w, pic_h) = (picture.0.max(1.0), picture.1.max(1.0));

    let vis_left = visible.origin.x.as_f32();
    let vis_top = visible.origin.y.as_f32();
    let vis_w = visible.size.width.as_f32().max(320.0);
    let vis_h = visible.size.height.as_f32().max(240.0);
    let vis_right = vis_left + vis_w;
    let vis_bottom = vis_top + vis_h;

    let card_left = panel_window.origin.x.as_f32() + panel_margin;
    let card_right =
        panel_window.origin.x.as_f32() + panel_window.size.width.as_f32() - panel_margin;

    let room_left = (card_left - PREVIEW_WINDOW_GAP - (vis_left + SCREEN_EDGE_GAP)).max(0.0);
    let room_right = ((vis_right - SCREEN_EDGE_GAP) - (card_right + PREVIEW_WINDOW_GAP)).max(0.0);
    let place_left = room_left >= room_right;
    let side_room = if place_left { room_left } else { room_right };

    let chrome_x = PREVIEW_CARD_PADDING * 2.0;
    let chrome_y = PREVIEW_CARD_PADDING * 2.0 + PREVIEW_FOOTER_HEIGHT;

    let max_img_w = (side_room - chrome_x)
        .min(PREVIEW_MAX_IMAGE_WIDTH)
        .min(vis_w * 0.72)
        .max(120.0);
    let max_img_h = (vis_h - SCREEN_EDGE_GAP * 2.0 - chrome_y)
        .min(PREVIEW_MAX_IMAGE_HEIGHT)
        .min(vis_h * 0.85)
        .max(90.0);

    // Never upscale past native size; users can scroll/pinch to zoom further.
    let scale = (max_img_w / pic_w).min(max_img_h / pic_h).min(1.0);
    let image_width = (pic_w * scale).max(1.0);
    let image_height = (pic_h * scale).max(1.0);

    let card_width = (image_width + chrome_x)
        .max(PREVIEW_MIN_CARD_WIDTH)
        .min((vis_w - SCREEN_EDGE_GAP * 2.0).max(PREVIEW_MIN_CARD_WIDTH));
    let card_height = image_height + chrome_y;

    let card_x = if place_left {
        (card_left - PREVIEW_WINDOW_GAP - card_width).max(vis_left + SCREEN_EDGE_GAP)
    } else {
        let min_x = vis_left + SCREEN_EDGE_GAP;
        let max_x = (vis_right - SCREEN_EDGE_GAP - card_width).max(min_x);
        (card_right + PREVIEW_WINDOW_GAP).clamp(min_x, max_x)
    };

    let anchor_center_y = panel_window.origin.y.as_f32()
        + anchor_in_panel.origin.y.as_f32()
        + anchor_in_panel.size.height.as_f32() / 2.0;
    let min_y = vis_top + SCREEN_EDGE_GAP;
    let max_y = (vis_bottom - SCREEN_EDGE_GAP - card_height).max(min_y);
    let card_y = (anchor_center_y - card_height / 2.0).clamp(min_y, max_y);

    let window_bounds = Bounds::new(
        point(
            px(card_x - PREVIEW_WINDOW_MARGIN),
            px(card_y - PREVIEW_WINDOW_MARGIN),
        ),
        size(
            px(card_width + PREVIEW_WINDOW_MARGIN * 2.0),
            px(card_height + PREVIEW_WINDOW_MARGIN * 2.0),
        ),
    );

    PreviewWindowLayout {
        window_bounds,
        card_width,
        card_height,
        image_width,
        image_height,
    }
}

/// The zoom a sidebar magnifier starts at: the picture's own pixels, one per
/// point, within the allowed range.
pub(crate) fn default_factor(native_width: f32, shown_width: f32) -> f32 {
    if shown_width <= 0.0 {
        return MIN_FACTOR;
    }
    (native_width / shown_width).clamp(2.0, 6.0)
}

/// Applies a wheel or pinch step to a sidebar zoom factor.
pub(crate) fn adjust_factor(factor: f32, step: f32) -> f32 {
    (factor * step.exp()).clamp(MIN_FACTOR, MAX_FACTOR)
}

/// Applies a wheel or pinch step to the standalone preview window's zoom factor.
pub(crate) fn adjust_preview_factor(factor: f32, step: f32) -> f32 {
    let next = (factor * step.exp()).clamp(PREVIEW_MIN_FACTOR, MAX_FACTOR);
    if next < 1.03 {
        PREVIEW_MIN_FACTOR
    } else {
        next
    }
}

/// The magnified picture's offset inside its box: the point under the pointer
/// maps to itself, which also keeps every edge of the picture outside the box.
pub(crate) fn magnified_offset(point: (f32, f32), factor: f32) -> (f32, f32) {
    (point.0 * (1.0 - factor), point.1 * (1.0 - factor))
}

/// Clamps a window-space pointer position into a box's local coordinates.
pub(crate) fn local_point(bounds: Bounds<Pixels>, position: Point<Pixels>) -> Point<Pixels> {
    let width = bounds.size.width.as_f32();
    let height = bounds.size.height.as_f32();
    let x = (position.x - bounds.origin.x).as_f32().clamp(0.0, width);
    let y = (position.y - bounds.origin.y).as_f32().clamp(0.0, height);
    point(px(x), px(y))
}

/// An element that records where it was laid out, for pointer math.
///
/// Absolutely positioned over its parent, so adding it changes no layout.
pub(crate) fn bounds_probe(cell: BoundsCell) -> impl gpui_kit::IntoElement {
    gpui_kit::canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// Draws a picture in a `width`×`height` box.
///
/// Without a magnifier the picture fills the box. With one it is drawn
/// `factor` times larger — from `detail` when that sharper decode is ready —
/// and shifted so the point under the pointer stays put.
pub(crate) fn picture_box(
    fit: Arc<RenderImage>,
    detail: Option<Arc<RenderImage>>,
    width: f32,
    height: f32,
    zoom: Option<(Point<Pixels>, f32)>,
    probe: BoundsCell,
) -> gpui_kit::Div {
    let frame = div()
        .relative()
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .rounded(px(4.0));
    let picture = match zoom {
        Some((point, factor)) if factor > 1.01 => {
            let (left, top) = magnified_offset((point.x.as_f32(), point.y.as_f32()), factor);
            img(detail.unwrap_or(fit))
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width * factor))
                .h(px(height * factor))
        }
        _ => img(fit).w(px(width)).h(px(height)),
    };
    frame.child(picture).child(bounds_probe(probe))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(width), px(height)),
        }
    }

    #[test]
    fn preview_opens_to_the_left_when_panel_is_on_the_right_edge() {
        let visible = bounds(0.0, 25.0, 1920.0, 1055.0);
        let panel = bounds(1446.0, 25.0, 466.0, 816.0);
        let anchor = bounds(32.0, 180.0, 40.0, 22.0);
        let layout = preview_window_layout(panel, 8.0, anchor, (1600.0, 900.0), visible);

        // Preview card right edge stays strictly left of the panel card's left edge.
        let preview_card_right =
            layout.window_bounds.origin.x.as_f32() + PREVIEW_WINDOW_MARGIN + layout.card_width;
        let panel_card_left = panel.origin.x.as_f32() + 8.0;
        assert!(
            (panel_card_left - preview_card_right - PREVIEW_WINDOW_GAP).abs() < 0.01,
            "preview should sit {PREVIEW_WINDOW_GAP}pt left of panel card"
        );
        // Much wider than the 450px panel!
        assert!(layout.image_width > 900.0);
        assert!((layout.image_width / layout.image_height - 16.0 / 9.0).abs() < 0.01);
    }

    #[test]
    fn preview_opens_to_the_right_when_panel_is_on_the_left_edge() {
        let visible = bounds(0.0, 25.0, 1920.0, 1055.0);
        let panel = bounds(10.0, 25.0, 466.0, 816.0);
        let anchor = bounds(32.0, 180.0, 40.0, 22.0);
        let layout = preview_window_layout(panel, 8.0, anchor, (1200.0, 800.0), visible);

        let preview_card_left = layout.window_bounds.origin.x.as_f32() + PREVIEW_WINDOW_MARGIN;
        let panel_card_right = panel.origin.x.as_f32() + panel.size.width.as_f32() - 8.0;
        assert!(
            (preview_card_left - panel_card_right - PREVIEW_WINDOW_GAP).abs() < 0.01,
            "preview should sit {PREVIEW_WINDOW_GAP}pt right of panel card"
        );
    }

    #[test]
    fn preview_clamps_vertically_inside_visible_display_bounds() {
        let visible = bounds(0.0, 25.0, 1920.0, 1055.0);
        let panel = bounds(1446.0, 25.0, 466.0, 816.0);

        let top_layout = preview_window_layout(
            panel,
            8.0,
            bounds(32.0, 10.0, 40.0, 22.0),
            (1200.0, 800.0),
            visible,
        );
        let top_card_y = top_layout.window_bounds.origin.y.as_f32() + PREVIEW_WINDOW_MARGIN;
        assert!(top_card_y >= visible.origin.y.as_f32() + SCREEN_EDGE_GAP - 0.01);

        let bottom_layout = preview_window_layout(
            panel,
            8.0,
            bounds(32.0, 780.0, 40.0, 22.0),
            (1200.0, 800.0),
            visible,
        );
        let bottom_card_bottom = bottom_layout.window_bounds.origin.y.as_f32()
            + PREVIEW_WINDOW_MARGIN
            + bottom_layout.card_height;
        assert!(
            bottom_card_bottom
                <= visible.origin.y.as_f32() + visible.size.height.as_f32() - SCREEN_EDGE_GAP
                    + 0.01
        );
    }

    #[test]
    fn small_pictures_are_not_upscaled_in_preview_window() {
        let visible = bounds(0.0, 25.0, 1920.0, 1055.0);
        let panel = bounds(1446.0, 25.0, 466.0, 816.0);
        let layout = preview_window_layout(
            panel,
            8.0,
            bounds(32.0, 200.0, 40.0, 22.0),
            (120.0, 40.0),
            visible,
        );
        assert_eq!((layout.image_width, layout.image_height), (120.0, 40.0));
        assert!(layout.card_width >= PREVIEW_MIN_CARD_WIDTH);
    }

    #[test]
    fn magnifier_keeps_the_pointed_pixel_in_place() {
        let factor = 3.0;
        let (left, top) = magnified_offset((50.0, 20.0), factor);
        assert_eq!(
            ((50.0 - left) / factor, (20.0 - top) / factor),
            (50.0, 20.0)
        );
        assert_eq!(magnified_offset((0.0, 0.0), factor), (0.0, 0.0));
    }

    #[test]
    fn zoom_factor_stays_in_range() {
        assert_eq!(default_factor(4000.0, 400.0), 6.0);
        assert_eq!(default_factor(500.0, 400.0), 2.0);
        assert_eq!(adjust_factor(7.9, 1.0), MAX_FACTOR);
        assert_eq!(adjust_factor(1.6, -1.0), MIN_FACTOR);
        assert!(adjust_factor(3.0, 0.1) > 3.0);
        assert_eq!(adjust_preview_factor(1.0, -0.5), PREVIEW_MIN_FACTOR);
        assert!(adjust_preview_factor(1.0, 0.3) > 1.0);
        assert_eq!(adjust_preview_factor(7.9, 1.0), MAX_FACTOR);
    }

    #[test]
    fn pointer_is_clamped_into_the_box() {
        let probe = bounds(100.0, 100.0, 50.0, 40.0);
        assert_eq!(
            local_point(probe, point(px(90.0), px(200.0))),
            point(px(0.0), px(40.0))
        );
        assert_eq!(
            local_point(probe, point(px(110.0), px(120.0))),
            point(px(10.0), px(20.0))
        );
    }
}
