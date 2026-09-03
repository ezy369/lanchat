//! FeiQ file attachment metadata format.
//!
//! When a message has the FILEATTACHOPT flag set, the extra field format is:
//!
//! ```text
//! <text message>\0<file1>\a<file2>\a...
//! ```
//!
//! Each file record: `fileId:filename:sizeHex:mtimeHex:fileTypeHex[:ext-attrs]\a`
//!
//! - `fileId`: numeric ID for referencing this file in transfer requests
//! - `filename`: original filename (`:` escaped as `::`)
//! - `sizeHex`: file size in hexadecimal ASCII
//! - `mtimeHex`: modification time (unix timestamp) in hexadecimal ASCII
//! - `fileTypeHex`: 0x1 = regular file, 0x2 = directory
//! - `ext-attrs`: optional extended attributes

use serde::{Deserialize, Serialize};

/// Separator between text and file list.
const TEXT_FILE_SEPARATOR: u8 = 0x00;
/// Separator between file records (and terminator).
#[allow(dead_code)]
const FILE_RECORD_SEPARATOR: u8 = 0x07; // BEL character

/// File type in the transfer protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileType {
    /// Regular file.
    Regular = 0x01,
    /// Directory (recursive transfer).
    Directory = 0x02,
}

impl FileType {
    pub fn from_raw(value: u64) -> Self {
        match value {
            0x02 => FileType::Directory,
            _ => FileType::Regular,
        }
    }

    pub fn to_raw(self) -> u64 {
        self as u64
    }
}

/// Information about a file attached to a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAttachmentInfo {
    /// File ID (used to reference this file in transfer requests).
    pub file_id: u32,
    /// Original filename.
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Modification time (unix timestamp).
    pub mtime: u64,
    /// File type (regular or directory).
    pub file_type: FileType,
}

impl FileAttachmentInfo {
    /// Serialize a file attachment to the protocol wire format.
    pub fn to_wire_format(&self) -> String {
        // Escape colons in filename
        let escaped_name = self.name.replace(':', "::");
        format!(
            "{}:{}:{:x}:{:x}:{:x}\u{7}",
            self.file_id, escaped_name, self.size, self.mtime, self.file_type.to_raw()
        )
    }
}

/// A request to download file data over TCP.
///
/// Format sent to the file server:
/// ```text
/// VersionStr:PacketNo:Name:Host:GETFILEDATA:<packetNo(hex)>:<fileId(hex)>:<offset(hex)>
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTransferRequest {
    /// Original packet number of the message containing the file.
    pub packet_no: u32,
    /// File ID to download.
    pub file_id: u32,
    /// Byte offset to start reading from.
    pub offset: u64,
}

impl FileTransferRequest {
    /// Format the extra field for a GETFILEDATA request.
    pub fn to_extra(&self) -> String {
        format!("{:x}:{:x}:{:x}", self.packet_no, self.file_id, self.offset)
    }

    /// Parse the extra field from a GETFILEDATA request.
    pub fn from_extra(extra: &str) -> Option<Self> {
        let parts: Vec<&str> = extra.split(':').collect();
        if parts.len() < 3 {
            return None;
        }
        Some(Self {
            packet_no: u32::from_str_radix(parts[0], 16).ok()?,
            file_id: u32::from_str_radix(parts[1], 16).ok()?,
            offset: u64::from_str_radix(parts[2], 16).ok()?,
        })
    }
}

/// Parse the extra field of a SENDMSG with FILEATTACHOPT.
///
/// Returns (text_message, Vec<FileAttachmentInfo>).
pub fn parse_file_message(extra: &str) -> (String, Vec<FileAttachmentInfo>) {
    let bytes = extra.as_bytes();

    // Find the \0 separator between text and file list
    let text_end = bytes.iter().position(|&b| b == TEXT_FILE_SEPARATOR).unwrap_or(bytes.len());
    let text = extra[..text_end].to_string();

    if text_end >= bytes.len() {
        return (text, Vec::new());
    }

    // Parse file records (after the \0, separated by \a)
    let file_section = &extra[text_end + 1..];
    let mut files = Vec::new();

    for record in file_section.split('\u{7}') {
        if record.is_empty() {
            continue;
        }
        if let Some(info) = parse_file_record(record) {
            files.push(info);
        }
    }

    (text, files)
}

/// Build the extra field for a message with file attachments.
pub fn build_file_message(text: &str, files: &[FileAttachmentInfo]) -> String {
    let mut result = text.to_string();
    result.push('\0');
    for file in files {
        result.push_str(&file.to_wire_format());
    }
    result
}

/// Parse a single file record: "fileId:filename:sizeHex:mtimeHex:fileTypeHex"
fn parse_file_record(record: &str) -> Option<FileAttachmentInfo> {
    // Split by ':' but handle escaped '::' in filename
    let parts = split_escaped(record, ':');
    if parts.len() < 5 {
        return None;
    }

    Some(FileAttachmentInfo {
        file_id: parts[0].parse::<u32>().ok()?,
        name: parts[1].clone(),
        size: u64::from_str_radix(&parts[2], 16).ok()?,
        mtime: u64::from_str_radix(&parts[3], 16).ok()?,
        file_type: FileType::from_raw(u64::from_str_radix(&parts[4], 16).unwrap_or(1)),
    })
}

/// Split a string by a delimiter, treating doubled delimiters as escaped literals.
fn split_escaped(s: &str, delim: char) -> Vec<String> {
    let escaped = format!("{delim}{delim}");
    let placeholder = "\x01ESCAPED\x01";
    let replaced = s.replace(&escaped, placeholder);
    replaced
        .split(delim)
        .map(|part| part.replace(placeholder, &delim.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_attachment_to_wire() {
        let info = FileAttachmentInfo {
            file_id: 1,
            name: "document.pdf".to_string(),
            size: 4096,
            mtime: 1700000000,
            file_type: FileType::Regular,
        };
        assert_eq!(info.to_wire_format(), "1:document.pdf:1000:6553f100:1\u{7}");
    }

    #[test]
    fn test_file_attachment_escape_colon() {
        let info = FileAttachmentInfo {
            file_id: 2,
            name: "file:name.txt".to_string(),
            size: 100,
            mtime: 0,
            file_type: FileType::Regular,
        };
        assert_eq!(info.to_wire_format(), "2:file::name.txt:64:0:1\u{7}");
    }

    #[test]
    fn test_parse_file_message() {
        let extra = "Hello\01:test.pdf:1000:6553f900:1\u{7}2:img.png:2000:6553f901:1\u{7}";
        let (text, files) = parse_file_message(extra);
        assert_eq!(text, "Hello");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_id, 1);
        assert_eq!(files[0].name, "test.pdf");
        assert_eq!(files[0].size, 0x1000);
        assert_eq!(files[1].file_id, 2);
        assert_eq!(files[1].name, "img.png");
    }

    #[test]
    fn test_parse_file_message_no_text() {
        let extra = "\01:file.zip:ff:0:1\u{7}";
        let (text, files) = parse_file_message(extra);
        assert_eq!(text, "");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].size, 255);
    }

    #[test]
    fn test_build_file_message() {
        let files = vec![FileAttachmentInfo {
            file_id: 1,
            name: "doc.txt".to_string(),
            size: 1024,
            mtime: 1700000000,
            file_type: FileType::Regular,
        }];
        let extra = build_file_message("Here you go", &files);
        assert!(extra.starts_with("Here you go\0"));
        assert!(extra.contains("1:doc.txt:400:6553f100:1"));
    }

    #[test]
    fn test_file_transfer_request_roundtrip() {
        let req = FileTransferRequest {
            packet_no: 12345,
            file_id: 1,
            offset: 4096,
        };
        let extra = req.to_extra();
        let parsed = FileTransferRequest::from_extra(&extra).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn test_directory_file_type() {
        let info = FileAttachmentInfo {
            file_id: 3,
            name: "my_folder".to_string(),
            size: 0,
            mtime: 0,
            file_type: FileType::Directory,
        };
        let wire = info.to_wire_format();
        assert!(wire.ends_with(":2\u{7}"));
    }

    #[test]
    fn test_split_escaped() {
        let parts = split_escaped("a:b::c:d", ':');
        assert_eq!(parts, vec!["a", "b:c", "d"]);
    }
}
