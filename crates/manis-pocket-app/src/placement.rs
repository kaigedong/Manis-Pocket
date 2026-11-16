//! Where the panel lands, kept apart from the app so the arithmetic can be tested
//! without a window server.
//!
//! Two rectangles matter here and they are not interchangeable:
//!
//! * the display's **full frame**, which is what the status icon's coordinates are
//!   measured against;
//! * the **visible frame**, which on macOS is `NSScreen.visibleFrame`: the display
//!   minus the menu bar and minus the Dock, wherever the Dock happens to be
//!   docked. The panel has to stay inside this one, otherwise a Dock on the right
//!   or along the bottom is drawn on top of the card.

use gpui_kit::{Bounds, Pixels, Size};

use crate::tray::TrayAnchor;

/// `PANEL_MARGIN` from the panel layout: the card sits inside a transparent
/// border of this width, so the origin is pulled in by the same amount.
const PANEL_MARGIN: f32 = 8.0;

/// Gap between the status icon and the card's top edge.
const ICON_GAP: f32 = 6.0;

/// Distance kept between the card and the visible frame's right edge when no
/// status icon has been reported yet.
const EDGE_GAP: f32 = 10.0;

/// Place the panel under the status icon: left edge at the icon's left, top edge
/// just below it, both in the logical points the layout uses.
///
/// `screen` is the full display frame — `anchor.y` is measured from its top — and
/// `visible` is the usable frame the whole panel has to stay inside.
pub fn anchored_origin(
    screen: &Bounds<Pixels>,
    visible: &Bounds<Pixels>,
    panel: Size<Pixels>,
    anchor: TrayAnchor,
) -> (f32, f32) {
    let left = anchor.x as f32 - PANEL_MARGIN;
    // The anchor already carries the icon's bottom edge in points.
    let below = screen.origin.y.as_f32() + anchor.y as f32 + ICON_GAP - PANEL_MARGIN;
    (
        clamp_axis(
            left,
            visible.origin.x.as_f32(),
            right(visible) - panel.width.as_f32(),
        ),
        // Never above the menu bar: the icon sits in it, and its height differs
        // per display.
        clamp_axis(
            below,
            visible.origin.y.as_f32(),
            bottom(visible) - panel.height.as_f32(),
        ),
    )
}

/// Top-right corner of the visible frame, `EDGE_GAP` in and `ICON_GAP` under the
/// menu bar.
///
/// The window carries a transparent margin for the card's shadow, so the offsets
/// apply to the card rather than to the window's invisible border.
pub fn default_origin(visible: &Bounds<Pixels>, panel: Size<Pixels>) -> (f32, f32) {
    (
        right(visible) - panel.width.as_f32() + PANEL_MARGIN - EDGE_GAP,
        visible.origin.y.as_f32() + ICON_GAP - PANEL_MARGIN,
    )
}

/// The middle of the visible frame, which is what the `center` setting means.
pub fn center_origin(visible: &Bounds<Pixels>, panel: Size<Pixels>) -> (f32, f32) {
    (
        visible.origin.x.as_f32() + (visible.size.width.as_f32() - panel.width.as_f32()) / 2.0,
        visible.origin.y.as_f32() + (visible.size.height.as_f32() - panel.height.as_f32()) / 2.0,
    )
}

/// Keep an origin inside the visible frame.
pub fn fit_inside(visible: &Bounds<Pixels>, panel: Size<Pixels>, origin: (f32, f32)) -> (f32, f32) {
    (
        clamp_axis(
            origin.0,
            visible.origin.x.as_f32(),
            right(visible) - panel.width.as_f32(),
        ),
        clamp_axis(
            origin.1,
            visible.origin.y.as_f32(),
            bottom(visible) - panel.height.as_f32(),
        ),
    )
}

fn right(visible: &Bounds<Pixels>) -> f32 {
    visible.origin.x.as_f32() + visible.size.width.as_f32()
}

fn bottom(visible: &Bounds<Pixels>) -> f32 {
    visible.origin.y.as_f32() + visible.size.height.as_f32()
}

/// `f32::clamp` panics on an inverted range, which a panel bigger than the usable
/// area would produce. Pinning such a panel to the frame's start is the honest
/// answer: it still has to be visible somewhere.
fn clamp_axis(value: f32, low: f32, high: f32) -> f32 {
    if high < low {
        low
    } else {
        value.clamp(low, high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, px};

    /// A 1920x1080 display with the menu bar and no Dock.
    fn screen() -> Bounds<Pixels> {
        Bounds::new(
            point(px(0.0), px(0.0)),
            gpui_kit::size(px(1920.0), px(1080.0)),
        )
    }

    /// The same display with the menu bar excluded and nothing else.
    fn visible() -> Bounds<Pixels> {
        Bounds::new(
            point(px(0.0), px(25.0)),
            gpui_kit::size(px(1920.0), px(1055.0)),
        )
    }

    fn panel() -> Size<Pixels> {
        gpui_kit::size(px(466.0), px(330.0))
    }

    /// The anchor is in points, as AppKit reports the status item.
    #[test]
    fn anchor_lands_under_the_icon() {
        // Points, as AppKit reports them: the icon sits at x=1200, bottom y=24.
        let anchor = TrayAnchor { x: 1200.0, y: 24.0 };
        let (x, y) = anchored_origin(&screen(), &visible(), panel(), anchor);
        assert_eq!(x, 1200.0 - PANEL_MARGIN);
        // The menu bar is 25pt tall here, so the panel starts below it.
        assert_eq!(y, 25.0);
    }

    /// The icon is not on this display: keep the panel inside the screen.
    #[test]
    fn anchor_off_screen_is_clamped() {
        let anchor = TrayAnchor { x: 5000.0, y: 24.0 };
        let (x, _y) = anchored_origin(&screen(), &visible(), panel(), anchor);
        assert_eq!(x, 1920.0 - 466.0);
    }

    /// A Dock on the right shrinks the visible frame from that side; a panel
    /// opened near the right edge has to slide left instead of sitting under it.
    #[test]
    fn right_dock_pushes_the_panel_clear_of_it() {
        // 1920 wide display, a 300pt Dock on the right.
        let docked = Bounds::new(
            point(px(0.0), px(25.0)),
            gpui_kit::size(px(1620.0), px(1055.0)),
        );
        let anchor = TrayAnchor { x: 1600.0, y: 24.0 };
        let (x, _y) = anchored_origin(&screen(), &docked, panel(), anchor);
        // 1620 - 466: the card's right edge lands on the Dock's left edge.
        assert_eq!(x, 1620.0 - 466.0);
    }

    /// A Dock on the left shrinks the visible frame's origin instead.
    #[test]
    fn left_dock_pushes_the_panel_right() {
        let docked = Bounds::new(
            point(px(300.0), px(25.0)),
            gpui_kit::size(px(1620.0), px(1055.0)),
        );
        let anchor = TrayAnchor { x: 40.0, y: 24.0 };
        let (x, _y) = anchored_origin(&screen(), &docked, panel(), anchor);
        assert_eq!(x, 300.0);
    }

    /// A panel taller than the usable area is pinned to its top, not allowed to
    /// overflow and not allowed to panic in `f32::clamp`.
    #[test]
    fn oversized_panel_does_not_panic() {
        let tall = gpui_kit::size(px(466.0), px(4000.0));
        let (x, y) = fit_inside(&visible(), tall, (100.0, 100.0));
        assert_eq!(x, 100.0);
        assert_eq!(y, 25.0);
    }

    /// A Dock along the bottom raises the panel's limit.
    #[test]
    fn bottom_dock_keeps_the_panel_above_it() {
        // Menu bar plus a 100pt Dock: 1080 - 25 - 100 = 955 usable.
        let docked = Bounds::new(
            point(px(0.0), px(25.0)),
            gpui_kit::size(px(1920.0), px(955.0)),
        );
        let (x, y) = fit_inside(&docked, panel(), (100.0, 900.0));
        assert_eq!(x, 100.0);
        // 980 is the bottom of the usable area; 980 - 330 is the highest origin.
        assert_eq!(y, 650.0);
    }

    /// Without an anchor the panel goes to the top-right of the usable area, not
    /// of the full display.
    #[test]
    fn default_origin_stays_inside_a_right_dock() {
        let docked = Bounds::new(
            point(px(0.0), px(25.0)),
            gpui_kit::size(px(1620.0), px(1055.0)),
        );
        let (x, y) = default_origin(&docked, panel());
        // Window origin; the card's right edge is EDGE_GAP inside the Dock.
        assert_eq!(x, 1620.0 - 466.0 + PANEL_MARGIN - EDGE_GAP);
        assert_eq!(y, 25.0 + ICON_GAP - PANEL_MARGIN);
    }

    /// Centring follows the usable area too, so the panel is not pushed sideways
    /// by a Dock.
    #[test]
    fn center_origin_uses_the_visible_frame() {
        let docked = Bounds::new(
            point(px(0.0), px(25.0)),
            gpui_kit::size(px(1620.0), px(1055.0)),
        );
        let (x, y) = center_origin(&docked, panel());
        assert_eq!(x, (1620.0 - 466.0) / 2.0);
        assert_eq!(y, 25.0 + (1055.0 - 330.0) / 2.0);
    }
}
