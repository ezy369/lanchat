//! Settings modal overlay.
//!
//! A compact form for editing the persisted configuration: display nickname,
//! download directory, and the network port. The modal is painted on top of the
//! main layout while open. Field values live in [`InputState`] entities owned by
//! [`crate::app::LanChatApp`]; the Save/Cancel actions are supplied by the
//! caller so this module stays a pure render function.

use gpui::prelude::*;
use gpui::{div, hsla, px, AnyElement, App, ClickEvent, Entity, FontWeight, Window};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::{h_flex, v_flex};

use crate::app::Palette;

/// Render the settings modal as a full-window overlay.
///
/// The returned element is absolutely positioned and covers the whole window,
/// dimming the content behind a centered card. Append it as the last child of
/// the root so it paints above everything else.
pub fn render_settings_modal(
    nickname: &Entity<InputState>,
    download_dir: &Entity<InputState>,
    port: &Entity<InputState>,
    palette: Palette,
    on_save: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let card = v_flex()
        .w(px(440.0))
        .max_h(px(560.0))
        .p(px(20.0))
        .gap(px(14.0))
        .rounded(px(12.0))
        .bg(palette.background)
        .border_1()
        .border_color(palette.border)
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .text_color(palette.foreground)
                .child("设置"),
        )
        .child(field("昵称", nickname, palette))
        .child(field("下载目录", download_dir, palette))
        .child(field("端口", port, palette))
        .child(
            div()
                .text_xs()
                .text_color(palette.muted_foreground)
                .child("下载目录立即生效；昵称与端口将在重启 LanChat 后对网络生效。"),
        )
        .child(
            h_flex()
                .w_full()
                .justify_end()
                .gap(px(8.0))
                .child(
                    Button::new("settings-cancel-btn")
                        .label("取消")
                        .on_click(on_cancel),
                )
                .child(
                    Button::new("settings-save-btn")
                        .primary()
                        .label("保存")
                        .on_click(on_save),
                ),
        );

    div()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(0.0, 0.0, 0.0, 0.45))
        .child(card)
        .into_any_element()
}

/// A single labeled form field (label above a text input).
fn field(label: &str, input: &Entity<InputState>, palette: Palette) -> AnyElement {
    v_flex()
        .w_full()
        .gap(px(4.0))
        .child(
            div()
                .text_xs()
                .text_color(palette.muted_foreground)
                .child(label.to_string()),
        )
        .child(Input::new(input))
        .into_any_element()
}
