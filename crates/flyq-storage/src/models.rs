//! Data models for storage.

use serde::{Deserialize, Serialize};

/// Media type: plain text message.
pub const MEDIA_TYPE_TEXT: u8 = 0;
/// Media type: inline image (thumbnail rendered in bubble, full image on disk).
pub const MEDIA_TYPE_IMAGE: u8 = 1;
/// Media type: file attachment (saved to download dir).
pub const MEDIA_TYPE_FILE: u8 = 2;

/// A stored chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub content: String,
    pub timestamp: i64,
    pub read: bool,
    pub packet_no: Option<u32>,
    /// Group ID for group messages; `None` for 1:1 messages.
    pub group_id: Option<String>,
    /// Media type: 0 = text, 1 = image, 2 = file.
    pub media_type: u8,
}

/// A stored peer record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPeer {
    pub addr: String,
    pub name: String,
    pub host: String,
    pub group: Option<String>,
    pub last_seen: i64,
    /// User-set display alias; `None` means use the peer's broadcast `name`.
    pub remark_name: Option<String>,
    /// Path to the peer's avatar image on local disk; `None` means use default.
    pub avatar_path: Option<String>,
}

/// Pagination request parameters.
#[derive(Debug, Clone, Copy)]
pub struct Page {
    /// Number of items per page (default: 50).
    pub limit: u32,
    /// Number of items to skip (default: 0).
    pub offset: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            limit: 50,
            offset: 0,
        }
    }
}

impl Page {
    /// Create a new page with the given limit and offset.
    pub fn new(limit: u32, offset: u32) -> Self {
        Self { limit, offset }
    }

    /// Create a page from a 1-based page number and size.
    pub fn from_page_number(page_number: u32, page_size: u32) -> Self {
        Self {
            limit: page_size,
            offset: (page_number.saturating_sub(1)) * page_size,
        }
    }
}

/// A paginated result set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PagedResult<T> {
    /// The items in this page.
    pub items: Vec<T>,
    /// Total number of items matching the query.
    pub total: u64,
    /// Current page offset.
    pub offset: u32,
    /// Page size limit.
    pub limit: u32,
}

impl<T> PagedResult<T> {
    /// Whether there are more items after this page.
    pub fn has_more(&self) -> bool {
        (self.offset as u64 + self.items.len() as u64) < self.total
    }

    /// Total number of pages.
    pub fn total_pages(&self) -> u32 {
        if self.limit == 0 {
            return 0;
        }
        ((self.total + self.limit as u64 - 1) / self.limit as u64) as u32
    }
}

/// A conversation summary for the sidebar list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    /// The peer address (IP:port).
    pub peer_addr: String,
    /// Display name of the peer.
    pub peer_name: String,
    /// Content of the last message.
    pub last_message: String,
    /// Timestamp of the last message.
    pub last_timestamp: i64,
    /// Number of unread messages in this conversation.
    pub unread_count: u32,
}

/// A full-text search result with relevance ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matched message.
    pub message: StoredMessage,
    /// Relevance score (lower = better match in FTS5).
    pub rank: f64,
    /// Snippet with highlighted match (using FTS5 snippet()).
    pub snippet: String,
}

/// Message statistics for a conversation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageStats {
    /// Total messages in the conversation.
    pub total: u64,
    /// Number of unread messages.
    pub unread: u64,
    /// Timestamp of the first message.
    pub first_timestamp: Option<i64>,
    /// Timestamp of the last message.
    pub last_timestamp: Option<i64>,
}

/// A chat group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    /// Unique group ID (UUID).
    pub id: String,
    /// Display name of the group.
    pub name: String,
    /// Member addresses (`ip:port` strings).
    pub members: Vec<String>,
    /// Creation timestamp (Unix seconds).
    pub created_at: i64,
}

/// A group summary for the sidebar list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupSummary {
    /// Group ID.
    pub group_id: String,
    /// Group display name.
    pub group_name: String,
    /// Content of the last message.
    pub last_message: String,
    /// Timestamp of the last message.
    pub last_timestamp: i64,
    /// Number of unread messages.
    pub unread_count: u32,
    /// Number of members.
    pub member_count: usize,
}
