//! Data storage layer using libSQL embedded.
//!
//! Provides persistent storage for:
//! - Chat messages (with FTS5 full-text search)
//! - Peer information
//! - Conversation summaries
//! - Paginated query results

pub mod db;
pub mod models;
pub mod queries;

pub use db::{Database, DbError};
pub use models::{
    ConversationSummary, MessageStats, Page, PagedResult, SearchResult, StoredMessage, StoredPeer,
};
