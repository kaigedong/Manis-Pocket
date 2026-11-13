//! Menu-bar / status-area presence.
//!
//! Both platforms report the same two user intents — open the panel, quit the
//! app — over a channel, so the window logic in `main` stays platform-agnostic.
//!
//! * macOS: `tray-icon` wraps `NSStatusItem`; its menu events arrive on a
//!   channel that a thread forwards here.
//! * Linux: `tray-icon` would pull in GTK and panics without `gtk::init`, which
//!   GPUI's Wayland backend never calls. `ksni` publishes a
//!   `StatusNotifierItem` over D-Bus instead, which is what bars actually read.

use futures::channel::mpsc::UnboundedSender;

/// Where the status icon is, in the physical pixels the tray reports.
///
/// Only the horizontal position is taken from the event: the rectangle the tray
/// library reports is the status bar window (full screen width), not the button,
/// so the menu bar height is a constant instead.
#[derive(Clone, Copy, Debug)]
pub struct TrayAnchor {
    /// Left edge of the icon, in points.
    pub x: f64,
    /// Bottom edge of the icon, in points, measured from the top of the screen.
    pub y: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum TrayCommand {
    /// Left click on the icon: show the panel, or hide it when it is already up.
    Toggle(Option<TrayAnchor>),
    Open,
    Settings,
    Quit,
}

/// Creates the status item and reports whether one exists. The tray lives for
/// the whole process, so it deliberately leaks its platform handle.
pub fn install(tx: UnboundedSender<TrayCommand>, menu_icon: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::install(tx, menu_icon)
    }

    #[cfg(target_os = "linux")]
    {
        let _ = menu_icon;
        linux::install(tx)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = tx;
        false
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    pub fn install(tx: UnboundedSender<TrayCommand>, menu_icon: &str) -> bool {
        let menu = Menu::new();
        let open_item = MenuItem::new("打开 Manis Pocket", true, None);
        let settings_item = MenuItem::new("设置…", true, None);
        let quit_item = MenuItem::new("退出 Manis Pocket", true, None);
        if menu.append(&open_item).is_err()
            || menu.append(&settings_item).is_err()
            || menu.append(&PredefinedMenuItem::separator()).is_err()
            || menu.append(&quit_item).is_err()
        {
            return false;
        }

        let Some(icon) = load_icon(menu_icon) else {
            return false;
        };

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            // A left click belongs to the panel; the menu is what right click is
            // for. Without this the click only ever opened the menu.
            .with_menu_on_left_click(false)
            .with_tooltip("Manis Pocket")
            // Menu-bar icons are monochrome on macOS, so let the system tint it.
            .with_icon_templated(icon)
            .build();

        let Ok(tray) = tray else {
            return false;
        };
        // The icon lives for the whole process; keeping it also lets the panel
        // ask where the status item is before opening.
        STATUS_ICON.with(|slot| *slot.borrow_mut() = Some(tray));

        let click_tx = tx.clone();
        std::thread::spawn(move || {
            use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
            while let Ok(event) = TrayIconEvent::receiver().recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    position,
                    ..
                } = event
                {
                    // The pointer is over the icon, so its x is the icon's x;
                    // the y comes from AppKit below.
                    // The event carries physical pixels; the layout works in
                    // points. AppKit reports the status item window as a
                    // zero-sized placeholder, so the pointer is the only
                    // trustworthy source and its x is remembered.
                    let anchor = TrayAnchor {
                        x: position.x / screen_scale(),
                        y: 24.0,
                    };
                    // The panel toggles, so a second click closes it again.
                    if click_tx
                        .unbounded_send(TrayCommand::Toggle(Some(anchor)))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });

        let open_id = open_item.id().clone();
        let settings_id = settings_item.id().clone();
        let quit_id = quit_item.id().clone();
        std::thread::spawn(move || {
            use tray_icon::menu::MenuEvent;
            while let Ok(event) = MenuEvent::receiver().recv() {
                let command = if event.id == open_id {
                    TrayCommand::Open
                } else if event.id == settings_id {
                    TrayCommand::Settings
                } else if event.id == quit_id {
                    TrayCommand::Quit
                } else {
                    continue;
                };
                if tx.unbounded_send(command).is_err() {
                    return;
                }
            }
        });

        true
    }

    /// The menu-bar glyph for the `menuIcon` setting.
    fn load_icon(variant: &str) -> Option<Icon> {
        let bytes: &[u8] = match variant {
            "clipboard" => include_bytes!("../assets/tray/tray-clipboard.png"),
            "scissors" => include_bytes!("../assets/tray/tray-scissors.png"),
            "paperclip" => include_bytes!("../assets/tray/tray-paperclip.png"),
            _ => include_bytes!("../assets/tray/tray-manispocket.png"),
        };
        let image = image::load_from_memory(bytes).ok()?.to_rgba8();
        let (width, height) = image.dimensions();
        Icon::from_rgba(image.into_raw(), width, height).ok()
    }
}

// The status item lives for the whole process. It is an `Rc`-based handle and may
// only be touched from the main thread, which is where both the tray and the
// panel are created.
#[cfg(target_os = "macos")]
thread_local! {
    static STATUS_ICON: std::cell::RefCell<Option<tray_icon::TrayIcon>> =
        const { std::cell::RefCell::new(None) };
}

/// Backing scale of the main screen, to turn tray event pixels into points.
#[cfg(target_os = "macos")]
fn screen_scale() -> f64 {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    MainThreadMarker::new()
        .and_then(|mtm| NSScreen::mainScreen(mtm))
        .map(|screen| f64::from(screen.backingScaleFactor()))
        .unwrap_or(1.0)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use ksni::blocking::TrayMethods;
    use ksni::menu::{MenuItem as KsniMenuItem, StandardItem};

    struct StatusItem {
        tx: UnboundedSender<TrayCommand>,
    }

    impl ksni::Tray for StatusItem {
        // A left click should open the panel rather than only showing a menu.
        const MENU_ON_ACTIVATE: bool = true;

        fn id(&self) -> String {
            "io.github.kaigedong.ManisPocket".into()
        }

        fn title(&self) -> String {
            "Manis Pocket".into()
        }

        /// A themed icon name, so the bar draws it in its own style.
        fn icon_name(&self) -> String {
            "edit-paste".into()
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.tx.unbounded_send(TrayCommand::Open);
        }

        fn menu(&self) -> Vec<KsniMenuItem<Self>> {
            vec![
                StandardItem {
                    label: "打开 Manis Pocket".into(),
                    activate: Box::new(|item: &mut Self| {
                        let _ = item.tx.unbounded_send(TrayCommand::Open);
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "设置…".into(),
                    activate: Box::new(|item: &mut Self| {
                        let _ = item.tx.unbounded_send(TrayCommand::Settings);
                    }),
                    ..Default::default()
                }
                .into(),
                KsniMenuItem::Separator,
                StandardItem {
                    label: "退出 Manis Pocket".into(),
                    activate: Box::new(|item: &mut Self| {
                        let _ = item.tx.unbounded_send(TrayCommand::Quit);
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub fn install(tx: UnboundedSender<TrayCommand>) -> bool {
        match (StatusItem { tx }).spawn() {
            Ok(handle) => {
                // The service owns a background thread; the handle only has to
                // stay alive for the process, which it does by being leaked.
                Box::leak(Box::new(handle));
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    /// The four menu-bar glyphs are committed PNGs, so a rename or a bad export
    /// would only show up as a missing tray icon at runtime.
    #[test]
    fn every_tray_icon_decodes() {
        let icons: [&[u8]; 4] = [
            include_bytes!("../assets/tray/tray-manispocket.png"),
            include_bytes!("../assets/tray/tray-clipboard.png"),
            include_bytes!("../assets/tray/tray-scissors.png"),
            include_bytes!("../assets/tray/tray-paperclip.png"),
        ];
        for bytes in icons {
            let image = image::load_from_memory(bytes)
                .expect("tray icon decodes")
                .to_rgba8();
            assert!(image.width() >= 16 && image.height() >= 16);
        }
    }
}
