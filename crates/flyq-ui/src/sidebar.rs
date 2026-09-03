//! Sidebar component showing online peers.

use flyq_protocol::PeerInfo;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, App, ClickEvent, FontWeight, Window};
use gpui_component::{h_flex, v_flex};

use crate::app::{status_color, Palette};

/// Render the full sidebar: a header plus the scrollable peer list.
pub fn render_sidebar(
    local_name: &str,
    peer_count: usize,
    rows: Vec<AnyElement>,
    palette: Palette,
) -> AnyElement {
    let header = v_flex()
        .px(px(14.0))
        .py(px(12.0))
        .gap(px(2.0))
        .border_b_1()
        .border_color(palette.border)
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::BOLD)
                .text_color(palette.foreground)
                .child("LanChat"),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted_foreground)
                .child(format!("{} · {} online", local_name, peer_count)),
        );

    let list = if rows.is_empty() {
        div()
            .px(px(14.0))
            .py(px(16.0))
            .text_xs()
            .text_color(palette.muted_foreground)
            .child("Searching for peers on the LAN…")
            .into_any_element()
    } else {
        div()
            .id("peer-list")
            .size_full()
            .py(px(4.0))
            .overflow_y_scroll()
            .children(rows)
            .into_any_element()
    };

    v_flex()
        .size_full()
        .child(header)
        .child(div().flex_1().overflow_hidden().child(list))
        .into_any_element()
}

/// Render a single clickable peer row.
///
/// The `on_select` callback is attached as the row's click handler; it is built
/// by the caller (which has access to the app entity) and merely wired up here.
pub fn peer_row(
    peer: &PeerInfo,
    selected: bool,
    typing: Option<&str>,
    palette: Palette,
    on_select: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let dot_color = status_color(peer.status, peer.online, palette);
    let subtitle = if typing.is_some() {
        "typing…".to_string()
    } else {
        peer.host.clone()
    };

    let mut row = div()
        .id(peer.peer_id())
        .on_click(on_select)
        .cursor_pointer()
        .w_full()
        .px(px(12.0))
        .py(px(8.0));

    row = if selected {
        row.bg(palette.accent)
    } else {
        row.hover(|s| s.bg(palette.muted))
    };

    row.child(
        h_flex()
            .gap(px(9.0))
            .items_center()
            .child(
                div()
                    .size(px(9.0))
                    .rounded_full()
                    .flex_shrink_0()
                    .bg(dot_color),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_hidden()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.foreground)
                            .child(peer.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted_foreground)
                            .child(subtitle),
                    ),
            ),
    )
    .into_any_element()
}
