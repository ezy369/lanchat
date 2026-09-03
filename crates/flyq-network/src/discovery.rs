//! UDP broadcast discovery service.
//!
//! Implements the IPMsg/FeiQ peer discovery protocol:
//! - Periodic BrEntry broadcast on all LAN interfaces
//! - Listens for BrEntry/BrExit/AnsEntry/BrAbsence from peers
//! - Emits discovery events via mpsc channel
//! - Detects peer timeouts (no heartbeat within threshold)
//!
//! Protocol flow:
//! 1. On startup: broadcast BrEntry to all interfaces
//! 2. On receiving BrEntry: add peer, respond with AnsEntry
//! 3. On receiving AnsEntry: update peer info
//! 4. On receiving BrExit: mark peer offline
//! 5. Periodic: re-broadcast BrEntry (default every 60s)
//! 6. Periodic: check for timed-out peers (no packet in 180s)

use flyq_protocol::command::flags;
use flyq_protocol::{Command, Packet, PacketBuilder, PacketParser, PeerInfo, UserStatus};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{debug, info, warn};

/// Default IPMsg UDP port.
pub const IPMSG_PORT: u16 = 2425;

/// How often to broadcast our presence (seconds).
const BROADCAST_INTERVAL_SECS: u64 = 60;

/// How long before a peer is considered timed out (seconds).
const PEER_TIMEOUT_SECS: u64 = 180;

/// Buffer size for incoming UDP packets.
const RECV_BUF_SIZE: usize = 8192;

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Packet parse error: {0}")]
    Parse(#[from] flyq_protocol::packet::ParseError),
    #[error("Service shut down")]
    Shutdown,
}

/// Events emitted by the discovery service.
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// A new peer was discovered (BrEntry received or AnsEntry for unknown peer).
    PeerJoined(PeerInfo),
    /// A peer went offline (BrExit received or timeout).
    PeerLeft(PeerInfo),
    /// A peer's status changed (e.g., entered absence mode).
    PeerStatusChanged {
        peer: PeerInfo,
        old_status: UserStatus,
        new_status: UserStatus,
    },
    /// A non-discovery packet was received (messages, typing indicators, etc.).
    /// The discovery service forwards these for the message handler to process.
    PacketReceived {
        packet: Packet,
        from: SocketAddr,
    },
}

/// Configuration for the discovery service.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Our display name.
    pub username: String,
    /// Our hostname.
    pub hostname: String,
    /// MAC address for FeiQ version string (12 hex chars, e.g. "74E6E2152F9D").
    pub mac_address: String,
    /// FeiQ level (typically 128 for FeiQ 3.x).
    pub feiq_level: u32,
    /// UDP port to bind (default: 2425).
    pub port: u16,
    /// Broadcast interval in seconds.
    pub broadcast_interval: Duration,
    /// Peer timeout in seconds.
    pub peer_timeout: Duration,
    /// Whether to use FeiQ extended version string (vs plain IPMsg "1").
    pub use_feiq_version: bool,
    /// Group name to broadcast (FeiQ group feature).
    pub group_name: Option<String>,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            username: "LanChat User".to_string(),
            hostname: hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            mac_address: generate_local_mac(),
            feiq_level: 128,
            port: IPMSG_PORT,
            broadcast_interval: Duration::from_secs(BROADCAST_INTERVAL_SECS),
            peer_timeout: Duration::from_secs(PEER_TIMEOUT_SECS),
            use_feiq_version: true,
            group_name: None,
        }
    }
}

/// UDP discovery service that broadcasts presence and listens for peers.
pub struct DiscoveryService {
    socket: Arc<UdpSocket>,
    config: DiscoveryConfig,
    packet_no: Arc<AtomicU32>,
    broadcast_addrs: Vec<SocketAddr>,
    event_tx: mpsc::Sender<DiscoveryEvent>,
    event_rx: Option<mpsc::Receiver<DiscoveryEvent>>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
}

impl DiscoveryService {
    /// Create a new discovery service with the given configuration.
    ///
    /// Binds to `0.0.0.0:{config.port}` and discovers broadcast addresses
    /// from all active network interfaces.
    pub async fn new(config: DiscoveryConfig) -> Result<Self, DiscoveryError> {
        let bind_addr = SocketAddr::from(([0, 0, 0, 0], config.port));
        let socket = UdpSocket::bind(bind_addr).await?;
        socket.set_broadcast(true)?;

        let broadcast_addrs = discover_broadcast_addrs(config.port);
        info!(
            "Discovery service bound to {}, broadcast targets: {:?}",
            bind_addr, broadcast_addrs
        );

        let (event_tx, event_rx) = mpsc::channel(64);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Initialize packet_no with a timestamp-like value (as FeiQ does).
        let initial_no = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;

        Ok(Self {
            socket: Arc::new(socket),
            config,
            packet_no: Arc::new(AtomicU32::new(initial_no)),
            broadcast_addrs,
            event_tx,
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            shutdown_rx,
        })
    }

    /// Take the event receiver. Can only be called once.
    ///
    /// The receiver yields `DiscoveryEvent` items as peers join/leave
    /// and packets are received.
    pub fn take_event_rx(&mut self) -> Option<mpsc::Receiver<DiscoveryEvent>> {
        self.event_rx.take()
    }

    /// Get a handle to trigger graceful shutdown.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            tx: self.shutdown_tx.clone().expect("shutdown_tx already taken"),
        }
    }

    /// Build a packet with our identity and the given command.
    fn build_packet(&self, cmd: Command, extra: Option<&str>) -> String {
        let no = self.packet_no.fetch_add(1, Ordering::Relaxed);
        let mut builder = if self.config.use_feiq_version {
            PacketBuilder::new_feiq(&self.config.mac_address, self.config.feiq_level)
        } else {
            PacketBuilder::new()
        };

        builder = builder
            .sender(&self.config.username, &self.config.hostname)
            .packet_no(no)
            .command(cmd);

        // FeiQ online broadcasts include ENCOPT|FILEATTACHOPT flags.
        if self.config.use_feiq_version && matches!(cmd, Command::BrEntry | Command::AnsEntry) {
            builder = builder.flag(flags::FEIQ_ONLINE_FLAGS);
        }

        // Add group name or extra data.
        let extra_data = extra.or(self.config.group_name.as_deref());
        if let Some(data) = extra_data {
            builder = builder.extra(data);
        }

        builder.build()
    }

    /// Broadcast our presence (BrEntry) to all discovered broadcast addresses.
    pub async fn announce_presence(&self) -> Result<(), DiscoveryError> {
        let packet = self.build_packet(Command::BrEntry, None);
        let bytes = packet.as_bytes();

        for &addr in &self.broadcast_addrs {
            match self.socket.send_to(bytes, addr).await {
                Ok(n) => debug!("Broadcast {} bytes to {}", n, addr),
                Err(e) => warn!("Failed to broadcast to {}: {}", addr, e),
            }
        }

        // Also send to 255.255.255.255 as fallback.
        let fallback = SocketAddr::from(([255, 255, 255, 255], self.config.port));
        let _ = self.socket.send_to(bytes, fallback).await;

        info!("Presence announced on {} broadcast addresses", self.broadcast_addrs.len());
        Ok(())
    }

    /// Broadcast our departure (BrExit).
    pub async fn announce_departure(&self) -> Result<(), DiscoveryError> {
        let packet = self.build_packet(Command::BrExit, None);
        let bytes = packet.as_bytes();

        for &addr in &self.broadcast_addrs {
            let _ = self.socket.send_to(bytes, addr).await;
        }
        let fallback = SocketAddr::from(([255, 255, 255, 255], self.config.port));
        let _ = self.socket.send_to(bytes, fallback).await;

        info!("Departure announced");
        Ok(())
    }

    /// Send a unicast packet to a specific peer.
    pub async fn send_to_peer(&self, cmd: Command, extra: Option<&str>, addr: SocketAddr) -> Result<(), DiscoveryError> {
        let packet = self.build_packet(cmd, extra);
        self.socket.send_to(packet.as_bytes(), addr).await?;
        Ok(())
    }

    /// Send a raw packet string to a specific address.
    pub async fn send_raw(&self, data: &str, addr: SocketAddr) -> Result<(), DiscoveryError> {
        self.socket.send_to(data.as_bytes(), addr).await?;
        Ok(())
    }

    /// Run the discovery service main loop.
    ///
    /// This spawns the service as a background task. It will:
    /// 1. Broadcast initial presence
    /// 2. Periodically re-broadcast
    /// 3. Listen for incoming packets and emit events
    /// 4. Check for peer timeouts
    ///
    /// Returns a JoinHandle for the spawned task.
    pub fn spawn(
        mut self,
        peer_manager: super::peer::PeerManager,
    ) -> (
        mpsc::Receiver<DiscoveryEvent>,
        tokio::task::JoinHandle<()>,
    ) {
        let event_rx = self.event_rx.take().expect("event_rx already taken");
        let socket = Arc::clone(&self.socket);
        let config = self.config.clone();
        let packet_no = Arc::clone(&self.packet_no);
        let broadcast_addrs = self.broadcast_addrs.clone();
        let event_tx = self.event_tx.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        let handle = tokio::spawn(async move {
            // Initial presence broadcast.
            let initial_packet = build_packet_inner(&config, &packet_no, Command::BrEntry, None);
            for &addr in &broadcast_addrs {
                let _ = socket.send_to(initial_packet.as_bytes(), addr).await;
            }
            let fallback = SocketAddr::from(([255, 255, 255, 255], config.port));
            let _ = socket.send_to(initial_packet.as_bytes(), fallback).await;
            info!("Discovery service started, initial broadcast sent");

            let mut broadcast_timer = interval(config.broadcast_interval);
            broadcast_timer.tick().await; // First tick is immediate, skip it.

            let mut timeout_timer = interval(Duration::from_secs(30));
            timeout_timer.tick().await;

            let mut buf = vec![0u8; RECV_BUF_SIZE];

            loop {
                tokio::select! {
                    // Check shutdown signal.
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("Discovery service shutting down");
                            // Send BrExit before leaving.
                            let exit_packet = build_packet_inner(&config, &packet_no, Command::BrExit, None);
                            for &addr in &broadcast_addrs {
                                let _ = socket.send_to(exit_packet.as_bytes(), addr).await;
                            }
                            let _ = socket.send_to(exit_packet.as_bytes(), fallback).await;
                            break;
                        }
                    }

                    // Periodic presence broadcast.
                    _ = broadcast_timer.tick() => {
                        let packet = build_packet_inner(&config, &packet_no, Command::BrEntry, None);
                        for &addr in &broadcast_addrs {
                            let _ = socket.send_to(packet.as_bytes(), addr).await;
                        }
                        let _ = socket.send_to(packet.as_bytes(), fallback).await;
                        debug!("Periodic presence broadcast sent");
                    }

                    // Peer timeout check.
                    _ = timeout_timer.tick() => {
                        let timed_out = peer_manager.remove_timed_out_peers(config.peer_timeout).await;
                        for peer in timed_out {
                            info!("Peer timed out: {} ({})", peer.name, peer.addr);
                            let _ = event_tx.send(DiscoveryEvent::PeerLeft(peer)).await;
                        }
                    }

                    // Receive incoming packet.
                    result = socket.recv_from(&mut buf) => {
                        match result {
                            Ok((len, from)) => {
                                match PacketParser::parse_bytes(&buf[..len]) {
                                    Ok(packet) => {
                                        // Filter out our own broadcasts (reflected by NIC/switch).
                                        if is_own_packet(&packet, &config) {
                                            debug!("Ignoring own packet reflected from {}", from);
                                            continue;
                                        }

                                        handle_packet(
                                            &packet,
                                            from,
                                            &socket,
                                            &config,
                                            &packet_no,
                                            &peer_manager,
                                            &event_tx,
                                        ).await;
                                    }
                                    Err(e) => {
                                        debug!("Failed to parse packet from {}: {}", from, e);
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("UDP recv error: {}", e);
                            }
                        }
                    }
                }
            }

            info!("Discovery service stopped");
        });

        (event_rx, handle)
    }
}

/// Handle for triggering graceful shutdown of the discovery service.
#[derive(Clone)]
pub struct ShutdownHandle {
    tx: tokio::sync::watch::Sender<bool>,
}

impl ShutdownHandle {
    /// Signal the discovery service to shut down.
    pub fn shutdown(&self) {
        let _ = self.tx.send(true);
    }
}

/// Process an incoming packet and emit appropriate events.
async fn handle_packet(
    packet: &Packet,
    from: SocketAddr,
    socket: &UdpSocket,
    config: &DiscoveryConfig,
    packet_no: &AtomicU32,
    peer_manager: &super::peer::PeerManager,
    event_tx: &mpsc::Sender<DiscoveryEvent>,
) {
    match packet.command {
        Command::BrEntry | Command::BrAbsence => {
            let mut peer = packet_to_peer(packet, from);
            if packet.command == Command::BrAbsence {
                peer.status = UserStatus::Away;
            }

            // Check if this is a new peer or an update.
            let existing = peer_manager.get_peer_by_addr(&from.ip()).await;
            let is_new = existing.is_none();

            peer_manager.add_or_update_peer(peer.clone()).await;

            if is_new {
                info!("New peer discovered: {} ({}) at {}", peer.name, peer.host, from);
                let _ = event_tx.send(DiscoveryEvent::PeerJoined(peer.clone())).await;
            } else if let Some(old) = existing {
                if old.status != peer.status {
                    let _ = event_tx
                        .send(DiscoveryEvent::PeerStatusChanged {
                            peer: peer.clone(),
                            old_status: old.status,
                            new_status: peer.status,
                        })
                        .await;
                }
            }

            // Respond with AnsEntry (unless it's an absence broadcast).
            if packet.command == Command::BrEntry {
                let response = build_packet_inner(config, packet_no, Command::AnsEntry, None);
                if let Err(e) = socket.send_to(response.as_bytes(), from).await {
                    debug!("Failed to send AnsEntry to {}: {}", from, e);
                }
            }
        }

        Command::AnsEntry => {
            // Response to our BrEntry — update peer info.
            let peer = packet_to_peer(packet, from);
            let existing = peer_manager.get_peer_by_addr(&from.ip()).await;
            let is_new = existing.is_none();

            peer_manager.add_or_update_peer(peer.clone()).await;

            if is_new {
                info!("Peer added via AnsEntry: {} ({}) at {}", peer.name, peer.host, from);
                let _ = event_tx.send(DiscoveryEvent::PeerJoined(peer)).await;
            }
        }

        Command::BrExit => {
            // Peer is going offline.
            if let Some(peer) = peer_manager.remove_peer_by_addr(&from.ip()).await {
                info!("Peer left: {} ({}) at {}", peer.name, peer.host, from);
                let _ = event_tx.send(DiscoveryEvent::PeerLeft(peer)).await;
            } else {
                debug!("BrExit from unknown peer: {}", from);
            }
        }

        _ => {
            // Non-discovery packet (messages, typing, file transfer, etc.)
            // Update peer's last_seen and forward the event.
            peer_manager.touch_peer(&from.ip()).await;
            let _ = event_tx
                .send(DiscoveryEvent::PacketReceived {
                    packet: packet.clone(),
                    from,
                })
                .await;
        }
    }
}

/// Convert a parsed Packet into a PeerInfo struct.
fn packet_to_peer(packet: &Packet, from: SocketAddr) -> PeerInfo {
    PeerInfo {
        name: packet.sender_name.clone(),
        host: packet.sender_host.clone(),
        addr: from.ip(),
        port: from.port(),
        online: true,
        group: packet.extra.clone().filter(|s| !s.is_empty()),
        mac: packet.sender_mac().map(|s| s.to_string()),
        is_feiq: packet.is_feiq(),
        status: UserStatus::Online,
    }
}

/// Build a packet string (standalone version for use in spawned tasks).
fn build_packet_inner(
    config: &DiscoveryConfig,
    packet_no: &AtomicU32,
    cmd: Command,
    extra: Option<&str>,
) -> String {
    let no = packet_no.fetch_add(1, Ordering::Relaxed);
    let mut builder = if config.use_feiq_version {
        PacketBuilder::new_feiq(&config.mac_address, config.feiq_level)
    } else {
        PacketBuilder::new()
    };

    builder = builder
        .sender(&config.username, &config.hostname)
        .packet_no(no)
        .command(cmd);

    if config.use_feiq_version && matches!(cmd, Command::BrEntry | Command::AnsEntry) {
        builder = builder.flag(flags::FEIQ_ONLINE_FLAGS);
    }

    let extra_data = extra.or(config.group_name.as_deref());
    if let Some(data) = extra_data {
        builder = builder.extra(data);
    }

    builder.build()
}

/// Discover broadcast addresses from all active network interfaces.
fn discover_broadcast_addrs(port: u16) -> Vec<SocketAddr> {
    let mut addrs = Vec::new();

    match if_addrs::get_if_addrs() {
        Ok(interfaces) => {
            for iface in interfaces {
                if iface.is_loopback() {
                    continue;
                }

                // Only handle IPv4 interfaces.
                if let if_addrs::IfAddr::V4(ref v4) = iface.addr {
                    // Use the broadcast address if available, otherwise compute it.
                    let bcast_ip = match v4.broadcast {
                        Some(b) => b,
                        None => {
                            // Compute from ip | !netmask.
                            let ip_octets = v4.ip.octets();
                            let mask_octets = v4.netmask.octets();
                            Ipv4Addr::from(std::array::from_fn(|i| {
                                ip_octets[i] | !mask_octets[i]
                            }))
                        }
                    };

                    let bcast_addr = SocketAddr::V4(SocketAddrV4::new(bcast_ip, port));
                    if !addrs.contains(&bcast_addr) {
                        addrs.push(bcast_addr);
                        debug!("Interface {}: broadcast {}", iface.name, bcast_addr);
                    }
                }
            }
        }
        Err(e) => {
            warn!("Failed to enumerate network interfaces: {}", e);
        }
    }

    // Always include the global broadcast as fallback.
    let global_bcast = SocketAddr::from(([255, 255, 255, 255], port));
    if !addrs.contains(&global_bcast) {
        addrs.push(global_bcast);
    }

    // If no interfaces found (e.g., in tests), add loopback broadcast.
    if addrs.len() <= 1 {
        let lo_bcast = SocketAddr::from(([127, 255, 255, 255], port));
        if !addrs.contains(&lo_bcast) {
            addrs.push(lo_bcast);
        }
    }

    addrs
}

/// Check if a received packet was sent by ourselves (reflected broadcast).
///
/// Detection strategy:
/// 1. If the packet has a FeiQ MAC address, compare with our configured MAC.
/// 2. Fallback: compare sender_name + sender_host with our config.
fn is_own_packet(packet: &Packet, config: &DiscoveryConfig) -> bool {
    // Primary check: MAC address match (unique per client).
    if let Some(mac) = packet.sender_mac() {
        if mac.eq_ignore_ascii_case(&config.mac_address) {
            return true;
        }
    }

    // Fallback: name + host match (for standard IPMsg without MAC).
    packet.sender_name == config.username && packet.sender_host == config.hostname
}

/// Generate a pseudo-MAC address for FeiQ version string.
///
/// Uses a random locally-administered MAC (bit 1 of first octet set).
fn generate_local_mac() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    // Hash hostname + timestamp for uniqueness.
    if let Ok(name) = hostname::get() {
        name.hash(&mut hasher);
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);

    let hash = hasher.finish();
    let bytes = hash.to_be_bytes();

    // Set locally-administered bit, clear multicast bit.
    let first = (bytes[0] | 0x02) & 0xFE;
    format!(
        "{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        first, bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}
