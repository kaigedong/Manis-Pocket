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
