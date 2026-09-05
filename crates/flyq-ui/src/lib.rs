//! GPUI-based user interface for LanChat.
//!
//! Provides the main application window with:
//! - Sidebar showing online peers
//! - Chat area for the selected conversation
//! - Message input
//!
//! The UI consumes [`flyq_core::UiEvent`] items from the core event loop and
//! renders them. Outgoing messages are dispatched through a shared
//! [`flyq_core::EventHandler`]. Tokio-backed work (network I/O, database) is
//! bridged into GPUI via the [`tokio_runtime`] module.

pub mod app;
pub mod chat;
mod notify;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub mod screenshot;
pub mod settings;
pub mod sidebar;
pub mod sound;
pub mod tokio_runtime;

rust_i18n::i18n!("locales");

pub use app::{ChatMsg, LanChatApp};
