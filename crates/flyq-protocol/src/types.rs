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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserStatus {
    /// User is online and available.
    Online,
    /// User is away.
    Away,
    /// User is busy / do not disturb.
    Busy,
    /// User is offline.
    Offline,
}

impl Default for UserStatus {
    fn default() -> Self {
        Self::Online
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
