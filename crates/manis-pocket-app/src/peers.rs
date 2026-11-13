//! Paired devices, as the settings window needs them.
//!
//! The core hands the list over as JSON strings so the platform layer does not
//! have to agree on a struct; this module turns each one into something a person
//! can read — a name, a short identifier and whether it is connected.

/// One paired device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PairedPeer {
    pub(crate) peer_id: String,
    pub(crate) display_name: String,
    pub(crate) is_online: bool,
}

impl PairedPeer {
    /// Parses one line of the core's peer list. Unknown or malformed entries are
    /// dropped rather than shown as raw JSON.
    pub(crate) fn from_json(line: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        let peer_id = value.get("peerId")?.as_str()?.to_string();
        if peer_id.is_empty() {
            return None;
        }
        Some(Self {
            peer_id,
            display_name: value
                .get("displayName")
                .and_then(|name| name.as_str())
                .unwrap_or_default()
                .to_string(),
            is_online: value
                .get("isOnline")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// What to put on the row: the device name, or the start of its id when the
    /// peer never told us one.
    pub(crate) fn label(&self) -> String {
        let name = self.display_name.trim();
        if name.is_empty() {
            short_peer_id(&self.peer_id)
        } else {
            name.to_string()
        }
    }

    /// The line under the name: state and a way to tell two peers apart.
    pub(crate) fn detail(&self) -> String {
        let state = if self.is_online { "在线" } else { "离线" };
        format!("{state} · {}", short_peer_id(&self.peer_id))
    }
}

/// The first and last few characters of a peer id: long enough to tell devices
/// apart, short enough to fit in a settings row.
pub(crate) fn short_peer_id(peer_id: &str) -> String {
    let id = peer_id.trim();
    if id.len() <= 12 {
        return id.to_string();
    }
    format!("{}…{}", &id[..6], &id[id.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(peer_id: &str, name: &str, online: bool) -> String {
        format!(
            r#"{{"peerId":"{peer_id}","displayName":"{name}","isAdmin":false,"isOnline":{online}}}"#
        )
    }

    #[test]
    fn a_peer_row_reads_as_a_device() {
        let peer = PairedPeer::from_json(&json("12D3KooWAbCdEfGhIjKlMnOp", "Mac mini", true))
            .expect("parse");
        assert_eq!(peer.label(), "Mac mini");
        assert_eq!(peer.detail(), "在线 · 12D3Ko…MnOp");
    }

    #[test]
    fn a_nameless_peer_falls_back_to_its_id() {
        let peer =
            PairedPeer::from_json(&json("12D3KooWAbCdEfGhIjKlMnOp", "", false)).expect("parse");
        assert_eq!(peer.label(), "12D3Ko…MnOp");
        assert!(peer.detail().starts_with("离线"));
    }

    #[test]
    fn malformed_peers_are_dropped_not_shown() {
        assert!(PairedPeer::from_json("not json").is_none());
        assert!(PairedPeer::from_json(r#"{"displayName":"x"}"#).is_none());
        assert!(PairedPeer::from_json(r#"{"peerId":""}"#).is_none());
    }

    #[test]
    fn short_ids_keep_short_ones_whole() {
        assert_eq!(short_peer_id("abc"), "abc");
        assert_eq!(short_peer_id("12D3KooWAbCdEfGhIjKlMnOp"), "12D3Ko…MnOp");
    }
}
