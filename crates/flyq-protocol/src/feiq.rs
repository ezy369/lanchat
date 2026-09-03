//! FeiQ protocol extensions.
//!
//! FeiQ extends the standard IPMsg protocol with:
//! - A rich version string (`1_lbt6_0#level#mac#0#0#0#4001#9`)
//! - File attachment metadata format
//! - Rich text formatting (`text{/font;...}`)
//! - Inline emoji codes (`/:)`, `/:D`, etc.)
//! - Typing indicators, screen shake, delivery/read receipts

pub mod emoji;
pub mod file_attachment;
pub mod rich_text;
pub mod version;

pub use emoji::EmojiCode;
pub use file_attachment::{FileAttachmentInfo, FileTransferRequest};
pub use rich_text::RichText;
pub use version::{FeiqVersion, ProtocolGeneration};
