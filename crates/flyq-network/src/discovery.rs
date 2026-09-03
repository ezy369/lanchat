//! UDP broadcast discovery service.
//!
//! Listens for IPMsg broadcast packets and sends our own presence announcements.

use flyq_protocol::{Command, Packet, PacketBuilder, PacketParser};
use std::net::SocketAddr;
use thiserror::Error;
use tokio::net::UdpSocket;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Packet parse error: {0}")]
    Parse(#[from] flyq_protocol::packet::ParseError),
}

/// Discovery service that broadcasts presence on the LAN.
pub struct DiscoveryService {
    socket: UdpSocket,
    packet_no: u32,
    sender_name: String,
    sender_host: String,
}

impl DiscoveryService {
    /// Create a new discovery service bound to the given address.
    pub async fn bind(addr: SocketAddr, name: &str, host: &str) -> Result<Self, DiscoveryError> {
        let socket = UdpSocket::bind(addr).await?;
        socket.set_broadcast(true)?;
        info!("Discovery service bound to {}", addr);

        Ok(Self {
            socket,
            packet_no: 0,
            sender_name: name.to_string(),
            sender_host: host.to_string(),
        })
    }

    /// Broadcast our presence to the LAN.
    pub async fn announce_presence(&mut self, broadcast_addr: SocketAddr) -> Result<(), DiscoveryError> {
        self.packet_no += 1;
        let packet = PacketBuilder::new()
            .sender(&self.sender_name, &self.sender_host)
            .packet_no(self.packet_no)
            .command(Command::BrEntry)
            .build();

        self.socket.send_to(packet.as_bytes(), broadcast_addr).await?;
        debug!("Broadcast presence to {}", broadcast_addr);
        Ok(())
    }

    /// Receive a packet from the network.
    pub async fn recv_packet(&self) -> Result<(Packet, SocketAddr), DiscoveryError> {
        let mut buf = vec![0u8; 4096];
        let (len, addr) = self.socket.recv_from(&mut buf).await?;
        let raw = String::from_utf8_lossy(&buf[..len]);
        let packet = PacketParser::parse(&raw)?;
        Ok((packet, addr))
    }

    /// Send a packet to a specific address.
    pub async fn send_to(&self, packet: &str, addr: SocketAddr) -> Result<(), DiscoveryError> {
        self.socket.send_to(packet.as_bytes(), addr).await?;
        Ok(())
    }
}
