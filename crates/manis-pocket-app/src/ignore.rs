//! Rules for what the clipboard watcher should not record.
//!
//! The macOS build lets a user ignore whole applications, ignore text matching a
//! regular expression, or invert the app rule to only track a listed allowlist.
//! All three are checked before an entry is written, so ignored copies never
//! enter history and never reach paired devices.

use crate::settings::Settings;

/// Whether a copy should be skipped entirely.
///
/// `application` is the bundle id / window class that produced it, when the
/// platform can say.
pub fn should_ignore(settings: &Settings, application: Option<&str>, text: &str) -> bool {
    if ignored_by_application(settings, application) {
        return true;
    }
    matches_any_pattern(&settings.ignore_regexp, text)
}

fn ignored_by_application(settings: &Settings, application: Option<&str>) -> bool {
    let listed = |name: &str| {
        settings
            .ignored_apps
            .iter()
            .any(|ignored| ignored.eq_ignore_ascii_case(name))
    };
    match (settings.ignore_all_apps_except_listed, application) {
        // Allowlist mode: anything not listed is ignored. An unknown source is
        // not on the list, so it is ignored too.
        (true, Some(name)) => !listed(name),
        (true, None) => true,
        (false, Some(name)) => listed(name),
        (false, None) => false,
    }
}

fn matches_any_pattern(patterns: &[String], text: &str) -> bool {
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim();
        !pattern.is_empty()
            && regex::Regex::new(pattern)
                .map(|compiled| compiled.is_match(text))
                .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::default()
    }

    #[test]
    fn nothing_is_ignored_by_default() {
        assert!(!should_ignore(&settings(), Some("com.apple.Terminal"), "hi"));
        assert!(!should_ignore(&settings(), None, "hi"));
    }

    #[test]
    fn listed_applications_are_ignored() {
        let mut rules = settings();
        rules.ignored_apps = vec!["com.agilebits.onepassword".into()];
        assert!(should_ignore(&rules, Some("com.agilebits.onepassword"), "secret"));
        assert!(!should_ignore(&rules, Some("com.apple.Terminal"), "hi"));
        // Unknown source is not on the ignore list.
        assert!(!should_ignore(&rules, None, "hi"));
    }

    #[test]
    fn allowlist_mode_ignores_everything_else() {
        let mut rules = settings();
        rules.ignore_all_apps_except_listed = true;
        rules.ignored_apps = vec!["com.apple.Terminal".into()];
        assert!(!should_ignore(&rules, Some("com.apple.Terminal"), "ok"));
        assert!(should_ignore(&rules, Some("com.google.Chrome"), "nope"));
        // With no source the entry cannot prove it is allowed.
        assert!(should_ignore(&rules, None, "nope"));
    }

    #[test]
    fn regular_expressions_are_matched() {
        let mut rules = settings();
        rules.ignore_regexp = vec![r"^\s*$".into(), "password".into()];
        assert!(should_ignore(&rules, None, "password=hunter2"));
        assert!(!should_ignore(&rules, None, "hello"));
    }

    #[test]
    fn invalid_patterns_are_skipped_not_fatal() {
        let mut rules = settings();
        rules.ignore_regexp = vec!["([unclosed".into()];
        assert!(!should_ignore(&rules, None, "anything"));
    }
}
