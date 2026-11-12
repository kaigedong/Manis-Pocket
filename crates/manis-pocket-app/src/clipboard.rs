//! Clipboard change detection.
//!
//! Two very different cost profiles, hence two implementations:
//!
//! * macOS has no pasteboard change notification; the cheapest signal is
//!   `NSPasteboard.changeCount`, an integer that can be polled for free. Data is
//!   only read once the count actually moves.
//! * Wayland exposes the selection to ordinary clients only through a protocol
//!   helper, so `wl-paste --watch` is used: it blocks until the selection
//!   changes and wakes us up. Nothing polls while the clipboard is idle.

use futures::channel::mpsc::UnboundedSender;

/// One clipboard payload this app can store.
#[derive(Clone, Debug)]
pub enum Copy {
    Text(String),
    /// PNG bytes, which is how both platforms represent an image off the
    /// clipboard once converted.
    Image(Vec<u8>),
    /// One or more file URIs, newline separated, exactly as the clipboard
    /// offers them.
    File(String),
}

/// Spawn the platform watcher. Events are `Some(copy)` for payloads that can be
/// stored and `None` for anything else (a file list, a colour, an empty
/// selection). `None` is never treated as "clipboard is empty" by the caller:
/// unknown payloads must not wipe history or peers.
pub fn spawn(tx: UnboundedSender<Option<Copy>>) {
    #[cfg(target_os = "macos")]
    macos::spawn(tx);

    #[cfg(target_os = "linux")]
    linux::spawn(tx);

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = tx;
}

/// Image first, then text: a screenshot carries both a PNG and a description on
/// some apps, and the image is the thing the user meant to copy.
#[cfg(target_os = "macos")]
fn read_copy() -> Option<Copy> {
    if let Some(png) = macos::read_png() {
        return Some(Copy::Image(png));
    }
    // Copying a file in Finder also puts its path on the pasteboard as text, so
    // the file type has to be checked first or the entry loses its identity.
    if let Some(files) = macos::read_file_url() {
        return Some(Copy::File(files));
    }
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut clipboard| clipboard.get_text().ok())
        .filter(|text| !text.is_empty())
        .map(Copy::Text)
}

/// Reads the selection through `wl-paste`.
///
/// arboard talks to `wlr-data-control`/`data_device` directly, and a Wayland client
/// that does not hold keyboard focus is not allowed to read the selection — which
/// is exactly the state a background clipboard manager is in. `wl-paste` goes
/// through the same protocol helper the compositor trusts, so it works.
#[cfg(target_os = "linux")]
fn read_copy() -> Option<Copy> {
    use std::process::Command;

    // The offered types decide what to fetch: `wl-paste` can read exactly one.
    let types = Command::new("wl-paste").arg("--list-types").output().ok()?;
    if !types.status.success() {
        return None;
    }
    let offered = String::from_utf8_lossy(&types.stdout);
    if offered.lines().any(|line| line.trim() == "image/png") {
        let output = Command::new("wl-paste")
            .args(["--type", "image/png", "--no-newline"])
            .output()
            .ok()?;
        if output.status.success() && !output.stdout.is_empty() {
            return Some(Copy::Image(output.stdout));
        }
    }

    // A file list arrives as `text/uri-list`; without this branch it falls
    // through to the plain-text read and is stored as a string of URLs.
    if offered.lines().any(|line| line.trim() == "text/uri-list") {
        let output = Command::new("wl-paste")
            .args(["--type", "text/uri-list", "--no-newline"])
            .output()
            .ok()?;
        let uris = String::from_utf8(output.stdout).ok()?;
        let joined: Vec<&str> = uris
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        if !joined.is_empty() {
            return Some(Copy::File(joined.join("\n")));
        }
    }

    let output = Command::new("wl-paste")
        .args(["--no-newline"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    (!text.is_empty()).then_some(Copy::Text(text))
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2_app_kit::NSPasteboard;
    use std::time::Duration;

    /// Poll interval for the pasteboard change counter.
    const TICK: Duration = Duration::from_millis(250);

    /// File URIs from the pasteboard, if a file list is on it.
    pub fn read_file_url() -> Option<String> {
        use objc2_app_kit::NSPasteboardTypeFileURL;

        let pasteboard = NSPasteboard::generalPasteboard();
        let value = unsafe { pasteboard.stringForType(NSPasteboardTypeFileURL) }?;
        let text = value.to_string();
        (!text.is_empty()).then_some(text)
    }

    /// PNG bytes from the pasteboard, if an image is on it.
    pub fn read_png() -> Option<Vec<u8>> {
        use objc2_app_kit::NSPasteboardTypePNG;

        let pasteboard = NSPasteboard::generalPasteboard();
        let data = unsafe { pasteboard.dataForType(NSPasteboardTypePNG) }?;
        let bytes = data.to_vec();
        (!bytes.is_empty()).then_some(bytes)
    }

    pub fn spawn(tx: UnboundedSender<Option<Copy>>) {
        std::thread::spawn(move || {
            let pasteboard = NSPasteboard::generalPasteboard();
            let mut last = pasteboard.changeCount();
            loop {
                std::thread::sleep(TICK);
                let current = pasteboard.changeCount();
                if current == last {
                    continue;
                }
                last = current;
                if tx.unbounded_send(read_copy()).is_err() {
                    return;
                }
            }
        });
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    /// `wl-paste --watch` runs a command on every selection change; `echo` is
    /// the cheapest thing that gives us a wake-up line to read.
    pub fn spawn(tx: UnboundedSender<Option<Copy>>) {
        std::thread::spawn(move || {
            let child = Command::new("wl-paste")
                .args(["--watch", "echo", "changed"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();

            let Ok(mut child) = child else {
                // wl-clipboard missing or no compositor: stay quiet rather than
                // spinning. The app still works for viewing history.
                return;
            };

            let Some(stdout) = child.stdout.take() else {
                return;
            };
            for line in BufReader::new(stdout).lines() {
                if line.is_err() {
                    break;
                }
                let copy = read_copy();
                if tx.unbounded_send(copy).is_err() {
                    break;
                }
            }
            let _ = child.kill();
        });
    }
}

/// Puts a payload that arrived from a peer onto the system clipboard.
///
/// Returns whether the write succeeded, which is what the sync engine records as
/// the apply result.
pub fn apply_remote(content: &manis_pocket_core::model::ClipboardContent) -> bool {
    let Some(bytes) = content.value.as_ref() else {
        return false;
    };
    if content.content_type == "public.file-url" {
        let Some(bytes) = content.value.as_ref() else {
            return false;
        };
        let text = String::from_utf8_lossy(bytes).to_string();
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
            use objc2_foundation::NSString;
            let pasteboard = NSPasteboard::generalPasteboard();
            unsafe { pasteboard.clearContents() };
            let value = NSString::from_str(&text);
            return unsafe { pasteboard.setString_forType(&value, NSPasteboardTypeFileURL) };
        }
        #[cfg(not(target_os = "macos"))]
        {
            return std::process::Command::new("wl-copy")
                .args(["--type", "text/uri-list"])
                .arg(text)
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
        }
    }
    if content.content_type == "public.png" {
        return crate::icons::write_png_to_pasteboard(bytes);
    }
    let Ok(text) = String::from_utf8(bytes.clone()) else {
        return false;
    };
    // Wayland only lets the focused client own the selection, and a background
    // manager is never focused: `arboard` fails there, `wl-copy` goes through
    // the protocol helper that is allowed to.
    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let child = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                let stdin = child
                    .stdin
                    .as_mut()
                    .ok_or_else(|| std::io::Error::other("wl-copy stdin closed"))?;
                stdin.write_all(text.as_bytes())?;
                child.wait().map(|status| status.success())
            });
        child.unwrap_or(false)
    }

    #[cfg(not(target_os = "linux"))]
    {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text))
            .is_ok()
    }
}

/// Whether the platform will let us send the paste shortcut.
///
/// Without this the auto-paste setting looks enabled but does nothing, which is
/// the worst possible outcome: the user cannot tell a missing permission from a
/// bug.
pub fn paste_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::process::Command::new("sh")
            .args(["-c", "command -v wtype"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

/// Opens the system pane where the permission is granted.
pub fn open_paste_permission_settings() -> bool {
    #[cfg(target_os = "macos")]
    let target = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
    #[cfg(not(target_os = "macos"))]
    let target = "accessibility";

    std::process::Command::new("open")
        .arg(target)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Puts plain text on the system clipboard.
pub fn write_text(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text.to_string()))
            .is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        use std::io::Write;
        std::process::Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                child
                    .stdin
                    .as_mut()
                    .map(|stdin| stdin.write_all(text.as_bytes()))
                    .transpose()?;
                child.wait()
            })
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

/// Presses the paste shortcut in whatever app is in front.
///
/// `pasteByDefault` needs this: the panel closes, focus returns to the previous
/// application, and the entry that was just copied should land there. macOS goes
/// through System Events (which needs Accessibility permission, the same one a
/// real auto-paste implementation requires); Wayland uses `wtype`.
pub fn paste_into_frontmost() -> bool {
    #[cfg(target_os = "macos")]
    let command = (
        "osascript",
        vec![
            "-e".to_string(),
            "tell application \"System Events\" to keystroke \"v\" using command down".to_string(),
        ],
    );
    #[cfg(not(target_os = "macos"))]
    let command: (&str, Vec<String>) = (
        "wtype",
        ["-M", "ctrl", "v", "-m", "ctrl"]
            .iter()
            .map(|arg| arg.to_string())
            .collect(),
    );

    std::process::Command::new(command.0)
        .args(command.1)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
