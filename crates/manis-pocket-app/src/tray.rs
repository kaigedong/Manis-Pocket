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

#[derive(Clone, Copy, Debug)]
pub enum TrayCommand {
    Open,
    Settings,
    Quit,
}

/// Creates the status item and reports whether one exists. The tray lives for
/// the whole process, so it deliberately leaks its platform handle.
pub fn install(tx: UnboundedSender<TrayCommand>) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::install(tx)
    }

    #[cfg(target_os = "linux")]
    {
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

    pub fn install(tx: UnboundedSender<TrayCommand>) -> bool {
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

        let Some(icon) = load_icon() else {
            return false;
        };

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Manis Pocket")
            // Menu-bar icons are monochrome on macOS, so let the system tint it.
            .with_icon_templated(icon)
            .build();

        let Ok(tray) = tray else {
            return false;
        };
        // Dropping the TrayIcon removes the status item.
        Box::leak(Box::new(tray));

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

    fn load_icon() -> Option<Icon> {
        let image = image::load_from_memory(include_bytes!("../assets/tray-icon.png"))
            .ok()?
            .to_rgba8();
        let (width, height) = image.dimensions();
        Icon::from_rgba(image.into_raw(), width, height).ok()
    }
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
