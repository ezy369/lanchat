//! IP Messenger protocol parser and builder.
//!
//! This crate implements the IPMsg protocol (UDP port 2425) used by
//! FeiQ, IP Messenger, and compatible LAN messaging tools.
//!
//! # Protocol Overview
//!
//! IPMsg uses UDP broadcast for presence and text messaging, and TCP
//! for file transfers. All packets follow a simple text format:
//!
//! ```text
//! version:packet_no:sender_name:sender_host:command_no[:extra]
//! ```
//!
//! # Example
//!
//! ```rust
//! use flyq_protocol::{Command, PacketBuilder};
//!
//! let packet = PacketBuilder::new()
//!     .sender("Alice", "ALICE-PC")
//!     .command(Command::NoOperation)
//!     .build();
//! ```

pub mod command;
pub mod feiq;
pub mod packet;
pub mod types;

pub use command::{Command, compose_command, extract_flags, flags};
pub use feiq::{EmojiCode, FeiqVersion, FileAttachmentInfo, FileTransferRequest, RichText};
pub use packet::{Packet, PacketBuilder, PacketParser};
pub use types::*;

/// IPMsg protocol version.
pub const IPMSG_VERSION: u32 = 0x0001;

/// Default IPMsg port.
pub const IPMSG_PORT: u16 = 2425;
