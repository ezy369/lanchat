//! Main application view.
//!
//! [`LanChatApp`] is the root GPUI entity. It owns the UI-side state (online
//! peers, per-conversation message lists, typing indicators, the selected peer
//! and the chat input), consumes [`UiEvent`]s emitted by the core event loop,
//! and dispatches outgoing messages through a shared [`EventHandler`].

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use flyq_core::{EventHandler, UiEvent};
use flyq_protocol::{PeerInfo, UserStatus};
use gpui::prelude::*;
use gpui::{div, px, AnyElement, App, AsyncApp, Context, Entity, Hsla, Window};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{h_flex, ActiveTheme, Root};
use tokio::sync::mpsc;
use tracing::warn;

use crate::chat;
use crate::sidebar;
use crate::tokio_runtime::Tokio;

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
    /// Chat message input state.
    input: Entity<InputState>,
    /// Shared handler used to send messages (persists + emits MessageSent).
    handler: Arc<EventHandler>,
    /// Tokio runtime handle for spawning send tasks from click handlers.
    rt: tokio::runtime::Handle,
}

impl LanChatApp {
    /// Create a new application view.
    ///
    /// * `local_name` — our display name.
    /// * `handler` — shared event handler for sending messages.
    /// * `ui_rx` — channel carrying [`UiEvent`]s from the core event loop.
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        local_name: String,
        handler: Arc<EventHandler>,
        mut ui_rx: mpsc::Receiver<UiEvent>,
    ) -> Self {
        let rt = Tokio::handle(cx);

        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Type a message, press Enter to send…")
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
            peers: Vec::new(),
            selected: None,
            conversations: HashMap::new(),
            typing: HashMap::new(),
            names: HashMap::new(),
            input,
            handler,
            rt,
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
            } => {
                self.names.insert(sender_addr, sender.clone());
                self.typing.remove(&sender_addr);
                self.conversations.entry(sender_addr).or_default().push(ChatMsg {
                    id,
                    text: content,
                    timestamp,
                    outgoing: false,
                    sender_name: sender,
                });
            }
            UiEvent::MessageSent {
                id,
                recipient,
                content,
                timestamp,
            } => {
                if let Ok(addr) = recipient.parse::<SocketAddr>() {
                    self.conversations.entry(addr).or_default().push(ChatMsg {
                        id,
                        text: content,
                        timestamp,
                        outgoing: true,
                        sender_name: self.local_name.clone(),
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
            UiEvent::Knock { from, name } => {
                // TODO(M4): trigger a screen-shake animation.
                tracing::info!("Screen shake from {} ({})", name, from);
            }
            UiEvent::DeliveryConfirmed { .. } | UiEvent::ReadConfirmed { .. } => {
                // TODO(M4): update per-message delivery/read status.
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
            cx.notify();
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
        let peer_count = self.peers.len();

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
        let sidebar_el = sidebar::render_sidebar(&local_name, peer_count, rows, palette);

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
                )
            }
            None => chat::render_chat_panel(None, &[], None, &self.input, palette, |_, _, _| {}),
        };

        div()
            .size_full()
            .bg(palette.background)
            .child(
                h_flex()
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
                    .child(div().flex_1().h_full().child(chat_el)),
            )
            .children(Root::render_dialog_layer(window, cx))
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
