//! Screenshot cropping overlay.
//!
//! Provides a fullscreen pop-up window where the user can drag to select
//! a rectangular region of a captured screenshot. Enter confirms the
//! selection (crops and invokes the callback); Escape cancels.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use flyq_core::EventHandler;
use gpui::prelude::*;
use gpui::{
    div, img, px, rgba, App, Bounds, Context, InteractiveElement, KeyDownEvent,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, MouseButton, ObjectFit, Styled, Window,
    WindowBounds, WindowKind, WindowOptions,
};
use rust_i18n::t;
use tracing::warn;

/// Fullscreen overlay for cropping a screenshot.
///
/// The overlay is created as a `PopUp` window (always-on-top) that covers
/// the entire primary display. The captured screenshot is shown as the
/// background, and the user drags to select a crop region.
pub struct ScreenshotOverlay {
    /// Path to the full-screen capture PNG.
    source: PathBuf,
    /// Destination directory for cropped images.
    images_dir: PathBuf,
    /// Tokio runtime handle for spawning the send task.
    rt: tokio::runtime::Handle,
    /// Shared event handler for sending the cropped image.
    handler: Arc<EventHandler>,
    /// Target peer address.
    peer_addr: SocketAddr,
    /// Start position of the current drag selection.
    start: Option<gpui::Point<gpui::Pixels>>,
    /// End position of the current drag selection.
    end: Option<gpui::Point<gpui::Pixels>>,
    /// Whether the user is currently dragging.
    dragging: bool,
}

impl ScreenshotOverlay {
    /// Create a new overlay.
    pub fn new(
        source: PathBuf,
        images_dir: PathBuf,
        rt: tokio::runtime::Handle,
        handler: Arc<EventHandler>,
        peer_addr: SocketAddr,
    ) -> Self {
        Self {
            source,
            images_dir,
            rt,
            handler,
            peer_addr,
            start: None,
            end: None,
            dragging: false,
        }
    }

    /// Open the overlay as a topmost fullscreen window.
    ///
    /// Returns `Ok(())` if the window was created successfully.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        source: PathBuf,
        images_dir: PathBuf,
        rt: tokio::runtime::Handle,
        handler: Arc<EventHandler>,
        peer_addr: SocketAddr,
        display_bounds: Bounds<gpui::Pixels>,
        _window: &mut Window,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Fullscreen(display_bounds)),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_resizable: false,
            is_movable: false,
            ..Default::default()
        };

        cx.open_window(options, |_window, cx| {
            cx.new(|_cx| Self::new(source, images_dir, rt, handler, peer_addr))
        })?;

        Ok(())
    }

    /// Compute the normalized (x, y, w, h) selection rectangle.
    fn selection_rect(&self) -> Option<(u32, u32, u32, u32)> {
        let (s, e) = (self.start?, self.end?);
        let x1 = s.x.as_f32().min(e.x.as_f32()).max(0.0) as u32;
        let y1 = s.y.as_f32().min(e.y.as_f32()).max(0.0) as u32;
        let x2 = s.x.as_f32().max(e.x.as_f32()).max(0.0) as u32;
        let y2 = s.y.as_f32().max(e.y.as_f32()).max(0.0) as u32;
        let w = x2.saturating_sub(x1);
        let h = y2.saturating_sub(y1);
        if w < 5 || h < 5 {
            return None;
        }
        Some((x1, y1, w, h))
    }

    /// Crop the selected region and send to the peer. Closes the window.
    fn confirm(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if let Some((x, y, w, h)) = self.selection_rect() {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let dest = self.images_dir.join(format!("screenshot_crop_{}.png", ts));

            match crate::screenshot::crop_image(&self.source, x, y, w, h, &dest) {
                Ok(path) => {
                    let handler = self.handler.clone();
                    let addr = self.peer_addr;
                    self.rt.spawn(async move {
                        if let Err(e) = handler.send_image_message(addr, path).await {
                            warn!("Failed to send cropped screenshot to {}: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    warn!("Failed to crop screenshot: {}", e);
                }
            }
        }
        window.remove_window();
    }

    /// Cancel the overlay without sending. Closes the window.
    fn cancel(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }
}

impl Render for ScreenshotOverlay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let source = self.source.clone();

        // Compute selection rectangle for the highlight overlay.
        let sel = self.selection_rect();

        // Build the selection highlight rectangle (if dragging or has selection).
        let selection_el = if let Some((sx, sy, sw, sh)) = sel {
            Some(
                div()
                    .absolute()
                    .left(px(sx as f32))
                    .top(px(sy as f32))
                    .w(px(sw as f32))
                    .h(px(sh as f32))
                    .border_1()
                    .border_color(rgba(0x4488FFFF))
                    .bg(rgba(0x4488FF15)),
            )
        } else {
            None
        };

        // Hint banner at the top of the screen.
        let hint = div()
            .absolute()
            .top(px(20.0))
            .left(px(0.0))
            .right(px(0.0))
            .flex()
            .justify_center()
            .child(
                div()
                    .bg(rgba(0x000000CC))
                    .px(px(16.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .text_color(rgba(0xFFFFFFFF))
                    .text_size(px(14.0))
                    .child(t!("overlay.hint").to_string()),
            );

        // The overlay div captures all mouse and keyboard events.
        div()
            .size_full()
            .relative()
            // Background: the captured screenshot image.
            .child(
                div()
                    .size_full()
                    .child(
                        img(source)
                            .object_fit(ObjectFit::Fill)
                            .size_full(),
                    ),
            )
            // Semi-transparent dark layer.
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .bg(rgba(0x00000055)),
            )
            // Selection highlight.
            .children(selection_el)
            // Hint text.
            .child(hint)
            // Event-capturing overlay (covers the full window).
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    // Mouse down: start selection.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this: &mut Self, event: &MouseDownEvent, _window, cx| {
                            this.start = Some(event.position);
                            this.end = Some(event.position);
                            this.dragging = true;
                            cx.notify();
                        }),
                    )
                    // Mouse move: update selection while dragging.
                    .on_mouse_move(cx.listener(|this: &mut Self, event: &MouseMoveEvent, _window, cx| {
                        if this.dragging {
                            this.end = Some(event.position);
                            cx.notify();
                        }
                    }))
                    // Mouse up: finalize selection.
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this: &mut Self, event: &MouseUpEvent, _window, cx| {
                            if this.dragging {
                                this.end = Some(event.position);
                                this.dragging = false;
                                cx.notify();
                            }
                        }),
                    )
                    // Keyboard: Enter to confirm, Escape to cancel.
                    .on_key_down(cx.listener(
                        |this: &mut Self, event: &KeyDownEvent, window, cx| {
                            match event.keystroke.key.as_str() {
                                "enter" => this.confirm(window, cx),
                                "escape" => this.cancel(window, cx),
                                _ => {}
                            }
                        },
                    )),
            )
    }
}
