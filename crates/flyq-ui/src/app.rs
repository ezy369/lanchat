//! Main application view.

use gpui::prelude::*;
use gpui::{div, Context, Window};
use gpui_component::{h_flex, ActiveTheme, Root};

use crate::chat::render_chat_panel;
use crate::sidebar::render_sidebar;

/// The main LanChat application view.
pub struct LanChatApp {
    /// Currently selected peer name (if any).
    selected_peer: Option<String>,
}

impl LanChatApp {
    /// Create a new application view.
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            selected_peer: None,
        }
    }
}

impl Render for LanChatApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .size_full()
                    .child(
                        // Left sidebar: peer list
                        div()
                            .w_64()
                            .h_full()
                            .bg(cx.theme().muted)
                            .border_r_1()
                            .border_color(cx.theme().border)
                            .child(render_sidebar()),
                    )
                    .child(
                        // Right: chat area
                        div()
                            .flex_1()
                            .h_full()
                            .child(render_chat_panel(self.selected_peer.as_deref())),
                    ),
            )
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
