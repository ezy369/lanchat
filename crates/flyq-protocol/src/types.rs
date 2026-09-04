//! Protocol type definitions.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};

/// A peer on the LAN.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    /// Display name of the peer.
    pub name: String,
    /// Hostname of the peer.
    pub host: String,
    /// IP address of the peer.
    pub addr: IpAddr,
    /// UDP port the peer listens on (default: 2425).
    pub port: u16,
    /// Whether the peer is currently online.
    pub online: bool,
    /// Group name (if any).
    pub group: Option<String>,
    /// MAC address (FeiQ extended version string).
    pub mac: Option<String>,
    /// Whether this peer is a FeiQ client (vs standard IPMsg).
    pub is_feiq: bool,
    /// Current user status.
    pub status: UserStatus,
}

impl PeerInfo {
    /// Create a PeerInfo from a socket address and basic info.
    pub fn new(name: &str, host: &str, socket_addr: SocketAddr) -> Self {
        Self {
            name: name.to_string(),
            host: host.to_string(),
            addr: socket_addr.ip(),
            port: socket_addr.port(),
            online: true,
            group: None,
            mac: None,
            is_feiq: false,
            status: UserStatus::Online,
        }
    }

    /// Get the socket address for this peer.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
    }

    /// Unique identifier for this peer (IP:port).
    pub fn peer_id(&self) -> String {
        format!("{}:{}", self.addr, self.port)
    }
}

/// Online status of a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UserStatus {
    /// User is online and available.
    #[default]
    Online,
    /// User is away.
    Away,
    /// User is busy / do not disturb.
    Busy,
    /// User is offline.
    Offline,
}

impl UserStatus {
    /// Encode as a `u8` for atomic storage (shared presence state).
    pub fn to_u8(self) -> u8 {
        match self {
            UserStatus::Online => 0,
            UserStatus::Away => 1,
            UserStatus::Busy => 2,
            UserStatus::Offline => 3,
        }
    }

    /// Decode from the `u8` produced by [`UserStatus::to_u8`]. Unknown values
    /// fall back to [`UserStatus::Online`].
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => UserStatus::Away,
            2 => UserStatus::Busy,
            3 => UserStatus::Offline,
            _ => UserStatus::Online,
        }
    }

    /// Whether this status is announced on the wire as IPMsg absence
    /// (`BrAbsence`). IPMsg has no distinct "busy" signal, so both Away and Busy
    /// map to absence; peers will show us as away.
    pub fn is_absence(self) -> bool {
        matches!(self, UserStatus::Away | UserStatus::Busy)
    }

    /// Human-readable label used by the UI.
    pub fn label(self) -> &'static str {
        match self {
            UserStatus::Online => "在线",
            UserStatus::Away => "离开",
            UserStatus::Busy => "忙碌",
            UserStatus::Offline => "离线",
        }
    }
}

/// A chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique message ID.
    pub id: String,
    /// Sender's display name.
    pub sender: String,
    /// Recipient's display name (or empty for broadcast).
    pub recipient: String,
    /// Message content.
    pub content: String,
    /// Timestamp.
    pub timestamp: chrono::DateTime<chrono::Local>,
    /// Whether this message has been read.
    pub read: bool,
}

/// A file attachment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAttachment {
    /// File ID (for protocol reference).
    pub file_id: u32,
    /// Original file name.
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// File type / extension.
    pub file_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_u8_roundtrip() {
        for status in [
            UserStatus::Online,
            UserStatus::Away,
            UserStatus::Busy,
            UserStatus::Offline,
        ] {
            assert_eq!(UserStatus::from_u8(status.to_u8()), status);
        }
    }

    #[test]
    fn status_from_unknown_u8_defaults_online() {
        assert_eq!(UserStatus::from_u8(99), UserStatus::Online);
    }

    #[test]
    fn absence_covers_away_and_busy_only() {
        assert!(UserStatus::Away.is_absence());
        assert!(UserStatus::Busy.is_absence());
        assert!(!UserStatus::Online.is_absence());
        assert!(!UserStatus::Offline.is_absence());
    }

    #[test]
    fn status_labels_are_distinct() {
        let labels = [
            UserStatus::Online.label(),
            UserStatus::Away.label(),
            UserStatus::Busy.label(),
            UserStatus::Offline.label(),
        ];
        for l in labels {
            assert!(!l.is_empty());
            assert_eq!(labels.iter().filter(|&&x| x == l).count(), 1);
        }
    }
}
