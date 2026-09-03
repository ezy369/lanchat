//! GPUI-based user interface for LanChat.
//!
//! Provides the main application window with:
//! - Sidebar showing online peers
//! - Chat area for the selected conversation
//! - Message input

pub mod app;
pub mod chat;
pub mod sidebar;

pub use app::LanChatApp;
