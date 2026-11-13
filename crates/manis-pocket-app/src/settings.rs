//! User settings.
//!
//! The SwiftUI build keeps its settings in `UserDefaults` under
//! `com.kaigedong.manispocket`, read through the `Defaults` library. The GPUI
//! build owns a JSON file instead, because the same code has to run on Linux
//! where there is no `UserDefaults` — and it imports the macOS values once so
//! an existing user keeps their configuration.
//!
//! Key names match the Swift build exactly; `Settings::default` mirrors the
//! defaults declared in `Defaults.Keys+Names.swift`.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Keys the Swift build declares. Kept as raw values so a key we do not model
/// yet still survives a load/save cycle.
pub type RawSettings = BTreeMap<String, String>;

/// Which panes the panel shows, mirroring the Swift keys of the same name.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub history_size: i64,
    pub unlimited_history: bool,
    pub paste_by_default: bool,
    pub show_footer: bool,
    pub show_search: bool,
    pub show_title: bool,
    pub show_application_icons: bool,
    pub show_in_status_bar: bool,
    pub show_special_symbols: bool,
    pub search_mode: String,
    pub sort_by: String,
    pub popup_position: String,
    /// Left edge of the status icon in points, learned from the last click.
    /// AppKit reports the status item window as a zero-sized placeholder, so
    /// this is the only way a shortcut-opened panel can line up with the icon.
    pub status_item_x: Option<f32>,
    /// Whether image copies are stored at all.
    pub save_images: bool,
    /// Whether text copies are stored at all.
    pub save_text: bool,
    pub highlight_match: String,
    pub menu_icon: String,
    pub window_width: f64,
    pub window_height: f64,
    pub sync_enabled: bool,
    pub sync_device_name: String,
    pub sync_device_id: String,
    pub sync_discoverable: bool,
    /// Applications whose copies are not recorded.
    pub ignored_apps: Vec<String>,
    /// Text patterns whose copies are not recorded.
    pub ignore_regexp: Vec<String>,
    /// When true, `ignored_apps` becomes an allowlist instead.
    pub ignore_all_apps_except_listed: bool,
    /// Where pinned entries sort (`pinTo`: top or bottom).
    pub pins_position: String,
    /// Width of the preview sidebar when it is open.
    pub preview_width: f64,
    /// Anything from UserDefaults we do not model yet.
    pub extras: RawSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // The Swift default is 500; a user's own value wins on import.
            history_size: 500,
            unlimited_history: false,
            paste_by_default: false,
            show_footer: true,
            show_search: true,
            show_title: true,
            show_application_icons: true,
            show_in_status_bar: true,
            show_special_symbols: true,
            search_mode: "exact".into(),
            sort_by: "lastCopiedAt".into(),
            // The panel belongs to the status icon; "cursor" was the SwiftUI
            // default and left the panel following the pointer.
            popup_position: "statusItem".into(),
            status_item_x: None,
            save_images: true,
            save_text: true,
            highlight_match: "bold".into(),
            menu_icon: "manispocket".into(),
            window_width: 450.0,
            window_height: 800.0,
            sync_enabled: false,
            sync_device_name: default_device_name(),
            sync_device_id: String::new(),
            sync_discoverable: false,
            ignored_apps: Vec::new(),
            ignore_regexp: Vec::new(),
            ignore_all_apps_except_listed: false,
            pins_position: "top".into(),
            preview_width: 400.0,
            extras: RawSettings::new(),
        }
    }
}

/// Where the settings file lives: next to the history database, so both move
/// together.
pub fn settings_path(database_path: &str) -> PathBuf {
    let database = PathBuf::from(database_path);
    database
        .parent()
        .map(|directory| directory.join("settings.json"))
        .unwrap_or_else(|| PathBuf::from("settings.json"))
}

impl Settings {
    /// Reads the JSON file, importing the macOS UserDefaults once when the file
    /// does not exist yet.
    pub fn load(database_path: &str) -> Self {
        let path = settings_path(database_path);
        if let Some(raw) = read_raw(&path) {
            return Self::from_raw(&raw);
        }

        let mut settings = Self::default();
        #[cfg(target_os = "macos")]
        settings.import_macos_defaults();
        if settings.sync_device_id.is_empty() {
            // Peers identify each other by this id, so it has to be stable.
            settings.sync_device_id = uuid::Uuid::new_v4().to_string();
        }
        // Persist the imported values so the import happens once.
        settings.save(database_path);
        settings
    }

    pub fn save(&self, database_path: &str) {
        let path = settings_path(database_path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, self.to_raw().to_json());
    }

    /// Applies values in the Swift build's own types (`1`/`0`, decimal numbers,
    /// bare strings), which is what `defaults read` prints.
    pub fn from_raw(raw: &RawSettings) -> Self {
        let mut settings = Self::default();
        let get = |key: &str| raw.get(key).map(String::as_str);

        if let Some(value) = get("historySize").and_then(|value| value.parse().ok()) {
            settings.history_size = value;
        }
        if let Some(value) = get("isUnlimitedHistory") {
            settings.unlimited_history = truthy(value);
        }
        if let Some(value) = get("pasteByDefault") {
            settings.paste_by_default = truthy(value);
        }
        if let Some(value) = get("showFooter") {
            settings.show_footer = truthy(value);
        }
        if let Some(value) = get("statusItemX") {
            settings.status_item_x = value.parse::<f32>().ok();
        }
        if let Some(value) = get("popupPosition") {
            settings.popup_position = value.to_string();
        }
        if let Some(value) = get("saveImages") {
            settings.save_images = truthy(value);
        }
        if let Some(value) = get("saveText") {
            settings.save_text = truthy(value);
        }
        if let Some(value) = get("showSearch") {
            settings.show_search = truthy(value);
        }
        if let Some(value) = get("showTitle") {
            settings.show_title = truthy(value);
        }
        if let Some(value) = get("showApplicationIcons") {
            settings.show_application_icons = truthy(value);
        }
        if let Some(value) = get("showInStatusBar") {
            settings.show_in_status_bar = truthy(value);
        }
        if let Some(value) = get("showSpecialSymbols") {
            settings.show_special_symbols = truthy(value);
        }
        if let Some(value) = get("searchMode") {
            settings.search_mode = value.to_string();
        }
        if let Some(value) = get("sortBy") {
            settings.sort_by = value.to_string();
        }
        if let Some(value) = get("statusItemX") {
            settings.status_item_x = value.parse::<f32>().ok();
        }
        if let Some(value) = get("popupPosition") {
            settings.popup_position = value.to_string();
        }
        if let Some(value) = get("highlightMatch") {
            settings.highlight_match = value.to_string();
        }
        if let Some(value) = get("menuIcon") {
            settings.menu_icon = value.to_string();
        }
        if let Some(value) = get("windowWidth").and_then(|value| value.parse().ok()) {
            settings.window_width = value;
        }
        if let Some(value) = get("windowHeight").and_then(|value| value.parse().ok()) {
            settings.window_height = value;
        }
        if let Some(value) = get("syncEnabled") {
            settings.sync_enabled = truthy(value);
        }
        if let Some(value) = get("syncDiscoverable") {
            settings.sync_discoverable = truthy(value);
        }
        if let Some(value) = get("syncDeviceName") {
            settings.sync_device_name = value.to_string();
        }
        if let Some(value) = get("syncDeviceID") {
            settings.sync_device_id = value.to_string();
        }
        if let Some(value) = get("ignoredApps") {
            settings.ignored_apps = split_list(value);
        }
        if let Some(value) = get("ignoreRegexp") {
            settings.ignore_regexp = split_list(value);
        }
        if let Some(value) = get("ignoreAllAppsExceptListed") {
            settings.ignore_all_apps_except_listed = truthy(value);
        }
        if let Some(value) = get("pinsPosition") {
            settings.pins_position = value.to_string();
        }
        if let Some(value) = get("previewWidth").and_then(|value| value.parse().ok()) {
            settings.preview_width = value;
        }
        if let Some(value) = get("showFooter") {
            settings.show_footer = truthy(value);
        }

        let modelled = [
            "historySize",
            "isUnlimitedHistory",
            "pasteByDefault",
            "showFooter",
            "showSearch",
            "showTitle",
            "showApplicationIcons",
            "showInStatusBar",
            "showSpecialSymbols",
            "searchMode",
            "sortBy",
            "popupPosition",
            "statusItemX",
            "highlightMatch",
            "menuIcon",
            "windowWidth",
            "windowHeight",
            "syncEnabled",
            "syncDiscoverable",
            "syncDeviceName",
            "syncDeviceID",
            "ignoredApps",
            "ignoreRegexp",
            "ignoreAllAppsExceptListed",
            "pinsPosition",
            "previewWidth",
            "showFooter",
            "popupPosition",
            "statusItemX",
            "saveImages",
            "saveText",
        ];
        settings.extras = raw
            .iter()
            .filter(|(key, _)| !modelled.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        settings
    }

    pub fn to_raw(&self) -> RawSettings {
        let mut raw = self.extras.clone();
        raw.insert("historySize".into(), self.history_size.to_string());
        raw.insert(
            "isUnlimitedHistory".into(),
            bool_value(self.unlimited_history),
        );
        raw.insert("pasteByDefault".into(), bool_value(self.paste_by_default));
        raw.insert("showFooter".into(), bool_value(self.show_footer));
        raw.insert("popupPosition".into(), self.popup_position.clone());
        if let Some(x) = self.status_item_x {
            raw.insert("statusItemX".into(), format!("{x}"));
        }
        raw.insert("saveImages".into(), bool_value(self.save_images));
        raw.insert("saveText".into(), bool_value(self.save_text));
        raw.insert("showSearch".into(), bool_value(self.show_search));
        raw.insert("showTitle".into(), bool_value(self.show_title));
        raw.insert(
            "showApplicationIcons".into(),
            bool_value(self.show_application_icons),
        );
        raw.insert(
            "showInStatusBar".into(),
            bool_value(self.show_in_status_bar),
        );
        raw.insert(
            "showSpecialSymbols".into(),
            bool_value(self.show_special_symbols),
        );
        raw.insert("searchMode".into(), self.search_mode.clone());
        raw.insert("sortBy".into(), self.sort_by.clone());
        raw.insert("popupPosition".into(), self.popup_position.clone());
        if let Some(x) = self.status_item_x {
            raw.insert("statusItemX".into(), format!("{x}"));
        }
        raw.insert("highlightMatch".into(), self.highlight_match.clone());
        raw.insert("menuIcon".into(), self.menu_icon.clone());
        raw.insert("windowWidth".into(), self.window_width.to_string());
        raw.insert("windowHeight".into(), self.window_height.to_string());
        raw.insert("syncEnabled".into(), bool_value(self.sync_enabled));
        raw.insert(
            "syncDiscoverable".into(),
            bool_value(self.sync_discoverable),
        );
        raw.insert("syncDeviceName".into(), self.sync_device_name.clone());
        raw.insert("syncDeviceID".into(), self.sync_device_id.clone());
        raw.insert("ignoredApps".into(), self.ignored_apps.join("\n"));
        raw.insert("ignoreRegexp".into(), self.ignore_regexp.join("\n"));
        raw.insert(
            "ignoreAllAppsExceptListed".into(),
            bool_value(self.ignore_all_apps_except_listed),
        );
        raw.insert("pinsPosition".into(), self.pins_position.clone());
        raw.insert("previewWidth".into(), self.preview_width.to_string());
        raw.insert("showFooter".into(), bool_value(self.show_footer));
        raw
    }

    /// One-time import of the SwiftUI build's settings on macOS.
    #[cfg(target_os = "macos")]
    fn import_macos_defaults(&mut self) {
        let Some(raw) = macos_defaults() else {
            return;
        };
        let merged = Self::from_raw(&raw);
        // Only take values the Swift build actually stored; its untouched
        // defaults would otherwise overwrite ours.
        self.history_size = if raw.contains_key("historySize") {
            merged.history_size
        } else {
            self.history_size
        };
        if raw.contains_key("showFooter") {
            self.show_footer = merged.show_footer;
        }
        if raw.contains_key("showSearch") {
            self.show_search = merged.show_search;
        }
        if raw.contains_key("showTitle") {
            self.show_title = merged.show_title;
        }
        if raw.contains_key("pasteByDefault") {
            self.paste_by_default = merged.paste_by_default;
        }
        if raw.contains_key("searchMode") {
            self.search_mode = merged.search_mode;
        }
        if raw.contains_key("sortBy") {
            self.sort_by = merged.sort_by;
        }
        if raw.contains_key("popupPosition") {
            self.popup_position = merged.popup_position;
        }
        self.extras.extend(merged.extras);
    }
}

/// Lists are stored one entry per line: the settings file is a flat string map,
/// which keeps the format readable and diff-friendly.
fn split_list(value: &str) -> Vec<String> {
    value
        .split('\n')
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn default_device_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|name| name.trim().to_string())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Manis Pocket".to_string())
}

fn truthy(value: &str) -> bool {
    matches!(value.trim(), "1" | "true" | "YES" | "yes")
}

fn bool_value(value: bool) -> String {
    if value { "1" } else { "0" }.to_string()
}

fn read_raw(path: &std::path::Path) -> Option<RawSettings> {
    let text = std::fs::read_to_string(path).ok()?;
    RawSettings::from_json(&text)
}

/// Minimal JSON object support: the settings file is a flat map of strings, so
/// pulling in a full serializer is not worth it.
trait JsonMap {
    fn to_json(&self) -> String;
    fn from_json(text: &str) -> Option<Self>
    where
        Self: Sized;
}

impl JsonMap for RawSettings {
    fn to_json(&self) -> String {
        let entries: Vec<String> = self
            .iter()
            .map(|(key, value)| format!("  {}: {}", quote(key), quote(value)))
            .collect();
        format!("{{\n{}\n}}\n", entries.join(",\n"))
    }

    fn from_json(text: &str) -> Option<Self> {
        let mut map = RawSettings::new();
        let chars = text.chars().peekable();
        let mut key: Option<String> = None;
        let mut buffer = String::new();
        let mut in_string = false;
        let mut escaped = false;

        for character in chars {
            if in_string {
                if escaped {
                    // Escapes are limited to what `quote` emits.
                    buffer.push(match character {
                        'n' => '\n',
                        other => other,
                    });
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    in_string = false;
                    match key.take() {
                        None => key = Some(std::mem::take(&mut buffer)),
                        Some(name) => {
                            map.insert(name, std::mem::take(&mut buffer));
                        }
                    }
                } else {
                    buffer.push(character);
                }
                continue;
            }
            match character {
                '"' => {
                    in_string = true;
                    buffer.clear();
                }
                '}' => break,
                _ => {}
            }
        }
        Some(map)
    }
}

fn quote(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

/// Reads the SwiftUI build's domain as `key -> value` text.
#[cfg(target_os = "macos")]
fn macos_defaults() -> Option<RawSettings> {
    use std::process::Command;

    // `defaults export -` prints an XML plist; reading individual keys keeps the
    // parsing to what `defaults read <domain> <key>` prints for one value.
    const KEYS: [&str; 8] = [
        "historySize",
        "showFooter",
        "showSearch",
        "showTitle",
        "pasteByDefault",
        "searchMode",
        "sortBy",
        "popupPosition",
    ];

    let mut raw = RawSettings::new();
    for key in KEYS {
        let output = Command::new("/usr/bin/defaults")
            .args(["read", "com.kaigedong.manispocket", key])
            .output()
            .ok()?;
        if !output.status.success() {
            continue;
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !value.is_empty() {
            raw.insert(key.to_string(), value);
        }
    }
    Some(raw)
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_swift_build() {
        let settings = Settings::default();
        assert_eq!(settings.history_size, 500);
        assert_eq!(settings.window_width, 450.0);
        assert_eq!(settings.window_height, 800.0);
        assert_eq!(settings.search_mode, "exact");
        assert!(settings.show_footer);
        assert!(!settings.unlimited_history);
    }

    #[test]
    fn stored_values_override_defaults() {
        let mut raw = RawSettings::new();
        raw.insert("historySize".into(), "1004".into());
        raw.insert("showFooter".into(), "0".into());
        raw.insert("searchMode".into(), "fuzzy".into());
        let settings = Settings::from_raw(&raw);
        assert_eq!(settings.history_size, 1004);
        assert!(!settings.show_footer);
        assert_eq!(settings.search_mode, "fuzzy");
        // Untouched keys keep their default.
        assert_eq!(settings.window_width, 450.0);
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let mut raw = RawSettings::new();
        raw.insert("ignoredApps".into(), "Slack".into());
        raw.insert("historySize".into(), "42".into());
        let settings = Settings::from_raw(&raw);
        let written = settings.to_raw();
        assert_eq!(
            written.get("ignoredApps").map(String::as_str),
            Some("Slack")
        );
        assert_eq!(written.get("historySize").map(String::as_str), Some("42"));
    }

    #[test]
    fn ignore_lists_round_trip() {
        let mut raw = RawSettings::new();
        raw.insert("ignoredApps".into(), "com.a\ncom.b".into());
        raw.insert("ignoreRegexp".into(), "password".into());
        let settings = Settings::from_raw(&raw);
        assert_eq!(settings.ignored_apps, vec!["com.a", "com.b"]);
        assert_eq!(settings.ignore_regexp, vec!["password"]);
        let written = settings.to_raw();
        assert_eq!(
            written.get("ignoredApps").map(String::as_str),
            Some("com.a\ncom.b")
        );
    }

    #[test]
    fn json_survives_quotes_and_newlines() {
        let mut raw = RawSettings::new();
        raw.insert("quoted".into(), "a\"b\\c\nd".into());
        let text = raw.to_json();
        let parsed = RawSettings::from_json(&text).expect("parse");
        assert_eq!(parsed.get("quoted").map(String::as_str), Some("a\"b\\c\nd"));
    }
}
