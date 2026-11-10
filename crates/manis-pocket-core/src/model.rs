use uniffi;

/// A single content entry within a clipboard item (e.g., text, image data, file URL).
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClipboardContent {
    /// UTI string like "public.utf8-plain-text", "public.png", etc.
    pub content_type: String,
    /// Raw bytes of the content value.
    pub value: Option<Vec<u8>>,
}

/// A clipboard history item — the central domain model.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClipboardItem {
    pub id: String,
    pub application: Option<String>,
    /// Unix epoch milliseconds
    pub first_copied_at: i64,
    /// Unix epoch milliseconds
    pub last_copied_at: i64,
    pub number_of_copies: i32,
    /// Pin key character (e.g., "b", "c"), or None if unpinned
    pub pin: Option<String>,
    pub title: String,
    pub contents: Vec<ClipboardContent>,
    /// Unix epoch milliseconds
    pub sync_timestamp: i64,
    pub sync_source: Option<String>,
    pub sync_deleted: bool,
}

/// Half-open Unicode scalar range in the original title for search highlighting.
#[derive(Clone, Debug, uniffi::Record)]
pub struct MatchRange {
    pub start: i64,
    pub end: i64,
}

/// Result of a search operation.
#[derive(Clone, Debug, uniffi::Record)]
pub struct SearchResult {
    pub item: ClipboardItem,
    /// Fuzzy match score (lower is better). None for exact/regex matches.
    pub score: Option<f64>,
    /// Unicode scalar ranges in the original title that match the query.
    pub ranges: Vec<MatchRange>,
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum SearchMode {
    Exact,
    Fuzzy,
    Regexp,
    Mixed,
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum SortBy {
    LastCopiedAt,
    FirstCopiedAt,
    NumberOfCopies,
}

#[derive(Debug, uniffi::Error, thiserror::Error)]
pub enum CoreError {
    #[error("Storage error: {msg}")]
    Storage { msg: String },

    #[error("Item not found: {id}")]
    NotFound { id: String },

    #[error("Invalid argument: {msg}")]
    InvalidArg { msg: String },

    #[error("Sync error: {msg}")]
    Sync { msg: String },
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        CoreError::Storage { msg: e.to_string() }
    }
}
