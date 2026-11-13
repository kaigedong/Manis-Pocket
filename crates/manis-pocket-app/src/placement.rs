//! Where the panel lands, kept apart from the app so the arithmetic can be tested
//! without a window server.

use gpui_kit::{Bounds, Pixels, Size};

use crate::tray::TrayAnchor;

/// `PANEL_MARGIN` from the panel layout: the card sits inside a transparent
/// border of this width, so the origin is pulled in by the same amount.
const PANEL_MARGIN: f32 = 8.0;

/// Place the panel under the status icon: left edge at the icon's left, top edge
/// just below it, both in the logical points the layout uses.
pub fn anchored_origin(
    screen: &Bounds<Pixels>,
    panel: Size<Pixels>,
    anchor: TrayAnchor,
) -> (f32, f32) {
    let left = anchor.x as f32 - PANEL_MARGIN;
    // The anchor already carries the icon's bottom edge in points.
    let below = anchor.y as f32 + 6.0 - PANEL_MARGIN;
    // An icon on a second display still has to land inside this one.
    let max_x = screen.origin.x.as_f32() + screen.size.width.as_f32() - panel.width.as_f32();
    (
        left.clamp(screen.origin.x.as_f32(), max_x),
        screen.origin.y.as_f32() + below,
    )
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, px};

    fn screen() -> Bounds<Pixels> {
        Bounds::new(
            point(px(0.0), px(0.0)),
            gpui_kit::size(px(1920.0), px(1080.0)),
        )
    }

    /// The anchor is in points, as AppKit reports the status item.
    #[test]
    fn anchor_lands_under_the_icon() {
        let panel = gpui_kit::size(px(466.0), px(330.0));
        // Points, as AppKit reports them: the icon sits at x=1200, bottom y=24.
        let anchor = TrayAnchor { x: 1200.0, y: 24.0 };
        let (x, y) = anchored_origin(&screen(), panel, anchor);
        assert_eq!(x, 1200.0 - PANEL_MARGIN);
        assert_eq!(y, 24.0 + 6.0 - PANEL_MARGIN);
    }

    /// The icon is not on this display: keep the panel inside the screen.
    #[test]
    fn anchor_off_screen_is_clamped() {
        let panel = gpui_kit::size(px(466.0), px(330.0));
        let anchor = TrayAnchor { x: 5000.0, y: 24.0 };
        let (x, _y) = anchored_origin(&screen(), panel, anchor);
        assert_eq!(x, 1920.0 - 466.0);
    }
}
