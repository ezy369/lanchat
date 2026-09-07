//! Global hotkey integration (Windows + macOS only).
//!
//! Registers OS-level global hotkeys using the `global-hotkey` crate and polls
//! for activation events on a timer — the same architecture as [`crate::tray`].
//!
//! Currently registered hotkeys:
//! - **Ctrl+Shift+A** — trigger screenshot crop overlay (requires a peer to be
//!   selected in the main window).

use std::time::Duration;

use flyq_ui::LanChatApp;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui::{AsyncApp, Entity, WindowHandle};
use gpui_component::Root;

/// How often to drain the hotkey event channel.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Register global hotkeys and spawn a foreground polling task.
///
/// Must be called on the main thread (i.e. from within a GPUI foreground task),
/// because `GlobalHotKeyManager::new()` requires an active OS event loop on the
/// calling thread — which GPUI's foreground executor provides.
pub fn run_hotkeys(cx: &mut AsyncApp, window: WindowHandle<Root>, app: Entity<LanChatApp>) {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to create global hotkey manager: {}", e);
            return;
        }
    };

    // Ctrl+Shift+A — screenshot crop overlay.
    let screenshot_hotkey = HotKey::new(
        Some(Modifiers::SHIFT | Modifiers::CONTROL),
        Code::KeyA,
    );

    if let Err(e) = manager.register(screenshot_hotkey) {
        tracing::warn!("Failed to register screenshot hotkey (Ctrl+Shift+A): {}", e);
        return;
    }

    let screenshot_id = screenshot_hotkey.id();
    tracing::info!("Global hotkey registered: Ctrl+Shift+A (screenshot)");

    cx.spawn(async move |cx| {
        // Own the manager for the loop's lifetime; dropping it unregisters
        // the hotkeys.
        let _manager = manager;
        loop {
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.state() == HotKeyState::Pressed && event.id() == screenshot_id {
                    let _ = window.update(cx, |_root, window, cx| {
                        app.update(cx, |app, cx| {
                            app.trigger_screenshot(window, cx);
                        });
                    });
                }
            }
            cx.background_executor().timer(POLL_INTERVAL).await;
        }
    })
    .detach();
}
