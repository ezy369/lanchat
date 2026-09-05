//! Message sending over UDP unicast.
//!
//! In the IPMsg/FeiQ protocol, all control messages (chat, receipts, typing
//! indicators, screen shake) are sent via UDP unicast to the peer's port 2425.
//! Only file transfers use TCP.
//!
//! `MessageSender` is a cloneable handle that shares the UDP socket with the
//! discovery service, allowing message sending from any task.

use flyq_protocol::command::flags;
use flyq_protocol::{Command, GroupPayload, PacketBuilder, UserStatus};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::net::UdpSocket;
use tracing::debug;

#[derive(Debug, Error)]
pub enum MessageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Message too large ({0} bytes, max 65507)")]
    TooLarge(usize),
}

/// Maximum UDP payload size (65535 - IP header - UDP header).
const MAX_UDP_PAYLOAD: usize = 65507;

/// Identity configuration for building outgoing packets.
#[derive(Debug, Clone)]
pub struct SenderIdentity {
    /// Our display name.
    pub username: String,
    /// Our hostname.
    pub hostname: String,
    /// MAC address for FeiQ version string.
    pub mac_address: String,
    /// FeiQ level.
    pub feiq_level: u32,
    /// Whether to use FeiQ extended version.
    pub use_feiq_version: bool,
}

/// Cloneable handle for sending messages via the shared UDP socket.
///
/// Created from `DiscoveryService::message_sender()` after the service is set up.
/// All methods are async and can be called from any Tokio task.
#[derive(Clone)]
pub struct MessageSender {
    socket: Arc<UdpSocket>,
    identity: SenderIdentity,
    packet_no: Arc<AtomicU32>,
    /// Broadcast addresses for LAN-wide presence announcements.
    broadcast_addrs: Vec<SocketAddr>,
    /// The port presence broadcasts target (usually 2425).
    port: u16,
    /// Shared presence status. The discovery loop reads this to decide whether
    /// its periodic broadcast is an entry or an absence announcement, and
    /// [`MessageSender::set_status`] writes it.
    status: Arc<AtomicU8>,
}

impl MessageSender {
    /// Create a new MessageSender.
    pub(crate) fn new(
        socket: Arc<UdpSocket>,
        identity: SenderIdentity,
        packet_no: Arc<AtomicU32>,
        broadcast_addrs: Vec<SocketAddr>,
        port: u16,
        status: Arc<AtomicU8>,
    ) -> Self {
        Self {
            socket,
            identity,
            packet_no,
            broadcast_addrs,
            port,
            status,
        }
    }

    /// Get the next packet number.
    fn next_packet_no(&self) -> u32 {
        self.packet_no.fetch_add(1, Ordering::Relaxed)
    }

    /// Build a packet with our identity.
    fn build(&self, cmd: Command, cmd_flags: u32, extra: Option<&str>) -> String {
        let mut builder = if self.identity.use_feiq_version {
            PacketBuilder::new_feiq(&self.identity.mac_address, self.identity.feiq_level)
        } else {
            PacketBuilder::new()
        };

        builder = builder
            .sender(&self.identity.username, &self.identity.hostname)
            .packet_no(self.next_packet_no())
            .command(cmd);

        if cmd_flags != 0 {
            builder = builder.flag(cmd_flags);
        }

        if let Some(data) = extra {
            builder = builder.extra(data);
        }

        builder.build()
    }

    /// Build a packet with a pre-allocated packet number.
    ///
    /// Used when the caller needs the packet number before the packet is built
    /// (e.g. to return it from a send method).
    fn build_with_no(&self, cmd: Command, cmd_flags: u32, extra: Option<&str>, no: u32) -> String {
        let mut builder = if self.identity.use_feiq_version {
            PacketBuilder::new_feiq(&self.identity.mac_address, self.identity.feiq_level)
        } else {
            PacketBuilder::new()
        };

        builder = builder
            .sender(&self.identity.username, &self.identity.hostname)
            .packet_no(no)
            .command(cmd);

        if cmd_flags != 0 {
            builder = builder.flag(cmd_flags);
        }

        if let Some(data) = extra {
            builder = builder.extra(data);
        }

        builder.build()
    }

    /// Send a raw packet string to a peer address.
    async fn send_packet(&self, packet: &str, to: SocketAddr) -> Result<(), MessageError> {
        let bytes = packet.as_bytes();
        if bytes.len() > MAX_UDP_PAYLOAD {
            return Err(MessageError::TooLarge(bytes.len()));
        }
        self.socket.send_to(bytes, to).await?;
        debug!("Sent {} bytes to {}: {}", bytes.len(), to, &packet[..packet.len().min(80)]);
        Ok(())
    }

    // ─── Chat Messages ──────────────────────────────────────────────────

    /// Send a chat message to a peer.
    ///
    /// If `request_receipt` is true, sets SENDCHECKOPT flag asking the peer
    /// to reply with a RecvMsg delivery confirmation.
    pub async fn send_message(
        &self,
        to: SocketAddr,
        content: &str,
        request_receipt: bool,
    ) -> Result<u32, MessageError> {
        let no = self.next_packet_no();
        let cmd_flags = if request_receipt {
            flags::IPMSG_SENDCHECKOPT | flags::IPMSG_UTF8OPT
        } else {
            flags::IPMSG_UTF8OPT
        };

        let builder = if self.identity.use_feiq_version {
            PacketBuilder::new_feiq(&self.identity.mac_address, self.identity.feiq_level)
        } else {
            PacketBuilder::new()
        };

        let packet = builder
            .sender(&self.identity.username, &self.identity.hostname)
            .packet_no(no)
            .command(Command::SendMsg)
            .flag(cmd_flags)
            .extra(content)
            .build();

        self.send_packet(&packet, to).await?;
        Ok(no)
    }

    /// Send a message with file attachments.
    ///
    /// The extra field format is: `text\0fileRecord1\x07fileRecord2\x07...\a`
    /// where each file record is `fileId:filename:sizeHex:mtimeHex:fileTypeHex`
    pub async fn send_message_with_files(
        &self,
        to: SocketAddr,
        content: &str,
        file_records: &str,
        request_receipt: bool,
    ) -> Result<u32, MessageError> {
        let no = self.next_packet_no();
        let mut cmd_flags = flags::IPMSG_FILEATTACHOPT | flags::IPMSG_UTF8OPT;
        if request_receipt {
            cmd_flags |= flags::IPMSG_SENDCHECKOPT;
        }

        // FeiQ file attachment format: text\0fileRecords\a
        let extra = format!("{}\0{}\x07", content, file_records);

        let builder = if self.identity.use_feiq_version {
            PacketBuilder::new_feiq(&self.identity.mac_address, self.identity.feiq_level)
        } else {
            PacketBuilder::new()
        };

        let packet = builder
            .sender(&self.identity.username, &self.identity.hostname)
            .packet_no(no)
            .command(Command::SendMsg)
            .flag(cmd_flags)
            .extra(&extra)
            .build();

        self.send_packet(&packet, to).await?;
        Ok(no)
    }

    /// Send a group chat message to a single member.
    ///
    /// Uses the FeiQ GroupMsg command with MULTICASTOPT + UTF8OPT flags.
    /// The extra field embeds the group name: `"{groupName}\0{messageText}"`.
    ///
    /// For group fan-out, call this once per member in the group.
    pub async fn send_group_message(
        &self,
        to: SocketAddr,
        group_name: &str,
        content: &str,
    ) -> Result<u32, MessageError> {
        let no = self.next_packet_no();
        let cmd_flags =
            flags::IPMSG_MULTICASTOPT | flags::IPMSG_UTF8OPT | flags::IPMSG_SENDCHECKOPT;
        let extra = GroupPayload::build_extra(group_name, content);

        let builder = if self.identity.use_feiq_version {
            PacketBuilder::new_feiq(&self.identity.mac_address, self.identity.feiq_level)
        } else {
            PacketBuilder::new()
        };

        let packet = builder
            .sender(&self.identity.username, &self.identity.hostname)
            .packet_no(no)
            .command(Command::GroupMsg)
            .flag(cmd_flags)
            .extra(&extra)
            .build();

        self.send_packet(&packet, to).await?;
        Ok(no)
    }

    // ─── Delivery & Read Receipts ───────────────────────────────────────

    /// Send a delivery receipt (RecvMsg) acknowledging receipt of a message.
    ///
    /// The extra field contains the original sender's packet_no.
    pub async fn send_delivery_receipt(
        &self,
        to: SocketAddr,
        original_packet_no: u32,
    ) -> Result<(), MessageError> {
        let extra = original_packet_no.to_string();
        let packet = self.build(Command::RecvMsg, 0, Some(&extra));
        self.send_packet(&packet, to).await
    }

    /// Send a read receipt (ReadMsg) indicating the user has read the message.
    ///
    /// The extra field contains the original sender's packet_no.
    pub async fn send_read_receipt(
        &self,
        to: SocketAddr,
        original_packet_no: u32,
    ) -> Result<(), MessageError> {
        let extra = original_packet_no.to_string();
        let packet = self.build(Command::ReadMsg, 0, Some(&extra));
        self.send_packet(&packet, to).await
    }

    /// Ask a peer to delete/recall a message we previously sent (DelMsg).
    ///
    /// The extra field contains the packet_no of the original message we sent.
    pub async fn send_del_msg(
        &self,
        to: SocketAddr,
        original_packet_no: u32,
    ) -> Result<(), MessageError> {
        let extra = original_packet_no.to_string();
        let packet = self.build(Command::DelMsg, 0, Some(&extra));
        self.send_packet(&packet, to).await
    }

    // ─── Typing Indicators ──────────────────────────────────────────────

    /// Send a "typing started" indicator to a peer.
    pub async fn send_typing_start(&self, to: SocketAddr) -> Result<(), MessageError> {
        let packet = self.build(Command::TypingStart, 0, None);
        self.send_packet(&packet, to).await
    }

    /// Send a "typing stopped" indicator to a peer.
    pub async fn send_typing_end(&self, to: SocketAddr) -> Result<(), MessageError> {
        let packet = self.build(Command::TypingEnd, 0, None);
        self.send_packet(&packet, to).await
    }

    // ─── FeiQ Extensions ────────────────────────────────────────────────

    /// Send a screen shake (knock/抖屏) to a peer.
    pub async fn send_knock(&self, to: SocketAddr) -> Result<(), MessageError> {
        let packet = self.build(Command::Knock, 0, None);
        self.send_packet(&packet, to).await
    }

    /// Send an image message (screenshot).
    ///
    /// The extra field is the 8-byte ASCII image ID used for TCP file retrieval.
    pub async fn send_image(
        &self,
        to: SocketAddr,
        image_id: &str,
    ) -> Result<u32, MessageError> {
        let no = self.next_packet_no();
        let packet = self.build_with_no(Command::SendImage, flags::IPMSG_FILEATTACHOPT, Some(image_id), no);
        self.send_packet(&packet, to).await?;
        Ok(no)
    }

    // ─── User List Protocol ──────────────────────────────────────────────

    /// Broadcast a request for peer lists (BrIsGetList).
    ///
    /// Asks all peers on the LAN: "does anyone have a peer list to share?"
    /// Peers that do will respond with OkGetList.
    pub async fn send_br_is_get_list(&self) -> Result<(), MessageError> {
        let packet = self.build(Command::BrIsGetList, 0, None);
        let bytes = packet.as_bytes();
        for &addr in &self.broadcast_addrs {
            let _ = self.socket.send_to(bytes, addr).await;
        }
        let fallback = SocketAddr::from(([255, 255, 255, 255], self.port));
        let _ = self.socket.send_to(bytes, fallback).await;
        debug!("Broadcast BrIsGetList to {} addresses", self.broadcast_addrs.len());
        Ok(())
    }

    /// Reply to a BrIsGetList: "yes, I have a list" (OkGetList).
    pub async fn send_ok_get_list(&self, to: SocketAddr) -> Result<(), MessageError> {
        let packet = self.build(Command::OkGetList, 0, None);
        self.send_packet(&packet, to).await
    }

    /// Request the actual peer list from a peer (GetList).
    pub async fn send_get_list(&self, to: SocketAddr) -> Result<(), MessageError> {
        let packet = self.build(Command::GetList, 0, None);
        self.send_packet(&packet, to).await
    }

    /// Send the peer list to a requesting peer (AnsList).
    ///
    /// The extra field contains newline-separated peer entries in the format:
    /// `ip:port:userName:hostName`
    pub async fn send_ans_list(
        &self,
        to: SocketAddr,
        peer_entries: &str,
    ) -> Result<(), MessageError> {
        let packet = self.build(Command::AnsList, 0, Some(peer_entries));
        self.send_packet(&packet, to).await
    }

    // ─── Presence ───────────────────────────────────────────────────────

    /// Send a BrEntry (presence announcement) to a specific peer.
    pub async fn send_entry(&self, to: SocketAddr) -> Result<(), MessageError> {
        let cmd_flags = if self.identity.use_feiq_version {
            flags::FEIQ_ONLINE_FLAGS
        } else {
            0
        };
        let packet = self.build(Command::BrEntry, cmd_flags, None);
        self.send_packet(&packet, to).await
    }

    /// Send a BrExit (departure) to a specific peer.
    pub async fn send_exit(&self, to: SocketAddr) -> Result<(), MessageError> {
        let packet = self.build(Command::BrExit, 0, None);
        self.send_packet(&packet, to).await
    }

    /// Send a BrAbsence (away mode) broadcast.
    pub async fn send_absence(&self, to: SocketAddr) -> Result<(), MessageError> {
        let packet = self.build(Command::BrAbsence, flags::IPMSG_ABSENCEOPT, None);
        self.send_packet(&packet, to).await
    }

    /// Our current presence status.
    pub fn status(&self) -> UserStatus {
        UserStatus::from_u8(self.status.load(Ordering::Relaxed))
    }

    /// Set our presence status and broadcast the change to the whole LAN.
    ///
    /// Online is announced with `BrEntry`; Away and Busy are announced with
    /// `BrAbsence` (IPMsg has no distinct busy signal, so peers show us as
    /// away). The new status is stored in the shared atomic so the discovery
    /// loop's periodic re-broadcast keeps advertising it.
    pub async fn set_status(&self, status: UserStatus) -> Result<(), MessageError> {
        self.status.store(status.to_u8(), Ordering::Relaxed);

        let (cmd, cmd_flags) = if status.is_absence() {
            (Command::BrAbsence, flags::IPMSG_ABSENCEOPT)
        } else {
            let online_flags = if self.identity.use_feiq_version {
                flags::FEIQ_ONLINE_FLAGS
            } else {
                0
            };
            (Command::BrEntry, online_flags)
        };

        let packet = self.build(cmd, cmd_flags, None);
        let bytes = packet.as_bytes();
        for &addr in &self.broadcast_addrs {
            let _ = self.socket.send_to(bytes, addr).await;
        }
        let fallback = SocketAddr::from(([255, 255, 255, 255], self.port));
        let _ = self.socket.send_to(bytes, fallback).await;

        debug!("Broadcast presence status {:?} to {} addresses", status, self.broadcast_addrs.len());
        Ok(())
    }

    // ─── Raw Packet ─────────────────────────────────────────────────────

    /// Send an arbitrary packet string to a peer (escape hatch).
    pub async fn send_raw(&self, to: SocketAddr, raw: &str) -> Result<(), MessageError> {
        self.send_packet(raw, to).await
    }

    /// Get the local socket address this sender is bound to.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}
