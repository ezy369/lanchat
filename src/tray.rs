//! System tray integration (Windows + macOS only).
//!
//! GPUI has no first-class tray support, so we drive the OS tray directly with
//! the `tray-icon` crate. Two constraints shape this module:
//!
//! 1. The tray icon must be created on, and its OS messages pumped by, the main
//!    thread. GPUI's foreground executor *is* the main thread and runs inside
//!    GPUI's native message loop, so building the tray from a foreground task
//!    satisfies this.
//! 2. `tray-icon` delivers events through process-global crossbeam channels
//!    ([`MenuEvent::receiver`] / [`TrayIconEvent::receiver`]) rather than
//!    callbacks. We therefore poll them on a short timer loop instead of
//!    blocking, which keeps GPUI's event loop responsive.
//!
//! The tray offers two actions: show the main window, and quit. Left-clicking
//! the icon also shows the window; the menu is the right-click affordance,
//! matching platform conventions.

use std::time::Duration;

use gpui::{AsyncApp, WindowHandle};
use gpui_component::Root;
use rust_i18n::t;
use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

/// Menu-item id for "show the main window".
const SHOW_ID: &str = "lanchat.tray.show";
/// Menu-item id for "quit the application".
const QUIT_ID: &str = "lanchat.tray.quit";

/// How often to drain the tray event channels. Short enough to feel instant,
/// long enough to be negligible on the main thread.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Build a small RGBA icon programmatically (no binary asset is shipped yet): a
/// filled, anti-aliased rounded square in the LanChat accent colour on a
/// transparent background.
fn build_icon() -> Icon {
    const SIZE: u32 = 32;
    let (r, g, b): (u8, u8, u8) = (46, 139, 139); // teal accent
    let inset = 1.0f32;
    let radius = 7.0f32;
    let max_center = SIZE as f32 - inset - radius;
    let min_center = inset + radius;

    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            // Distance from the point to the rounded-rectangle corner centres.
            let dx = (min_center - px).max(px - max_center).max(0.0);
            let dy = (min_center - py).max(py - max_center).max(0.0);
            let dist = (dx * dx + dy * dy).sqrt() - radius;
            // 1x anti-aliasing band around the edge.
            let coverage = (0.5 - dist).clamp(0.0, 1.0);
            let i = ((y * SIZE + x) * 4) as usize;
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = (coverage * 255.0) as u8;
        }
    }

    Icon::from_rgba(rgba, SIZE, SIZE).expect("generated icon is valid RGBA")
}

/// Bring the main window to the foreground.
///
/// A closed window is not an error worth surfacing (the user may have closed it
/// deliberately), so failures are logged at debug level only.
fn activate_window(window: &WindowHandle<Root>, cx: &mut AsyncApp) {
    if let Err(e) = window.update(cx, |_, win, _| win.activate_window()) {
        tracing::debug!("Could not activate main window: {}", e);
    }
}

/// Create the tray icon and spawn the foreground task that keeps it alive and
/// routes its events back into GPUI.
///
/// Must be called on the main thread (i.e. from within a GPUI foreground task).
/// The spawned task owns the [`tray_icon::TrayIcon`] for the application's
/// lifetime; the OS removes the icon when that value is dropped, so the task
/// intentionally never returns except on quit.
pub fn run_tray(cx: &mut AsyncApp, window: WindowHandle<Root>) {
    let menu = Menu::new();
    let show_item = MenuItem::with_id(SHOW_ID, t!("tray.show_window").to_string(), true, None);
    let quit_item = MenuItem::with_id(QUIT_ID, t!("tray.quit").to_string(), true, None);
    let separator = PredefinedMenuItem::separator();
    let items: [&dyn IsMenuItem; 3] = [&show_item, &separator, &quit_item];
    if let Err(e) = menu.append_items(&items) {
        tracing::warn!("Failed to build tray menu: {}; continuing without tray", e);
        return;
    }

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("LanChat")
        .with_icon(build_icon())
        // Left click shows the window (handled below); keep the menu for the
        // right click, matching platform conventions.
        .with_menu_on_left_click(false)
        .build();

    let tray = match tray {
        Ok(tray) => tray,
        Err(e) => {
            tracing::warn!("Failed to create system tray icon: {}; continuing without tray", e);
            return;
        }
    };

    tracing::info!("System tray initialized");

    cx.spawn(async move |cx| {
        // Own the icon for the whole loop; dropping it would remove the tray.
        let _tray = tray;
        loop {
            // Menu activations: "显示主窗口" / "退出".
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let id = event.id().clone();
                if id == SHOW_ID {
                    activate_window(&window, cx);
                } else if id == QUIT_ID {
                    cx.update(|app| app.quit());
                    return;
                }
            }

            // A left-click on the icon itself also shows the window.
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    activate_window(&window, cx);
                }
            }

            cx.background_executor().timer(POLL_INTERVAL).await;
        }
    })
    .detach();
}
