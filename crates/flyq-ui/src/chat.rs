//! Chat panel component.

use gpui::prelude::*;
use gpui::{div, AnyElement, FontWeight};
use gpui_component::v_flex;

/// Render the chat panel for a selected peer.
pub fn render_chat_panel(peer_name: Option<&str>) -> AnyElement {
    let content = match peer_name {
        Some(name) => div()
            .text_lg()
            .font_weight(FontWeight::BOLD)
            .child(format!("Chat with {}", name)),
        None => div()
            .text_lg()
            .opacity(0.5)
            .child("Select a peer to start chatting"),
    };

    v_flex()
        .size_full()
        .p_4()
        .child(content)
        .into_any_element()
}
