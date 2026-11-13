//! Linux application identity and icons.
//!
//! Wayland deliberately does not tell a clipboard manager which application owns
//! the selection, so the source app has to be inferred from the compositor. On
//! Hyprland that is `hyprctl activewindow`; the focused window at the moment of a
//! copy is the one that copied.
//!
//! Icons then come from the desktop's icon theme: a `.desktop` entry maps the
//! window class to an icon name, and the theme directories hold the pixels.

use std::path::{Path, PathBuf};
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

fn load_icon(class: &str) -> Option<Arc<RenderImage>> {
    let name = icon_name_for_class(class)?;
    let path = find_icon_file(&name)?;
    let bytes = std::fs::read(path).ok()?;
    // Theme directories also ship SVG; without a rasteriser those fall back to
    // the letter badge.
    let decoded = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (width, height) = decoded.dimensions();
    let buffer: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(width, height, decoded.into_raw())?;
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

/// Searches the usual theme layout for a raster icon.
fn find_icon_file(icon: &str) -> Option<PathBuf> {
    const THEMES: [&str; 6] = [
        "hicolor",
        "Adwaita",
        "Papirus",
        "Papirus-Dark",
        "breeze",
        "Yaru",
    ];
    const SIZES: [&str; 5] = ["48x48", "64x64", "128x128", "256x256", "32x32"];

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

    let candidates = roots
        .iter()
        .flat_map(|root| {
            THEMES.iter().flat_map(move |theme| {
                SIZES.iter().map(move |size| {
                    root.join(theme)
                        .join(size)
                        .join("apps")
                        .join(format!("{icon}.png"))
                })
            })
        })
        .chain([
            PathBuf::from("/usr/share/pixmaps").join(format!("{icon}.png")),
            PathBuf::from("/usr/share/pixmaps").join(format!("{icon}.xpm")),
        ]);

    candidates
        .into_iter()
        .find(|path| Path::new(path).is_file())
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
}
