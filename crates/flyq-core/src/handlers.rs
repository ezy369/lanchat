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
use flyq_protocol::{Command, Packet, PeerInfo, UserStatus};
use flyq_storage::{Database, Page, StoredMessage};
use std::net::SocketAddr;
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
