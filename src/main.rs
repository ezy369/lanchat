//! LanChat - A modern LAN messenger built with Rust and GPUI.
//!
//! A high-quality open source alternative to FeiQ/IP Messenger.

use std::sync::Arc;

use flyq_core::{run_event_loop, EventHandler, UiEvent};
use flyq_network::{DiscoveryConfig, DiscoveryService, PeerManager};
use flyq_storage::Database;
use flyq_ui::tokio_runtime;
use flyq_ui::LanChatApp;
use gpui::prelude::*;
use gpui::{px, size, Bounds, WindowBounds, WindowOptions};
use gpui_component::Root;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

/// Relative path of the embedded libSQL database.
const DB_PATH: &str = "lanchat.db";

/// Bootstrap the storage + network stack on the Tokio runtime.
///
/// Returns the UI event receiver, the shared event handler, and our display
/// name. The discovery loop and the core event loop are spawned as detached
/// Tokio tasks that live for the whole application lifetime.
async fn bootstrap() -> (mpsc::Receiver<UiEvent>, Arc<EventHandler>, String) {
    // Storage.
    let db = Database::open(DB_PATH)
        .await
        .expect("failed to open database");
    db.migrate().await.expect("failed to migrate database");

    // Peer tracking (shared between discovery and the event handler).
    let peer_manager = PeerManager::new();

    // Discovery configuration: use the machine hostname as our display name.
    let mut config = DiscoveryConfig::default();
    config.username = config.hostname.clone();
    let local_name = config.username.clone();
    let local_id = format!("{}:{}", config.hostname.clone(), config.port);

    let discovery = DiscoveryService::new(config)
        .await
        .expect("failed to start discovery service");
    let sender = discovery.message_sender();
    let (event_rx, _discovery_task) = discovery.spawn(peer_manager.clone());

    // UI event channel + handler.
    let (ui_tx, ui_rx) = mpsc::channel::<UiEvent>(256);
    let handler = Arc::new(EventHandler::new(
        local_id,
        db,
        peer_manager,
        sender,
        ui_tx,
    ));

    // Core event loop (network events -> storage + UI events).
    tokio::spawn(run_event_loop(Arc::clone(&handler), event_rx));

    tracing::info!("Bootstrap complete; display name = {}", local_name);
    (ui_rx, handler, local_name)
}

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("lanchat=info".parse().unwrap()),
        )
        .init();

    tracing::info!("Starting LanChat...");

    gpui_platform::application()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            // MUST be first.
            gpui_component::init(cx);
            // Host the Tokio runtime used by the network/storage layers.
            tokio_runtime::init(cx);

            // Build the network + storage stack before opening the window.
            let rt = tokio_runtime::Tokio::handle(cx);
            let (ui_rx, handler, local_name) = rt.block_on(bootstrap());

            cx.spawn(async move |cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: gpui::Point::default(),
                            size: size(px(1024.0), px(680.0)),
                        })),
                        titlebar: Some(gpui::TitlebarOptions {
                            title: Some("LanChat".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    move |window, cx| {
                        let view = cx.new(|cx| {
                            LanChatApp::new(window, cx, local_name, handler, ui_rx)
                        });
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("Failed to open main window");
            })
            .detach();
        });
}
