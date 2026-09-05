//! LanChat - A modern LAN messenger built with Rust and GPUI.
//!
//! A high-quality open source alternative to FeiQ/IP Messenger.

use std::sync::Arc;

use flyq_core::{AppConfig, EventHandler, UiEvent, run_event_loop};
use flyq_network::{DiscoveryConfig, DiscoveryService, FileRegistry, PeerManager, Transport};
use flyq_storage::Database;
use flyq_ui::LanChatApp;
use flyq_ui::tokio_runtime;
use gpui::prelude::*;
use gpui::{Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::Root;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

rust_i18n::i18n!("locales");

// The system tray is only wired on Windows and macOS, mirroring the
// target-gated `tray-icon` dependency in Cargo.toml (the Linux backend pulls
// GTK/libappindicator system libraries that GPUI apps don't otherwise need).
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod tray;

/// Relative path of the embedded libSQL database.
const DB_PATH: &str = "lanchat.db";

/// Bootstrap the storage + network stack on the Tokio runtime.
///
/// Returns the UI event receiver, the shared event handler, our display name,
/// and the loaded configuration (so the UI can prefill the settings panel). The
/// discovery loop and the core event loop are spawned as detached Tokio tasks
/// that live for the whole application lifetime.
async fn bootstrap() -> (
    mpsc::Receiver<UiEvent>,
    Arc<EventHandler>,
    String,
    AppConfig,
) {
    // Load persisted user settings (nickname / download dir / port).
    let app_config = AppConfig::load();

    // Storage.
    let db = Database::open(DB_PATH)
        .await
        .expect("failed to open database");
    db.migrate().await.expect("failed to migrate database");

    // Peer tracking (shared between discovery and the event handler).
    let peer_manager = PeerManager::new();

    // Discovery configuration: apply the persisted nickname and port. The
    // hostname always comes from the machine; the nickname defaults to it and
    // only overrides when the user set a non-blank value.
    let mut config = DiscoveryConfig::default();
    if !app_config.nickname.trim().is_empty() {
        config.username = app_config.nickname.clone();
    }
    config.port = app_config.port;
    config.initial_status = app_config.status;
    let local_name = config.username.clone();
    let local_host = config.hostname.clone();
    let port = config.port;
    let local_id = format!("{}:{}", config.hostname.clone(), config.port);

    let discovery = DiscoveryService::new(config)
        .await
        .expect("failed to start discovery service");
    let sender = discovery.message_sender();
    let (event_rx, _discovery_task) = discovery.spawn(peer_manager.clone());

    // File transfer: shared registry + TCP server on the IPMsg port.
    let registry = FileRegistry::new();
    let tcp_addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    match Transport::bind(tcp_addr, registry.clone()).await {
        Ok(transport) => {
            // Leak the shutdown sender on purpose: dropping it would make
            // serve()'s `shutdown.changed()` resolve immediately and spin.
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            std::mem::forget(shutdown_tx);
            tokio::spawn(async move {
                if let Err(e) = transport.serve(shutdown_rx).await {
                    tracing::error!("File transfer server stopped: {}", e);
                }
            });
        }
        Err(e) => {
            tracing::error!(
                "Failed to bind TCP file transfer server on {}: {}",
                tcp_addr,
                e
            );
        }
    }

    // Download directory for incoming files (from config).
    let download_dir = app_config.download_dir.clone();
    if let Err(e) = std::fs::create_dir_all(&download_dir) {
        tracing::warn!("Failed to create download dir {:?}: {}", download_dir, e);
    }

    // UI event channel + handler.
    let (ui_tx, ui_rx) = mpsc::channel::<UiEvent>(256);
    let handler = Arc::new(EventHandler::new(
        local_id,
        db,
        peer_manager,
        sender,
        ui_tx,
        registry,
        download_dir,
        local_name.clone(),
        local_host,
    ));

    // Core event loop (network events -> storage + UI events).
    tokio::spawn(run_event_loop(Arc::clone(&handler), event_rx));

    tracing::info!(
        "Bootstrap complete; display name = {}, port = {}",
        local_name,
        port
    );
    (ui_rx, handler, local_name, app_config)
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
            let (ui_rx, handler, local_name, app_config) = rt.block_on(bootstrap());

            // Apply persisted locale before any UI renders.
            rust_i18n::set_locale(&app_config.language);

            cx.spawn(async move |cx| {
                let window = cx
                    .open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(Bounds {
                                origin: gpui::Point::new(px(40.0), px(40.0)),
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
                                LanChatApp::new(window, cx, local_name, app_config, handler, ui_rx)
                            });
                            cx.new(|cx| Root::new(view, window, cx))
                        },
                    )
                    .expect("Failed to open main window");

                // Add the system tray icon on platforms that support it. This
                // runs on the foreground (main) thread, which tray-icon requires
                // for both creation and message pumping.
                #[cfg(any(target_os = "windows", target_os = "macos"))]
                tray::run_tray(cx, window);
                #[cfg(not(any(target_os = "windows", target_os = "macos")))]
                let _ = window;
            })
            .detach();
        });
}
