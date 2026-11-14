//! Application icons for history rows.
//!
//! macOS has no cheap way to render a bundle's icon: the icon lives inside the
//! app bundle and is only reachable through AppKit. It is fetched once per bundle
//! and cached, because the list renders thousands of rows.
//!
//! Linux resolves them from the desktop icon theme through the window class
//! (`apps_linux`), which is a different lookup but the same cache: both platforms
//! fill it before painting and only read it while painting.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use gpui_kit::RenderImage;

type IconCache = HashMap<String, Option<Arc<RenderImage>>>;
static ICONS: OnceLock<Mutex<IconCache>> = OnceLock::new();

fn cache() -> &'static Mutex<IconCache> {
    ICONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The icon if it has already been loaded. Safe to call while painting: asking
/// AppKit for an icon at that point spins a nested runloop, re-enters the list
/// and aborts with "cannot read ListState while it is already being updated".
pub fn cached_icon(bundle_id: &str) -> Option<Arc<RenderImage>> {
    cache().lock().ok()?.get(bundle_id).cloned().flatten()
}

/// Loads the icons for these apps and parks them in the cache, so a later paint
/// only reads. Must be called outside the render pass.
pub fn preload<'a>(bundle_ids: impl IntoIterator<Item = &'a str>) {
    for bundle_id in bundle_ids {
        let known = cache()
            .lock()
            .map(|cache| cache.contains_key(bundle_id))
            .unwrap_or(true);
        if known {
            continue;
        }
        let icon = load(bundle_id);
        if let Ok(mut cache) = cache().lock() {
            cache.insert(bundle_id.to_string(), icon);
        }
    }
}

/// The platform's icon lookup: AppKit on macOS, the icon theme on Linux.
#[cfg(target_os = "macos")]
fn load(bundle_id: &str) -> Option<Arc<RenderImage>> {
    macos::load_scaled(bundle_id, ICON_TEXTURE_SIZE).map(|(image, _, _)| image)
}

#[cfg(not(target_os = "macos"))]
fn load(bundle_id: &str) -> Option<Arc<RenderImage>> {
    crate::apps_linux::icon_for_app(bundle_id)
}

/// The application's display name, for the filter bar tooltips.
#[cfg(target_os = "macos")]
pub fn app_name(bundle_id: &str) -> String {
    static NAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let names = NAMES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = names
        .lock()
        .ok()
        .and_then(|names| names.get(bundle_id).cloned())
    {
        return hit;
    }
    let name = macos::bundle_display_name(bundle_id).unwrap_or_else(|| bundle_id.to_string());
    if let Ok(mut names) = names.lock() {
        names.insert(bundle_id.to_string(), name.clone());
    }
    name
}

#[cfg(not(target_os = "macos"))]
pub fn app_name(bundle_id: &str) -> String {
    bundle_id.to_string()
}

/// Row thumbnails, keyed by history item id. Filled off the paint path and only
/// read while painting, for the same re-entrancy reason as the app icons.
///
/// One static for all three functions: a `static` inside a function is its own
/// instance, so separate declarations would read and write different maps.
type Thumbs = HashMap<String, Option<(Arc<RenderImage>, f32, f32)>>;
static ROW_THUMBS: OnceLock<Mutex<Thumbs>> = OnceLock::new();

fn thumbs() -> &'static Mutex<Thumbs> {
    ROW_THUMBS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn cached_thumb(id: &str) -> Option<(Arc<RenderImage>, f32, f32)> {
    thumbs().lock().ok()?.get(id).cloned().flatten()
}

pub fn store_thumb(id: &str, thumb: Option<(Arc<RenderImage>, f32, f32)>) {
    if let Ok(mut cache) = thumbs().lock() {
        // Bound the cache so scrolling a long history cannot grow it forever.
        if cache.len() > 256 {
            cache.clear();
        }
        cache.insert(id.to_string(), thumb);
    }
}

pub fn has_thumb(id: &str) -> bool {
    thumbs()
        .lock()
        .map(|cache| cache.contains_key(id))
        .unwrap_or(true)
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

/// The texture size an app icon decodes to. Rows draw them at `ICON_SIZE`; 64
/// covers a 2x display without keeping the 1024 px artwork in memory.
#[cfg(target_os = "macos")]
const ICON_TEXTURE_SIZE: u32 = 64;

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use image::{Frame, ImageBuffer, Rgba};
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSString};

    /// The name Finder shows for the app, e.g. "Safari" for com.apple.Safari.
    pub fn bundle_display_name(bundle_id: &str) -> Option<String> {
        use objc2_foundation::NSFileManager;
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        let identifier = NSString::from_str(bundle_id);
        let url = unsafe { workspace.URLForApplicationWithBundleIdentifier(&identifier) }?;
        let path = url.path()?;
        let name = unsafe { NSFileManager::defaultManager().displayNameAtPath(&path) };
        Some(name.to_string())
    }

    /// Decodes the icon, scaled down to at most `max` pixels on its longest side.
    pub fn load_scaled(bundle_id: &str, max: u32) -> Option<(Arc<RenderImage>, f32, f32)> {
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        let identifier = NSString::from_str(bundle_id);
        let url = unsafe { workspace.URLForApplicationWithBundleIdentifier(&identifier) }?;
        let path = url.path()?;
        // A bundle id can resolve to something that is not an app bundle (a helper,
        // a framework). Asking for its icon returns the generic document picture,
        // which is worse than the letter badge, so those get no icon at all.
        if path.pathExtension().to_string() != "app" {
            return None;
        }
        let image = unsafe { workspace.iconForFile(&path) };
        let tiff = unsafe { image.TIFFRepresentation() }?;
        let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)?;
        let empty = NSDictionary::new();
        let png =
            unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &empty) }?;
        let bytes = png.to_vec();
        // AppKit hands back the artwork at its native size, often 1024 px. Shrinking
        // that in one pass to 15 px samples far too few pixels and the icons turn
        // to mush, so halve repeatedly with a good filter first.
        let mut decoded = image::load_from_memory(&bytes).ok()?;
        if decoded.width() > max {
            // A box filter down to twice the target is cheap and averages every
            // source pixel; one Lanczos pass then finishes the job. Repeated
            // Lanczos passes looked fine but took seconds across a menu bar.
            decoded = decoded.thumbnail(max * 2, max * 2);
            if decoded.width() > max {
                let scale = f64::from(max) / f64::from(decoded.width());
                let width = max;
                let height = (f64::from(decoded.height()) * scale).round().max(1.0) as u32;
                decoded =
                    decoded.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
            }
        }
        let decoded = decoded.to_rgba8();
        let (width, height) = decoded.dimensions();
        let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(width, height, decoded.into_raw())?;
        Some((
            Arc::new(RenderImage::new(vec![Frame::new(buffer)])),
            width as f32,
            height as f32,
        ))
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// The pipeline must hand the renderer a small texture: the artwork arrives at
    /// 1024 px and a row draws it at 15, so anything larger is wasted VRAM and a
    /// blurrier result, since the downscale then happens per frame.
    #[test]
    fn an_app_icon_is_decoded_at_the_size_the_rows_draw_it() {
        let Some((image, width, height)) =
            macos::load_scaled("com.apple.Safari", ICON_TEXTURE_SIZE)
        else {
            // A headless test machine may have no Safari installed.
            return;
        };
        assert!(width <= ICON_TEXTURE_SIZE as f32, "width was {width}");
        assert!(height <= ICON_TEXTURE_SIZE as f32, "height was {height}");
        let decoded = image.size(0);
        assert!(decoded.width.0 <= ICON_TEXTURE_SIZE as i32);
    }

    /// A bundle id that resolves to nothing yields no icon, so the row falls back
    /// to its letter badge instead of drawing a placeholder.
    #[test]
    fn an_unknown_bundle_has_no_icon() {
        assert!(macos::load_scaled("com.example.not-an-installed-app", 64).is_none());
    }
}
