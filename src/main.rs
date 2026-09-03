//! LanChat - A modern LAN messenger built with Rust and GPUI.
//!
//! A high-quality open source alternative to FeiQ/IP Messenger.

use gpui::prelude::*;
use gpui::{px, size, Bounds, WindowBounds, WindowOptions};
use gpui_component::Root;
use tracing_subscriber::EnvFilter;

use flyq_ui::LanChatApp;

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("lanchat=info".parse().unwrap()))
        .init();

    tracing::info!("Starting LanChat...");

    gpui_platform::application()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            // MUST be first
            gpui_component::init(cx);

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
                    |window, cx| {
                        let view = cx.new(|cx| LanChatApp::new(window, cx));
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("Failed to open main window");
            })
            .detach();
        });
}
