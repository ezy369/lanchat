//! Event handlers for network and UI events.
//!
//! Processes `DiscoveryEvent` items from the network layer and translates them
//! into application-level actions: storing messages, sending receipts, and
//! emitting UI events.

use flyq_network::{DiscoveryEvent, MessageSender, PeerManager};
use flyq_protocol::command::flags;
use flyq_protocol::{Command, Packet, PeerInfo};
use flyq_storage::{Database, StoredMessage};
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Events emitted to the UI layer for rendering updates.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// A new peer appeared in the sidebar.
    PeerOnline(PeerInfo),
    /// A peer went offline.
    PeerOffline(PeerInfo),
    /// A new chat message was received.
    MessageReceived {
        id: String,
        sender: String,
        sender_addr: SocketAddr,
        content: String,
        timestamp: i64,
    },
    /// A message was sent successfully (for confirming in UI).
    MessageSent {
        id: String,
        recipient: String,
        content: String,
        timestamp: i64,
    },
    /// Delivery receipt received for a sent message.
    DeliveryConfirmed {
        original_packet_no: u32,
        from: SocketAddr,
    },
    /// Read receipt received for a sent message.
    ReadConfirmed {
        original_packet_no: u32,
        from: SocketAddr,
    },
    /// Peer is typing a message.
    TypingStart {
        from: SocketAddr,
        name: String,
    },
    /// Peer stopped typing.
    TypingEnd {
        from: SocketAddr,
        name: String,
    },
    /// Screen shake request from a peer.
    Knock {
        from: SocketAddr,
        name: String,
    },
    /// A peer's status changed (away/back).
    PeerStatusChanged {
        peer: PeerInfo,
    },
}

/// Application event handler that bridges network events to storage and UI.
pub struct EventHandler {
    /// Our local address identifier (IP:port string for storage).
    local_id: String,
    /// Database handle for persisting messages.
    db: Database,
    /// Peer manager for looking up peer info.
    peer_manager: PeerManager,
    /// Message sender for replies (receipts, etc.).
    sender: MessageSender,
    /// Channel to emit UI events.
    ui_tx: mpsc::Sender<UiEvent>,
}

impl EventHandler {
    /// Create a new event handler.
    pub fn new(
        local_id: String,
        db: Database,
        peer_manager: PeerManager,
        sender: MessageSender,
        ui_tx: mpsc::Sender<UiEvent>,
    ) -> Self {
        Self {
            local_id,
            db,
            peer_manager,
            sender,
            ui_tx,
        }
    }

    /// Access the shared peer manager (e.g. to look up peer details).
    pub fn peer_manager(&self) -> &PeerManager {
        &self.peer_manager
    }

    /// Process a discovery event from the network layer.
    pub async fn handle_discovery_event(&self, event: DiscoveryEvent) {
        match event {
            DiscoveryEvent::PeerJoined(peer) => self.handle_peer_joined(peer).await,
            DiscoveryEvent::PeerLeft(peer) => self.handle_peer_left(peer).await,
            DiscoveryEvent::PeerStatusChanged { peer, .. } => {
                self.handle_peer_status(peer).await
            }
            DiscoveryEvent::PacketReceived { packet, from } => {
                self.handle_packet(packet, from).await
            }
        }
    }

    /// Handle a new peer coming online.
    async fn handle_peer_joined(&self, peer: PeerInfo) {
        info!("Peer online: {} ({}) at {}", peer.name, peer.host, peer.addr);

        // Persist peer to database.
        let stored = flyq_storage::StoredPeer {
            addr: peer.peer_id(),
            name: peer.name.clone(),
            host: peer.host.clone(),
            group: peer.group.clone(),
            last_seen: now_timestamp(),
        };
        if let Err(e) = self.db.upsert_peer(&stored).await {
            warn!("Failed to persist peer {}: {}", peer.name, e);
        }

        // Notify UI.
        let _ = self.ui_tx.send(UiEvent::PeerOnline(peer)).await;
    }

    /// Handle a peer going offline.
    async fn handle_peer_left(&self, peer: PeerInfo) {
        info!("Peer offline: {} ({}) at {}", peer.name, peer.host, peer.addr);
        let _ = self.ui_tx.send(UiEvent::PeerOffline(peer)).await;
    }

    /// Handle a peer status change.
    async fn handle_peer_status(&self, peer: PeerInfo) {
        let _ = self.ui_tx.send(UiEvent::PeerStatusChanged { peer }).await;
    }

    /// Route an incoming packet to the appropriate handler.
    async fn handle_packet(&self, packet: Packet, from: SocketAddr) {
        match packet.command {
            Command::SendMsg => self.handle_send_msg(packet, from).await,
            Command::RecvMsg => self.handle_recv_msg(packet, from).await,
            Command::ReadMsg => self.handle_read_msg(packet, from).await,
            Command::TypingStart => self.handle_typing(packet, from, true).await,
            Command::TypingEnd => self.handle_typing(packet, from, false).await,
            Command::Knock => self.handle_knock(packet, from).await,
            Command::SendImage => self.handle_image(packet, from).await,
            cmd => {
                debug!("Unhandled command {:?} from {}", cmd, from);
            }
        }
    }

    /// Handle an incoming chat message (SendMsg).
    async fn handle_send_msg(&self, packet: Packet, from: SocketAddr) {
        let content = packet.extra.clone().unwrap_or_default();
        let timestamp = now_timestamp();
        let msg_id = Uuid::new_v4().to_string();
        let peer_id = format!("{}:{}", from.ip(), from.port());

        // Check if this message has file attachments.
        let has_files = packet.has_flag(flags::IPMSG_FILEATTACHOPT);
        let display_content = if has_files {
            // Split text from file records at \0.
            content.split('\0').next().unwrap_or("").to_string()
        } else {
            content.clone()
        };

        info!(
            "Message from {} ({}): {:?}",
            packet.sender_name, from, &display_content[..display_content.len().min(50)]
        );

        // Persist to database.
        let stored = StoredMessage {
            id: msg_id.clone(),
            sender: peer_id.clone(),
            recipient: self.local_id.clone(),
            content: display_content.clone(),
            timestamp,
            read: false,
        };
        if let Err(e) = self.db.insert_message(&stored).await {
            warn!("Failed to store message: {}", e);
        }

        // Send delivery receipt if requested (SENDCHECKOPT flag).
        if packet.has_flag(flags::IPMSG_SENDCHECKOPT) {
            if let Err(e) = self.sender.send_delivery_receipt(from, packet.packet_no).await {
                debug!("Failed to send delivery receipt to {}: {}", from, e);
            }
        }

        // Notify UI.
        let _ = self
            .ui_tx
            .send(UiEvent::MessageReceived {
                id: msg_id,
                sender: packet.sender_name,
                sender_addr: from,
                content: display_content,
                timestamp,
            })
            .await;
    }

    /// Handle a delivery receipt (RecvMsg) — confirms our message was received.
    async fn handle_recv_msg(&self, packet: Packet, from: SocketAddr) {
        let original_no = packet
            .extra
            .as_deref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);

        debug!("Delivery receipt from {} for packet {}", from, original_no);
        let _ = self
            .ui_tx
            .send(UiEvent::DeliveryConfirmed {
                original_packet_no: original_no,
                from,
            })
            .await;
    }

    /// Handle a read receipt (ReadMsg) — confirms our message was read.
    async fn handle_read_msg(&self, packet: Packet, from: SocketAddr) {
        let original_no = packet
            .extra
            .as_deref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);

        debug!("Read receipt from {} for packet {}", from, original_no);
        let _ = self
            .ui_tx
            .send(UiEvent::ReadConfirmed {
                original_packet_no: original_no,
                from,
            })
            .await;
    }

    /// Handle typing indicators.
    async fn handle_typing(&self, packet: Packet, from: SocketAddr, started: bool) {
        let name = packet.sender_name.clone();
        let event = if started {
            UiEvent::TypingStart { from, name }
        } else {
            UiEvent::TypingEnd { from, name }
        };
        let _ = self.ui_tx.send(event).await;
    }

    /// Handle screen shake (knock).
    async fn handle_knock(&self, packet: Packet, from: SocketAddr) {
        info!("Screen shake from {} ({})", packet.sender_name, from);
        let _ = self
            .ui_tx
            .send(UiEvent::Knock {
                from,
                name: packet.sender_name,
            })
            .await;
    }

    /// Handle image message (SendImage).
    async fn handle_image(&self, packet: Packet, from: SocketAddr) {
        // Image messages use FILEATTACHOPT — the extra field contains the image ID.
        // The actual image data is retrieved via TCP GetFileData.
        debug!("Image message from {}: {:?}", from, packet.extra);
        // For now, treat as a regular message with an image placeholder.
        let content = format!("[图片] {}", packet.extra.as_deref().unwrap_or(""));
        let timestamp = now_timestamp();
        let msg_id = Uuid::new_v4().to_string();
        let peer_id = format!("{}:{}", from.ip(), from.port());

        let stored = StoredMessage {
            id: msg_id.clone(),
            sender: peer_id,
            recipient: self.local_id.clone(),
            content: content.clone(),
            timestamp,
            read: false,
        };
        if let Err(e) = self.db.insert_message(&stored).await {
            warn!("Failed to store image message: {}", e);
        }

        let _ = self
            .ui_tx
            .send(UiEvent::MessageReceived {
                id: msg_id,
                sender: packet.sender_name,
                sender_addr: from,
                content,
                timestamp,
            })
            .await;
    }

    /// Send a chat message to a peer and persist it locally.
    ///
    /// Returns the message ID and packet number.
    pub async fn send_chat_message(
        &self,
        to: SocketAddr,
        content: &str,
        request_receipt: bool,
    ) -> Result<(String, u32), Box<dyn std::error::Error + Send + Sync>> {
        let msg_id = Uuid::new_v4().to_string();
        let timestamp = now_timestamp();
        let peer_id = format!("{}:{}", to.ip(), to.port());

        // Send over UDP.
        let packet_no = self.sender.send_message(to, content, request_receipt).await?;

        // Persist locally.
        let stored = StoredMessage {
            id: msg_id.clone(),
            sender: self.local_id.clone(),
            recipient: peer_id.clone(),
            content: content.to_string(),
            timestamp,
            read: true, // Our own messages are always "read".
        };
        if let Err(e) = self.db.insert_message(&stored).await {
            warn!("Failed to store sent message: {}", e);
        }

        // Notify UI.
        let _ = self
            .ui_tx
            .send(UiEvent::MessageSent {
                id: msg_id.clone(),
                recipient: peer_id,
                content: content.to_string(),
                timestamp,
            })
            .await;

        Ok((msg_id, packet_no))
    }
}

/// Get current unix timestamp in seconds.
fn now_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Spawn the event processing loop.
///
/// Consumes discovery events from `event_rx` and dispatches them through
/// the `EventHandler`. Runs until the event channel is closed.
///
/// The handler is shared via [`Arc`] so the UI layer can also call into it
/// (e.g. [`EventHandler::send_chat_message`]) while the loop is running.
pub async fn run_event_loop(
    handler: std::sync::Arc<EventHandler>,
    mut event_rx: mpsc::Receiver<DiscoveryEvent>,
) {
    info!("Event handler loop started");
    while let Some(event) = event_rx.recv().await {
        handler.handle_discovery_event(event).await;
    }
    info!("Event handler loop stopped (channel closed)");
}
