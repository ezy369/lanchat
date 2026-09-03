//! Peer management with timeout detection.
//!
//! Tracks online/offline status of discovered peers, including
//! last-seen timestamps for automatic timeout cleanup.

use flyq_protocol::{PeerInfo, UserStatus};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::Instant;
use tracing::debug;

/// Internal record wrapping PeerInfo with a last-seen timestamp.
#[derive(Debug, Clone)]
struct PeerRecord {
    info: PeerInfo,
    last_seen: Instant,
}

/// Manages the list of known peers on the LAN.
#[derive(Clone)]
pub struct PeerManager {
    peers: Arc<RwLock<HashMap<IpAddr, PeerRecord>>>,
}

impl PeerManager {
    /// Create a new empty peer manager.
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a new peer or update an existing one.
    ///
    /// Resets the last_seen timestamp on every call.
    pub async fn add_or_update_peer(&self, peer: PeerInfo) {
        let mut peers = self.peers.write().await;
        let record = PeerRecord {
            info: peer,
            last_seen: Instant::now(),
        };
        peers.insert(record.info.addr, record);
    }

    /// Add or update a peer (alias for backward compatibility).
    pub async fn add_peer(&self, peer: PeerInfo) {
        self.add_or_update_peer(peer).await;
    }

    /// Get a peer by IP address.
    pub async fn get_peer_by_addr(&self, addr: &IpAddr) -> Option<PeerInfo> {
        let peers = self.peers.read().await;
        peers.get(addr).map(|r| r.info.clone())
    }

    /// Get a peer by IP address (alias).
    pub async fn get_peer(&self, addr: &IpAddr) -> Option<PeerInfo> {
        self.get_peer_by_addr(addr).await
    }

    /// Remove a peer by IP address, returning the removed PeerInfo.
    pub async fn remove_peer_by_addr(&self, addr: &IpAddr) -> Option<PeerInfo> {
        let mut peers = self.peers.write().await;
        peers.remove(addr).map(|r| r.info)
    }

    /// Remove a peer by IP address (alias).
    pub async fn remove_peer(&self, addr: &IpAddr) -> Option<PeerInfo> {
        self.remove_peer_by_addr(addr).await
    }

    /// Update the last_seen timestamp for a peer (keeps them alive).
    pub async fn touch_peer(&self, addr: &IpAddr) {
        let mut peers = self.peers.write().await;
        if let Some(record) = peers.get_mut(addr) {
            record.last_seen = Instant::now();
        }
    }

    /// Remove peers that haven't been seen within the timeout duration.
    ///
    /// Returns the list of removed peers (for event emission).
    pub async fn remove_timed_out_peers(&self, timeout: Duration) -> Vec<PeerInfo> {
        let now = Instant::now();
        let mut peers = self.peers.write().await;
        let mut removed = Vec::new();

        peers.retain(|_addr, record| {
            if now.duration_since(record.last_seen) > timeout {
                debug!(
                    "Peer timed out: {} ({}) — last seen {:?} ago",
                    record.info.name,
                    record.info.addr,
                    now.duration_since(record.last_seen)
                );
                removed.push(record.info.clone());
                false
            } else {
                true
            }
        });

        removed
    }

    /// Get all online peers.
    pub async fn online_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers
            .values()
            .filter(|r| r.info.online && r.info.status != UserStatus::Offline)
            .map(|r| r.info.clone())
            .collect()
    }

    /// Get all known peers (including offline/timed-out ones still in cache).
    pub async fn all_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().map(|r| r.info.clone()).collect()
    }

    /// Get the total number of known peers.
    pub async fn peer_count(&self) -> usize {
        let peers = self.peers.read().await;
        peers.len()
    }

    /// Update a peer's status (e.g., Online → Away).
    pub async fn set_peer_status(&self, addr: &IpAddr, status: UserStatus) {
        let mut peers = self.peers.write().await;
        if let Some(record) = peers.get_mut(addr) {
            record.info.status = status;
            record.info.online = status != UserStatus::Offline;
        }
    }

    /// Mark a peer as offline without removing it (for graceful exit).
    pub async fn mark_peer_offline(&self, addr: &IpAddr) -> Option<PeerInfo> {
        let mut peers = self.peers.write().await;
        if let Some(record) = peers.get_mut(addr) {
            record.info.online = false;
            record.info.status = UserStatus::Offline;
            Some(record.info.clone())
        } else {
            None
        }
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new()
    }
}
