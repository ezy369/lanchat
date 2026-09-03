//! LAN discovery and transport layer.
//!
//! This crate handles:
//! - UDP broadcast for peer discovery (IPMsg port 2425)
//! - TCP connections for reliable message delivery and file transfer
//! - Peer lifecycle management (online/offline detection)

pub mod discovery;
pub mod peer;
pub mod transport;

pub use discovery::DiscoveryService;
pub use peer::PeerManager;
pub use transport::Transport;
