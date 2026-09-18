use crate::model::{ClipboardItem, SortBy};

/// Sort clipboard items by the given criteria, with pinned items at top or bottom.
pub fn sort_items(
    items: Vec<ClipboardItem>,
    sort_by: SortBy,
    pin_to_top: bool,
) -> Vec<ClipboardItem> {
    let mut sorted = items;

    // Primary sort
    sorted.sort_by(|a, b| match sort_by {
        SortBy::FirstCopiedAt => b.first_copied_at.cmp(&a.first_copied_at),
        SortBy::NumberOfCopies => b.number_of_copies.cmp(&a.number_of_copies),
        SortBy::LastCopiedAt => b.last_copied_at.cmp(&a.last_copied_at),
    });

    // Stable re-sort to push pinned items to top or bottom
    sorted.sort_by(|a, b| {
        let a_pinned = a.pin.is_some();
        let b_pinned = b.pin.is_some();

        if pin_to_top {
            // Pinned items should come first
            b_pinned.cmp(&a_pinned)
        } else {
            // Pinned items should come last
            a_pinned.cmp(&b_pinned)
        }
    });

    sorted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipboardContent;

    fn make_item(id: &str, last: i64, first: i64, copies: i32, pin: Option<&str>) -> ClipboardItem {
        ClipboardItem {
            id: id.to_string(),
            application: None,
            first_copied_at: first,
            last_copied_at: last,
            number_of_copies: copies,
            pin: pin.map(|s| s.to_string()),
            title: format!("Item {}", id),
            contents: vec![],
            sync_timestamp: 0,
            sync_source: None,
            sync_deleted: false,
        }
    }

    #[test]
    fn test_sort_by_last_copied() {
        let items = vec![
            make_item("1", 100, 50, 1, None),
            make_item("2", 200, 50, 1, None),
            make_item("3", 150, 50, 1, None),
        ];
        let sorted = sort_items(items, SortBy::LastCopiedAt, true);
        assert_eq!(sorted[0].id, "2");
        assert_eq!(sorted[1].id, "3");
        assert_eq!(sorted[2].id, "1");
    }

    #[test]
    fn test_sort_by_number_of_copies() {
        let items = vec![
            make_item("1", 100, 50, 3, None),
            make_item("2", 100, 50, 1, None),
            make_item("3", 100, 50, 5, None),
        ];
        let sorted = sort_items(items, SortBy::NumberOfCopies, true);
        assert_eq!(sorted[0].id, "3");
        assert_eq!(sorted[1].id, "1");
        assert_eq!(sorted[2].id, "2");
    }

    #[test]
    fn test_sort_pinned_to_top() {
        let items = vec![
            make_item("1", 200, 50, 1, None),
            make_item("2", 100, 50, 1, Some("b")),
            make_item("3", 150, 50, 1, None),
        ];
        let sorted = sort_items(items, SortBy::LastCopiedAt, true);
        assert_eq!(sorted[0].id, "2"); // pinned
        assert_eq!(sorted[1].id, "1"); // 200
        assert_eq!(sorted[2].id, "3"); // 150
    }

    #[test]
    fn test_sort_pinned_to_bottom() {
        let items = vec![
            make_item("1", 200, 50, 1, None),
            make_item("2", 100, 50, 1, Some("b")),
            make_item("3", 150, 50, 1, None),
        ];
        let sorted = sort_items(items, SortBy::LastCopiedAt, false);
        assert_eq!(sorted[0].id, "1"); // 200
        assert_eq!(sorted[1].id, "3"); // 150
        assert_eq!(sorted[2].id, "2"); // pinned at bottom
    }
}
