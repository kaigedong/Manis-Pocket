//! How a history entry is presented as a list row.
//!
//! Kept free of GPUI types so the text rules can be tested directly: these are
//! the strings a user reads, and the macOS build has specific rules for them
//! (text entries show their text, image entries show nothing useful and need a
//! label instead, times are relative rather than absolute).

use manis_pocket_core::model::{ClipboardContent, ClipboardItem};

/// What kind of payload a row shows, which decides its icon and fallback label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Text,
    Image,
    File,
    Other,
}

const TEXT_TYPES: [&str; 3] = [
    "public.utf8-plain-text",
    "text/plain",
    "text/plain;charset=utf-8",
];

const IMAGE_TYPES: [&str; 3] = ["public.png", "public.tiff", "image/png"];

pub fn row_kind(item: &ClipboardItem) -> RowKind {
    let has = |types: &[&str]| {
        item.contents
            .iter()
            .any(|content| types.contains(&content.content_type.as_str()))
    };
    if has(&TEXT_TYPES) {
        RowKind::Text
    } else if has(&IMAGE_TYPES) {
        RowKind::Image
    } else if has(&["public.file-url"]) {
        RowKind::File
    } else {
        RowKind::Other
    }
}

/// The row's first line.
///
/// Text entries have a title already (the first line of what was copied). Rows
/// without text would otherwise render as an empty line, so they get a label.
pub fn row_title(item: &ClipboardItem) -> String {
    row_title_with_symbols(item, true)
}

/// Renders a title the way the SwiftUI build does.
///
/// `showSpecialSymbols` turns the whitespace that would otherwise be invisible
/// into visible glyphs; without it the title is simply trimmed.
pub fn row_title_with_symbols(item: &ClipboardItem, show_symbols: bool) -> String {
    let raw = item.title.as_str();
    let title = if show_symbols {
        decorate_symbols(raw)
    } else {
        raw.trim().to_string()
    };
    let title = title.trim_end_matches(' ');
    if !title.is_empty() {
        return title.to_string();
    }
    match row_kind(item) {
        RowKind::Text => String::new(),
        RowKind::Image => "图片".to_string(),
        RowKind::File => "文件".to_string(),
        RowKind::Other => fallback_from_type(item.contents.first()),
    }
}

/// Replaces leading/trailing spaces and line breaks with visible markers.
fn decorate_symbols(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let leading = chars.iter().take_while(|c| **c == ' ').count();
    for _ in 0..leading {
        out.push('·');
    }
    let body: String = chars[leading..].iter().collect();
    let trimmed_end = body.trim_end_matches(' ');
    let trailing = body.chars().count() - trimmed_end.chars().count();
    for c in trimmed_end.chars() {
        match c {
            '\n' => out.push('⏎'),
            '\t' => out.push('⇥'),
            other => out.push(other),
        }
    }
    for _ in 0..trailing {
        out.push('·');
    }
    out
}

fn fallback_from_type(content: Option<&ClipboardContent>) -> String {
    match content {
        Some(content) => format!("[{}]", content.content_type),
        None => "空剪贴板".to_string(),
    }
}

/// Shortens a title the way `truncationMode(.middle)` does, keeping both ends.
///
/// GPUI can only truncate the tail, so the string is shortened here before it is
/// laid out. `max_chars` is a character budget, not a measured width.
pub fn truncate_middle(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars || max_chars < 3 {
        return text.to_string();
    }
    // One character is spent on the ellipsis.
    let keep = max_chars - 1;
    let head = keep.div_ceil(2);
    let tail = keep - head;
    let mut shortened: String = chars[..head].iter().collect();
    shortened.push('…');
    shortened.extend(chars[chars.len() - tail..].iter());
    shortened
}

/// A colour swatch shown before the title when the copied text *is* a colour.
pub fn hex_color(text: &str) -> Option<(u8, u8, u8)> {
    let value = text.trim().trim_start_matches('#');
    if !(value.len() == 6 || value.len() == 3) || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let expand = |part: &str| {
        if part.len() == 1 {
            u8::from_str_radix(&format!("{part}{part}"), 16).ok()
        } else {
            u8::from_str_radix(part, 16).ok()
        }
    };
    let step = value.len() / 3;
    Some((
        expand(&value[0..step])?,
        expand(&value[step..step * 2])?,
        expand(&value[step * 2..])?,
    ))
}

/// A stand-in for the source application's icon.
///
/// Real icons need `NSWorkspace`/`GIcon` lookups per entry; until those land the
/// row shows the app's initial on a colour derived from its bundle id, so rows
/// stay distinguishable and stable between runs.
pub fn app_badge(bundle_id: &str) -> (String, (u8, u8, u8)) {
    let name = application_name(bundle_id);
    let initial = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    // FNV-1a over the bundle id: stable, and spreads the hues.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bundle_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    let hue = (hash % 360) as f32;
    (initial, hsl_to_rgb(hue, 0.55, 0.45))
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = lightness - c / 2.0;
    let (r, g, b) = match hue as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to_byte = |value: f32| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_byte(r), to_byte(g), to_byte(b))
}

/// Splits a title into (segment, is_match) pieces for highlighting.
///
/// Ranges come from the core's search results as Unicode scalar offsets. They may
/// be unordered, overlap, or reach past a shortened title, so they are clamped and
/// merged here before rendering.
pub fn highlight_spans(text: &str, ranges: &[(usize, usize)]) -> Vec<(String, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut bounds: Vec<(usize, usize)> = ranges
        .iter()
        .filter_map(|(start, end)| {
            let start = (*start).min(len);
            let end = (*end).min(len);
            (start < end).then_some((start, end))
        })
        .collect();
    if bounds.is_empty() {
        return vec![(text.to_string(), false)];
    }
    bounds.sort_unstable();

    let mut spans = Vec::new();
    let mut cursor = 0;
    for (start, end) in bounds {
        // Overlapping ranges are folded into the current match.
        let start = start.max(cursor);
        if start >= end {
            continue;
        }
        if start > cursor {
            spans.push((chars[cursor..start].iter().collect::<String>(), false));
        }
        // Adjacent matches stay one span so the highlight has no seams.
        let segment: String = chars[start..end].iter().collect();
        match spans.last_mut() {
            Some((previous, true)) => previous.push_str(&segment),
            _ => spans.push((segment, true)),
        }
        cursor = end;
    }
    if cursor < len {
        spans.push((chars[cursor..].iter().collect::<String>(), false));
    }
    spans
}

/// Free pin letters, in order. Pins are labelled a…z so a pinned entry can be
/// pasted straight from the keyboard.
pub fn available_pins(used: &[String]) -> Vec<String> {
    ('a'..='z')
        .map(|letter| letter.to_string())
        .filter(|letter| !used.iter().any(|taken| taken.eq_ignore_ascii_case(letter)))
        .collect()
}

/// Items arranged into the list's two sections.
pub struct Arranged {
    pub items: Vec<ClipboardItem>,
    /// How many leading entries are pinned; zero means there is no pinned
    /// section to draw.
    pub pinned_count: usize,
}

/// Puts pinned entries first (or last) and keeps each group in its own order.
///
/// Pinned entries sort by their pin letter so the keyboard order matches what the
/// user sees; the rest keep the order the core returned.
pub fn arrange(items: Vec<ClipboardItem>, pins_position: &str, sort_by: &str) -> Arranged {
    let (mut pinned, mut rest): (Vec<_>, Vec<_>) =
        items.into_iter().partition(|item| item.pin.is_some());
    pinned.sort_by(|a, b| a.pin.cmp(&b.pin));

    // History order follows the `sortBy` setting; pinned entries keep their
    // letter order regardless.
    match sort_by {
        "firstCopiedAt" => rest.sort_by_key(|item| item.first_copied_at),
        "numberOfCopies" => rest.sort_by_key(|item| std::cmp::Reverse(item.number_of_copies)),
        _ => rest.sort_by_key(|item| std::cmp::Reverse(item.last_copied_at)),
    }

    let pinned_count = pinned.len();
    let mut arranged = Vec::with_capacity(pinned_count + rest.len());
    if pins_position == "bottom" {
        arranged.append(&mut rest);
        arranged.append(&mut pinned);
        return Arranged {
            items: arranged,
            pinned_count: 0,
        };
    }
    arranged.append(&mut pinned);
    arranged.append(&mut rest);
    Arranged {
        items: arranged,
        pinned_count,
    }
}

/// Lines shown in the hover/preview sidebar for one entry.
///
/// The macOS preview shows the full text, where it came from and when; the same
/// information is assembled here so it can be tested without a window.
pub fn preview_lines(item: &ClipboardItem, now_ms: i64, max_chars: usize) -> Vec<String> {
    let body = match row_kind(item) {
        RowKind::Text => {
            let text = item
                .contents
                .iter()
                .find(|content| TEXT_TYPES.contains(&content.content_type.as_str()))
                .and_then(|content| content.value.as_ref())
                .and_then(|value| String::from_utf8(value.clone()).ok())
                .unwrap_or_else(|| item.title.clone());
            if text.chars().count() > max_chars {
                let mut shortened: String = text.chars().take(max_chars).collect();
                shortened.push('…');
                shortened
            } else {
                text
            }
        }
        // The list only carries content types, so the size is unknown until the
        // payload is fetched; say "图片" rather than a misleading "0 字节".
        RowKind::Image => match content_len(item) {
            0 => "图片".to_string(),
            bytes => format!("图片（{bytes} 字节）"),
        },
        RowKind::File => "文件".to_string(),
        RowKind::Other => format!("[{}]", content_type_of(item)),
    };
    vec![body, row_meta(item, now_ms)]
}

fn content_len(item: &ClipboardItem) -> usize {
    item.contents
        .first()
        .and_then(|content| content.value.as_ref())
        .map(Vec::len)
        .unwrap_or(0)
}

fn content_type_of(item: &ClipboardItem) -> String {
    item.contents
        .first()
        .map(|content| content.content_type.clone())
        .unwrap_or_else(|| "空".to_string())
}

/// The row's second line: where it came from and when.
///
/// The macOS row itself is single-line — this text belongs to the hover preview
/// pane, which is not built yet.
#[allow(dead_code)]
pub fn row_meta(item: &ClipboardItem, now_ms: i64) -> String {
    let source = item
        .application
        .as_deref()
        .map(application_name)
        .unwrap_or_else(|| "未知来源".to_string());
    format!("{source}  {}", relative_time(item.last_copied_at, now_ms))
}

/// Bundle identifiers are what the pasteboard gives us; show something a person
/// recognises instead of `com.tinyspeck.slackmacgap`.
pub fn application_name(bundle_id: &str) -> String {
    match bundle_id {
        "com.tinyspeck.slackmacgap" => "Slack".to_string(),
        "com.apple.Terminal" => "Terminal".to_string(),
        "com.google.Chrome" => "Chrome".to_string(),
        "com.apple.Safari" => "Safari".to_string(),
        "com.microsoft.VSCode" => "Visual Studio Code".to_string(),
        "com.apple.finder" => "Finder".to_string(),
        "com.hnc.Discord" => "Discord".to_string(),
        other => other
            // Fall back to the last dotted component, which is usually the name.
            .rsplit('.')
            .next()
            .unwrap_or(other)
            .to_string(),
    }
}

/// Relative time, as the macOS panel shows it.
#[allow(dead_code)]
pub fn relative_time(timestamp_ms: i64, now_ms: i64) -> String {
    let seconds = (now_ms - timestamp_ms) / 1000;
    match seconds {
        i64::MIN..=5 => "刚刚".to_string(),
        6..=59 => format!("{seconds} 秒前"),
        60..=3599 => format!("{} 分钟前", seconds / 60),
        3600..=86_399 => format!("{} 小时前", seconds / 3600),
        86_400..=604_799 => format!("{} 天前", seconds / 86_400),
        _ => format!("{} 个月前", seconds / 2_592_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, types: &[&str]) -> ClipboardItem {
        ClipboardItem {
            id: "1".into(),
            application: None,
            first_copied_at: 0,
            last_copied_at: 0,
            number_of_copies: 1,
            pin: None,
            title: title.into(),
            contents: types
                .iter()
                .map(|content_type| {
                    // Text entries carry their text, like the real thing; other
                    // types just need a non-empty payload.
                    let is_text = TEXT_TYPES.contains(content_type);
                    ClipboardContent {
                        content_type: (*content_type).into(),
                        value: Some(if is_text {
                            title.as_bytes().to_vec()
                        } else {
                            b"x".to_vec()
                        }),
                    }
                })
                .collect(),
            sync_timestamp: 0,
            sync_source: None,
            sync_deleted: false,
        }
    }

    #[test]
    fn text_rows_show_their_text() {
        let text = item("hello", &["public.utf8-plain-text"]);
        assert_eq!(row_kind(&text), RowKind::Text);
        assert_eq!(row_title(&text), "hello");
    }

    #[test]
    fn image_rows_get_a_label_instead_of_an_empty_line() {
        let image = item("", &["public.png"]);
        assert_eq!(row_kind(&image), RowKind::Image);
        assert_eq!(row_title(&image), "图片");
    }

    #[test]
    fn unknown_rows_fall_back_to_their_type() {
        let other = item("", &["com.example.custom"]);
        assert_eq!(row_kind(&other), RowKind::Other);
        assert_eq!(row_title(&other), "[com.example.custom]");
    }

    #[test]
    fn bundle_ids_are_shown_as_application_names() {
        assert_eq!(application_name("com.tinyspeck.slackmacgap"), "Slack");
        // Unknown bundles fall back to the last component.
        assert_eq!(application_name("com.example.Notes"), "Notes");
    }

    #[test]
    fn times_are_relative() {
        let now = 1_000_000_000_000;
        assert_eq!(relative_time(now, now), "刚刚");
        assert_eq!(relative_time(now - 30_000, now), "30 秒前");
        assert_eq!(relative_time(now - 300_000, now), "5 分钟前");
        assert_eq!(relative_time(now - 7_200_000, now), "2 小时前");
        assert_eq!(relative_time(now - 172_800_000, now), "2 天前");
    }

    #[test]
    fn special_symbols_mark_invisible_whitespace() {
        assert_eq!(decorate_symbols("  hello"), "··hello");
        assert_eq!(decorate_symbols("hello  "), "hello··");
        assert_eq!(decorate_symbols("a\nb\tc"), "a⏎b⇥c");
        // Without the setting the title is only trimmed.
        let mut entry = item("  padded  ", &["public.utf8-plain-text"]);
        entry.title = "  padded  ".into();
        assert_eq!(row_title_with_symbols(&entry, false), "padded");
        assert_eq!(row_title_with_symbols(&entry, true), "··padded··");
    }

    #[test]
    fn sort_order_follows_the_setting() {
        let mut first = item("first", &["public.utf8-plain-text"]);
        first.first_copied_at = 10;
        first.last_copied_at = 100;
        first.number_of_copies = 1;
        let mut last = item("last", &["public.utf8-plain-text"]);
        last.first_copied_at = 20;
        last.last_copied_at = 50;
        last.number_of_copies = 9;

        let by_last = arrange(vec![first.clone(), last.clone()], "top", "lastCopiedAt");
        assert_eq!(by_last.items[0].title, "first", "newest last_copied_at first");

        let by_first = arrange(vec![first.clone(), last.clone()], "top", "firstCopiedAt");
        assert_eq!(by_first.items[0].title, "first", "oldest first_copied_at first");

        let by_copies = arrange(vec![first, last], "top", "numberOfCopies");
        assert_eq!(by_copies.items[0].title, "last", "most copies first");
    }

    #[test]
    fn pin_letters_skip_the_ones_in_use() {
        assert_eq!(available_pins(&[])[0], "a");
        assert_eq!(
            available_pins(&["a".into(), "c".into()])[0],
            "b",
            "letters already taken are skipped"
        );
        let used: Vec<String> = ('a'..='z').map(|c| c.to_string()).collect();
        assert!(available_pins(&used).is_empty());
    }

    #[test]
    fn pinned_entries_move_to_the_top_in_letter_order() {
        let mut pinned_b = item("b", &["public.utf8-plain-text"]);
        pinned_b.pin = Some("b".into());
        let mut pinned_a = item("a", &["public.utf8-plain-text"]);
        pinned_a.pin = Some("a".into());
        let loose = item("loose", &["public.utf8-plain-text"]);

        let arranged = arrange(vec![loose.clone(), pinned_b, pinned_a], "top", "lastCopiedAt");
        assert_eq!(arranged.pinned_count, 2);
        // Titles identify the entries: the test helper's rows share one id.
        assert_eq!(arranged.items[0].pin, Some("a".to_string()));
        assert_eq!(arranged.items[1].pin, Some("b".to_string()));
        assert!(arranged.items[2].pin.is_none());

        // Bottom pins keep them after the history and expose no pinned section.
        let bottom = arrange(
            vec![loose, item("x", &["public.utf8-plain-text"])],
            "bottom",
            "lastCopiedAt",
        );
        assert_eq!(bottom.pinned_count, 0);
    }

    #[test]
    fn search_ranges_turn_into_highlight_spans() {
        use manis_pocket_core::history::HistoryManager;
        use manis_pocket_core::model::SearchMode;

        let manager = HistoryManager::new(":memory:".to_string()).unwrap();
        let mut entry = item("deploy the service", &["public.utf8-plain-text"]);
        entry.id = "e2e".into();
        manager.add(entry, 100, false).unwrap();

        let results = manager.search("deploy", manager.load_metadata().unwrap(), SearchMode::Mixed);
        assert_eq!(results.len(), 1);
        let ranges: Vec<(usize, usize)> = results[0]
            .ranges
            .iter()
            .map(|range| (range.start as usize, range.end as usize))
            .collect();
        assert!(!ranges.is_empty(), "the core reports match ranges");

        let spans = highlight_spans("deploy the service", &ranges);
        assert!(spans.iter().any(|(text, matched)| *matched && text.eq_ignore_ascii_case("deploy")));
    }

    #[test]
    fn highlight_splits_around_matches() {
        assert_eq!(highlight_spans("hello", &[]), vec![("hello".to_string(), false)]);
        assert_eq!(
            highlight_spans("hello world", &[(6, 11)]),
            vec![
                ("hello ".to_string(), false),
                ("world".to_string(), true)
            ]
        );
        // Two matches produce alternating spans.
        let spans = highlight_spans("ab cd", &[(0, 2), (3, 5)]);
        assert_eq!(
            spans,
            vec![
                ("ab".to_string(), true),
                (" ".to_string(), false),
                ("cd".to_string(), true)
            ]
        );
    }

    #[test]
    fn highlight_tolerates_messy_ranges() {
        // Overlapping ranges are merged, and out-of-bounds ones clamped.
        let spans = highlight_spans("abcdef", &[(1, 4), (2, 3), (4, 99)]);
        assert_eq!(
            spans,
            vec![("a".to_string(), false), ("bcdef".to_string(), true)]
        );
        // Ranges entirely past the text are dropped rather than panicking.
        assert_eq!(highlight_spans("ab", &[(5, 9)]), vec![("ab".to_string(), false)]);
    }

    #[test]
    fn preview_shows_the_text_and_its_source() {
        let mut entry = item("hello world", &["public.utf8-plain-text"]);
        entry.application = Some("com.apple.Terminal".into());
        entry.last_copied_at = 999_000;
        let lines = preview_lines(&entry, 1_000_000, 100);
        assert_eq!(lines[0], "hello world");
        assert!(lines[1].starts_with("Terminal"));

        // Long text is cut with an ellipsis rather than blowing up the sidebar.
        let long = item(&"x".repeat(500), &["public.utf8-plain-text"]);
        let lines = preview_lines(&long, 1_000_000, 20);
        assert_eq!(lines[0].chars().count(), 21);

        // Non-text entries describe themselves instead of showing nothing.
        let image = item("", &["public.png"]);
        assert!(preview_lines(&image, 0, 100)[0].starts_with("图片"));
    }

    #[test]
    fn middle_truncation_keeps_both_ends() {
        assert_eq!(truncate_middle("short", 10), "short");
        let shortened = truncate_middle("abcdefghijklmnopqrst", 10);
        assert_eq!(shortened.chars().count(), 10);
        assert!(shortened.starts_with("abcde"));
        assert!(shortened.ends_with("qrst"));
        assert!(shortened.contains('…'));
        // CJK text is truncated by characters, not bytes.
        assert_eq!(truncate_middle("这是很长的一段中文标题内容", 8).chars().count(), 8);
    }

    #[test]
    fn hex_colours_are_recognised() {
        assert_eq!(hex_color("#FF0000"), Some((255, 0, 0)));
        assert_eq!(hex_color("00ff00"), Some((0, 255, 0)));
        assert_eq!(hex_color("#0f0"), Some((0, 255, 0)));
        assert_eq!(hex_color("not a colour"), None);
        assert_eq!(hex_color("#12345"), None);
    }

    #[test]
    fn app_badges_are_stable_and_distinct() {
        let (initial, colour) = app_badge("com.tinyspeck.slackmacgap");
        assert_eq!(initial, "S");
        // Same input, same output across calls (no randomness).
        assert_eq!((initial, colour), app_badge("com.tinyspeck.slackmacgap"));
        assert_ne!(
            app_badge("com.apple.Terminal").1,
            app_badge("com.google.Chrome").1
        );
    }

    #[test]
    #[allow(dead_code)]
    fn meta_prefixes_the_source_and_time() {
        let mut text = item("hello", &["public.utf8-plain-text"]);
        text.application = Some("com.apple.Terminal".into());
        text.last_copied_at = 999_000;
        assert_eq!(row_meta(&text, 1_000_000), "Terminal  刚刚");
    }
}
