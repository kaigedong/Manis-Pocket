//! Where the panel lands, kept apart from the app so the arithmetic can be tested
//! without a window server.

use gpui_kit::{Bounds, Pixels, Size};

use crate::tray::TrayAnchor;

/// `PANEL_MARGIN` from the panel layout: the card sits inside a transparent
/// border of this width, so the origin is pulled in by the same amount.
const PANEL_MARGIN: f32 = 8.0;

/// Menu bar height in points; the panel hangs just below it.
const MENU_BAR_HEIGHT: f32 = 28.0;

/// Place the panel under the status icon: left edge at the icon's left, top edge
/// just below it, both in the logical points the layout uses.
pub fn anchored_origin(
    screen: &Bounds<Pixels>,
    panel: Size<Pixels>,
    anchor: TrayAnchor,
    scale: f32,
) -> (f32, f32) {
    let left = (anchor.x / f64::from(scale)) as f32 - PANEL_MARGIN;
    // The icon sits in the menu bar, whose height does not depend on the icon.
    let below = MENU_BAR_HEIGHT + 6.0 - PANEL_MARGIN;
    // An icon on a second display still has to land inside this one.
    let max_x = screen.origin.x.as_f32() + screen.size.width.as_f32() - panel.width.as_f32();
    (
        left.clamp(screen.origin.x.as_f32(), max_x),
        screen.origin.y.as_f32() + below,
    )
}

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

    /// A Retina click reports physical pixels; the panel is laid out in points.
    #[test]
    fn retina_anchor_lands_under_the_icon() {
        let panel = gpui_kit::size(px(466.0), px(330.0));
        let anchor = TrayAnchor { x: 2400.0 };
        let (x, y) = anchored_origin(&screen(), panel, anchor, 2.0);
        assert_eq!(x, 1200.0 - PANEL_MARGIN);
        assert_eq!(y, MENU_BAR_HEIGHT + 6.0 - PANEL_MARGIN);
    }

    /// The icon is not on this display: keep the panel inside the screen.
    #[test]
    fn anchor_off_screen_is_clamped() {
        let panel = gpui_kit::size(px(466.0), px(330.0));
        let anchor = TrayAnchor { x: 5000.0 };
        let (x, _y) = anchored_origin(&screen(), panel, anchor, 2.0);
        assert_eq!(x, 1920.0 - 466.0);
    }
}
