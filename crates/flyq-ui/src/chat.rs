//! Chat panel component.

use chrono::{DateTime, Local};
use gpui::prelude::*;
use rust_i18n::t;
use gpui::{div, img, px, white, AnyElement, App, ClickEvent, Entity, FontWeight, ObjectFit, Window};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::{h_flex, v_flex, Sizable};

use crate::app::{ChatMsg, MsgStatus, Palette};

/// Render the chat panel.
///
/// When `peer_name` is `None`, an empty placeholder is shown. Otherwise the
/// panel contains a header, a scrollable message list, an optional typing
/// hint, and an input row with a send button wired to `on_send`.
pub fn render_chat_panel(
    peer_name: Option<&str>,
    messages: &[ChatMsg],
    typing: Option<&str>,
    input: &Entity<InputState>,
    palette: Palette,
    on_send: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_knock: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_send_file: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_send_folder: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_send_image: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_screenshot: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let Some(name) = peer_name else {
        return empty_chat(palette);
    };

    // ── Header ──────────────────────────────────────────────────────────
    let header = h_flex()
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .items_center()
        .border_b_1()
        .border_color(palette.border)
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::BOLD)
                .text_color(palette.foreground)
                .child(name.to_string()),
        )
        .child(div().flex_1())
        .child(
            h_flex()
                .gap(px(8.0))
                .child(
                    Button::new("send-file-btn")
                        .small()
                        .label(&t!("chat.send_file").to_string())
                        .on_click(on_send_file),
                )
                .child(
                    Button::new("send-folder-btn")
                        .small()
                        .label(&t!("chat.send_folder").to_string())
                        .on_click(on_send_folder),
                )
                .child(
                    Button::new("send-image-btn")
                        .small()
                        .label(&t!("chat.send_image").to_string())
                        .on_click(on_send_image),
                )
                .child(
                    Button::new("screenshot-btn")
                        .small()
                        .label(&t!("chat.screenshot").to_string())
                        .on_click(on_screenshot),
                )
                .child(
                    Button::new("knock-btn")
                        .small()
                        .label(&t!("chat.knock").to_string())
                        .on_click(on_knock),
                ),
        );

    // ── Message list ────────────────────────────────────────────────────
    let message_area = if messages.is_empty() {
        div()
            .flex_1()
            .w_full()
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted_foreground)
                            .child(t!("chat.empty_conversation").to_string()),
                    ),
            )
            .into_any_element()
    } else {
        let mut bubbles: Vec<AnyElement> = Vec::with_capacity(messages.len());
        for msg in messages {
            bubbles.push(message_bubble(msg, palette));
        }
        div()
            .id("message-list")
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .px(px(16.0))
            .py(px(12.0))
            .child(v_flex().w_full().gap(px(10.0)).children(bubbles))
            .into_any_element()
    };

    // ── Typing hint ─────────────────────────────────────────────────────
    let typing_hint = typing.map(|t| {
        div()
            .px(px(16.0))
            .pb(px(4.0))
            .text_xs()
            .text_color(palette.muted_foreground)
            .child(t!("chat.typing", name = t).to_string())
            .into_any_element()
    });

    // ── Input row ───────────────────────────────────────────────────────
    let input_row = h_flex()
        .w_full()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(12.0))
        .items_center()
        .border_t_1()
        .border_color(palette.border)
        .child(div().flex_1().child(Input::new(input)))
        .child(
            Button::new("send-btn")
                .primary()
                .label(&t!("chat.send").to_string())
                .on_click(on_send),
        );

    v_flex()
        .size_full()
        .child(header)
        .child(message_area)
        .children(typing_hint)
        .child(input_row)
        .into_any_element()
}

/// Render a group chat panel.
///
/// Similar to the regular chat panel but with a simplified header (no
/// knock/file/folder buttons) and a "#" prefix on the group name.
pub fn render_group_chat_panel(
    group_name: &str,
    messages: &[ChatMsg],
    input: &Entity<InputState>,
    palette: Palette,
    on_send: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    _on_send_file: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    // ── Header ──────────────────────────────────────────────────────────
    let header = h_flex()
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .items_center()
        .border_b_1()
        .border_color(palette.border)
        .child(
            div()
                .text_base()
                .font_weight(FontWeight::BOLD)
                .text_color(palette.foreground)
                .child(format!("# {}", group_name)),
        )
        .child(div().flex_1());

    // ── Message list ────────────────────────────────────────────────────
    let message_area = if messages.is_empty() {
        div()
            .flex_1()
            .w_full()
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted_foreground)
                            .child(t!("chat.group_empty").to_string()),
                    ),
            )
            .into_any_element()
    } else {
        let mut bubbles: Vec<AnyElement> = Vec::with_capacity(messages.len());
        for msg in messages {
            bubbles.push(message_bubble(msg, palette));
        }
        div()
            .id("group-message-list")
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .px(px(16.0))
            .py(px(12.0))
            .child(v_flex().w_full().gap(px(10.0)).children(bubbles))
            .into_any_element()
    };

    // ── Input row ───────────────────────────────────────────────────────
    let input_row = h_flex()
        .w_full()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(12.0))
        .items_center()
        .border_t_1()
        .border_color(palette.border)
        .child(div().flex_1().child(Input::new(input)))
        .child(
            Button::new("group-send-btn")
                .primary()
                .label(&t!("chat.send").to_string())
                .on_click(on_send),
        );

    v_flex()
        .size_full()
        .child(header)
        .child(message_area)
        .child(input_row)
        .into_any_element()
}

/// Render a single message bubble, aligned right for outgoing messages.
fn message_bubble(msg: &ChatMsg, palette: Palette) -> AnyElement {
    let (bg, fg) = if msg.outgoing {
        (palette.primary, white())
    } else {
        (palette.muted, palette.foreground)
    };

    let time = format_time(msg.timestamp);
    let meta = if msg.outgoing {
        let mark = match msg.status {
            MsgStatus::Sending => "…",
            MsgStatus::Sent => "✓",
            MsgStatus::Delivered => "✓",
            MsgStatus::Read => "✓✓",
        };
        format!("{} · {} · {}", time, t!("chat.you"), mark)
    } else {
        format!("{} · {}", time, msg.sender_name)
    };

    let mut col = v_flex().max_w(px(460.0)).gap(px(2.0));
    col = if msg.outgoing {
        col.items_end()
    } else {
        col.items_start()
    };

    let row = h_flex().w_full();
    let row = if msg.outgoing {
        row.justify_end()
    } else {
        row.justify_start()
    };

    // Render the bubble content: image or text.
    let content = if msg.media_type == 1 {
        if let Some(ref path) = msg.image_path {
            // Image thumbnail — constrained to 240px wide, auto height.
            div()
                .rounded(px(10.0))
                .overflow_hidden()
                .child(
                    img(path.clone())
                        .w(px(240.0))
                        .h(px(180.0))
                        .object_fit(ObjectFit::Cover),
                )
                .into_any_element()
        } else {
            // Image message without a local path — show placeholder.
            div()
                .px(px(12.0))
                .py(px(8.0))
                .rounded(px(10.0))
                .bg(bg)
                .text_color(fg)
                .text_sm()
                .child(t!("chat.image_placeholder").to_string())
                .into_any_element()
        }
    } else {
        div()
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(10.0))
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .child(msg.text.clone())
            .into_any_element()
    };

    row.child(
        col.child(content)
            .child(
                div()
                    .text_xs()
                    .text_color(palette.muted_foreground)
                    .child(meta),
            ),
    )
    .into_any_element()
}

/// Placeholder shown when no peer is selected.
fn empty_chat(palette: Palette) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_lg()
                .text_color(palette.muted_foreground)
                .child(t!("chat.select_peer").to_string()),
        )
        .into_any_element()
}

/// Format a unix timestamp (seconds) as local `HH:MM`.
fn format_time(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.with_timezone(&Local).format("%H:%M").to_string())
        .unwrap_or_default()
}
