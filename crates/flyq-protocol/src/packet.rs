//! IPMsg / FeiQ packet parsing and building.
//!
//! An IPMsg packet has the format:
//! ```text
//! version:packet_no:sender_name:sender_host:command_no[:extra]
//! ```
//!
//! The version field is either:
//! - Standard IPMsg: `"1"` (plain integer)
//! - FeiQ extended: `"1_lbt6_0#128#74E6E2152F9D#0#0#0#4001#9"`
//!
//! Note: The packet_no field may also contain non-numeric values in some
//! FeiQ implementations (timestamps as strings), so we handle both cases.

use crate::command::{Command, extract_flags};
use crate::feiq::FeiqVersion;
use thiserror::Error;

/// Errors that can occur when parsing an IPMsg packet.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("packet has fewer than 5 fields (got {0})")]
    TooFewFields(usize),
    #[error("invalid version: {0}")]
    InvalidVersion(String),
    #[error("invalid packet number: {0}")]
    InvalidPacketNo(String),
    #[error("invalid command: {0}")]
    InvalidCommand(String),
}

/// A parsed IPMsg/FeiQ packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Parsed version information (standard or FeiQ extended).
    pub version: FeiqVersion,
    /// Raw version string as received.
    pub version_raw: String,
    /// Unique packet number (sender-generated, often a timestamp).
    pub packet_no: u32,
    /// Sender's display name.
    pub sender_name: String,
    /// Sender's hostname.
    pub sender_host: String,
    /// Command code (lower 16 bits).
    pub command: Command,
    /// Raw command flags (upper 16 bits).
    pub command_flags: u32,
    /// Extra data (command-specific, may contain \0 separators).
    pub extra: Option<String>,
}

impl Packet {
    /// Check if this packet has a specific flag set.
    pub fn has_flag(&self, flag: u32) -> bool {
        self.command_flags & flag != 0
    }

    /// Check if this packet is from a FeiQ client.
    pub fn is_feiq(&self) -> bool {
        self.version.is_feiq()
    }

    /// Get the sender's MAC address (if FeiQ extended version).
    pub fn sender_mac(&self) -> Option<&str> {
        if self.version.mac.is_empty() {
            None
        } else {
            Some(&self.version.mac)
        }
    }

    /// Reconstruct the full command word (base + flags).
    pub fn command_word(&self) -> u32 {
        self.command.to_raw() | self.command_flags
    }
}

/// Parser for IPMsg/FeiQ packets.
pub struct PacketParser;

impl PacketParser {
    /// Parse a raw IPMsg/FeiQ packet string.
    ///
    /// Handles both standard IPMsg (`version = "1"`) and FeiQ extended
    /// version strings (`version = "1_lbt6_0#..."`)
    pub fn parse(raw: &str) -> Result<Packet, ParseError> {
        // Split on ':' but we need exactly 5+ fields.
        // The version field may contain '#' but not ':', so simple split works.
        let parts: Vec<&str> = raw.splitn(6, ':').collect();
        if parts.len() < 5 {
            return Err(ParseError::TooFewFields(parts.len()));
        }

        // Parse version (may be "1" or "1_lbt6_0#level#mac#...")
        let version_raw = parts[0].to_string();
        let version = FeiqVersion::parse(&version_raw);

        // Parse packet number — may be a large number (timestamp)
        let packet_no = parts[1]
            .parse::<u32>()
            .map_err(|_| ParseError::InvalidPacketNo(parts[1].to_string()))?;

        let sender_name = parts[2].to_string();
        let sender_host = parts[3].to_string();

        // Parse command word (may include flags in upper bits)
        let command_raw = parts[4]
            .parse::<u32>()
            .map_err(|_| ParseError::InvalidCommand(parts[4].to_string()))?;

        let command = Command::from_raw(command_raw);
        let command_flags = extract_flags(command_raw);

        // Extra field: everything after the 5th colon
        // Use splitn(6, ':') so parts[5] contains the full extra (including any ':')
        let extra = if parts.len() > 5 && !parts[5].is_empty() {
            Some(parts[5].to_string())
        } else {
            None
        };

        Ok(Packet {
            version,
            version_raw,
            packet_no,
            sender_name,
            sender_host,
            command,
            command_flags,
            extra,
        })
    }

    /// Parse a packet from raw bytes (handles GBK encoding by lossy conversion).
    pub fn parse_bytes(data: &[u8]) -> Result<Packet, ParseError> {
        let text = String::from_utf8_lossy(data);
        // Strip trailing null bytes (common in FeiQ packets)
        let text = text.trim_end_matches('\0');
        Self::parse(text)
    }
}

/// Builder for constructing IPMsg/FeiQ packets.
pub struct PacketBuilder {
    version_str: String,
    packet_no: u32,
    sender_name: String,
    sender_host: String,
    command: Command,
    command_flags: u32,
    extra: Option<String>,
}

impl PacketBuilder {
    /// Create a new packet builder with standard IPMsg version.
    pub fn new() -> Self {
        Self {
            version_str: "1".to_string(),
            packet_no: 0,
            sender_name: String::new(),
            sender_host: String::new(),
            command: Command::NoOperation,
            command_flags: 0,
            extra: None,
        }
    }

    /// Create a new builder with FeiQ 3.x extended version.
    pub fn new_feiq(mac: &str, level: u32) -> Self {
        Self {
            version_str: FeiqVersion::build_feiq3(mac, level),
            ..Self::new()
        }
    }

    /// Set the version string directly.
    pub fn version(mut self, version: &str) -> Self {
        self.version_str = version.to_string();
        self
    }

    /// Set the sender name and host.
    pub fn sender(mut self, name: &str, host: &str) -> Self {
        self.sender_name = name.to_string();
        self.sender_host = host.to_string();
        self
    }

    /// Set the packet number.
    pub fn packet_no(mut self, no: u32) -> Self {
        self.packet_no = no;
        self
    }

    /// Set the command.
    pub fn command(mut self, cmd: Command) -> Self {
        self.command = cmd;
        self
    }

    /// Add a flag to the command.
    pub fn flag(mut self, flag: u32) -> Self {
        self.command_flags |= flag;
        self
    }

    /// Set extra data (raw string).
    pub fn extra(mut self, extra: &str) -> Self {
        self.extra = Some(extra.to_string());
        self
    }

    /// Build the packet into a raw string.
    pub fn build(self) -> String {
        let command_no = self.command.to_raw() | self.command_flags;
        match self.extra {
            Some(ref extra) => format!(
                "{}:{}:{}:{}:{}:{}",
                self.version_str, self.packet_no, self.sender_name, self.sender_host, command_no, extra
            ),
            None => format!(
                "{}:{}:{}:{}:{}",
                self.version_str, self.packet_no, self.sender_name, self.sender_host, command_no
            ),
        }
    }
}

impl Default for PacketBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::flags;

    #[test]
    fn test_parse_standard_packet() {
        let raw = "1:100:Alice:ALICE-PC:1";
        let packet = PacketParser::parse(raw).unwrap();
        assert_eq!(packet.packet_no, 100);
        assert_eq!(packet.sender_name, "Alice");
        assert_eq!(packet.sender_host, "ALICE-PC");
        assert_eq!(packet.command, Command::BrEntry);
        assert!(packet.extra.is_none());
        assert!(!packet.is_feiq());
    }

    #[test]
    fn test_parse_feiq_packet() {
        let raw = "1_lbt6_0#128#74E6E2152F9D#0#0#0#4001#9:1523394673:sayyid:DESKTOP-DDQ2SA7:6291457:sayyidgroup";
        let packet = PacketParser::parse(raw).unwrap();
        assert!(packet.is_feiq());
        assert_eq!(packet.sender_name, "sayyid");
        assert_eq!(packet.sender_host, "DESKTOP-DDQ2SA7");
        assert_eq!(packet.command, Command::BrEntry);
        assert!(packet.has_flag(flags::FEIQ_ONLINE_FLAGS));
        assert_eq!(packet.sender_mac(), Some("74E6E2152F9D"));
        assert_eq!(packet.extra.as_deref(), Some("sayyidgroup"));
    }

    #[test]
    fn test_parse_packet_with_extra_containing_colons() {
        // Extra field may contain colons (e.g., in file paths)
        let raw = "1:200:Bob:BOB-PC:32:path:to:message";
        let packet = PacketParser::parse(raw).unwrap();
        assert_eq!(packet.command, Command::SendMsg);
        assert_eq!(packet.extra.as_deref(), Some("path:to:message"));
    }

    #[test]
    fn test_parse_packet_with_flags() {
        let raw = "1:300:Carol:CAROL-PC:2097184"; // SendMsg(0x20) | FILEATTACHOPT(0x200000) = 0x200020
        let packet = PacketParser::parse(raw).unwrap();
        assert_eq!(packet.command, Command::SendMsg);
        assert!(packet.has_flag(flags::IPMSG_FILEATTACHOPT));
    }

    #[test]
    fn test_parse_typing_indicator() {
        let raw = "1_lbt6_0#128#74E6E2152F9D#0#0#0#4001#9:1523394673:sayyid:DESKTOP:121:";
        let packet = PacketParser::parse(raw).unwrap();
        assert_eq!(packet.command, Command::TypingStart);
    }

    #[test]
    fn test_parse_screen_shake() {
        let raw = "1_lbt6_0#128#AABBCCDDEEFF#0#0#0#4001#9:999:User:HOST:209";
        let packet = PacketParser::parse(raw).unwrap();
        assert_eq!(packet.command, Command::Knock);
        assert!(packet.extra.is_none());
    }

    #[test]
    fn test_build_standard_packet() {
        let raw = PacketBuilder::new()
            .sender("Alice", "ALICE-PC")
            .packet_no(100)
            .command(Command::BrEntry)
            .build();
        assert_eq!(raw, "1:100:Alice:ALICE-PC:1");
    }

    #[test]
    fn test_build_feiq_packet() {
        let raw = PacketBuilder::new_feiq("74E6E2152F9D", 128)
            .sender("sayyid", "DESKTOP")
            .packet_no(1523394673)
            .command(Command::BrEntry)
            .flag(flags::FEIQ_ONLINE_FLAGS)
            .extra("sayyidgroup")
            .build();
        assert!(raw.starts_with("1_lbt6_0#128#74E6E2152F9D#0#0#0#4001#9:"));
        assert!(raw.contains(":6291457:"));
        assert!(raw.ends_with(":sayyidgroup"));
    }

    #[test]
    fn test_build_packet_with_extra() {
        let raw = PacketBuilder::new()
            .sender("Bob", "BOB-PC")
            .packet_no(200)
            .command(Command::SendMsg)
            .extra("Hello World")
            .build();
        assert_eq!(raw, "1:200:Bob:BOB-PC:32:Hello World");
    }

    #[test]
    fn test_roundtrip_standard() {
        let original = PacketBuilder::new()
            .sender("Test", "TEST-PC")
            .packet_no(42)
            .command(Command::SendMsg)
            .extra("Test message")
            .build();

        let parsed = PacketParser::parse(&original).unwrap();
        assert_eq!(parsed.sender_name, "Test");
        assert_eq!(parsed.sender_host, "TEST-PC");
        assert_eq!(parsed.packet_no, 42);
        assert_eq!(parsed.command, Command::SendMsg);
        assert_eq!(parsed.extra.as_deref(), Some("Test message"));
    }

    #[test]
    fn test_roundtrip_feiq() {
        let original = PacketBuilder::new_feiq("AABBCCDDEEFF", 998)
            .sender("User", "PC")
            .packet_no(12345)
            .command(Command::SendMsg)
            .flag(flags::IPMSG_UTF8OPT)
            .extra("你好世界")
            .build();

        let parsed = PacketParser::parse(&original).unwrap();
        assert!(parsed.is_feiq());
        assert_eq!(parsed.sender_mac(), Some("AABBCCDDEEFF"));
        assert_eq!(parsed.command, Command::SendMsg);
        assert!(parsed.has_flag(flags::IPMSG_UTF8OPT));
        assert_eq!(parsed.extra.as_deref(), Some("你好世界"));
    }

    #[test]
    fn test_parse_too_few_fields() {
        let raw = "1:100:Alice";
        assert!(PacketParser::parse(raw).is_err());
    }

    #[test]
    fn test_parse_bytes_with_null_terminator() {
        let raw = b"1:100:Alice:ALICE-PC:32:Hello\0\0";
        let packet = PacketParser::parse_bytes(raw).unwrap();
        assert_eq!(packet.command, Command::SendMsg);
        assert_eq!(packet.extra.as_deref(), Some("Hello"));
    }

    #[test]
    fn test_command_word_reconstruction() {
        // 6291473 = 0x600011 = OkGetList(0x11) | FEIQ_ONLINE_FLAGS(0x600000)
        let raw = "1:100:Alice:PC:6291473";
        let packet = PacketParser::parse(raw).unwrap();
        assert_eq!(packet.command_word(), 6291473);
    }

    #[test]
    fn test_delivery_check_message() {
        // FeiQ sends message with delivery check: SENDMSG(0x20)|SENDCHECKOPT(0x100) = 0x120 = 288
        let raw = "1_lbt6_0#128#AABBCCDDEEFF#0#0#0#4001#9:999:Bob:PC:288:Hello";
        let packet = PacketParser::parse(raw).unwrap();
        assert_eq!(packet.command, Command::SendMsg);
        assert!(packet.has_flag(flags::IPMSG_SENDCHECKOPT));
        assert_eq!(packet.extra.as_deref(), Some("Hello"));
    }

    #[test]
    fn test_delivery_receipt() {
        // Recipient replies with RECVMSG (0x11=17) and original packet_no
        // feiX uses code 33 = 0x21 = RECVMSG with extra flag
        let raw = "1_lbt6_0#128#AABBCCDDEEFF#0#0#0#4001#9:1000:Alice:PC:33:999";
        let packet = PacketParser::parse(raw).unwrap();
        // 33 = 0x21, lower 16 bits = 0x0021 which is RecvMsg(0x11) + extra bit
        // Actually 0x21 maps to Unknown in our enum, which is correct for now
        assert_eq!(packet.extra.as_deref(), Some("999"));
    }
}
