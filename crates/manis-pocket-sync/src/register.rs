use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// Keep the complete CBOR request comfortably below libp2p's default 1 MiB limit.
pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 512 * 1024;

/// Upper bound for a binary clipboard payload, measured in decoded bytes.
///
/// 10 MiB covers large 4K/5K PNG screenshots. The value travels base64-encoded
/// inside the CBOR `Apply` request, so the transport ceiling is 20 MiB (see
/// `network.rs`): 10 MiB of bytes becomes ~13.3 MiB of base64 plus the envelope.
pub const MAX_CLIPBOARD_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// The only binary MIME this protocol version ships.
///
/// Senders normalise images to PNG because Wayland consumers ask for
/// `image/png`; macOS-native formats (TIFF/HEIC) cannot be applied on Linux.
pub const SHAREABLE_IMAGE_MIME: &str = "image/png";

/// A Lamport revision. The peer ID breaks ties for concurrent local copies.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Revision {
    pub counter: u64,
    pub peer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardValue {
    Text(String),
    /// Binary payload (base64-encoded `data`) such as an image.
    ///
    /// Only [`SHAREABLE_IMAGE_MIME`] is accepted by this protocol version.
    Binary {
        mime: String,
        data: String,
    },
    /// The system clipboard is empty. This is the only value that clears the
    /// peer's clipboard.
    Unavailable,
}

impl ClipboardValue {
    /// Whether this value may be published to peers.
    ///
    /// Unsupported or oversized payloads are *not* shareable, and callers must
    /// skip them instead of downgrading to [`ClipboardValue::Unavailable`]:
    /// clearing the peer's clipboard because we copied something we cannot ship
    /// (a file, a huge image) is never what the user asked for.
    #[must_use]
    pub fn is_shareable(&self) -> bool {
        match self {
            ClipboardValue::Text(text) => {
                !text.is_empty() && text.len() <= MAX_CLIPBOARD_TEXT_BYTES
            }
            ClipboardValue::Binary { mime, data } => {
                mime == SHAREABLE_IMAGE_MIME
                    && decoded_base64_len(data)
                        .is_some_and(|len| len > 0 && len <= MAX_CLIPBOARD_IMAGE_BYTES)
            }
            ClipboardValue::Unavailable => true,
        }
    }

    /// The text carried by this value, if any.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ClipboardValue::Text(text) => Some(text),
            ClipboardValue::Binary { .. } | ClipboardValue::Unavailable => None,
        }
    }
}

/// Length of a standard-base64 string after decoding, without allocating.
///
/// Returns `None` when the input is not valid base64 (bad length, padding in
/// the middle, or characters outside the standard alphabet).
#[must_use]
pub fn decoded_base64_len(data: &str) -> Option<usize> {
    let bytes = data.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return None;
    }
    let body = &bytes[..bytes.len() - padding];
    if body
        .iter()
        .any(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'+' || *byte == b'/'))
    {
        return None;
    }
    Some(bytes.len() / 4 * 3 - padding)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardUpdate {
    pub event_id: String,
    pub revision: Revision,
    pub value: ClipboardValue,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardRegister {
    pub clock: u64,
    pub head: Option<ClipboardUpdate>,
}

#[derive(Serialize, Deserialize)]
struct StoredRegister {
    local_peer_id: String,
    register: ClipboardRegister,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeResult {
    Applied,
    AlreadyCurrent,
    Stale,
    Invalid,
}

impl ClipboardRegister {
    pub fn local_change(
        &mut self,
        peer_id: &str,
        value: ClipboardValue,
    ) -> Option<ClipboardUpdate> {
        // Skip unsupported payloads instead of publishing `Unavailable`: an
        // unshareable copy must leave the peer's clipboard untouched.
        if !value.is_shareable() {
            log::debug!("skipping unshareable local clipboard change: {value:?}");
            return None;
        }
        if self.head.as_ref().is_some_and(|head| head.value == value) {
            return None;
        }
        let next_counter = self
            .clock
            .max(self.head.as_ref().map_or(0, |head| head.revision.counter))
            .checked_add(1)?;
        if next_counter == u64::MAX {
            return None;
        }
        self.clock = next_counter;
        let update = ClipboardUpdate {
            event_id: uuid::Uuid::new_v4().to_string(),
            revision: Revision {
                counter: self.clock,
                peer_id: peer_id.to_owned(),
            },
            value,
        };
        self.head = Some(update.clone());
        Some(update)
    }

    #[must_use]
    pub fn compare(&self, update: &ClipboardUpdate) -> MergeResult {
        if !is_valid(update) {
            return MergeResult::Invalid;
        }
        match self.head.as_ref() {
            Some(head) if head.revision == update.revision => {
                if head == update {
                    MergeResult::AlreadyCurrent
                } else {
                    MergeResult::Invalid
                }
            }
            Some(head) if head.revision > update.revision => MergeResult::Stale,
            _ => MergeResult::Applied,
        }
    }

    pub fn apply(&mut self, update: ClipboardUpdate) -> MergeResult {
        let result = self.compare(&update);
        if result == MergeResult::Applied {
            self.clock = self.clock.max(update.revision.counter);
            self.head = Some(update);
        }
        result
    }
}

/// Reads the clipboard register that belongs to `local_peer_id`.
///
/// # Errors
/// An [`io::Error`] when the file cannot be read, when it holds another
/// identity's register, or when the stored head fails validation.
pub fn load_register(path: &Path, local_peer_id: &str) -> io::Result<ClipboardRegister> {
    if !path.exists() {
        return Ok(ClipboardRegister::default());
    }
    let stored: StoredRegister = serde_json::from_slice(&fs::read(path)?)?;
    if stored.local_peer_id != local_peer_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "clipboard state belongs to another identity",
        ));
    }
    if stored
        .register
        .head
        .as_ref()
        .is_some_and(|head| !is_valid(head))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid stored clipboard update",
        ));
    }
    let mut register = stored.register;
    register.clock = register.clock.max(
        register
            .head
            .as_ref()
            .map_or(0, |head| head.revision.counter),
    );
    Ok(register)
}

/// Writes the register next to `path`, replacing an existing file only once the
/// new contents are on disk.
///
/// # Errors
/// An [`io::Error`] when the register cannot be serialised, the temporary file
/// cannot be written, or the rename fails.
pub fn save_register(
    path: &Path,
    local_peer_id: &str,
    register: &ClipboardRegister,
) -> io::Result<()> {
    let stored = StoredRegister {
        local_peer_id: local_peer_id.to_owned(),
        register: register.clone(),
    };
    let bytes = serde_json::to_vec(&stored)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let write_result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

#[must_use]
pub fn is_valid(update: &ClipboardUpdate) -> bool {
    !update.event_id.is_empty()
        && update.revision.counter > 0
        && update.revision.counter < u64::MAX
        && !update.revision.peer_id.is_empty()
        && update.value.is_shareable()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_copies_choose_same_winner_on_both_devices() {
        let mut a = ClipboardRegister::default();
        let mut b = ClipboardRegister::default();
        let a_update = a
            .local_change("a", ClipboardValue::Text("A".into()))
            .unwrap();
        let b_update = b
            .local_change("b", ClipboardValue::Text("B".into()))
            .unwrap();
        assert_eq!(a.apply(b_update.clone()), MergeResult::Applied);
        assert_eq!(b.apply(a_update), MergeResult::Stale);
        assert_eq!(a.head, b.head);
    }

    #[test]
    fn duplicate_event_is_idempotent_and_local_copy_advances_clock() {
        let mut a = ClipboardRegister::default();
        let first = a
            .local_change("a", ClipboardValue::Text("one".into()))
            .unwrap();
        let mut b = ClipboardRegister::default();
        assert_eq!(b.apply(first.clone()), MergeResult::Applied);
        assert_eq!(b.apply(first), MergeResult::AlreadyCurrent);
        let second = b
            .local_change("b", ClipboardValue::Text("two".into()))
            .unwrap();
        assert_eq!(second.revision.counter, 2);
        assert_eq!(a.apply(second), MergeResult::Applied);
    }

    #[test]
    fn non_text_copy_retires_previous_text() {
        let mut register = ClipboardRegister::default();
        register.local_change("a", ClipboardValue::Text("secret".into()));
        let unavailable = register
            .local_change("a", ClipboardValue::Unavailable)
            .unwrap();
        assert_eq!(unavailable.value, ClipboardValue::Unavailable);
        assert_eq!(
            register.head.as_ref().unwrap().value,
            ClipboardValue::Unavailable
        );
    }

    #[test]
    fn rejects_oversized_text_without_clearing_the_peer() {
        let update = ClipboardUpdate {
            event_id: "one".into(),
            revision: Revision {
                counter: 1,
                peer_id: "a".into(),
            },
            value: ClipboardValue::Text("x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1)),
        };
        assert_eq!(
            ClipboardRegister::default().compare(&update),
            MergeResult::Invalid
        );

        let mut register = ClipboardRegister::default();
        let first = register
            .local_change("a", ClipboardValue::Text("one".into()))
            .unwrap();
        assert!(
            register
                .local_change(
                    "a",
                    ClipboardValue::Text("x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1))
                )
                .is_none()
        );
        // The previous shareable value survives; nothing is cleared.
        assert_eq!(register.head.as_ref().unwrap(), &first);
        assert_eq!(register.clock, 1);
    }

    #[test]
    fn binary_payload_round_trips_and_is_validated() {
        let png = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            [0x89u8, b'P', b'N', b'G', 1, 2, 3],
        );
        let mut register = ClipboardRegister::default();
        let update = register
            .local_change(
                "a",
                ClipboardValue::Binary {
                    mime: SHAREABLE_IMAGE_MIME.into(),
                    data: png.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            update.value,
            ClipboardValue::Binary {
                mime: SHAREABLE_IMAGE_MIME.into(),
                data: png.clone()
            }
        );
        assert!(is_valid(&update));

        // An unsupported MIME or malformed base64 is rejected on receipt and is
        // skipped (not cleared) locally.
        let wrong_mime = ClipboardValue::Binary {
            mime: "image/tiff".into(),
            data: png.clone(),
        };
        assert!(!wrong_mime.is_shareable());
        let bad_data = ClipboardValue::Binary {
            mime: SHAREABLE_IMAGE_MIME.into(),
            data: "not base64!!".into(),
        };
        assert!(!bad_data.is_shareable());
        assert!(register.local_change("a", wrong_mime).is_none());
        assert!(register.local_change("a", bad_data).is_none());
        // The image we just copied is still the current value.
        assert_eq!(register.head.as_ref().unwrap().value, update.value);
    }

    #[test]
    fn oversized_image_is_skipped_seen_as_invalid_on_receipt() {
        let oversized = ClipboardValue::Binary {
            mime: SHAREABLE_IMAGE_MIME.into(),
            data: "A".repeat((MAX_CLIPBOARD_IMAGE_BYTES / 3 + 8) * 4),
        };
        assert!(!oversized.is_shareable());
        let update = ClipboardUpdate {
            event_id: "big".into(),
            revision: Revision {
                counter: 1,
                peer_id: "a".into(),
            },
            value: oversized.clone(),
        };
        assert_eq!(
            ClipboardRegister::default().compare(&update),
            MergeResult::Invalid
        );
        assert!(
            ClipboardRegister::default()
                .local_change("a", oversized)
                .is_none()
        );
    }

    #[test]
    fn invalid_and_conflicting_updates_do_not_advance_the_clock() {
        let mut register = ClipboardRegister::default();
        let original = register
            .local_change("a", ClipboardValue::Text("one".into()))
            .unwrap();
        let mut conflict = original.clone();
        conflict.event_id = "other-event".into();
        assert_eq!(register.apply(conflict), MergeResult::Invalid);
        let mut invalid = original;
        invalid.revision.counter = u64::MAX;
        assert_eq!(register.apply(invalid), MergeResult::Invalid);
        assert_eq!(register.clock, 1);
        assert_eq!(
            register
                .local_change("a", ClipboardValue::Text("two".into()))
                .unwrap()
                .revision
                .counter,
            2
        );
    }

    #[test]
    fn saved_revision_survives_restart() {
        let path =
            std::env::temp_dir().join(format!("clipboard-register-{}.json", uuid::Uuid::new_v4()));
        let mut register = ClipboardRegister::default();
        register.local_change("a", ClipboardValue::Text("one".into()));
        save_register(&path, "a", &register).unwrap();
        let mut restored = load_register(&path, "a").unwrap();
        assert_eq!(restored, register);
        assert!(load_register(&path, "b").is_err());
        let next = restored
            .local_change("a", ClipboardValue::Text("two".into()))
            .unwrap();
        assert_eq!(next.revision.counter, 2);
        fs::remove_file(path).unwrap();
    }
}
