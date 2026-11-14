//! Linux application identity and icons.
//!
//! Wayland deliberately does not tell a clipboard manager which application owns
//! the selection, so the source app has to be inferred from the compositor. On
//! Hyprland that is `hyprctl activewindow`; the focused window at the moment of a
//! copy is the one that copied.
//!
//! Icons then come from the desktop's icon theme: a `.desktop` entry maps the
//! window class to an icon name, and the theme directories hold the pixels.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use gpui_kit::RenderImage;

/// Window class of the focused window, e.g. `Alacritty`.
pub fn active_app() -> Option<String> {
    let output = Command::new("hyprctl").arg("activewindow").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let class = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("class:"))
        .map(|value| value.trim().to_string())?;
    // Hyprland reports an empty class for some windows (and for no window).
    if class.is_empty() || class == "null" {
        return None;
    }
    Some(class)
}

/// Icon for a window class, decoded once and cached.
pub fn icon_for_app(class: &str) -> Option<Arc<RenderImage>> {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, Option<Arc<RenderImage>>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));

    if let Some(hit) = cache.lock().ok()?.get(class) {
        return hit.clone();
    }
    let icon = load_icon(class);
    cache.lock().ok()?.insert(class.to_string(), icon.clone());
    icon
}

/// Pixels the row draws the icon from. The row is 15pt, so this stays sharp on a
/// 2x screen without holding a full 512x512 tile per application.
const ICON_PIXELS: u32 = 64;

fn load_icon(class: &str) -> Option<Arc<RenderImage>> {
    let name = icon_name_for_class(class)?;
    let path = find_icon_file(&name)?;
    let bytes = std::fs::read(path).ok()?;
    // Themes ship the same app as SVG or PNG, so both are decoded here: PNG
    // through `image`, SVG through resvg (which GPUI already builds for its own
    // SVG elements). RenderImage wants premultiplied RGBA, which is exactly what
    // the rasteriser produces.
    if let Ok(decoded) = image::load_from_memory(&bytes) {
        let decoded = decoded.to_rgba8();
        let (width, height) = decoded.dimensions();
        let buffer: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
            image::ImageBuffer::from_raw(width, height, decoded.into_raw())?;
        return Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])));
    }
    rasterize_svg(&bytes, ICON_PIXELS)
}

/// Renders an SVG icon into RGBA at roughly `target` pixels on its long side.
fn rasterize_svg(bytes: &[u8], target: u32) -> Option<Arc<RenderImage>> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let (width, height) = (size.width(), size.height());
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    // Keep the aspect ratio: some icons carry a little padding on one axis.
    let scale = (target as f32 / width).min(target as f32 / height);
    let pixels = (
        (width * scale).round().max(1.0) as u32,
        (height * scale).round().max(1.0) as u32,
    );
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixels.0, pixels.1)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let buffer: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(pixels.0, pixels.1, pixmap.take())?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// Maps a window class to a themed icon name via the `.desktop` entries.
fn icon_name_for_class(class: &str) -> Option<String> {
    let wanted = class.to_lowercase();
    for directory in application_dirs() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mut icon = None;
            let mut matches = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.to_lowercase() == wanted);
            for line in contents.lines() {
                if let Some(value) = line.strip_prefix("Icon=") {
                    icon = Some(value.trim().to_string());
                }
                if let Some(value) = line.strip_prefix("StartupWMClass=") {
                    matches |= value.trim().to_lowercase() == wanted;
                }
                if let Some(value) = line.strip_prefix("Name=") {
                    matches |= value.trim().to_lowercase() == wanted;
                }
            }
            if matches && let Some(icon) = icon.filter(|icon| !icon.is_empty()) {
                return Some(icon);
            }
        }
    }
    // Some applications name their icon exactly after the class.
    Some(class.to_string())
}

fn application_dirs() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        directories.push(PathBuf::from(data_home).join("applications"));
    } else if let Some(home) = std::env::var_os("HOME") {
        directories.push(PathBuf::from(home).join(".local/share/applications"));
    }
    if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS") {
        for directory in std::env::split_paths(&data_dirs) {
            directories.push(directory.join("applications"));
        }
    } else {
        directories.push(PathBuf::from("/usr/local/share/applications"));
        directories.push(PathBuf::from("/usr/share/applications"));
    }
    directories
}

/// Searches the usual theme layout for the application's icon.
///
/// The directories are read instead of probing a fixed list of sizes: themes are
/// free to ship only the sizes they like (Zed installs a single 512x512 tile), and
/// a size the list forgot used to end as the letter badge.
fn find_icon_file(icon: &str) -> Option<PathBuf> {
    const PREFERRED: [&str; 6] = [
        "hicolor",
        "Adwaita",
        "Papirus",
        "Papirus-Dark",
        "breeze",
        "Yaru",
    ];

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(&home).join(".local/share/icons"));
        roots.push(PathBuf::from(home).join(".icons"));
    }
    if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS") {
        for directory in std::env::split_paths(&data_dirs) {
            roots.push(directory.join("icons"));
        }
    } else {
        roots.push(PathBuf::from("/usr/share/icons"));
    }

    let mut themes: Vec<String> = PREFERRED.iter().map(|name| (*name).to_string()).collect();
    for root in &roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !themes.contains(&name) {
                    themes.push(name);
                }
            }
        }
    }

    let mut rasters: Vec<(u32, PathBuf)> = Vec::new();
    let mut vectors: Vec<PathBuf> = Vec::new();
    for root in &roots {
        for theme in &themes {
            let base = root.join(theme);
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            for entry in entries.flatten() {
                if !entry.path().is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let apps = entry.path().join("apps");
                let png = apps.join(format!("{icon}.png"));
                if png.is_file() {
                    let size = name
                        .split('x')
                        .next()
                        .and_then(|edge| edge.parse::<u32>().ok())
                        .unwrap_or(0);
                    rasters.push((size, png));
                }
                let svg = apps.join(format!("{icon}.svg"));
                if svg.is_file() {
                    vectors.push(svg);
                }
            }
        }
    }
    // Largest first: a 512 tile downscales cleanly, a 32 tile does not.
    rasters.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    if let Some((_, path)) = rasters.into_iter().next() {
        return Some(path);
    }
    if let Some(path) = vectors.into_iter().next() {
        return Some(path);
    }
    ["png", "svg", "xpm"]
        .iter()
        .map(|extension| PathBuf::from("/usr/share/pixmaps").join(format!("{icon}.{extension}")))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_theme_icons_are_not_fatal() {
        assert!(find_icon_file("definitely-not-an-icon-name").is_none());
    }

    #[test]
    fn application_directories_always_include_something() {
        assert!(!application_dirs().is_empty());
    }

    /// Icon themes ship SVG as often as PNG, and the row needs pixels either way.
    #[test]
    fn svg_icons_rasterize_at_the_requested_size() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">
            <rect width="16" height="16" fill="#0A84FF"/>
        </svg>"##;
        let image = rasterize_svg(svg, 64).expect("the SVG rasterises");
        let size = image.size(0);
        assert_eq!(i32::from(size.width), 64);
        assert_eq!(i32::from(size.height), 64);
        assert_eq!(image.as_bytes(0).map(<[u8]>::len), Some(64 * 64 * 4));
    }
}
