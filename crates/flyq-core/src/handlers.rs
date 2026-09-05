//! Event handlers for network and UI events.
//!
//! Processes `DiscoveryEvent` items from the network layer and translates them
//! into application-level actions: storing messages, sending receipts, and
//! emitting UI events.

use flyq_network::{
    DiscoveryEvent, FileDownloader, FileOffer, FileRegistry, MessageSender, PeerManager,
    ProgressCallback,
};
use flyq_protocol::command::flags;
use flyq_protocol::{Command, GroupPayload, Packet, PeerInfo, UserStatus};
use flyq_storage::{Database, Group, Page, StoredMessage};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// A chat message loaded from local history (database).
#[derive(Debug, Clone)]
pub struct HistoryMsg {
    /// Stored message id.
    pub id: String,
    /// Message text.
    pub text: String,
    /// Unix timestamp (seconds).
    pub timestamp: i64,
    /// True when we sent this message.
    pub outgoing: bool,
    /// Media type: 0 = text, 1 = image, 2 = file.
    pub media_type: u8,
}

/// A single file offered by a peer in an incoming message.
#[derive(Debug, Clone)]
pub struct IncomingFile {
    /// The sender-assigned file ID (used to request the bytes over TCP).
    pub file_id: u32,
    /// Original filename.
    pub filename: String,
    /// Declared size in bytes.
    pub size: u64,
    /// True when this offer is a directory (recursive folder transfer).
    pub is_dir: bool,
}

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
        /// The sender-assigned packet number, used to acknowledge reads.
        packet_no: u32,
    },
    /// A message was sent successfully (for confirming in UI).
    MessageSent {
        id: String,
        recipient: String,
        content: String,
        timestamp: i64,
        /// Our packet number, used to match delivery/read receipts.
        packet_no: u32,
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
    /// Conversation history loaded from the local database.
    HistoryLoaded {
        peer: SocketAddr,
        messages: Vec<HistoryMsg>,
    },
    /// A peer sent a message with file attachments. The UI should prompt the
    /// user to accept or reject each file.
    FileOfferReceived {
        from: SocketAddr,
        name: String,
        /// The chat message id the files arrived with (may be empty text).
        msg_id: String,
        files: Vec<IncomingFile>,
    },
    /// An incoming file transfer is progressing.
    FileProgress {
        transfer_id: String,
        filename: String,
        received: u64,
        total: u64,
    },
    /// An incoming file transfer completed successfully.
    FileComplete {
        transfer_id: String,
        filename: String,
        path: PathBuf,
    },
    /// A file transfer failed.
    FileFailed {
        transfer_id: String,
        filename: String,
        error: String,
    },
    /// A peer asked us to delete/recall a message (DelMsg).
    MessageDeleted {
        sender_addr: SocketAddr,
        packet_no: u32,
    },
    /// A group chat message was received.
    GroupMessageReceived {
        group_id: String,
        group_name: String,
        sender: String,
        sender_addr: SocketAddr,
        content: String,
        timestamp: i64,
        packet_no: u32,
    },
    /// A new group was created locally.
    GroupCreated {
        group_id: String,
        group_name: String,
        members: Vec<String>,
    },
    /// A group was deleted.
    GroupDeleted {
        group_id: String,
    },
    /// An image message was received and downloaded to local disk.
    ImageReceived {
        id: String,
        sender: String,
        sender_addr: SocketAddr,
        image_path: String,
        timestamp: i64,
        packet_no: u32,
    },
    /// An image message we sent was confirmed delivered.
    ImageSent {
        id: String,
        recipient: SocketAddr,
        image_path: String,
        timestamp: i64,
        packet_no: u32,
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
    /// Registry of files we offer for download to peers.
    registry: FileRegistry,
    /// Directory where incoming (accepted) files are saved. Wrapped in a lock
    /// so the settings panel can change it at runtime.
    download_dir: RwLock<PathBuf>,
    /// Our display name, sent when requesting a file download.
    local_name: String,
    /// Our hostname, sent when requesting a file download.
    local_host: String,
}

impl EventHandler {
    /// Create a new event handler.
    pub fn new(
        local_id: String,
        db: Database,
        peer_manager: PeerManager,
        sender: MessageSender,
        ui_tx: mpsc::Sender<UiEvent>,
        registry: FileRegistry,
        download_dir: PathBuf,
        local_name: String,
        local_host: String,
    ) -> Self {
        Self {
            local_id,
            db,
            peer_manager,
            sender,
            ui_tx,
            registry,
            download_dir: RwLock::new(download_dir),
            local_name,
            local_host,
        }
    }

    /// Current directory where incoming files are saved.
    pub fn download_dir(&self) -> PathBuf {
        self.download_dir
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| crate::config::default_download_dir())
    }

    /// Update the download directory at runtime (used by the settings panel).
    ///
    /// Best-effort creates the directory; returns the `create_dir_all` result so
    /// the caller can surface an error, but stores the new path regardless.
    pub fn set_download_dir(&self, dir: PathBuf) -> std::io::Result<()> {
        let result = std::fs::create_dir_all(&dir);
        if let Ok(mut guard) = self.download_dir.write() {
            *guard = dir.clone();
        }
        info!("Download directory set to {:?}", dir);
        result
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
            Command::DelMsg => self.handle_del_msg(packet, from).await,
            Command::TypingStart => self.handle_typing(packet, from, true).await,
            Command::TypingEnd => self.handle_typing(packet, from, false).await,
            Command::Knock => self.handle_knock(packet, from).await,
            Command::SendImage => self.handle_image(packet, from).await,
            Command::AnsReadMsg => self.handle_ans_read_msg(packet, from).await,
            Command::OpenYou => self.handle_open_you(packet, from).await,
            Command::BrIsGetList => self.handle_br_is_get_list(packet, from).await,
            Command::OkGetList => self.handle_ok_get_list(packet, from).await,
            Command::GetList => self.handle_get_list(packet, from).await,
            Command::AnsList => self.handle_ans_list(packet, from).await,
            // Commands handled by the discovery layer — no application-level action needed.
            Command::NoOperation | Command::BrEntry | Command::BrExit
            | Command::AnsEntry | Command::BrAbsence => {
                debug!("Discovery-layer command {:?} from {} (no-op at handler level)", packet.command, from);
            }
            // Commands handled by the TCP transport layer.
            Command::GetFileData | Command::ReleaseFiles | Command::GetDirFiles => {
                debug!("Transport-layer command {:?} from {} (no-op at handler level)", packet.command, from);
            }
            Command::GroupMsg => {
                self.handle_group_msg(from, &packet).await;
            }
            Command::Unknown => {
                debug!("Unknown command from {} (flags=0x{:08x})", from, packet.command_flags);
            }
        }
    }

    /// Handle an incoming chat message (SendMsg).
    async fn handle_send_msg(&self, packet: Packet, from: SocketAddr) {
        let content = packet.extra.clone().unwrap_or_default();
        let timestamp = now_timestamp();
        let msg_id = Uuid::new_v4().to_string();
        let peer_id = format!("{}:{}", from.ip(), from.port());

        // Check if this message has file attachments and parse them.
        let has_files = packet.has_flag(flags::IPMSG_FILEATTACHOPT);
        let (text_part, files) = if has_files {
            // Format: "text\0record1\x07record2\x07..."
            let (text, records) = match content.split_once('\0') {
                Some((t, r)) => (t.to_string(), r),
                None => (content.clone(), ""),
            };
            let parsed: Vec<IncomingFile> = records
                .split('\x07')
                .filter(|r| !r.is_empty())
                .filter_map(|r| FileOffer::from_wire_record(r, Path::new(".")))
                .map(|offer| IncomingFile {
                    file_id: offer.file_id,
                    filename: offer.filename,
                    size: offer.size,
                    is_dir: offer.file_type == 2,
                })
                .collect();
            (text, parsed)
        } else {
            (content.clone(), Vec::new())
        };

        // Build the display/persisted content. If there's no accompanying text
        // but there are files, show a "[文件]" placeholder with the filenames.
        let display_content = if text_part.trim().is_empty() && !files.is_empty() {
            let names: Vec<String> = files
                .iter()
                .map(|f| {
                    if f.is_dir {
                        format!("{}/", f.filename)
                    } else {
                        f.filename.clone()
                    }
                })
                .collect();
            format!("[文件] {}", names.join(", "))
        } else {
            text_part.clone()
        };

        info!(
            "Message from {} ({}): {:?} ({} file(s))",
            packet.sender_name,
            from,
            &display_content[..display_content.len().min(50)],
            files.len()
        );

        // Persist to database.
        let stored = StoredMessage {
            id: msg_id.clone(),
            sender: peer_id.clone(),
            recipient: self.local_id.clone(),
            content: display_content.clone(),
            timestamp,
            read: false,
            packet_no: Some(packet.packet_no),
            group_id: None,
            media_type: 0,
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

        // Notify UI of the chat message.
        let sender_name = packet.sender_name.clone();
        let _ = self
            .ui_tx
            .send(UiEvent::MessageReceived {
                id: msg_id.clone(),
                sender: sender_name.clone(),
                sender_addr: from,
                content: display_content,
                timestamp,
                packet_no: packet.packet_no,
            })
            .await;

        // If files are attached, prompt the UI to accept/reject them.
        if !files.is_empty() {
            let _ = self
                .ui_tx
                .send(UiEvent::FileOfferReceived {
                    from,
                    name: sender_name,
                    msg_id,
                    files,
                })
                .await;
        }
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

    /// Handle a message deletion request (DelMsg).
    ///
    /// The sender asks us to delete the message identified by their original
    /// `packet_no`. We match on (sender_addr, packet_no) in the database,
    /// delete it, and notify the UI so the conversation view can update.
    async fn handle_del_msg(&self, packet: Packet, from: SocketAddr) {
        let original_no = packet
            .extra
            .as_deref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);

        let sender_addr = format!("{}:{}", from.ip(), from.port());

        if original_no == 0 {
            debug!("DelMsg from {} with invalid packet_no, ignoring", from);
            return;
        }

        match self
            .db
            .delete_message_by_packet_no(&sender_addr, original_no)
            .await
        {
            Ok(true) => {
                info!("Deleted message from {} (packet_no {})", from, original_no);
                let _ = self
                    .ui_tx
                    .send(UiEvent::MessageDeleted {
                        sender_addr: from,
                        packet_no: original_no,
                    })
                    .await;
            }
            Ok(false) => {
                debug!(
                    "DelMsg from {} for packet_no {} — no matching message found",
                    from, original_no
                );
            }
            Err(e) => {
                warn!("Failed to delete message (DelMsg from {}): {}", from, e);
            }
        }
    }

    /// Handle an AnsReadMsg — the peer confirms it processed our ReadMsg receipt.
    ///
    /// This is a protocol-level acknowledgment with no user-visible effect.
    /// Our UI already updated the read status when we received the ReadMsg.
    async fn handle_ans_read_msg(&self, packet: Packet, from: SocketAddr) {
        let original_no = packet
            .extra
            .as_deref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        debug!(
            "AnsReadMsg from {}: peer acknowledged read receipt for packet {}",
            from, original_no
        );
    }

    /// Handle an OpenYou command (FeiQ extension, purpose unknown).
    ///
    /// This command is defined in the FeiQ protocol headers but its semantics
    /// are undocumented. For now we log it for diagnostic purposes and take
    /// no action.
    async fn handle_open_you(&self, packet: Packet, from: SocketAddr) {
        debug!(
            "OpenYou from {} (sender={}, extra={:?}) — no-op (purpose unknown)",
            from, packet.sender_name, packet.extra
        );
    }

    // ─── User List Protocol (BrIsGetList / OkGetList / GetList / AnsList) ──

    /// Handle BrIsGetList — a peer asks "does anyone have a list?"
    ///
    /// If we have any known peers, reply with OkGetList to say "yes, ask me".
    async fn handle_br_is_get_list(&self, _packet: Packet, from: SocketAddr) {
        let peer_count = self.peer_manager.peer_count().await;
        if peer_count > 0 {
            debug!(
                "BrIsGetList from {}: we have {} peers, replying OkGetList",
                from, peer_count
            );
            if let Err(e) = self.sender.send_ok_get_list(from).await {
                warn!("Failed to send OkGetList to {}: {}", from, e);
            }
        } else {
            debug!("BrIsGetList from {}: no peers known, ignoring", from);
        }
    }

    /// Handle OkGetList — a peer says "yes, I have a list".
    ///
    /// Follow up with a GetList request to actually fetch the list.
    async fn handle_ok_get_list(&self, _packet: Packet, from: SocketAddr) {
        debug!("OkGetList from {}: requesting peer list", from);
        if let Err(e) = self.sender.send_get_list(from).await {
            warn!("Failed to send GetList to {}: {}", from, e);
        }
    }

    /// Handle GetList — a peer requests our full peer list.
    ///
    /// Build an AnsList response with all known online peers.
    async fn handle_get_list(&self, _packet: Packet, from: SocketAddr) {
        let peers = self.peer_manager.online_peers().await;
        if peers.is_empty() {
            debug!("GetList from {}: no online peers to share", from);
            return;
        }

        let entries: Vec<String> = peers
            .iter()
            .filter(|p| p.addr != from.ip()) // Don't list the requester themselves.
            .map(|p| format!("{}:{}:{}:{}", p.addr, p.port, p.name, p.host))
            .collect();

        if entries.is_empty() {
            debug!("GetList from {}: only peer is the requester, skipping", from);
            return;
        }

        let payload = entries.join("\n");
        debug!(
            "GetList from {}: sending {} peer entries",
            from,
            entries.len()
        );
        if let Err(e) = self.sender.send_ans_list(from, &payload).await {
            warn!("Failed to send AnsList to {}: {}", from, e);
        }
    }

    /// Handle AnsList — a peer sent us their peer list.
    ///
    /// Parse the newline-separated entries (format: `ip:port:name:host`) and
    /// add any unknown peers to our PeerManager so they appear in the sidebar.
    async fn handle_ans_list(&self, packet: Packet, from: SocketAddr) {
        let entries = match &packet.extra {
            Some(e) if !e.is_empty() => e,
            _ => {
                debug!("AnsList from {}: empty list", from);
                return;
            }
        };

        let mut added = 0u32;
        for line in entries.split('\n') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Format: ip:port:name:host
            let mut parts = line.splitn(4, ':');
            let (Some(ip_str), Some(port_str), Some(name), Some(host)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                debug!("AnsList: skipping malformed entry: {:?}", line);
                continue;
            };

            let (Ok(ip), Ok(port)) = (ip_str.parse::<IpAddr>(), port_str.parse::<u16>()) else {
                debug!(
                    "AnsList: skipping entry with bad ip/port: {:?}",
                    line
                );
                continue;
            };

            // Skip ourselves.
            if ip == from.ip() && port == from.port() {
                continue;
            }

            let addr = SocketAddr::new(ip, port);
            // Don't overwrite existing peers — only add new ones.
            if self.peer_manager.get_peer_by_addr(&ip).await.is_some() {
                continue;
            }

            let peer = PeerInfo::new(name, host, addr);
            self.peer_manager.add_or_update_peer(peer).await;
            added += 1;
        }

        if added > 0 {
            info!(
                "AnsList from {}: added {} new peers",
                from, added
            );
        }
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
    ///
    /// Downloads the image from the sender via TCP GetFileData, saves it
    /// to the local image cache directory, persists the message with
    /// media_type=1, and emits an ImageReceived event.
    async fn handle_image(&self, packet: Packet, from: SocketAddr) {
        debug!("Image message from {}: {:?}", from, packet.extra);

        let peer_id = format!("{}:{}", from.ip(), from.port());
        let timestamp = now_timestamp();
        let msg_id = Uuid::new_v4().to_string();

        // Parse the file_id from the extra field (hex string, e.g. "00000001").
        let extra = packet.extra.as_deref().unwrap_or("");
        let file_id = match u32::from_str_radix(extra.trim(), 16) {
            Ok(id) => id,
            Err(_) => {
                // Try parsing as decimal as a fallback.
                match extra.trim().parse::<u32>() {
                    Ok(id) => id,
                    Err(_) => {
                        warn!("Invalid image id in SendImage from {}: {:?}", from, extra);
                        // Fall back to placeholder text.
                        let content = format!("[图片] {}", extra);
                        let stored = StoredMessage {
                            id: msg_id.clone(),
                            sender: peer_id,
                            recipient: self.local_id.clone(),
                            content,
                            timestamp,
                            read: false,
                            packet_no: Some(packet.packet_no),
                            group_id: None,
                            media_type: 0,
                        };
                        let _ = self.db.insert_message(&stored).await;
                        let _ = self
                            .ui_tx
                            .send(UiEvent::MessageReceived {
                                id: msg_id,
                                sender: packet.sender_name,
                                sender_addr: from,
                                content: "[图片]".to_string(),
                                timestamp,
                                packet_no: packet.packet_no,
                            })
                            .await;
                        return;
                    }
                }
            }
        };

        // Create the images cache directory.
        let images_dir = self.download_dir().join("images");
        if let Err(e) = std::fs::create_dir_all(&images_dir) {
            warn!("Failed to create images dir {:?}: {}", images_dir, e);
            return;
        }

        // Download the image via TCP.
        let image_filename = format!("{}_{}.png", from.ip(), file_id);
        let dest = images_dir.join(&image_filename);

        info!(
            "Downloading image {} (file_id={}) from {}",
            image_filename, file_id, from
        );
        match FileDownloader::download(
            from,
            file_id,
            0,
            u64::MAX, // Read until sender closes connection.
            &dest,
            &self.local_name,
            &self.local_host,
            None, // No progress callback for images.
        )
        .await
        {
            Ok(bytes) => {
                info!(
                    "Image {} saved ({} bytes) to {:?}",
                    image_filename, bytes, dest
                );

                let image_path = dest.to_string_lossy().to_string();

                // Persist with media_type = 1 (image).
                let stored = StoredMessage {
                    id: msg_id.clone(),
                    sender: peer_id,
                    recipient: self.local_id.clone(),
                    content: image_path.clone(),
                    timestamp,
                    read: false,
                    packet_no: Some(packet.packet_no),
                    group_id: None,
                    media_type: 1,
                };
                if let Err(e) = self.db.insert_message(&stored).await {
                    warn!("Failed to store image message: {}", e);
                }

                let _ = self
                    .ui_tx
                    .send(UiEvent::ImageReceived {
                        id: msg_id,
                        sender: packet.sender_name,
                        sender_addr: from,
                        image_path,
                        timestamp,
                        packet_no: packet.packet_no,
                    })
                    .await;
            }
            Err(e) => {
                warn!("Failed to download image from {}: {}", from, e);
                // Store as a failed image message with placeholder.
                let content = "[图片下载失败]".to_string();
                let stored = StoredMessage {
                    id: msg_id.clone(),
                    sender: peer_id,
                    recipient: self.local_id.clone(),
                    content: content.clone(),
                    timestamp,
                    read: false,
                    packet_no: Some(packet.packet_no),
                    group_id: None,
                    media_type: 0,
                };
                let _ = self.db.insert_message(&stored).await;
                let _ = self
                    .ui_tx
                    .send(UiEvent::MessageReceived {
                        id: msg_id,
                        sender: packet.sender_name,
                        sender_addr: from,
                        content,
                        timestamp,
                        packet_no: packet.packet_no,
                    })
                    .await;
            }
        }
    }

    /// Handle an incoming GroupMsg (0x23) — group chat message.
    ///
    /// Parses the extra field to extract group name and message text using
    /// the `GroupPayload` format (`"{groupName}\0{messageText}"`). Matches
    /// the group name to a local group; if no match is found, auto-creates
    /// a new group with just the sender as member.
    async fn handle_group_msg(&self, from: SocketAddr, packet: &Packet) {
        let extra = match &packet.extra {
            Some(e) => e.as_str(),
            None => {
                debug!("GroupMsg from {} with no extra field", from);
                return;
            }
        };

        let (group_name, content) = GroupPayload::parse_extra(extra);

        // Find or auto-create group by name.
        let group = match self.db.get_group_by_name(group_name).await {
            Ok(Some(g)) => g,
            Ok(None) => {
                // Auto-create group with sender as sole member.
                let new_group = Group {
                    id: Uuid::new_v4().to_string(),
                    name: group_name.to_string(),
                    members: vec![format!("{}:{}", from.ip(), from.port())],
                    created_at: now_timestamp(),
                };
                if let Err(e) = self.db.insert_group(&new_group).await {
                    warn!("Failed to auto-create group '{}': {}", group_name, e);
                    return;
                }
                let _ = self
                    .ui_tx
                    .send(UiEvent::GroupCreated {
                        group_id: new_group.id.clone(),
                        group_name: new_group.name.clone(),
                        members: new_group.members.clone(),
                    })
                    .await;
                info!("Auto-created group '{}' (id={})", group_name, new_group.id);
                new_group
            }
            Err(e) => {
                warn!("Failed to look up group '{}': {}", group_name, e);
                return;
            }
        };

        let timestamp = now_timestamp();
        let msg_id = Uuid::new_v4().to_string();
        let peer_id = format!("{}:{}", from.ip(), from.port());

        // Persist group message.
        let stored = StoredMessage {
            id: msg_id.clone(),
            sender: peer_id.clone(),
            recipient: String::new(),
            content: content.to_string(),
            timestamp,
            read: false,
            packet_no: Some(packet.packet_no),
            group_id: Some(group.id.clone()),
            media_type: 0,
        };
        if let Err(e) = self.db.insert_message(&stored).await {
            warn!("Failed to store group message: {}", e);
        }

        // Send delivery receipt if requested.
        if packet.has_flag(flags::IPMSG_SENDCHECKOPT) {
            let _ = self
                .sender
                .send_delivery_receipt(from, packet.packet_no)
                .await;
        }

        // Emit UI event.
        let _ = self
            .ui_tx
            .send(UiEvent::GroupMessageReceived {
                group_id: group.id,
                group_name: group.name,
                sender: packet.sender_name.clone(),
                sender_addr: from,
                content: content.to_string(),
                timestamp,
                packet_no: packet.packet_no,
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
            packet_no: Some(packet_no),
            group_id: None,
            media_type: 0,
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
                packet_no,
            })
            .await;

        Ok((msg_id, packet_no))
    }

    /// Send a group chat message to all members of a group.
    ///
    /// Fans out `send_group_message` to every member address, persists the
    /// message locally with `group_id`, and emits a `GroupMessageReceived`
    /// event so the UI shows our own message in the group conversation.
    pub async fn send_group_chat_message(
        &self,
        group_id: &str,
        content: &str,
    ) -> Result<(String, u32), Box<dyn std::error::Error + Send + Sync>> {
        // Look up group.
        let groups = self.db.get_groups().await?;
        let group = groups
            .iter()
            .find(|g| g.id == group_id)
            .ok_or_else(|| format!("Group {} not found", group_id))?;

        let msg_id = Uuid::new_v4().to_string();
        let timestamp = now_timestamp();
        let mut last_packet_no = 0u32;

        // Fan-out: send to each member.
        for member_addr_str in &group.members {
            if let Ok(addr) = member_addr_str.parse::<SocketAddr>() {
                match self
                    .sender
                    .send_group_message(addr, &group.name, content)
                    .await
                {
                    Ok(no) => last_packet_no = no,
                    Err(e) => {
                        warn!("Failed to send group message to {}: {}", addr, e);
                    }
                }
            } else {
                warn!("Invalid member address: {}", member_addr_str);
            }
        }

        // Persist locally.
        let stored = StoredMessage {
            id: msg_id.clone(),
            sender: self.local_id.clone(),
            recipient: String::new(),
            content: content.to_string(),
            timestamp,
            read: true,
            packet_no: Some(last_packet_no),
            group_id: Some(group.id.clone()),
            media_type: 0,
        };
        if let Err(e) = self.db.insert_message(&stored).await {
            warn!("Failed to store sent group message: {}", e);
        }

        // Emit UI event for our own message.
        let local_addr = self.sender.local_addr().unwrap_or_else(|_| {
            SocketAddr::from(([127, 0, 0, 1], 2425))
        });
        let _ = self
            .ui_tx
            .send(UiEvent::GroupMessageReceived {
                group_id: group.id.clone(),
                group_name: group.name.clone(),
                sender: "You".to_string(),
                sender_addr: local_addr,
                content: content.to_string(),
                timestamp,
                packet_no: last_packet_no,
            })
            .await;

        Ok((msg_id, last_packet_no))
    }

    /// Send read receipts (ReadMsg) for a batch of received packet numbers.
    ///
    /// Called when the user opens a conversation so the peer knows their
    /// messages have been read. Each receipt carries the original sender's
    /// packet number, per the IPMsg/FeiQ convention.
    pub async fn send_read_receipts(&self, to: SocketAddr, packet_nos: &[u32]) {
        for &no in packet_nos {
            if let Err(e) = self.sender.send_read_receipt(to, no).await {
                debug!("Failed to send read receipt to {} for {}: {}", to, no, e);
            }
        }
    }

    /// Send a screen-shake (knock) request to a peer.
    pub async fn send_knock(&self, to: SocketAddr) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.sender.send_knock(to).await?;
        Ok(())
    }

    /// Ask a peer to recall/delete a message we previously sent.
    ///
    /// Sends DelMsg over UDP and also removes the message from our local
    /// database so both sides stay in sync.
    pub async fn send_del_msg(
        &self,
        to: SocketAddr,
        msg_id: &str,
        packet_no: u32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.sender.send_del_msg(to, packet_no).await?;
        // Also delete from our own database.
        if let Err(e) = self.db.delete_message(msg_id).await {
            warn!("Failed to locally delete recalled message {}: {}", msg_id, e);
        }
        Ok(())
    }

    /// Broadcast a BrIsGetList to discover peers beyond our direct broadcast range.
    ///
    /// Peers with known contacts will respond with OkGetList → GetList → AnsList,
    /// adding new peers to the sidebar.
    pub async fn request_peer_list(&self) {
        if let Err(e) = self.sender.send_br_is_get_list().await {
            warn!("Failed to broadcast BrIsGetList: {}", e);
        }
    }

    /// Set our presence status and broadcast the change to the LAN.
    ///
    /// Online is announced with `BrEntry`; Away/Busy with `BrAbsence`. The new
    /// status also becomes the one the discovery loop re-advertises on its
    /// periodic heartbeat.
    pub async fn set_status(&self, status: UserStatus) {
        if let Err(e) = self.sender.set_status(status).await {
            warn!("Failed to broadcast status {:?}: {}", status, e);
        }
    }

    /// Load conversation history for `peer` from the database and emit it to
    /// the UI as [`UiEvent::HistoryLoaded`].
    ///
    /// Messages are returned in chronological order (oldest first). `limit`
    /// caps how many of the most recent messages are loaded.
    pub async fn request_history(&self, peer: SocketAddr, limit: u32) {
        let peer_str = format!("{}:{}", peer.ip(), peer.port());
        let page = Page::new(limit, 0);
        match self
            .db
            .get_messages_paged(&self.local_id, &peer_str, page)
            .await
        {
            Ok(result) => {
                let messages = result
                    .items
                    .into_iter()
                    .map(|m| HistoryMsg {
                        id: m.id,
                        text: m.content,
                        timestamp: m.timestamp,
                        outgoing: m.sender == self.local_id,
                        media_type: m.media_type,
                    })
                    .collect();
                let _ = self
                    .ui_tx
                    .send(UiEvent::HistoryLoaded { peer, messages })
                    .await;
            }
            Err(e) => warn!("Failed to load history for {}: {}", peer, e),
        }
    }

    /// Send one or more local files to a peer.
    ///
    /// Each path is registered in the shared [`FileRegistry`] so the peer can
    /// pull the bytes over TCP. A single SendMsg packet carries the optional
    /// `text` plus all file records. Returns the assigned packet number.
    pub async fn send_files(
        &self,
        to: SocketAddr,
        text: &str,
        paths: &[PathBuf],
    ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
        let mut records: Vec<String> = Vec::new();
        let mut names: Vec<String> = Vec::new();

        for path in paths {
            match self.registry.register(path).await {
                Ok(offer) => {
                    let display_name = if offer.file_type == 2 {
                        format!("{}/", offer.filename)
                    } else {
                        offer.filename.clone()
                    };
                    names.push(display_name);
                    records.push(offer.to_wire_record());
                }
                Err(e) => {
                    warn!("Failed to register file {:?}: {}", path, e);
                }
            }
        }

        if records.is_empty() {
            return Err("no files could be registered".into());
        }

        // FeiQ joins multiple records with \x07 (the sender adds a trailing \x07).
        let record_str = records.join("\x07");
        let packet_no = self
            .sender
            .send_message_with_files(to, text, &record_str, true)
            .await?;

        // Persist a placeholder message so the send shows up in history.
        let msg_id = Uuid::new_v4().to_string();
        let timestamp = now_timestamp();
        let peer_id = format!("{}:{}", to.ip(), to.port());
        let display = if text.trim().is_empty() {
            format!("[文件] {}", names.join(", "))
        } else {
            format!("{}\n[文件] {}", text, names.join(", "))
        };

        let stored = StoredMessage {
            id: msg_id.clone(),
            sender: self.local_id.clone(),
            recipient: peer_id.clone(),
            content: display.clone(),
            timestamp,
            read: true,
            packet_no: Some(packet_no),
            group_id: None,
            media_type: 0,
        };
        if let Err(e) = self.db.insert_message(&stored).await {
            warn!("Failed to store sent file message: {}", e);
        }

        let _ = self
            .ui_tx
            .send(UiEvent::MessageSent {
                id: msg_id,
                recipient: peer_id,
                content: display,
                timestamp,
                packet_no,
            })
            .await;

        Ok(packet_no)
    }

    /// Send an image message to a peer.
    ///
    /// The image is registered in the shared [`FileRegistry`] so the peer can
    /// pull the bytes over TCP via GetFileData. A `SendImage` packet carries
    /// the 8-byte hex file ID in its extra field. The message is persisted
    /// locally with `media_type = 1` and an `ImageSent` event is emitted.
    pub async fn send_image_message(
        &self,
        to: SocketAddr,
        path: PathBuf,
    ) -> Result<(String, u32), Box<dyn std::error::Error + Send + Sync>> {
        let offer = self.registry.register(&path).await?;
        let file_id_hex = format!("{:08x}", offer.file_id);

        info!(
            "Sending image {} (file_id={}, {} bytes) to {}",
            offer.filename, file_id_hex, offer.size, to
        );

        let packet_no = self.sender.send_image(to, &file_id_hex).await?;

        let msg_id = Uuid::new_v4().to_string();
        let timestamp = now_timestamp();
        let peer_id = format!("{}:{}", to.ip(), to.port());
        let image_path = path.to_string_lossy().to_string();

        // Persist locally.
        let stored = StoredMessage {
            id: msg_id.clone(),
            sender: self.local_id.clone(),
            recipient: peer_id.clone(),
            content: image_path.clone(),
            timestamp,
            read: true,
            packet_no: Some(packet_no),
            group_id: None,
            media_type: 1,
        };
        if let Err(e) = self.db.insert_message(&stored).await {
            warn!("Failed to store sent image message: {}", e);
        }

        // Notify UI.
        let _ = self
            .ui_tx
            .send(UiEvent::ImageSent {
                id: msg_id.clone(),
                recipient: to,
                image_path,
                timestamp,
                packet_no,
            })
            .await;

        Ok((msg_id, packet_no))
    }

    /// Accept an incoming file offer: download it from the peer to our
    /// downloads directory, streaming progress to the UI.
    ///
    /// This runs the full download to completion (awaited). The caller is
    /// expected to invoke it from a spawned task.
    pub async fn accept_file(
        &self,
        from: SocketAddr,
        file_id: u32,
        filename: String,
        size: u64,
        is_dir: bool,
    ) {
        if is_dir {
            self.accept_dir(from, file_id, filename).await;
            return;
        }

        let transfer_id = format!("{}:{}", from, file_id);

        // Ensure the download directory exists.
        let download_dir = self.download_dir();
        if let Err(e) = std::fs::create_dir_all(&download_dir) {
            let _ = self
                .ui_tx
                .send(UiEvent::FileFailed {
                    transfer_id,
                    filename,
                    error: format!("无法创建下载目录: {}", e),
                })
                .await;
            return;
        }

        // Avoid clobbering an existing file by prefixing with the file id.
        let sanitized = filename
            .replace(['/', '\\', ':'], "_");
        let mut dest = download_dir.join(&sanitized);
        if dest.exists() {
            dest = download_dir
                .join(format!("{}_{}", file_id, sanitized));
        }

        // Throttled progress callback: only notify on each whole-percent change.
        let ui_tx = self.ui_tx.clone();
        let cb_id = transfer_id.clone();
        let cb_name = filename.clone();
        let last_pct = Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
        let progress: ProgressCallback = Arc::new(move |received: u64, total: u64| {
            let pct = if total == 0 { 100 } else { received * 100 / total };
            let prev = last_pct.load(std::sync::atomic::Ordering::Relaxed);
            if pct != prev {
                last_pct.store(pct, std::sync::atomic::Ordering::Relaxed);
                let _ = ui_tx.try_send(UiEvent::FileProgress {
                    transfer_id: cb_id.clone(),
                    filename: cb_name.clone(),
                    received,
                    total,
                });
            }
        });

        info!("Accepting file {} ({} bytes) from {}", filename, size, from);
        match FileDownloader::download(
            from,
            file_id,
            0,
            size,
            &dest,
            &self.local_name,
            &self.local_host,
            Some(progress),
        )
        .await
        {
            Ok(_) => {
                info!("File {} saved to {:?}", filename, dest);
                let _ = self
                    .ui_tx
                    .send(UiEvent::FileComplete {
                        transfer_id,
                        filename,
                        path: dest,
                    })
                    .await;
            }
            Err(e) => {
                warn!("Failed to download {} from {}: {}", filename, from, e);
                let _ = self
                    .ui_tx
                    .send(UiEvent::FileFailed {
                        transfer_id,
                        filename,
                        error: e.to_string(),
                    })
                    .await;
            }
        }
    }

    /// Accept an incoming directory (folder) offer: fetch its recursive listing
    /// and download the whole tree under our downloads directory, streaming
    /// aggregated progress to the UI.
    async fn accept_dir(&self, from: SocketAddr, dir_file_id: u32, folder_name: String) {
        let transfer_id = format!("{}:{}", from, dir_file_id);

        // Ensure the download directory exists.
        let download_dir = self.download_dir();
        if let Err(e) = std::fs::create_dir_all(&download_dir) {
            let _ = self
                .ui_tx
                .send(UiEvent::FileFailed {
                    transfer_id,
                    filename: folder_name,
                    error: format!("无法创建下载目录: {}", e),
                })
                .await;
            return;
        }

        // Avoid clobbering an existing folder by prefixing with the file id.
        let sanitized = folder_name.replace(['/', '\\', ':'], "_");
        let mut dest_base = download_dir.join(&sanitized);
        if dest_base.exists() {
            dest_base = download_dir
                .join(format!("{}_{}", dir_file_id, sanitized));
        }

        // Throttled aggregated progress: only notify on each whole-percent change.
        let ui_tx = self.ui_tx.clone();
        let cb_id = transfer_id.clone();
        let cb_name = folder_name.clone();
        let last_pct = Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
        let progress: ProgressCallback = Arc::new(move |received: u64, total: u64| {
            let pct = if total == 0 { 100 } else { received * 100 / total };
            let prev = last_pct.load(std::sync::atomic::Ordering::Relaxed);
            if pct != prev {
                last_pct.store(pct, std::sync::atomic::Ordering::Relaxed);
                let _ = ui_tx.try_send(UiEvent::FileProgress {
                    transfer_id: cb_id.clone(),
                    filename: cb_name.clone(),
                    received,
                    total,
                });
            }
        });

        info!("Accepting folder {} from {}", folder_name, from);
        match FileDownloader::download_dir(
            from,
            dir_file_id,
            &dest_base,
            &self.local_name,
            &self.local_host,
            Some(progress),
        )
        .await
        {
            Ok(summary) => {
                info!(
                    "Folder {} saved to {:?} ({} files, {} dirs, {} bytes)",
                    folder_name, dest_base, summary.files, summary.dirs, summary.bytes
                );
                let _ = self
                    .ui_tx
                    .send(UiEvent::FileComplete {
                        transfer_id,
                        filename: folder_name,
                        path: dest_base,
                    })
                    .await;
            }
            Err(e) => {
                warn!(
                    "Failed to download folder {} from {}: {}",
                    folder_name, from, e
                );
                let _ = self
                    .ui_tx
                    .send(UiEvent::FileFailed {
                        transfer_id,
                        filename: folder_name,
                        error: e.to_string(),
                    })
                    .await;
            }
        }
    }

    /// Reject an incoming file offer. No bytes are transferred. The UI removes
    /// the prompt locally, so no event is emitted here.
    pub async fn reject_file(&self, from: SocketAddr, file_id: u32, filename: String) {
        info!("Rejected file {} (id={}) from {}", filename, file_id, from);
    }

    // ─── Group Lifecycle ─────────────────────────────────────────────────

    /// Create a new group with the given name and member addresses.
    ///
    /// Generates a UUID, inserts the group into the database, and emits a
    /// `GroupCreated` event so the UI can update the sidebar.
    pub async fn create_group(
        &self,
        name: &str,
        members: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let group = Group {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            members: members.clone(),
            created_at: now_timestamp(),
        };
        self.db.insert_group(&group).await?;
        info!("Created group '{}' (id={}) with {} members", name, group.id, members.len());

        let _ = self
            .ui_tx
            .send(UiEvent::GroupCreated {
                group_id: group.id.clone(),
                group_name: group.name.clone(),
                members,
            })
            .await;

        Ok(group.id)
    }

    /// Delete a group by ID. Associated messages are not deleted.
    pub async fn delete_group(
        &self,
        group_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.db.delete_group(group_id).await?;
        info!("Deleted group {}", group_id);

        let _ = self
            .ui_tx
            .send(UiEvent::GroupDeleted {
                group_id: group_id.to_string(),
            })
            .await;

        Ok(())
    }

    /// Load all groups from the database.
    pub async fn get_groups(
        &self,
    ) -> Result<Vec<Group>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.db.get_groups().await?)
    }

    /// Load group message history and emit it as HistoryLoaded-like events.
    ///
    /// For group conversations, we emit `GroupMessageReceived` events for each
    /// historical message so the UI can render them in the group chat panel.
    pub async fn request_group_history(&self, group_id: &str, limit: u32) {
        let page = Page::new(limit, 0);
        match self.db.get_group_messages_paged(group_id, page).await {
            Ok(result) => {
                for msg in result.items {
                    let _ = self
                        .ui_tx
                        .send(UiEvent::GroupMessageReceived {
                            group_id: group_id.to_string(),
                            group_name: String::new(), // UI already knows from GroupCreated
                            sender: if msg.sender == self.local_id {
                                "You".to_string()
                            } else {
                                msg.sender.clone()
                            },
                            sender_addr: msg.sender.parse().unwrap_or_else(|_| {
                                SocketAddr::from(([127, 0, 0, 1], 2425))
                            }),
                            content: msg.content,
                            timestamp: msg.timestamp,
                            packet_no: msg.packet_no.unwrap_or(0),
                        })
                        .await;
                }
            }
            Err(e) => warn!("Failed to load group history for {}: {}", group_id, e),
        }
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
