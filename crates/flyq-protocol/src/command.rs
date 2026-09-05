//! IPMsg and FeiQ command codes.
//!
//! The 32-bit command word layout:
//! - Bits 0-7 (0x00-0xFF): base command code
//! - Bits 8+ (0x100+): option flags
//!
//! Standard IPMsg uses commands 0x00-0x62.
//! FeiQ extends this with additional commands up to 0xD1.

use serde::{Deserialize, Serialize};

/// IPMsg and FeiQ command codes (base command, lower 8 bits only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Command {
    // ─── Standard IPMsg commands ───────────────────────────────────────────

    /// No-operation (used for presence broadcast / heartbeat).
    NoOperation = 0x00,
    /// User came online (broadcast entry).
    BrEntry = 0x01,
    /// User went offline (broadcast exit).
    BrExit = 0x02,
    /// Response to BrEntry with our info (also used as entry query response).
    AnsEntry = 0x03,
    /// User is in absence/away mode.
    BrAbsence = 0x04,

    // ─── User list commands ────────────────────────────────────────────────

    /// Request peer list (broadcast).
    BrIsGetList = 0x10,
    /// Acknowledge list request (you may proceed).
    OkGetList = 0x11,
    /// Request the actual list.
    GetList = 0x12,
    /// Response with the list data.
    AnsList = 0x13,

    // ─── Message commands ──────────────────────────────────────────────────

    /// Send a message to a peer.
    SendMsg = 0x20,
    /// Acknowledge receipt of a message (delivery receipt).
    RecvMsg = 0x21,
    /// Message was read (read receipt).
    ReadMsg = 0x30,
    /// Delete a message.
    DelMsg = 0x31,
    /// Acknowledge read receipt.
    AnsReadMsg = 0x32,

    // ─── File transfer commands ────────────────────────────────────────────

    /// File download request (TCP connection to sender).
    GetFileData = 0x60,
    /// Cancel/release file transfer.
    ReleaseFiles = 0x61,
    /// Directory file transfer request (recursive).
    GetDirFiles = 0x62,

    // ─── FeiQ extended commands ────────────────────────────────────────────

    /// Unknown purpose (defined in FeiQ headers, not observed in use).
    OpenYou = 0x77,
    /// Typing indicator ON — sent while user is composing a message.
    TypingStart = 0x79,
    /// Typing indicator OFF — sent ~2s after user stops typing.
    TypingEnd = 0x7A,
    /// Group chat message base command (used with EXTOPT flag 0x400000).
    /// The body is Blowfish-CBC encrypted using sender's MAC as key.
    GroupMsg = 0x23,
    /// Image/screenshot message (sent with FILEATTACHOPT flag).
    /// Extra field = 8-byte ASCII image ID.
    SendImage = 0xC0,
    /// Screen shake / window shake (抖屏) — no extra field.
    Knock = 0xD1,

    /// Unknown or unsupported command.
    Unknown = 0xFF,
}

/// Command flags (bits 8+ of the 32-bit command word).
///
/// In IPMsg, the command word is composed as:
/// ```text
/// bits  0-7:  base command code
/// bits  8-31: option flags (OR'd together)
/// ```
pub mod flags {
    /// Absence mode (user is away). Also used by FeiQ as "send check" flag.
    pub const IPMSG_ABSENCEOPT: u32 = 0x00000100;
    /// Server/broadcast mode.
    pub const IPMSG_SERVEROPT: u32 = 0x00000200;
    /// Broadcast message (sent to all users on subnet).
    pub const IPMSG_BROADCASTOPT: u32 = 0x00000400;
    /// Multicast message (sent to a group of users).
    pub const IPMSG_MULTICASTOPT: u32 = 0x00000800;
    /// FeiQ: request delivery confirmation (same bit as ABSENCEOPT).
    pub const IPMSG_SENDCHECKOPT: u32 = 0x00000100;
    /// Dial-up mode.
    pub const IPMSG_DIALUPOPT: u32 = 0x00010000;
    /// Request read confirmation (receiver must send ReadMsg).
    pub const IPMSG_READCHECKOPT: u32 = 0x00100000;
    /// Message has file attachments.
    pub const IPMSG_FILEATTACHOPT: u32 = 0x00200000;
    /// Message is encrypted (standard IPMsg uses Blowfish with packet_no as key).
    pub const IPMSG_ENCOPT: u32 = 0x00400000;
    /// Extended command (FeiQ extension, used for group chat).
    pub const IPMSG_EXTOPT: u32 = 0x00400000;
    /// UTF-8 encoding (instead of GBK/system locale).
    pub const IPMSG_UTF8OPT: u32 = 0x00800000;
    /// FeiQ combined online broadcast flags (ENCOPT | FILEATTACHOPT).
    pub const FEIQ_ONLINE_FLAGS: u32 = 0x00600000;
}

impl Command {
    /// Parse a command from its raw u32 value.
    /// Only the lower 8 bits determine the base command.
    pub fn from_raw(value: u32) -> Self {
        match value & 0x000000FF {
            0x00 => Command::NoOperation,
            0x01 => Command::BrEntry,
            0x02 => Command::BrExit,
            0x03 => Command::AnsEntry,
            0x04 => Command::BrAbsence,
            0x10 => Command::BrIsGetList,
            0x11 => Command::OkGetList,
            0x12 => Command::GetList,
            0x13 => Command::AnsList,
            0x20 => Command::SendMsg,
            0x21 => Command::RecvMsg,
            0x23 => Command::GroupMsg,
            0x30 => Command::ReadMsg,
            0x31 => Command::DelMsg,
            0x32 => Command::AnsReadMsg,
            0x60 => Command::GetFileData,
            0x61 => Command::ReleaseFiles,
            0x62 => Command::GetDirFiles,
            0x77 => Command::OpenYou,
            0x79 => Command::TypingStart,
            0x7A => Command::TypingEnd,
            0xC0 => Command::SendImage,
            0xD1 => Command::Knock,
            _ => Command::Unknown,
        }
    }

    /// Get the raw u8 value of this command (base command only, no flags).
    pub fn to_raw(self) -> u32 {
        self as u32
    }

    /// Check if this command is a FeiQ extension (not part of standard IPMsg).
    pub fn is_feiq_extension(self) -> bool {
        matches!(
            self,
            Command::OpenYou
                | Command::TypingStart
                | Command::TypingEnd
                | Command::SendImage
                | Command::Knock
                | Command::GroupMsg
                | Command::AnsReadMsg
        )
    }

    /// Check if this command involves file transfer.
    pub fn is_file_command(self) -> bool {
        matches!(
            self,
            Command::GetFileData | Command::ReleaseFiles | Command::GetDirFiles | Command::SendImage
        )
    }

    /// Check if this command is a message-related command.
    pub fn is_message_command(self) -> bool {
        matches!(
            self,
            Command::SendMsg | Command::RecvMsg | Command::ReadMsg | Command::DelMsg | Command::GroupMsg
        )
    }
}

/// Extract option flags from a raw command word (everything above bit 7).
pub fn extract_flags(command_word: u32) -> u32 {
    command_word & 0xFFFFFF00
}

/// Compose a full command word from base command and flags.
pub fn compose_command(cmd: Command, flags: u32) -> u32 {
    cmd.to_raw() | flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_commands_roundtrip() {
        let cmds = [
            Command::NoOperation,
            Command::BrEntry,
            Command::BrExit,
            Command::AnsEntry,
            Command::BrAbsence,
            Command::SendMsg,
            Command::RecvMsg,
            Command::ReadMsg,
            Command::DelMsg,
            Command::AnsReadMsg,
            Command::GetFileData,
            Command::ReleaseFiles,
            Command::GetDirFiles,
        ];
        for cmd in cmds {
            assert_eq!(Command::from_raw(cmd.to_raw()), cmd);
        }
    }

    #[test]
    fn test_feiq_extended_commands_roundtrip() {
        let cmds = [
            Command::TypingStart,
            Command::TypingEnd,
            Command::SendImage,
            Command::Knock,
            Command::GroupMsg,
        ];
        for cmd in cmds {
            assert_eq!(Command::from_raw(cmd.to_raw()), cmd);
        }
    }

    #[test]
    fn test_command_with_file_attach_flag() {
        let raw = Command::SendMsg.to_raw() | flags::IPMSG_FILEATTACHOPT;
        assert_eq!(raw, 0x00200020);
        assert_eq!(Command::from_raw(raw), Command::SendMsg);
        assert_eq!(extract_flags(raw), flags::IPMSG_FILEATTACHOPT);
    }

    #[test]
    fn test_feiq_online_broadcast() {
        // FeiQ sends BrEntry with ENCOPT|FILEATTACHOPT flags = 0x600001
        let raw = Command::BrEntry.to_raw() | flags::FEIQ_ONLINE_FLAGS;
        assert_eq!(raw, 0x600001);
        assert_eq!(Command::from_raw(raw), Command::BrEntry);
    }

    #[test]
    fn test_group_chat_command() {
        // Group messages: GroupMsg(0x23) | ENCOPT(0x400000) = 0x400023
        let raw = Command::GroupMsg.to_raw() | flags::IPMSG_ENCOPT;
        assert_eq!(raw, 0x400023);
        assert_eq!(Command::from_raw(raw), Command::GroupMsg);
    }

    #[test]
    fn test_delivery_check_message() {
        // SENDMSG(0x20) | SENDCHECKOPT(0x100) = 0x120 = 288
        let raw = Command::SendMsg.to_raw() | flags::IPMSG_SENDCHECKOPT;
        assert_eq!(raw, 288);
        assert_eq!(Command::from_raw(raw), Command::SendMsg);
        assert!(extract_flags(raw) & flags::IPMSG_SENDCHECKOPT != 0);
    }

    #[test]
    fn test_unknown_command() {
        assert_eq!(Command::from_raw(0x99), Command::Unknown);
    }

    #[test]
    fn test_is_feiq_extension() {
        assert!(Command::TypingStart.is_feiq_extension());
        assert!(Command::Knock.is_feiq_extension());
        assert!(!Command::SendMsg.is_feiq_extension());
        assert!(!Command::BrEntry.is_feiq_extension());
    }

    #[test]
    fn test_is_file_command() {
        assert!(Command::GetFileData.is_file_command());
        assert!(Command::SendImage.is_file_command());
        assert!(!Command::SendMsg.is_file_command());
    }

    #[test]
    fn test_compose_command() {
        let word = compose_command(Command::SendMsg, flags::IPMSG_UTF8OPT);
        assert_eq!(word, 0x00800020);
        assert_eq!(Command::from_raw(word), Command::SendMsg);
        assert_eq!(extract_flags(word), flags::IPMSG_UTF8OPT);
    }

    #[test]
    fn test_correct_command_values() {
        // Verify against actual IPMsg protocol specification
        assert_eq!(Command::SendMsg.to_raw(), 0x20);
        assert_eq!(Command::RecvMsg.to_raw(), 0x21);
        assert_eq!(Command::ReadMsg.to_raw(), 0x30);
        assert_eq!(Command::GetFileData.to_raw(), 0x60);
        assert_eq!(Command::ReleaseFiles.to_raw(), 0x61);
        assert_eq!(Command::GetDirFiles.to_raw(), 0x62);
        assert_eq!(Command::TypingStart.to_raw(), 0x79);
        assert_eq!(Command::TypingEnd.to_raw(), 0x7A);
        assert_eq!(Command::Knock.to_raw(), 0xD1);
    }
}
