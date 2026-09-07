//! Settings modal overlay.
//!
//! A compact form for editing the persisted configuration: display nickname,
//! download directory, the network port, and the UI language. The modal is
//! painted on top of the main layout while open. Field values live in
//! [`InputState`] entities owned by [`crate::app::LanChatApp`]; the
//! Save/Cancel actions are supplied by the caller so this module stays a pure
//! render function.

use flyq_core::{SUPPORTED_LOCALES, SUPPORTED_THEME_MODES};
use gpui::prelude::*;
use gpui::{div, hsla, px, AnyElement, App, ClickEvent, Entity, FontWeight, Window};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::{h_flex, v_flex};
use rust_i18n::t;
use std::sync::Arc;

use crate::app::Palette;

/// Display label for a locale identifier.
fn locale_label(id: &str) -> &'static str {
    match id {
        "zh-CN" => "中文",
        "en" => "English",
        _ => "unknown",
    }
}

/// Display label for a theme mode identifier.
fn theme_label_text(mode: &str) -> String {
    match mode {
        "system" => t!("settings.theme_system").to_string(),
        "light" => t!("settings.theme_light").to_string(),
        "dark" => t!("settings.theme_dark").to_string(),
        _ => mode.to_string(),
    }
}

/// Render the settings modal as a full-window overlay.
///
/// The returned element is absolutely positioned and covers the whole window,
/// dimming the content behind a centered card. Append it as the last child of
/// the root so it paints above everything else.
pub fn render_settings_modal(
    nickname: &Entity<InputState>,
    download_dir: &Entity<InputState>,
    port: &Entity<InputState>,
    selected_language: &str,
    sound_enabled: bool,
    selected_theme: &str,
    palette: Palette,
    on_save: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_language_change: Arc<dyn Fn(&str, &mut App) + 'static>,
    on_sound_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_theme_change: Arc<dyn Fn(&str, &mut App) + 'static>,
) -> AnyElement {
    // Build the language toggle row.
    let lang_label = div()
        .text_xs()
        .text_color(palette.muted_foreground)
        .child(t!("settings.language").to_string());

    let mut lang_buttons = h_flex().gap(px(6.0));
    for locale_id in SUPPORTED_LOCALES {
        let is_selected = *locale_id == selected_language;
        let label = locale_label(locale_id);
        let on_change = on_language_change.clone();
        let id_str: &'static str = locale_id;
        lang_buttons = lang_buttons.child(
            div()
                .id(format!("lang-{}", locale_id))
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_sm()
                .text_color(if is_selected { palette.background } else { palette.foreground })
                .bg(if is_selected { palette.primary } else { palette.muted })
                .hover(|s| if is_selected { s } else { s.bg(palette.accent) })
                .child(label.to_string())
                .on_click(move |_, _, cx| {
                    on_change(id_str, cx);
                }),
        );
    }

    // Build the theme toggle row.
    let theme_label = div()
        .text_xs()
        .text_color(palette.muted_foreground)
        .child(t!("settings.theme").to_string());

    let mut theme_buttons = h_flex().gap(px(6.0));
    for mode_id in SUPPORTED_THEME_MODES {
        let is_selected = *mode_id == selected_theme;
        let label = theme_label_text(mode_id);
        let on_change = on_theme_change.clone();
        let id_str: &'static str = mode_id;
        theme_buttons = theme_buttons.child(
            div()
                .id(format!("theme-{}", mode_id))
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_sm()
                .text_color(if is_selected { palette.background } else { palette.foreground })
                .bg(if is_selected { palette.primary } else { palette.muted })
                .hover(|s| if is_selected { s } else { s.bg(palette.accent) })
                .child(label.to_string())
                .on_click(move |_, _, cx| {
                    on_change(id_str, cx);
                }),
        );
    }

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
                .child(t!("settings.title").to_string()),
        )
        .child(field(&t!("settings.nickname"), nickname, palette))
        .child(field(&t!("settings.download_dir"), download_dir, palette))
        .child(field(&t!("settings.port"), port, palette))
        .child(
            v_flex()
                .w_full()
                .gap(px(4.0))
                .child(lang_label)
                .child(lang_buttons),
        )
        .child(
            v_flex()
                .w_full()
                .gap(px(4.0))
                .child(theme_label)
                .child(theme_buttons),
        )
        .child(
            h_flex()
                .w_full()
                .gap(px(8.0))
                .items_center()
                .child(
                    div()
                        .id("sound-toggle")
                        .w(px(18.0))
                        .h(px(18.0))
                        .rounded(px(4.0))
                        .border_1()
                        .border_color(palette.border)
                        .bg(if sound_enabled { palette.primary } else { palette.background })
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.background)
                                .child(if sound_enabled { "✓" } else { "" }),
                        )
                        .on_click(on_sound_toggle),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.foreground)
                        .child(t!("settings.sound_enabled").to_string()),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted_foreground)
                .child(t!("settings.help_text").to_string()),
        )
        .child(
            h_flex()
                .w_full()
                .justify_end()
                .gap(px(8.0))
                .child(
                    Button::new("settings-cancel-btn")
                        .label(&t!("settings.cancel").to_string())
                        .on_click(on_cancel),
                )
                .child(
                    Button::new("settings-save-btn")
                        .primary()
                        .label(&t!("settings.save").to_string())
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
