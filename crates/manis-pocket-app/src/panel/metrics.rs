//! Sizes the list and the window are built from, in logical points.

/// Content height of one row, from the macOS build (`Popup.itemHeight`).
/// `ListItem` adds 4 px of padding above and below, and the 13 px text then makes
/// a row about 28 px tall: the SwiftUI build fits 22 px because its default
/// leading is tighter, and GPUI exposes no line-height setter.
pub(crate) const ROW_MIN_HEIGHT: f32 = 20.0;
/// Thumbnail height for image rows. The list takes one measurement for every
/// row, so this has to fit inside the normal row pitch.
pub(crate) const ROW_THUMB_HEIGHT: f32 = 22.0;
/// What one row actually occupies, used for the window's height.
pub(crate) const ROW_PITCH: f32 = 30.0;
/// Height of the application filter bar.
pub(crate) const APP_FILTER_HEIGHT: f32 = 28.0;
/// Height of the status line under the footer.
pub(crate) const STATUS_HEIGHT: f32 = 30.0;
pub(crate) const ROW_CORNER: f32 = 4.0;
pub(crate) const ICON_SIZE: f32 = 15.0;
/// Selection is the system accent at 80% (`ListItemView.swift:115`); GPUI has no
/// system-accent API, so this is the default macOS blue.
pub(crate) const SELECTION_ACCENT: u32 = 0x0A84FF;
/// Measured chrome around the list: the search row and the footer.
pub(crate) const HEADER_HEIGHT: f32 = 44.0;
pub(crate) const FOOTER_HEIGHT: f32 = 122.0;

/// Character budget standing in for `truncationMode(.middle)`.
pub(crate) const TITLE_BUDGET: usize = 28;

/// Transparent margin around the card, so its shadow and rounded corners have
/// somewhere to draw. The window is larger than the card by twice this.
pub(crate) const PANEL_MARGIN: f32 = 8.0;
