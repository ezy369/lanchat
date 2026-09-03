//! Peer management.
//!
//! Tracks online/offline status of discovered peers.

use flyq_protocol::PeerInfo;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Manages the list of known peers on the LAN.
#[derive(Clone)]
pub struct PeerManager {
    peers: Arc<RwLock<HashMap<IpAddr, PeerInfo>>>,
}

impl PeerManager {
    /// Create a new empty peer manager.
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add or update a peer.
    pub async fn add_peer(&self, peer: PeerInfo) {
        let mut peers = self.peers.write().await;
        peers.insert(peer.addr, peer);
    }

    /// Remove a peer by IP address.
    pub async fn remove_peer(&self, addr: &IpAddr) -> Option<PeerInfo> {
        let mut peers = self.peers.write().await;
        peers.remove(addr)
    }

    /// Get all online peers.
    pub async fn online_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().filter(|p| p.online).cloned().collect()
    }

    /// Get a peer by IP address.
    pub async fn get_peer(&self, addr: &IpAddr) -> Option<PeerInfo> {
        let peers = self.peers.read().await;
        peers.get(addr).cloned()
    }

    /// Get the total number of known peers.
    pub async fn peer_count(&self) -> usize {
        let peers = self.peers.read().await;
        peers.len()
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new()
    }
}
