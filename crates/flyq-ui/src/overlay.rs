//! Screenshot cropping overlay.
//!
//! Provides a fullscreen pop-up window where the user can drag to select
//! a rectangular region of a captured screenshot. Enter confirms the
//! selection (crops and invokes the callback); Escape cancels.
//!
//! After an initial selection is made, the user can:
//! - Drag edges or corners to resize the selection
//! - Drag the selection body to reposition it
//! - Right-click or press Escape to cancel

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use flyq_core::EventHandler;
use gpui::prelude::*;
use gpui::{
    div, img, px, rgba, App, Bounds, Context, InteractiveElement, KeyDownEvent, MouseDownEvent,
    MouseButton, MouseMoveEvent, MouseUpEvent, ObjectFit, Styled, Window, WindowBounds,
    WindowKind, WindowOptions,
};
use rust_i18n::t;
use tracing::warn;

/// Pixels within this distance of an edge/corner count as "on the handle".
const EDGE_THRESHOLD: f32 = 8.0;

/// Visual size of corner handle squares.
const CORNER_SIZE: f32 = 10.0;

/// What the user is currently dragging.
#[derive(Clone, Copy, PartialEq, Debug)]
enum DragMode {
    /// Creating a new selection from scratch.
    NewSelection,
    /// Moving the entire selection.
    Move { dx: f32, dy: f32 },
    /// Resizing from an edge or corner.
    /// `fixed` is the anchor point that stays put; `horizontal` / `vertical`
    /// indicate which axes the drag updates.
    Resize {
        fixed_x: f32,
        fixed_y: f32,
        horizontal: bool,
        vertical: bool,
    },
}

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
    /// Current drag mode.
    drag_mode: DragMode,
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
            drag_mode: DragMode::NewSelection,
        }
    }

    /// Open the overlay as a topmost fullscreen window.
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

    /// Float version of selection rect for hit-testing.
    fn selection_rect_f32(&self) -> Option<(f32, f32, f32, f32)> {
        let (s, e) = (self.start?, self.end?);
        let x1 = s.x.as_f32().min(e.x.as_f32()).max(0.0);
        let y1 = s.y.as_f32().min(e.y.as_f32()).max(0.0);
        let x2 = s.x.as_f32().max(e.x.as_f32()).max(0.0);
        let y2 = s.y.as_f32().max(e.y.as_f32()).max(0.0);
        let w = x2 - x1;
        let h = y2 - y1;
        if w < 5.0 || h < 5.0 {
            return None;
        }
        Some((x1, y1, w, h))
    }

    /// Detect what edge/corner/region the mouse is hovering over.
    fn hit_test(&self, mx: f32, my: f32) -> Option<DragMode> {
        let (sx, sy, sw, sh) = self.selection_rect_f32()?;
        let (ex, ey) = (sx + sw, sy + sh);
        let t = EDGE_THRESHOLD;

        // Near left edge?
        let near_left = (mx - sx).abs() < t;
        // Near right edge?
        let near_right = (mx - ex).abs() < t;
        // Near top edge?
        let near_top = (my - sy).abs() < t;
        // Near bottom edge?
        let near_bottom = (my - ey).abs() < t;
        // Within horizontal bounds?
        let in_x = mx >= sx - t && mx <= ex + t;
        // Within vertical bounds?
        let in_y = my >= sy - t && my <= ey + t;

        // Corners (check first, they take priority).
        if near_left && near_top {
            return Some(DragMode::Resize {
                fixed_x: ex,
                fixed_y: ey,
                horizontal: true,
                vertical: true,
            });
        }
        if near_right && near_top {
            return Some(DragMode::Resize {
                fixed_x: sx,
                fixed_y: ey,
                horizontal: true,
                vertical: true,
            });
        }
        if near_left && near_bottom {
            return Some(DragMode::Resize {
                fixed_x: ex,
                fixed_y: sy,
                horizontal: true,
                vertical: true,
            });
        }
        if near_right && near_bottom {
            return Some(DragMode::Resize {
                fixed_x: sx,
                fixed_y: sy,
                horizontal: true,
                vertical: true,
            });
        }

        // Edges.
        if near_top && in_x {
            return Some(DragMode::Resize {
                fixed_x: 0.0,
                fixed_y: ey,
                horizontal: false,
                vertical: true,
            });
        }
        if near_bottom && in_x {
            return Some(DragMode::Resize {
                fixed_x: 0.0,
                fixed_y: sy,
                horizontal: false,
                vertical: true,
            });
        }
        if near_left && in_y {
            return Some(DragMode::Resize {
                fixed_x: ex,
                fixed_y: 0.0,
                horizontal: true,
                vertical: false,
            });
        }
        if near_right && in_y {
            return Some(DragMode::Resize {
                fixed_x: sx,
                fixed_y: 0.0,
                horizontal: true,
                vertical: false,
            });
        }

        // Inside selection body → move.
        if mx >= sx && mx <= ex && my >= sy && my <= ey {
            return Some(DragMode::Move {
                dx: mx - sx,
                dy: my - sy,
            });
        }

        None
    }

    /// Apply a drag position update based on the current drag mode.
    fn apply_drag(&mut self, pos: gpui::Point<gpui::Pixels>) {
        let px = pos.x.as_f32().max(0.0);
        let py = pos.y.as_f32().max(0.0);

        match self.drag_mode {
            DragMode::NewSelection => {
                self.end = Some(pos);
            }
            DragMode::Move { dx, dy } => {
                if let Some((_, _, w, h)) = self.selection_rect_f32() {
                    let new_sx = (px - dx).max(0.0);
                    let new_sy = (py - dy).max(0.0);
                    self.start = Some(gpui::Point {
                        x: gpui::px(new_sx),
                        y: gpui::px(new_sy),
                    });
                    self.end = Some(gpui::Point {
                        x: gpui::px(new_sx + w),
                        y: gpui::px(new_sy + h),
                    });
                }
            }
            DragMode::Resize {
                fixed_x,
                fixed_y,
                horizontal,
                vertical,
            } => {
                let (mut s_x, mut s_y, mut e_x, mut e_y) = {
                    let s = self.start.unwrap_or(gpui::Point {
                        x: gpui::px(0.0),
                        y: gpui::px(0.0),
                    });
                    let e = self.end.unwrap_or(gpui::Point {
                        x: gpui::px(0.0),
                        y: gpui::px(0.0),
                    });
                    (s.x.as_f32(), s.y.as_f32(), e.x.as_f32(), e.y.as_f32())
                };

                if horizontal {
                    // Determine which x is closer to the drag position
                    // and update it; the other stays at fixed_x.
                    if (s_x - fixed_x).abs() > (e_x - fixed_x).abs() {
                        s_x = px;
                    } else {
                        e_x = px;
                    }
                }
                if vertical {
                    if (s_y - fixed_y).abs() > (e_y - fixed_y).abs() {
                        s_y = py;
                    } else {
                        e_y = py;
                    }
                }

                self.start = Some(gpui::Point {
                    x: gpui::px(s_x.max(0.0)),
                    y: gpui::px(s_y.max(0.0)),
                });
                self.end = Some(gpui::Point {
                    x: gpui::px(e_x.max(0.0)),
                    y: gpui::px(e_y.max(0.0)),
                });
            }
        }
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
        let has_selection = sel.is_some();

        // ── Selection highlight rectangle ──
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

        // ── Corner handles (8px squares at each corner) ──
        let corner_handles: Vec<_> = if let Some((sx, sy, sw, sh)) = sel {
            let half = CORNER_SIZE / 2.0;
            let corners = [
                (sx as f32 - half, sy as f32 - half),
                (sx as f32 + sw as f32 - half, sy as f32 - half),
                (sx as f32 - half, sy as f32 + sh as f32 - half),
                (sx as f32 + sw as f32 - half, sy as f32 + sh as f32 - half),
            ];
            corners
                .into_iter()
                .map(|(cx, cy)| {
                    div()
                        .absolute()
                        .left(px(cx))
                        .top(px(cy))
                        .w(px(CORNER_SIZE))
                        .h(px(CORNER_SIZE))
                        .bg(rgba(0xFFFFFFFF))
                        .border_1()
                        .border_color(rgba(0x4488FFFF))
                })
                .collect()
        } else {
            Vec::new()
        };

        // ── Edge midpoint handles ──
        let edge_handles: Vec<_> = if let Some((sx, sy, sw, sh)) = sel {
            let half = CORNER_SIZE / 2.0;
            let mid_x = sx as f32 + sw as f32 / 2.0 - half;
            let mid_y = sy as f32 + sh as f32 / 2.0 - half;
            let edges = [
                (mid_x, sy as f32 - half),                    // top
                (mid_x, sy as f32 + sh as f32 - half),        // bottom
                (sx as f32 - half, mid_y),                     // left
                (sx as f32 + sw as f32 - half, mid_y),        // right
            ];
            edges
                .into_iter()
                .map(|(ex, ey)| {
                    div()
                        .absolute()
                        .left(px(ex))
                        .top(px(ey))
                        .w(px(CORNER_SIZE))
                        .h(px(CORNER_SIZE))
                        .bg(rgba(0xFFFFFFFF))
                        .border_1()
                        .border_color(rgba(0x4488FFFF))
                })
                .collect()
        } else {
            Vec::new()
        };

        // ── Hint banner at the top ──
        let hint_text = if has_selection {
            t!("overlay.hint_adjust").to_string()
        } else {
            t!("overlay.hint").to_string()
        };
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
                    .child(hint_text),
            );

        // ── Size indicator below the selection ──
        let size_label = if let Some((sx, sy, sw, sh)) = sel {
            Some(
                div()
                    .absolute()
                    .left(px(sx as f32))
                    .top(px(sy as f32 + sh as f32 + 6.0))
                    .bg(rgba(0x000000CC))
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .text_color(rgba(0xFFFFFFFF))
                    .text_size(px(12.0))
                    .child(format!("{} × {}", sw, sh)),
            )
        } else {
            None
        };

        // ── Main layout ──
        div()
            .size_full()
            .relative()
            // Background: the captured screenshot image.
            .child(
                div().size_full().child(
                    img(source).object_fit(ObjectFit::Fill).size_full(),
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
            // Selection highlight (visual only).
            .children(selection_el)
            // Corner handles (visual only).
            .children(corner_handles)
            // Edge midpoint handles (visual only).
            .children(edge_handles)
            // Size label.
            .children(size_label)
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
                    // Left mouse down: start drag (resize/move/new selection).
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this: &mut Self, event: &MouseDownEvent, _window, cx| {
                            let mode = this
                                .hit_test(event.position.x.as_f32(), event.position.y.as_f32())
                                .unwrap_or(DragMode::NewSelection);

                            if mode == DragMode::NewSelection {
                                this.start = Some(event.position);
                                this.end = Some(event.position);
                            }

                            this.drag_mode = mode;
                            this.dragging = true;
                            cx.notify();
                        }),
                    )
                    // Right mouse down: cancel.
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this: &mut Self, _event: &MouseDownEvent, window, cx| {
                            this.cancel(window, cx);
                        }),
                    )
                    // Mouse move: update drag or show hover feedback.
                    .on_mouse_move(
                        cx.listener(|this: &mut Self, event: &MouseMoveEvent, _window, cx| {
                            if this.dragging {
                                this.apply_drag(event.position);
                                cx.notify();
                            }
                        }),
                    )
                    // Mouse up: finalize drag.
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this: &mut Self, event: &MouseUpEvent, _window, cx| {
                            if this.dragging {
                                if this.drag_mode == DragMode::NewSelection {
                                    this.end = Some(event.position);
                                }
                                this.dragging = false;
                                this.drag_mode = DragMode::NewSelection;
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
