//! Core business logic and application state.
//!
//! This crate glues together protocol, network, storage, and UI layers.
//! It owns the event processing loop that translates network events into
//! storage operations and UI notifications.

pub mod app_state;
pub mod config;
pub mod handlers;

pub use app_state::AppState;
pub use config::{AppConfig, DEFAULT_LOCALE, DEFAULT_PORT, SUPPORTED_LOCALES};
pub use handlers::{run_event_loop, EventHandler, HistoryMsg, IncomingFile, UiEvent};
pub use flyq_storage::Group;
