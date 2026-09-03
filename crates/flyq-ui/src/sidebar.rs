//! Sidebar component showing online peers.

use gpui::prelude::*;
use gpui::{div, px, AnyElement, FontWeight};
use gpui_component::v_flex;

/// Render the sidebar showing the list of online peers.
pub fn render_sidebar() -> AnyElement {
    v_flex()
        .size_full()
        .p_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .mb_2()
                .px(px(8.0))
                .child("LanChat"),
        )
        .child(
            div()
                .text_xs()
                .px(px(8.0))
                .text_color(gpui::white())
                .opacity(0.5)
                .child("No peers discovered yet"),
        )
        .into_any_element()
}
