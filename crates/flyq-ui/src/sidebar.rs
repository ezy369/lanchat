//! Sidebar component showing online peers.

use flyq_protocol::{PeerInfo, UserStatus};
use gpui::prelude::*;
use rust_i18n::t;
use gpui::{div, px, white, AnyElement, App, ClickEvent, FontWeight, Window};
use gpui_component::button::Button;
use gpui_component::{h_flex, v_flex, Sizable};

use crate::app::{status_color, Palette};

/// Map UserStatus to a translated label.
fn status_label(status: UserStatus) -> String {
    match status {
        UserStatus::Online => t!("status.online").to_string(),
        UserStatus::Away => t!("status.away").to_string(),
        UserStatus::Busy => t!("status.busy").to_string(),
        UserStatus::Offline => t!("status.offline").to_string(),
    }
}

/// Render the full sidebar: a header plus the scrollable peer list.
///
/// The header shows our own presence status as a clickable pill; clicking it
/// invokes `on_cycle_status` to advance 在线 → 离开 → 忙碌.
pub fn render_sidebar(
    local_name: &str,
    local_status: UserStatus,
    peer_count: usize,
    rows: Vec<AnyElement>,
    group_rows: Vec<AnyElement>,
    palette: Palette,
    on_open_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_cycle_status: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let title_block = v_flex()
        .flex_1()
        .overflow_hidden()
        .gap(px(2.0))
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::BOLD)
                .text_color(palette.foreground)
                .child(t!("sidebar.title").to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted_foreground)
                .child(t!("sidebar.online_count", name = local_name, count = peer_count).to_string()),
        );

    let top_row = h_flex()
        .w_full()
        .gap(px(8.0))
        .items_center()
        .child(title_block)
        .child(
            Button::new("settings-btn")
                .xsmall()
                .label(&t!("sidebar.settings").to_string())
                .on_click(on_open_settings),
        );

    let dot_color = status_color(local_status, true, palette);
    let status_pill = div()
        .id("status-pill")
        .on_click(on_cycle_status)
        .cursor_pointer()
        .px(px(9.0))
        .py(px(4.0))
        .rounded(px(11.0))
        .bg(palette.muted)
        .hover(|s| s.bg(palette.accent))
        .child(
            h_flex()
                .gap(px(6.0))
                .items_center()
                .child(
                    div()
                        .size(px(8.0))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(dot_color),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.foreground)
                        .child(t!("sidebar.my_status", status = status_label(local_status)).to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted_foreground)
                        .child(t!("sidebar.switch").to_string()),
                ),
        );

    let header = v_flex()
        .w_full()
        .px(px(14.0))
        .py(px(12.0))
        .gap(px(9.0))
        .border_b_1()
        .border_color(palette.border)
        .child(top_row)
        .child(status_pill);

    let has_groups = !group_rows.is_empty();
    let has_peers = !rows.is_empty();

    let list = if !has_groups && !has_peers {
        div()
            .px(px(14.0))
            .py(px(16.0))
            .text_xs()
            .text_color(palette.muted_foreground)
            .child(t!("sidebar.searching").to_string())
            .into_any_element()
    } else {
        let mut items: Vec<AnyElement> = Vec::new();

        // Groups section.
        if has_groups {
            items.push(
                div()
                    .px(px(14.0))
                    .pt(px(8.0))
                    .pb(px(2.0))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(palette.muted_foreground)
                    .child(t!("sidebar.groups").to_string())
                    .into_any_element(),
            );
            items.extend(group_rows);
        }

        // Peers section.
        if has_peers {
            items.push(
                div()
                    .px(px(14.0))
                    .pt(px(8.0))
                    .pb(px(2.0))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(palette.muted_foreground)
                    .child(t!("sidebar.contacts").to_string())
                    .into_any_element(),
            );
            items.extend(rows);
        }

        div()
            .id("peer-list")
            .size_full()
            .py(px(4.0))
            .overflow_y_scroll()
            .children(items)
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
        t!("sidebar.typing").to_string()
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

/// Render a single clickable group row.
///
/// Groups are displayed with a "#" badge instead of a status dot and show
/// the member count as a subtitle.
pub fn group_row(
    group_id: &str,
    group_name: &str,
    member_count: usize,
    selected: bool,
    palette: Palette,
    on_select: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut row = div()
        .id(format!("group-{}", group_id))
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
                    .size(px(24.0))
                    .rounded(px(6.0))
                    .flex_shrink_0()
                    .bg(palette.primary)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(white())
                            .child("#"),
                    ),
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
                            .child(group_name.to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted_foreground)
                            .child(t!("sidebar.members", count = member_count).to_string()),
                    ),
            ),
    )
    .into_any_element()
}
