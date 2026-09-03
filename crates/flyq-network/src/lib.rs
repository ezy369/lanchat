//! LAN discovery and transport layer.
//!
//! This crate handles:
//! - UDP broadcast for peer discovery (IPMsg port 2425)
//! - TCP connections for reliable message delivery and file transfer
//! - Peer lifecycle management (online/offline/timeout detection)

pub mod discovery;
pub mod peer;
pub mod transport;

pub use discovery::{
    DiscoveryConfig, DiscoveryError, DiscoveryEvent, DiscoveryService, ShutdownHandle, IPMSG_PORT,
};
pub use peer::PeerManager;
pub use transport::Transport;
