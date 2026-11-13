//! The parts of Manis Pocket that are not tied to a window: the history, the
//! search, and the sync protocol.
//!
//! The app crate draws the UI and the platform crates talk to the clipboard; this
//! crate owns the data and the rules, so both can be tested without a display.

#![warn(missing_docs)]

/// Error types shared with the platform layers.
pub mod error;
/// The clipboard history: storage, deduplication, pins and trimming.
pub mod history;
/// The domain types the UI and the sync protocol exchange.
pub mod model;
/// The clipboard observer interface the platform layers implement.
pub mod platform;
/// Query matching, from exact substrings to regular expressions.
pub mod search;
/// Ordering rules for pinned and unpinned entries.
pub mod sort;
/// The SQLite schema, its migrations and the SwiftData import.
pub mod storage;
/// The peer protocol: what is shared, how versions merge, what is rejected.
pub mod sync;

uniffi::setup_scaffolding!();
