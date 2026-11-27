//! Hover-to-zoom for image entries.
//!
//! A row thumbnail is 22 pt tall, too small to tell two screenshots apart. Two
//! things fix that:
//!
//! * hovering a thumbnail opens a card with the whole picture, placed against
//!   the thumbnail so the pointer can travel into it;
//! * inside that card, and on the preview sidebar's picture, the pointer acts as
//!   a magnifier: the picture is drawn larger and shifted so the pixel under the
//!   pointer stays under the pointer. The wheel or a pinch changes how much.
//!
//! The geometry lives here as plain functions so it can be tested without a
//! window; the panel owns the state and the event wiring.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{Bounds, ParentElement, Pixels, Point, Styled, div, px};
use gpui_kit::{RenderImage, img};

/// A decoded picture and the size, in points, it is meant to be drawn at.
pub(crate) type Picture = (Arc<RenderImage>, f32, f32);

/// Where a measured element landed in the window, filled in during prepaint.
pub(crate) type BoundsCell = Rc<Cell<Option<Bounds<Pixels>>>>;

/// Widest decode kept for the magnifier. A 5K screenshot stays sharp at native
/// size, and anything wider is downscaled rather than uploaded whole.
pub(crate) const DETAIL_MAX_WIDTH: u32 = 4096;
/// Zoom range for the wheel and pinch. Below 1.5× the magnifier barely moves.
pub(crate) const MIN_FACTOR: f32 = 1.5;
pub(crate) const MAX_FACTOR: f32 = 8.0;
/// Inner padding of the hover card, around the picture.
pub(crate) const CARD_PADDING: f32 = 6.0;
/// The hover card never grows taller than this, however much room there is.
const CARD_MAX_IMAGE_HEIGHT: f32 = 420.0;
/// Distance kept between the card and the panel's edge.
const CARD_EDGE_GAP: f32 = 8.0;

/// Which picture the pointer is magnifying.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Surface {
    /// The card opened by hovering a row thumbnail.
    Card,
    /// The picture at the top of the preview sidebar.
    Sidebar,
}

/// The pointer over a picture: where, in the picture's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Magnifier {
    pub surface: Surface,
    pub point: Point<Pixels>,
}

/// The hover card for one entry.
#[derive(Clone, Debug)]
pub(crate) struct HoverCard {
    pub id: String,
    /// The thumbnail that opened it, in window coordinates.
    pub anchor: Bounds<Pixels>,
    /// The card stays open while the pointer is on either of these.
    pub over_thumb: bool,
    pub over_card: bool,
}

/// Where the hover card goes and how large its picture is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CardLayout {
    pub left: f32,
    pub top: f32,
    pub image_width: f32,
    pub image_height: f32,
}

/// Places the hover card for a picture of `picture` size next to `anchor`.
///
/// `column` is the horizontal span the card may use (the list column) and
/// `viewport_height` the window's height. The card goes below the thumbnail
/// when the picture fits there or when there is more room below than above,
/// and touches the thumbnail so the pointer can move into it without the card
/// closing on the way.
pub(crate) fn card_layout(
    anchor: Bounds<Pixels>,
    picture: (f32, f32),
    column: (f32, f32),
    viewport_height: f32,
) -> CardLayout {
    let (picture_width, picture_height) = (picture.0.max(1.0), picture.1.max(1.0));
    let anchor_top = anchor.origin.y.as_f32();
    let anchor_bottom = anchor_top + anchor.size.height.as_f32();
    let chrome = CARD_PADDING * 2.0;

    let room_below = (viewport_height - CARD_EDGE_GAP - anchor_bottom - chrome).max(0.0);
    let room_above = (anchor_top - CARD_EDGE_GAP - chrome).max(0.0);
    let wanted = picture_height.min(CARD_MAX_IMAGE_HEIGHT);
    let below = room_below >= wanted || room_below >= room_above;
    let room = if below { room_below } else { room_above };

    let max_width = (column.1 - column.0 - chrome).max(1.0);
    let max_height = room.clamp(1.0, CARD_MAX_IMAGE_HEIGHT);
    // Never enlarge: past native size the card would only show blur, and the
    // magnifier is there for a closer look.
    let scale = (max_width / picture_width)
        .min(max_height / picture_height)
        .min(1.0);
    let image_width = picture_width * scale;
    let image_height = picture_height * scale;

    let card_width = image_width + chrome;
    let card_height = image_height + chrome;
    // Start under the thumbnail, but keep the whole card inside the column.
    // Not `clamp`: it panics when a very narrow column leaves max below min.
    let preferred_left = anchor.origin.x.as_f32() - CARD_PADDING;
    let left = preferred_left.min(column.1 - card_width).max(column.0);
    let top = if below {
        anchor_bottom
    } else {
        anchor_top - card_height
    };

    CardLayout {
        left,
        top,
        image_width,
        image_height,
    }
}

/// The zoom a magnifier starts at: the picture's own pixels, one per point,
/// within the allowed range.
pub(crate) fn default_factor(native_width: f32, shown_width: f32) -> f32 {
    if shown_width <= 0.0 {
        return MIN_FACTOR;
    }
    (native_width / shown_width).clamp(2.0, 6.0)
}

/// Applies a wheel or pinch step to a zoom factor.
///
/// `step` is positive to zoom in; the change is multiplicative so each notch
/// feels the same at 2× and at 6×.
pub(crate) fn adjust_factor(factor: f32, step: f32) -> f32 {
    (factor * step.exp()).clamp(MIN_FACTOR, MAX_FACTOR)
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
    gpui_kit::point(px(x), px(y))
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
        Some((point, factor)) => {
            let (left, top) = magnified_offset((point.x.as_f32(), point.y.as_f32()), factor);
            img(detail.unwrap_or(fit))
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width * factor))
                .h(px(height * factor))
        }
        None => img(fit).w(px(width)).h(px(height)),
    };
    frame.child(picture).child(bounds_probe(probe))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, size};

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(width), px(height)),
        }
    }

    #[test]
    fn card_opens_below_a_thumbnail_near_the_top() {
        let layout = card_layout(
            bounds(30.0, 80.0, 40.0, 22.0),
            (800.0, 600.0),
            (16.0, 450.0),
            800.0,
        );
        assert_eq!(layout.top, 102.0);
        // 434 wide column minus padding: the picture is scaled to fit.
        assert!((layout.image_width - 422.0).abs() < 0.01);
        assert!((layout.image_height - 316.5).abs() < 0.01);
    }

    #[test]
    fn card_opens_above_a_thumbnail_near_the_bottom() {
        let layout = card_layout(
            bounds(30.0, 700.0, 40.0, 22.0),
            (300.0, 200.0),
            (16.0, 450.0),
            800.0,
        );
        assert_eq!(layout.top, 700.0 - 200.0 - CARD_PADDING * 2.0);
        assert_eq!((layout.image_width, layout.image_height), (300.0, 200.0));
    }

    #[test]
    fn card_stays_inside_the_column() {
        let layout = card_layout(
            bounds(400.0, 80.0, 40.0, 22.0),
            (300.0, 100.0),
            (16.0, 450.0),
            800.0,
        );
        assert_eq!(layout.left + layout.image_width + CARD_PADDING * 2.0, 450.0);
        let layout = card_layout(
            bounds(0.0, 80.0, 40.0, 22.0),
            (300.0, 100.0),
            (16.0, 450.0),
            800.0,
        );
        assert_eq!(layout.left, 16.0);
    }

    #[test]
    fn small_pictures_are_not_enlarged() {
        let layout = card_layout(
            bounds(30.0, 80.0, 40.0, 22.0),
            (120.0, 40.0),
            (16.0, 450.0),
            800.0,
        );
        assert_eq!((layout.image_width, layout.image_height), (120.0, 40.0));
    }

    #[test]
    fn magnifier_keeps_the_pointed_pixel_in_place() {
        let factor = 3.0;
        let (left, top) = magnified_offset((50.0, 20.0), factor);
        // The picture point that lands under the pointer is the same one.
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
