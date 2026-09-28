use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// Keep the complete CBOR request comfortably below libp2p's default 1 MiB limit.
pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 512 * 1024;

/// A Lamport revision. The peer ID breaks ties for concurrent local copies.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Revision {
    pub counter: u64,
    pub peer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardValue {
    Text(String),
    /// The system clipboard is empty or contains data this protocol will not share.
    Unavailable,
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
        let value = match value {
            ClipboardValue::Text(text)
                if text.is_empty() || text.len() > MAX_CLIPBOARD_TEXT_BYTES =>
            {
                ClipboardValue::Unavailable
            }
            other => other,
        };
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

pub fn is_valid(update: &ClipboardUpdate) -> bool {
    !update.event_id.is_empty()
        && update.revision.counter > 0
        && update.revision.counter < u64::MAX
        && !update.revision.peer_id.is_empty()
        && match &update.value {
            ClipboardValue::Text(text) => {
                !text.is_empty() && text.len() <= MAX_CLIPBOARD_TEXT_BYTES
            }
            ClipboardValue::Unavailable => true,
        }
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
    fn rejects_oversized_text() {
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
        let local = ClipboardRegister::default()
            .local_change(
                "a",
                ClipboardValue::Text("x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1)),
            )
            .unwrap();
        assert_eq!(local.value, ClipboardValue::Unavailable);
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
