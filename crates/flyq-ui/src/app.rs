//! Main application view.
//!
//! [`LanChatApp`] is the root GPUI entity. It owns the UI-side state (online
//! peers, per-conversation message lists, typing indicators, the selected peer
//! and the chat input), consumes [`UiEvent`]s emitted by the core event loop,
//! and dispatches outgoing messages through a shared [`EventHandler`].

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use flyq_core::{AppConfig, EventHandler, UiEvent};
use flyq_protocol::{PeerInfo, UserStatus};
use gpui::prelude::*;
use gpui::{
    div, px, white, AnyElement, App, AsyncApp, Context, Entity, ExternalPaths, Hsla,
    PathPromptOptions, Window,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{h_flex, v_flex, ActiveTheme, Root, Sizable};
use tokio::sync::mpsc;
use tracing::warn;

use crate::chat;
use crate::notify;
use crate::settings;
use crate::sidebar;
use crate::tokio_runtime::Tokio;

/// Delivery/read state of an outgoing message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgStatus {
    /// Queued / being sent (not yet confirmed on the wire).
    Sending,
    /// Sent over UDP, no receipt yet.
    Sent,
    /// Peer confirmed delivery (RecvMsg).
    Delivered,
    /// Peer confirmed the message was read (ReadMsg).
    Read,
}

/// A single chat message as rendered in the UI.
#[derive(Clone, Debug)]
pub struct ChatMsg {
    /// Stable message id.
    pub id: String,
    /// Message text.
    pub text: String,
    /// Unix timestamp (seconds).
    pub timestamp: i64,
    /// True when we sent this message; false when received.
    pub outgoing: bool,
    /// Display name of the sender.
    pub sender_name: String,
    /// IPMsg packet number, used to match receipts (received) or acknowledge
    /// reads (incoming). `None` for messages loaded from history.
    pub packet_no: Option<u32>,
    /// Delivery/read state (meaningful for outgoing messages).
    pub status: MsgStatus,
}

/// An incoming file offer awaiting the user's accept/reject decision.
#[derive(Clone, Debug)]
pub struct FileOfferPrompt {
    /// Peer address the file was offered by.
    pub from: SocketAddr,
    /// Display name of the offering peer.
    pub from_name: String,
    /// Sender-assigned file id (needed to request the bytes over TCP).
    pub file_id: u32,
    /// Original filename.
    pub filename: String,
    /// Declared size in bytes.
    pub size: u64,
    /// True when the offer is a directory (recursive folder transfer).
    pub is_dir: bool,
}

/// Lifecycle stage of a tracked file transfer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransferStage {
    /// Currently downloading.
    Downloading,
    /// Completed and saved to disk.
    Complete,
    /// Failed (network error, etc.).
    Failed,
}

/// A tracked file transfer (currently only incoming downloads).
#[derive(Clone, Debug)]
pub struct Transfer {
    /// Original filename.
    pub filename: String,
    /// Bytes received so far.
    pub received: u64,
    /// Total expected bytes.
    pub total: u64,
    /// Current lifecycle stage.
    pub stage: TransferStage,
    /// Saved path once complete.
    pub path: Option<PathBuf>,
    /// Error message once failed.
    pub error: Option<String>,
}

/// Copyable snapshot of the theme colors used by the render helpers.
///
/// Captured once per render so that closures building child elements do not
/// need to borrow the GPUI context (which would conflict with the mutable
/// borrows required elsewhere in `render`).
#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Hsla,
    pub sidebar: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    pub primary: Hsla,
    pub accent: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
}

impl Palette {
    /// Read the palette from the current theme.
    pub fn from_app(cx: &App) -> Self {
        let t = cx.theme();
        Self {
            background: t.background,
            sidebar: t.sidebar,
            muted: t.muted,
            muted_foreground: t.muted_foreground,
            foreground: t.foreground,
            border: t.border,
            primary: t.primary,
            accent: t.accent,
            success: t.success,
            warning: t.warning,
            danger: t.danger,
        }
    }
}

/// The main LanChat application view.
pub struct LanChatApp {
    /// Our own display name (shown in the sidebar header).
    local_name: String,
    /// Our own presence status (shown in the sidebar; broadcast to peers).
    local_status: UserStatus,
    /// Currently online peers.
    peers: Vec<PeerInfo>,
    /// Addr of the peer whose conversation is open (if any).
    selected: Option<SocketAddr>,
    /// Message history keyed by peer address.
    conversations: HashMap<SocketAddr, Vec<ChatMsg>>,
    /// Peers currently typing, keyed by address (value = display name).
    typing: HashMap<SocketAddr, String>,
    /// Known display names keyed by address (survives a peer going offline).
    names: HashMap<SocketAddr, String>,
    /// Packet numbers of incoming messages we've already acknowledged as read.
    read_acked: HashSet<u32>,
    /// Conversations whose history has already been requested from the DB.
    history_loaded: HashSet<SocketAddr>,
    /// When a screen-shake (knock) animation started, if active.
    shake: Option<Instant>,
    /// Transient banner text for an incoming knock, with its start time.
    knock_banner: Option<(String, Instant)>,
    /// Incoming file offers awaiting an accept/reject decision.
    file_offers: Vec<FileOfferPrompt>,
    /// Tracked file transfers keyed by transfer id.
    transfers: HashMap<String, Transfer>,
    /// Chat message input state.
    input: Entity<InputState>,
    /// Shared handler used to send messages (persists + emits MessageSent).
    handler: Arc<EventHandler>,
    /// Tokio runtime handle for spawning send tasks from click handlers.
    rt: tokio::runtime::Handle,
    /// Current application configuration (nickname / download dir / port).
    config: AppConfig,
    /// Whether the settings modal is visible.
    settings_open: bool,
    /// Settings form: display nickname field.
    settings_nickname: Entity<InputState>,
    /// Settings form: download directory field.
    settings_download: Entity<InputState>,
    /// Settings form: network port field.
    settings_port: Entity<InputState>,
}

impl LanChatApp {
    /// Create a new application view.
    ///
    /// * `local_name` — our display name.
    /// * `config` — the loaded application configuration (nickname / download
    ///   dir / port), used to prefill the settings panel.
    /// * `handler` — shared event handler for sending messages.
    /// * `ui_rx` — channel carrying [`UiEvent`]s from the core event loop.
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        local_name: String,
        config: AppConfig,
        handler: Arc<EventHandler>,
        mut ui_rx: mpsc::Receiver<UiEvent>,
    ) -> Self {
        let rt = Tokio::handle(cx);

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Type a message, press Enter to send…")
        });

        // Settings form fields, prefilled from the loaded config.
        let settings_nickname = cx.new(|cx| InputState::new(window, cx).placeholder("昵称"));
        settings_nickname.update(cx, |i, cx| {
            i.set_value(&config.nickname, window, cx);
        });
        let settings_download = cx.new(|cx| InputState::new(window, cx).placeholder("下载目录"));
        settings_download.update(cx, |i, cx| {
            i.set_value(config.download_dir.to_string_lossy().as_ref(), window, cx);
        });
        let settings_port = cx.new(|cx| InputState::new(window, cx).placeholder("端口"));
        settings_port.update(cx, |i, cx| {
            i.set_value(config.port.to_string(), window, cx);
        });

        // Enter sends the message; other input events are ignored for now.
        cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, _input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { shift, .. } => {
                    if !*shift {
                        this.send_current_message(window, cx);
                    }
                }
                _ => {}
            },
        )
        .detach();

        let app = Self {
            local_name,
            local_status: config.status,
            peers: Vec::new(),
            selected: None,
            conversations: HashMap::new(),
            typing: HashMap::new(),
            names: HashMap::new(),
            read_acked: HashSet::new(),
            history_loaded: HashSet::new(),
            shake: None,
            knock_banner: None,
            file_offers: Vec::new(),
            transfers: HashMap::new(),
            input,
            handler,
            rt,
            config,
            settings_open: false,
            settings_nickname,
            settings_download,
            settings_port,
        };

        // Consume UI events on GPUI's executor. `tokio::sync::mpsc::recv` is
        // runtime-agnostic, so awaiting it here is fine even though the sender
        // lives on the Tokio runtime.
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Some(event) = ui_rx.recv().await {
                if this
                    .update(cx, |app, cx| app.apply_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        app
    }

    /// Apply a single UI event to the state and request a re-render.
    fn apply_event(&mut self, event: UiEvent, cx: &mut Context<Self>) {
        match event {
            UiEvent::PeerOnline(peer) => {
                self.names.insert(peer.socket_addr(), peer.name.clone());
                match self
                    .peers
                    .iter_mut()
                    .find(|p| p.addr == peer.addr && p.port == peer.port)
                {
                    Some(existing) => *existing = peer,
                    None => self.peers.push(peer),
                }
                self.sort_peers();
            }
            UiEvent::PeerOffline(peer) => {
                self.peers
                    .retain(|p| !(p.addr == peer.addr && p.port == peer.port));
                self.typing.remove(&peer.socket_addr());
            }
            UiEvent::PeerStatusChanged { peer } => {
                self.names.insert(peer.socket_addr(), peer.name.clone());
                if let Some(existing) = self
                    .peers
                    .iter_mut()
                    .find(|p| p.addr == peer.addr && p.port == peer.port)
                {
                    existing.status = peer.status;
                    existing.online = peer.online;
                }
                self.sort_peers();
            }
            UiEvent::MessageReceived {
                id,
                sender,
                sender_addr,
                content,
                timestamp,
                packet_no,
            } => {
                self.names.insert(sender_addr, sender.clone());
                self.typing.remove(&sender_addr);
                // Notify only when this conversation is not already open; if the
                // user is looking at it, a system toast is just noise.
                let viewing = self.selected == Some(sender_addr);
                if !viewing {
                    notify::message(&self.rt, &sender, &content);
                }
                self.conversations.entry(sender_addr).or_default().push(ChatMsg {
                    id,
                    text: content,
                    timestamp,
                    outgoing: false,
                    sender_name: sender,
                    packet_no: Some(packet_no),
                    status: MsgStatus::Read,
                });
                // If this conversation is currently open, acknowledge the read
                // immediately so the peer sees the "read" state.
                if viewing {
                    self.acknowledge_reads(sender_addr);
                }
            }
            UiEvent::MessageSent {
                id,
                recipient,
                content,
                timestamp,
                packet_no,
            } => {
                if let Ok(addr) = recipient.parse::<SocketAddr>() {
                    self.conversations.entry(addr).or_default().push(ChatMsg {
                        id,
                        text: content,
                        timestamp,
                        outgoing: true,
                        sender_name: self.local_name.clone(),
                        packet_no: Some(packet_no),
                        status: MsgStatus::Sent,
                    });
                } else {
                    warn!("MessageSent recipient not a socket addr: {}", recipient);
                }
            }
            UiEvent::TypingStart { from, name } => {
                self.typing.insert(from, name);
            }
            UiEvent::TypingEnd { from, .. } => {
                self.typing.remove(&from);
            }
            UiEvent::Knock { from: _, name } => {
                // Trigger the screen-shake animation and a transient banner.
                self.shake = Some(Instant::now());
                self.knock_banner = Some((format!("{} 抖了抖你", name), Instant::now()));
            }
            UiEvent::DeliveryConfirmed {
                original_packet_no,
                from,
            } => {
                self.set_outgoing_status(from, original_packet_no, MsgStatus::Delivered);
            }
            UiEvent::ReadConfirmed {
                original_packet_no,
                from,
            } => {
                self.set_outgoing_status(from, original_packet_no, MsgStatus::Read);
            }
            UiEvent::HistoryLoaded { peer, messages } => {
                let name = self.peer_name(&peer);
                let local_name = self.local_name.clone();
                let entry = self.conversations.entry(peer).or_default();
                for h in messages {
                    if entry.iter().any(|m| m.id == h.id) {
                        continue; // Already present (e.g. a live message).
                    }
                    entry.push(ChatMsg {
                        id: h.id,
                        text: h.text,
                        timestamp: h.timestamp,
                        outgoing: h.outgoing,
                        sender_name: if h.outgoing {
                            local_name.clone()
                        } else {
                            name.clone()
                        },
                        packet_no: None,
                        status: MsgStatus::Read,
                    });
                }
                // Stable sort keeps same-second messages in insertion order.
                entry.sort_by_key(|m| m.timestamp);
            }
            UiEvent::FileOfferReceived {
                from,
                name,
                msg_id: _,
                files,
            } => {
                // Always notify: an incoming file needs the user's attention to
                // accept or reject it.
                notify::file_offer(&self.rt, &name, files.len());
                for f in files {
                    self.file_offers.push(FileOfferPrompt {
                        from,
                        from_name: name.clone(),
                        file_id: f.file_id,
                        filename: f.filename,
                        size: f.size,
                        is_dir: f.is_dir,
                    });
                }
            }
            UiEvent::FileProgress {
                transfer_id,
                filename,
                received,
                total,
            } => {
                let t = self.transfers.entry(transfer_id).or_insert(Transfer {
                    filename,
                    received,
                    total,
                    stage: TransferStage::Downloading,
                    path: None,
                    error: None,
                });
                t.received = received;
                t.total = total;
                t.stage = TransferStage::Downloading;
            }
            UiEvent::FileComplete {
                transfer_id,
                filename,
                path,
            } => {
                notify::file_complete(&self.rt, &filename);
                let t = self.transfers.entry(transfer_id).or_insert(Transfer {
                    filename,
                    received: 0,
                    total: 0,
                    stage: TransferStage::Complete,
                    path: Some(path.clone()),
                    error: None,
                });
                t.stage = TransferStage::Complete;
                t.path = Some(path);
                t.received = t.total;
                t.error = None;
            }
            UiEvent::FileFailed {
                transfer_id,
                filename,
                error,
            } => {
                let t = self.transfers.entry(transfer_id).or_insert(Transfer {
                    filename,
                    received: 0,
                    total: 0,
                    stage: TransferStage::Failed,
                    path: None,
                    error: Some(error.clone()),
                });
                t.stage = TransferStage::Failed;
                t.error = Some(error);
            }
            UiEvent::MessageDeleted {
                sender_addr,
                packet_no,
            } => {
                // Remove the message from the conversation if it's currently loaded.
                if let Some(msgs) = self.conversations.get_mut(&sender_addr) {
                    msgs.retain(|m| m.packet_no != Some(packet_no));
                }
            }
        }
        cx.notify();
    }

    /// Keep the peer list sorted by name (case-insensitive).
    fn sort_peers(&mut self) {
        self.peers
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }

    /// Select a peer to open its conversation.
    pub fn select_peer(&mut self, addr: SocketAddr, cx: &mut Context<Self>) {
        if self.selected != Some(addr) {
            self.selected = Some(addr);
        }
        // Lazily load persisted history the first time a conversation is opened.
        self.ensure_history(addr);
        // Opening a conversation acknowledges any unread incoming messages.
        self.acknowledge_reads(addr);
        cx.notify();
    }

    /// Request conversation history from the database if not already loaded.
    ///
    /// Results arrive asynchronously via [`UiEvent::HistoryLoaded`].
    fn ensure_history(&mut self, addr: SocketAddr) {
        if self.history_loaded.contains(&addr) {
            return;
        }
        self.history_loaded.insert(addr);
        let handler = self.handler.clone();
        self.rt.spawn(async move {
            handler.request_history(addr, 100).await;
        });
    }

    /// Send read receipts for all not-yet-acknowledged incoming messages from
    /// `addr`, recording them so we don't acknowledge twice.
    fn acknowledge_reads(&mut self, addr: SocketAddr) {
        let mut pending: Vec<u32> = Vec::new();
        if let Some(msgs) = self.conversations.get(&addr) {
            for m in msgs {
                if !m.outgoing {
                    if let Some(no) = m.packet_no {
                        if !self.read_acked.contains(&no) {
                            pending.push(no);
                        }
                    }
                }
            }
        }
        if pending.is_empty() {
            return;
        }
        for &no in &pending {
            self.read_acked.insert(no);
        }
        let handler = self.handler.clone();
        self.rt.spawn(async move {
            handler.send_read_receipts(addr, &pending).await;
        });
    }

    /// Upgrade the delivery/read status of an outgoing message matched by its
    /// packet number. Statuses only ever advance (Read is never downgraded).
    fn set_outgoing_status(&mut self, from: SocketAddr, packet_no: u32, status: MsgStatus) {
        if let Some(msgs) = self.conversations.get_mut(&from) {
            for m in msgs.iter_mut() {
                if m.outgoing && m.packet_no == Some(packet_no) {
                    if status_rank(status) > status_rank(m.status) {
                        m.status = status;
                    }
                    break;
                }
            }
        }
    }

    /// Resolve the display name for a peer address.
    fn peer_name(&self, addr: &SocketAddr) -> String {
        self.names
            .get(addr)
            .cloned()
            .or_else(|| {
                self.peers
                    .iter()
                    .find(|p| &p.socket_addr() == addr)
                    .map(|p| p.name.clone())
            })
            .unwrap_or_else(|| addr.to_string())
    }

    /// Read the input, clear it, and send the message to the selected peer.
    fn send_current_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(to) = self.selected else {
            return;
        };

        // Clear the input immediately for a responsive feel.
        self.input.update(cx, |input, cx| input.set_value("", window, cx));

        let handler = self.handler.clone();
        self.rt.spawn(async move {
            if let Err(e) = handler.send_chat_message(to, &text, true).await {
                warn!("Failed to send message to {}: {}", to, e);
            }
        });
    }

    /// Accept a pending incoming file offer: remove the prompt and start the
    /// download on the Tokio runtime.
    fn accept_offer(&mut self, offer: &FileOfferPrompt, cx: &mut Context<Self>) {
        self.file_offers
            .retain(|o| !(o.from == offer.from && o.file_id == offer.file_id));
        let handler = self.handler.clone();
        let from = offer.from;
        let file_id = offer.file_id;
        let filename = offer.filename.clone();
        let size = offer.size;
        let is_dir = offer.is_dir;
        self.rt.spawn(async move {
            handler.accept_file(from, file_id, filename, size, is_dir).await;
        });
        cx.notify();
    }

    /// Reject a pending incoming file offer: remove the prompt and notify the
    /// core (no bytes are transferred).
    fn reject_offer(&mut self, offer: &FileOfferPrompt, cx: &mut Context<Self>) {
        self.file_offers
            .retain(|o| !(o.from == offer.from && o.file_id == offer.file_id));
        let handler = self.handler.clone();
        let from = offer.from;
        let file_id = offer.file_id;
        let filename = offer.filename.clone();
        self.rt.spawn(async move {
            handler.reject_file(from, file_id, filename).await;
        });
        cx.notify();
    }

    /// Remove a finished (or failed) transfer card from the panel.
    fn dismiss_transfer(&mut self, transfer_id: &str, cx: &mut Context<Self>) {
        self.transfers.remove(transfer_id);
        cx.notify();
    }

    /// Open the settings modal.
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = true;
        cx.notify();
    }

    /// Close the settings modal without saving.
    fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        cx.notify();
    }

    /// Cycle our presence status (在线 → 离开 → 忙碌 → 在线), broadcast the new
    /// status to the LAN, and persist it so it is restored on next launch.
    fn cycle_status(&mut self, cx: &mut Context<Self>) {
        let next = match self.local_status {
            UserStatus::Online => UserStatus::Away,
            UserStatus::Away => UserStatus::Busy,
            _ => UserStatus::Online,
        };
        self.local_status = next;
        self.config.status = next;

        let handler = self.handler.clone();
        let to_save = self.config.clone();
        self.rt.spawn(async move {
            handler.set_status(next).await;
            if let Err(e) = to_save.save() {
                warn!("Failed to save config: {}", e);
            }
        });
        cx.notify();
    }

    /// Read the settings form, persist it, and apply what can be applied live.
    ///
    /// The download directory is applied immediately (the core handler picks it
    /// up for subsequent transfers); the nickname updates our sidebar display at
    /// once but only reaches the network after a restart, as does the port.
    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let nickname = self.settings_nickname.read(cx).value().to_string();
        let download_str = self.settings_download.read(cx).value().to_string();
        let port_str = self.settings_port.read(cx).value().to_string();

        let port = port_str.trim().parse::<u16>().unwrap_or(self.config.port);
        let download_dir = if download_str.trim().is_empty() {
            self.config.download_dir.clone()
        } else {
            PathBuf::from(download_str.trim())
        };

        let mut config = AppConfig {
            nickname,
            download_dir,
            port,
            status: self.config.status,
        };
        config.normalize();

        // Apply the download directory live.
        if let Err(e) = self.handler.set_download_dir(config.download_dir.clone()) {
            warn!(
                "Failed to create download dir {:?}: {}",
                config.download_dir, e
            );
        }

        // Persist to disk off the UI thread.
        let to_save = config.clone();
        self.rt.spawn(async move {
            if let Err(e) = to_save.save() {
                warn!("Failed to save config: {}", e);
            }
        });

        // Reflect the new nickname in our own display immediately.
        self.local_name = config.nickname.clone();
        self.config = config;
        self.settings_open = false;
        cx.notify();
    }
}

impl Render for LanChatApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::from_app(cx);
        let this = cx.entity().clone();
        let input_entity = self.input.clone();
        let handler = self.handler.clone();
        let rt = self.rt.clone();
        let selected = self.selected;
        let local_name = self.local_name.clone();
        let local_status = self.local_status;
        let peer_count = self.peers.len();

        // ── Screen-shake (knock) animation ───────────────────────────────
        // A decaying sinusoidal offset applied to the content layer for ~0.5s.
        let mut shake_dx = 0.0f32;
        let mut shake_dy = 0.0f32;
        if let Some(start) = self.shake {
            const DURATION: f32 = 0.5;
            let t = start.elapsed().as_secs_f32();
            if t >= DURATION {
                self.shake = None;
            } else {
                let decay = 1.0 - t / DURATION;
                let amp = 14.0 * decay;
                shake_dx = (t * 55.0).sin() * amp;
                shake_dy = (t * 71.0).sin() * amp * 0.5;
                window.request_animation_frame();
            }
        }

        // ── Transient knock banner ───────────────────────────────────────
        let mut banner: Option<String> = None;
        if let Some((text, start)) = self.knock_banner.clone() {
            if start.elapsed().as_secs_f32() >= 3.0 {
                self.knock_banner = None;
            } else {
                banner = Some(text);
                window.request_animation_frame();
            }
        }

        // ── Sidebar peer rows ────────────────────────────────────────────
        let mut rows: Vec<AnyElement> = Vec::with_capacity(self.peers.len());
        for peer in &self.peers {
            let addr = peer.socket_addr();
            let is_selected = Some(addr) == selected;
            let typing = self.typing.get(&addr).cloned();
            let row_this = this.clone();
            let row_input = input_entity.clone();
            let row = sidebar::peer_row(
                peer,
                is_selected,
                typing.as_deref(),
                palette,
                move |_, window, cx| {
                    row_this.update(cx, |app, cx| app.select_peer(addr, cx));
                    row_input.update(cx, |i, cx| i.focus(window, cx));
                },
            );
            rows.push(row);
        }
        let settings_this = this.clone();
        let status_this = this.clone();
        let sidebar_el = sidebar::render_sidebar(
            &local_name,
            local_status,
            peer_count,
            rows,
            palette,
            move |_, _, cx| {
                settings_this.update(cx, |app, cx| app.open_settings(cx));
            },
            move |_, _, cx| {
                status_this.update(cx, |app, cx| app.cycle_status(cx));
            },
        );

        // ── Chat panel ───────────────────────────────────────────────────
        let chat_el = match selected {
            Some(addr) => {
                let name = self.peer_name(&addr);
                let messages = self.conversations.get(&addr).cloned().unwrap_or_default();
                let typing = self.typing.get(&addr).cloned();
                let send_this = this.clone();
                let send_input = input_entity.clone();
                let send_handler = handler.clone();
                let send_rt = rt.clone();
                let knock_handler = handler.clone();
                let knock_rt = rt.clone();
                let file_handler = handler.clone();
                let file_rt = rt.clone();
                let folder_handler = handler.clone();
                let folder_rt = rt.clone();
                chat::render_chat_panel(
                    Some(&name),
                    &messages,
                    typing.as_deref(),
                    &self.input,
                    palette,
                    move |_, window, cx| {
                        let text = send_input.read(cx).value().to_string();
                        let text = text.trim().to_string();
                        if text.is_empty() {
                            return;
                        }
                        send_input.update(cx, |i, cx| i.set_value("", window, cx));
                        let handler = send_handler.clone();
                        send_rt.spawn(async move {
                            if let Err(e) = handler.send_chat_message(addr, &text, true).await {
                                warn!("Failed to send message to {}: {}", addr, e);
                            }
                        });
                        // Keep a reference to the entity alive for future use.
                        let _ = &send_this;
                    },
                    move |_, _, _| {
                        let handler = knock_handler.clone();
                        knock_rt.spawn(async move {
                            if let Err(e) = handler.send_knock(addr).await {
                                warn!("Failed to send knock to {}: {}", addr, e);
                            }
                        });
                    },
                    move |_, _, cx| {
                        // Native multi-file picker (runs on the platform thread).
                        let rx = cx.prompt_for_paths(PathPromptOptions {
                            files: true,
                            directories: false,
                            multiple: true,
                            prompt: None,
                        });
                        let handler = file_handler.clone();
                        file_rt.spawn(async move {
                            match rx.await {
                                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                                    if let Err(e) = handler.send_files(addr, "", &paths).await {
                                        warn!("Failed to send files to {}: {}", addr, e);
                                    }
                                }
                                Ok(Err(e)) => warn!("File picker error: {}", e),
                                _ => {}
                            }
                        });
                    },
                    move |_, _, cx| {
                        // Native directory picker (folder transfer).
                        let rx = cx.prompt_for_paths(PathPromptOptions {
                            files: false,
                            directories: true,
                            multiple: true,
                            prompt: None,
                        });
                        let handler = folder_handler.clone();
                        folder_rt.spawn(async move {
                            match rx.await {
                                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                                    if let Err(e) = handler.send_files(addr, "", &paths).await {
                                        warn!("Failed to send folder to {}: {}", addr, e);
                                    }
                                }
                                Ok(Err(e)) => warn!("Folder picker error: {}", e),
                                _ => {}
                            }
                        });
                    },
                )
            }
            None => chat::render_chat_panel(
                None,
                &[],
                None,
                &self.input,
                palette,
                |_, _, _| {},
                |_, _, _| {},
                |_, _, _| {},
                |_, _, _| {},
            ),
        };

        let drop_handler = handler.clone();
        let drop_rt = rt.clone();
        let drop_selected = selected;

        let content = h_flex()
            .size_full()
            .child(
                div()
                    .w(px(248.0))
                    .h_full()
                    .bg(palette.sidebar)
                    .border_r_1()
                    .border_color(palette.border)
                    .child(sidebar_el),
            )
            .child(
                div()
                    .id("chat-drop")
                    .flex_1()
                    .h_full()
                    .drag_over::<ExternalPaths>(move |style, _paths, _window, _cx| {
                        style.bg(palette.muted).border_color(palette.primary)
                    })
                    .on_drop::<ExternalPaths>(move |paths, _window, _cx| {
                        if let Some(to) = drop_selected {
                            let files: Vec<PathBuf> = paths.paths().to_vec();
                            if !files.is_empty() {
                                let handler = drop_handler.clone();
                                drop_rt.spawn(async move {
                                    if let Err(e) = handler.send_files(to, "", &files).await {
                                        warn!("Failed to send dropped files to {}: {}", to, e);
                                    }
                                });
                            }
                        }
                    })
                    .child(chat_el),
            );

        let mut root = div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(palette.background)
            .child(
                div()
                    .absolute()
                    .top(px(shake_dy))
                    .left(px(shake_dx))
                    .size_full()
                    .child(content),
            );

        if let Some(text) = banner {
            root = root.child(
                div().absolute().top(px(18.0)).w_full().child(
                    h_flex().w_full().justify_center().child(
                        div()
                            .px(px(16.0))
                            .py(px(7.0))
                            .rounded(px(16.0))
                            .bg(palette.primary)
                            .text_color(white())
                            .text_sm()
                            .child(text),
                    ),
                ),
            );
        }

        // ── Transfer panel (pending offers + progress/completion) ─────────
        let mut transfer_cards: Vec<AnyElement> = Vec::new();
        for offer in self.file_offers.clone() {
            let accept_this = this.clone();
            let accept_offer = offer.clone();
            let reject_this = this.clone();
            let reject_offer = offer.clone();
            let accept_id = format!("accept-{}-{}", offer.from, offer.file_id);
            let reject_id = format!("reject-{}-{}", offer.from, offer.file_id);
            let card = div()
                .w_full()
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(palette.sidebar)
                .border_1()
                .border_color(palette.primary)
                .child(
                    v_flex()
                        .w_full()
                        .gap(px(6.0))
                        .child(
                            h_flex()
                                .w_full()
                                .gap(px(6.0))
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .overflow_x_hidden()
                                        .text_sm()
                                        .text_color(palette.foreground)
                                        .child(if offer.is_dir {
                                            format!("📁 {}/", offer.filename)
                                        } else {
                                            offer.filename.clone()
                                        }),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette.muted_foreground)
                                        .child(format_size(offer.size)),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted_foreground)
                                .child(format!(
                                    "{} 想发送{}给你",
                                    offer.from_name,
                                    if offer.is_dir { "文件夹" } else { "文件" }
                                )),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .justify_end()
                                .gap(px(8.0))
                                .child(
                                    Button::new(reject_id)
                                        .xsmall()
                                        .label("拒绝")
                                        .on_click(move |_, _, cx| {
                                            reject_this.update(cx, |app, cx| {
                                                app.reject_offer(&reject_offer, cx)
                                            });
                                        }),
                                )
                                .child(
                                    Button::new(accept_id)
                                        .primary()
                                        .xsmall()
                                        .label("接收")
                                        .on_click(move |_, _, cx| {
                                            accept_this.update(cx, |app, cx| {
                                                app.accept_offer(&accept_offer, cx)
                                            });
                                        }),
                                ),
                        ),
                );
            transfer_cards.push(card.into_any_element());
        }

        for (tid, t) in self.transfers.clone() {
            let dismiss_this = this.clone();
            let dismiss_id = tid.clone();
            let mut body = v_flex().w_full().gap(px(6.0)).child(
                h_flex()
                    .w_full()
                    .gap(px(6.0))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .overflow_x_hidden()
                            .text_sm()
                            .text_color(palette.foreground)
                            .child(t.filename.clone()),
                    ),
            );

            match t.stage {
                TransferStage::Downloading => {
                    let pct = if t.total > 0 {
                        (t.received as f32 / t.total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    body = body
                        .child(
                            div()
                                .w_full()
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .bg(palette.muted)
                                .child(
                                    div()
                                        .h_full()
                                        .rounded(px(3.0))
                                        .bg(palette.primary)
                                        .w(px(236.0 * pct)),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted_foreground)
                                .child(format!(
                                    "{} / {}",
                                    format_size(t.received),
                                    format_size(t.total)
                                )),
                        );
                }
                TransferStage::Complete => {
                    let reveal_path = t.path.clone();
                    body = body
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.success)
                                .child("已保存"),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .justify_end()
                                .gap(px(8.0))
                                .child(
                                    Button::new(format!("dismiss-{}", tid))
                                        .xsmall()
                                        .label("×")
                                        .on_click(move |_, _, cx| {
                                            dismiss_this.update(cx, |app, cx| {
                                                app.dismiss_transfer(&dismiss_id, cx)
                                            });
                                        }),
                                )
                                .children(reveal_path.map(|p| {
                                    Button::new(format!("open-{}", tid))
                                        .xsmall()
                                        .label("打开文件夹")
                                        .on_click(move |_, _, cx| cx.reveal_path(&p))
                                        .into_any_element()
                                })),
                        );
                }
                TransferStage::Failed => {
                    body = body
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.danger)
                                .child(t.error.clone().unwrap_or_else(|| "传输失败".to_string())),
                        )
                        .child(
                            h_flex().w_full().justify_end().child(
                                Button::new(format!("dismiss-{}", tid))
                                    .xsmall()
                                    .label("×")
                                    .on_click(move |_, _, cx| {
                                        dismiss_this.update(cx, |app, cx| {
                                            app.dismiss_transfer(&dismiss_id, cx)
                                        });
                                    }),
                            ),
                        );
                }
            }

            let card = div()
                .w_full()
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(palette.sidebar)
                .border_1()
                .border_color(palette.border)
                .child(body);
            transfer_cards.push(card.into_any_element());
        }

        if !transfer_cards.is_empty() {
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(16.0))
                    .right(px(16.0))
                    .w(px(280.0))
                    .child(v_flex().w_full().gap(px(8.0)).children(transfer_cards)),
            );
        }

        if self.settings_open {
            let save_this = this.clone();
            let cancel_this = this.clone();
            let modal = settings::render_settings_modal(
                &self.settings_nickname,
                &self.settings_download,
                &self.settings_port,
                palette,
                move |_, _, cx| {
                    save_this.update(cx, |app, cx| app.save_settings(cx));
                },
                move |_, _, cx| {
                    cancel_this.update(cx, |app, cx| app.close_settings(cx));
                },
            );
            root = root.child(modal);
        }

        root.children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

/// Map a user status to an indicator color.
pub fn status_color(status: UserStatus, online: bool, palette: Palette) -> Hsla {
    if !online {
        return palette.muted_foreground;
    }
    match status {
        UserStatus::Online => palette.success,
        UserStatus::Away => palette.warning,
        UserStatus::Busy => palette.danger,
        UserStatus::Offline => palette.muted_foreground,
    }
}

/// Ordering rank for [`MsgStatus`], so statuses only ever advance.
fn status_rank(status: MsgStatus) -> u8 {
    match status {
        MsgStatus::Sending => 0,
        MsgStatus::Sent => 1,
        MsgStatus::Delivered => 2,
        MsgStatus::Read => 3,
    }
}

/// Format a byte count as a human-readable size (e.g. `1.5 MB`).
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}
