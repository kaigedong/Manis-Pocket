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
    /// Stable identifier, also used as the search key and the sync identity.
    pub id: String,
    /// Bundle id or window class of the app that copied it, when known.
    pub application: Option<String>,
    /// Unix epoch milliseconds
    pub first_copied_at: i64,
    /// Unix epoch milliseconds
    pub last_copied_at: i64,
    /// How often the same payload has been copied.
    pub number_of_copies: i32,
    /// Pin key character (e.g., "b", "c"), or None if unpinned
    pub pin: Option<String>,
    /// The line the list shows: text, a file name, or a label for an image.
    pub title: String,
    /// One entry per representation. Metadata loads leave these empty.
    pub contents: Vec<ClipboardContent>,
    /// Unix epoch milliseconds
    pub sync_timestamp: i64,
    /// Peer that sent it, when it did not originate here.
    pub sync_source: Option<String>,
    /// Tombstone flag: the entry was deleted on another device.
    pub sync_deleted: bool,
}

/// Half-open Unicode scalar range in the original title for search highlighting.
#[derive(Clone, Debug, uniffi::Record)]
pub struct MatchRange {
    /// First matching scalar.
    pub start: i64,
    /// One past the last matching scalar.
    pub end: i64,
}

/// Result of a search operation.
#[derive(Clone, Debug, uniffi::Record)]
pub struct SearchResult {
    /// The entry that matched.
    pub item: ClipboardItem,
    /// Fuzzy match score (lower is better). None for exact/regex matches.
    pub score: Option<f64>,
    /// Unicode scalar ranges in the original title that match the query.
    pub ranges: Vec<MatchRange>,
}

/// How a query is matched against titles.
#[derive(Clone, Debug, uniffi::Enum)]
pub enum SearchMode {
    /// Case-insensitive substring.
    Exact,
    /// Characters in order with gaps allowed.
    Fuzzy,
    /// A regular expression, which may fail to compile.
    Regexp,
    /// Exact, falling back to fuzzy when nothing matches literally.
    Mixed,
}

/// The order the list is shown in.
#[derive(Clone, Debug, uniffi::Enum)]
pub enum SortBy {
    /// Most recently copied first.
    LastCopiedAt,
    /// Oldest copy first.
    FirstCopiedAt,
    /// Most copied first.
    NumberOfCopies,
}

/// Everything the core can fail with.
#[derive(Debug, uniffi::Error, thiserror::Error)]
pub enum CoreError {
    /// The database, the filesystem or a stored payload was unusable.
    #[error("Storage error: {msg}")]
    Storage {
        /// What went wrong, already formatted for the caller.
        msg: String,
    },

    /// The caller named an entry that is not there.
    #[error("Item not found: {id}")]
    NotFound {
        /// The id that was looked up.
        id: String,
    },

    /// The caller passed something the core cannot use.
    #[error("Invalid argument: {msg}")]
    InvalidArg {
        /// What was wrong with the argument.
        msg: String,
    },

    /// The sync engine or its protocol rejected the operation.
    #[error("Sync error: {msg}")]
    Sync {
        /// What went wrong, already formatted for the caller.
        msg: String,
    },
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        CoreError::Storage { msg: e.to_string() }
    }
}
