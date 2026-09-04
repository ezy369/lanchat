//! LAN discovery and transport layer.
//!
//! This crate handles:
//! - UDP broadcast for peer discovery (IPMsg port 2425)
//! - UDP unicast for message delivery (chat, receipts, typing indicators)
//! - TCP connections for file transfer (GetFileData protocol)
//! - Peer lifecycle management (online/offline/timeout detection)

pub mod discovery;
pub mod message;
pub mod peer;
pub mod transport;

pub use discovery::{
    DiscoveryConfig, DiscoveryError, DiscoveryEvent, DiscoveryService, ShutdownHandle, IPMSG_PORT,
};
pub use message::{MessageError, MessageSender, SenderIdentity};
pub use peer::PeerManager;
pub use transport::{
    safe_relative_join, DirDownloadSummary, FileDownloader, FileOffer, FileRegistry,
    ProgressCallback, Transport, TransportError,
};
