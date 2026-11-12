//! Application icons for history rows.
//!
//! macOS has no cheap way to render a bundle's icon: the icon lives inside the
//! app bundle and is only reachable through AppKit. It is fetched once per bundle
//! and cached, because the list renders thousands of rows.
//!
//! Linux has no equivalent (icons come from the desktop's icon theme and need the
//! `.desktop` entry), so there the row keeps the letter badge.

#[cfg(target_os = "macos")]
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::{Mutex, OnceLock};

use gpui_kit::RenderImage;

#[cfg(target_os = "macos")]
pub fn icon_for_bundle(bundle_id: &str) -> Option<Arc<RenderImage>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<RenderImage>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Some(hit) = cache.lock().ok()?.get(bundle_id) {
        return hit.clone();
    }
    let icon = macos::load(bundle_id);
    cache
        .lock()
        .ok()?
        .insert(bundle_id.to_string(), icon.clone());
    icon
}

#[cfg(not(target_os = "macos"))]
pub fn icon_for_bundle(bundle_id: &str) -> Option<Arc<RenderImage>> {
    // On Linux the "bundle id" is the window class from the compositor.
    crate::apps_linux::icon_for_app(bundle_id)
}

/// Puts PNG bytes on the general pasteboard.
///
/// `arboard` refuses images on some platforms and cannot express "this is a PNG
/// blob", so the macOS path writes the pasteboard type directly.
#[cfg(target_os = "macos")]
pub fn write_png_to_pasteboard(bytes: &[u8]) -> bool {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG};
    use objc2_foundation::NSData;

    let data = NSData::with_bytes(bytes);
    unsafe {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        pasteboard.setData_forType(Some(&data), NSPasteboardTypePNG)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn write_png_to_pasteboard(bytes: &[u8]) -> bool {
    use std::process::{Command, Stdio};
    // `wl-copy` is the Wayland-native way to publish an image selection.
    Command::new("wl-copy")
        .arg("--type")
        .arg("image/png")
        .stdin(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| std::io::Error::other("wl-copy stdin closed"))?;
            stdin.write_all(bytes)?;
            child.wait().map(|status| status.success())
        })
        .unwrap_or(false)
}

/// Shows the system About panel, like the SwiftUI footer's "About" row.
#[cfg(target_os = "macos")]
pub fn show_about_panel() {
    let Some(main_thread) = objc2::MainThreadMarker::new() else {
        return;
    };
    let application = objc2_app_kit::NSApplication::sharedApplication(main_thread);
    unsafe { application.orderFrontStandardAboutPanel(None) };
}

/// Bundle identifier of the frontmost application.
///
/// The macOS build records this with every copy, which is what lets a row show
/// where it came from.
#[cfg(target_os = "macos")]
pub fn frontmost_bundle_id() -> Option<String> {
    let workspace = unsafe { objc2_app_kit::NSWorkspace::sharedWorkspace() };
    // `frontmostApplication` is nil for processes that are not a bundled app, so
    // fall back to whichever running application reports itself as active.
    let application = match unsafe { workspace.frontmostApplication() } {
        Some(application) => application,
        None => workspace
            .runningApplications()
            .into_iter()
            .find(|candidate| unsafe { candidate.isActive() })?,
    };
    // While the panel is open it is the frontmost app, and it has no bundle id.
    // Reporting "unknown" is right: the user knows they just copied something,
    // and attributing it to ourselves would be wrong.
    if unsafe { application.processIdentifier() } == std::process::id() as i32 {
        return None;
    }
    let identifier = unsafe { application.bundleIdentifier() };
    Some(identifier?.to_string())
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use image::{Frame, ImageBuffer, Rgba};
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSString};

    pub fn load(bundle_id: &str) -> Option<Arc<RenderImage>> {
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        let identifier = NSString::from_str(bundle_id);
        let url = unsafe { workspace.URLForApplicationWithBundleIdentifier(&identifier) }?;
        let path = url.path()?;
        let image = unsafe { workspace.iconForFile(&path) };
        let tiff = unsafe { image.TIFFRepresentation() }?;
        let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)?;
        let empty = NSDictionary::new();
        let png =
            unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &empty) }?;
        let bytes = png.to_vec();
        // Decoding through the `image` crate keeps the pixel-format handling in
        // one place instead of duplicating it against AppKit.
        let decoded = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let (width, height) = decoded.dimensions();
        let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(width, height, decoded.into_raw())?;
        Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
    }
}

/// Decodes PNG bytes for display, scaled down to `max_width`.
///
/// Returns the image and the size it should occupy. A 4000 px screenshot would
/// otherwise be uploaded as a 4000 px texture just to be drawn 380 px wide.
pub fn png_to_render_image(bytes: &[u8], max_width: u32) -> Option<(Arc<RenderImage>, f32, f32)> {
    use image::{Frame, ImageBuffer, Rgba};

    let mut decoded = image::load_from_memory(bytes).ok()?;
    if decoded.width() > max_width {
        let height = (decoded.height() as f64 * (max_width as f64 / decoded.width() as f64))
            .round()
            .max(1.0) as u32;
        decoded = decoded.resize(max_width, height, image::imageops::FilterType::Triangle);
    }
    let (width, height) = (decoded.width(), decoded.height());
    let rgba = decoded.to_rgba8();
    let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, rgba.into_raw())?;
    Some((
        Arc::new(RenderImage::new(vec![Frame::new(buffer)])),
        width as f32,
        height as f32,
    ))
}
