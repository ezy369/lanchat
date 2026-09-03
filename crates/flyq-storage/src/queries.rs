//! SQL query helpers with pagination and full-text search.

use crate::db::{Database, DbError};
use crate::models::{
    ConversationSummary, MessageStats, Page, PagedResult, SearchResult, StoredMessage, StoredPeer,
};

impl Database {
    // ─── Message CRUD ───────────────────────────────────────────────────────

    /// Insert a message into the database.
    pub async fn insert_message(&self, msg: &StoredMessage) -> Result<(), DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO messages (id, sender, recipient, content, timestamp, read)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .await?;
        stmt.execute((
            msg.id.as_str(),
            msg.sender.as_str(),
            msg.recipient.as_str(),
            msg.content.as_str(),
            msg.timestamp,
            msg.read as i64,
        ))
        .await?;
        Ok(())
    }

    /// Delete a message by ID.
    pub async fn delete_message(&self, id: &str) -> Result<bool, DbError> {
        let affected = self
            .conn
            .execute("DELETE FROM messages WHERE id = ?1", [id])
            .await?;
        Ok(affected > 0)
    }

    /// Mark a single message as read.
    pub async fn mark_message_read(&self, id: &str) -> Result<(), DbError> {
        self.conn
            .execute("UPDATE messages SET read = 1 WHERE id = ?1", [id])
            .await?;
        Ok(())
    }

    /// Mark all messages from a specific sender as read.
    pub async fn mark_conversation_read(
        &self,
        my_addr: &str,
        peer_addr: &str,
    ) -> Result<u64, DbError> {
        let affected = self
            .conn
            .execute(
                "UPDATE messages SET read = 1
                 WHERE sender = ?1 AND recipient = ?2 AND read = 0",
                (peer_addr, my_addr),
            )
            .await?;
        Ok(affected)
    }

    // ─── Paginated Queries ──────────────────────────────────────────────────

    /// Get messages between two users with pagination, ordered by timestamp ascending.
    pub async fn get_messages_paged(
        &self,
        user_a: &str,
        user_b: &str,
        page: Page,
    ) -> Result<PagedResult<StoredMessage>, DbError> {
        // Get total count first.
        let mut count_stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM messages
                 WHERE (sender = ?1 AND recipient = ?2) OR (sender = ?2 AND recipient = ?1)",
            )
            .await?;
        let mut count_rows = count_stmt.query((user_a, user_b)).await?;
        let total = match count_rows.next().await? {
            Some(row) => row.get::<i64>(0)? as u64,
            None => 0,
        };

        // Fetch the page.
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sender, recipient, content, timestamp, read
                 FROM messages
                 WHERE (sender = ?1 AND recipient = ?2) OR (sender = ?2 AND recipient = ?1)
                 ORDER BY timestamp DESC
                 LIMIT ?3 OFFSET ?4",
            )
            .await?;

        let mut rows = stmt
            .query((user_a, user_b, page.limit, page.offset))
            .await?;
        let mut messages = Vec::new();

        while let Some(row) = rows.next().await? {
            messages.push(row_to_message(&row)?);
        }

        // Reverse to get chronological order (oldest first within page).
        messages.reverse();

        Ok(PagedResult {
            items: messages,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    /// Get messages between two users, ordered by timestamp (legacy API, returns latest `limit`).
    pub async fn get_messages(
        &self,
        user_a: &str,
        user_b: &str,
        limit: u32,
    ) -> Result<Vec<StoredMessage>, DbError> {
        let page = Page::new(limit, 0);
        let result = self.get_messages_paged(user_a, user_b, page).await?;
        Ok(result.items)
    }

    /// Get all messages involving a specific user (sent or received) with pagination.
    pub async fn get_messages_by_user(
        &self,
        user: &str,
        page: Page,
    ) -> Result<PagedResult<StoredMessage>, DbError> {
        let mut count_stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM messages WHERE sender = ?1 OR recipient = ?1",
            )
            .await?;
        let mut count_rows = count_stmt.query([user]).await?;
        let total = match count_rows.next().await? {
            Some(row) => row.get::<i64>(0)? as u64,
            None => 0,
        };

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sender, recipient, content, timestamp, read
                 FROM messages
                 WHERE sender = ?1 OR recipient = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .await?;

        let mut rows = stmt.query((user, page.limit, page.offset)).await?;
        let mut messages = Vec::new();

        while let Some(row) = rows.next().await? {
            messages.push(row_to_message(&row)?);
        }

        messages.reverse();

        Ok(PagedResult {
            items: messages,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    // ─── Full-Text Search ───────────────────────────────────────────────────

    /// Search messages using FTS5 full-text search.
    ///
    /// The query syntax follows FTS5 rules:
    /// - Simple words: `hello world` (matches documents containing both)
    /// - Phrases: `"hello world"` (exact phrase match)
    /// - Prefix: `hel*` (matches hello, help, etc.)
    /// - Boolean: `hello OR world`, `hello NOT goodbye`
    ///
    /// Results are ranked by relevance (BM25) and paginated.
    pub async fn search_messages(
        &self,
        query: &str,
        page: Page,
    ) -> Result<PagedResult<SearchResult>, DbError> {
        // Count total matches.
        let mut count_stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH ?1")
            .await?;
        let mut count_rows = count_stmt.query([query]).await?;
        let total = match count_rows.next().await? {
            Some(row) => row.get::<i64>(0)? as u64,
            None => 0,
        };

        // Fetch ranked results with snippets.
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    f.msg_id,
                    m.sender,
                    m.recipient,
                    m.content,
                    m.timestamp,
                    m.read,
                    rank,
                    snippet(messages_fts, 0, '<<', '>>', '...', 32)
                 FROM messages_fts f
                 JOIN messages m ON m.id = f.msg_id
                 WHERE messages_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2 OFFSET ?3",
            )
            .await?;

        let mut rows = stmt.query((query, page.limit, page.offset)).await?;
        let mut results = Vec::new();

        while let Some(row) = rows.next().await? {
            let msg_id: String = row.get(0)?;
            let sender: String = row.get(1)?;
            let recipient: String = row.get(2)?;
            let content: String = row.get(3)?;
            let timestamp: i64 = row.get(4)?;
            let read: i64 = row.get(5)?;
            let rank: f64 = row.get(6)?;
            let snippet: String = row.get(7)?;

            results.push(SearchResult {
                message: StoredMessage {
                    id: msg_id,
                    sender,
                    recipient,
                    content,
                    timestamp,
                    read: read != 0,
                },
                rank,
                snippet,
            });
        }

        Ok(PagedResult {
            items: results,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    /// Search messages within a specific conversation.
    pub async fn search_in_conversation(
        &self,
        user_a: &str,
        user_b: &str,
        query: &str,
        page: Page,
    ) -> Result<PagedResult<SearchResult>, DbError> {
        let mut count_stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM messages_fts f
                 JOIN messages m ON m.id = f.msg_id
                 WHERE messages_fts MATCH ?1
                   AND ((m.sender = ?2 AND m.recipient = ?3) OR (m.sender = ?3 AND m.recipient = ?2))",
            )
            .await?;
        let mut count_rows = count_stmt.query((query, user_a, user_b)).await?;
        let total = match count_rows.next().await? {
            Some(row) => row.get::<i64>(0)? as u64,
            None => 0,
        };

        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    f.msg_id,
                    m.sender,
                    m.recipient,
                    m.content,
                    m.timestamp,
                    m.read,
                    rank,
                    snippet(messages_fts, 0, '<<', '>>', '...', 32)
                 FROM messages_fts f
                 JOIN messages m ON m.id = f.msg_id
                 WHERE messages_fts MATCH ?1
                   AND ((m.sender = ?2 AND m.recipient = ?3) OR (m.sender = ?3 AND m.recipient = ?2))
                 ORDER BY rank
                 LIMIT ?4 OFFSET ?5",
            )
            .await?;

        let mut rows = stmt
            .query((query, user_a, user_b, page.limit, page.offset))
            .await?;
        let mut results = Vec::new();

        while let Some(row) = rows.next().await? {
            let msg_id: String = row.get(0)?;
            let sender: String = row.get(1)?;
            let recipient: String = row.get(2)?;
            let content: String = row.get(3)?;
            let timestamp: i64 = row.get(4)?;
            let read: i64 = row.get(5)?;
            let rank: f64 = row.get(6)?;
            let snippet: String = row.get(7)?;

            results.push(SearchResult {
                message: StoredMessage {
                    id: msg_id,
                    sender,
                    recipient,
                    content,
                    timestamp,
                    read: read != 0,
                },
                rank,
                snippet,
            });
        }

        Ok(PagedResult {
            items: results,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    // ─── Conversation Management ────────────────────────────────────────────

    /// Get a list of conversations with last message preview and unread counts.
    ///
    /// Returns conversations sorted by most recent activity.
    pub async fn get_conversations(&self, my_addr: &str) -> Result<Vec<ConversationSummary>, DbError> {
        // Get distinct conversation partners with their latest message.
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    CASE WHEN sender = ?1 THEN recipient ELSE sender END AS peer,
                    content,
                    timestamp
                 FROM messages
                 WHERE sender = ?1 OR recipient = ?1
                 ORDER BY timestamp DESC",
            )
            .await?;

        let mut rows = stmt.query([my_addr]).await?;
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut conversations: Vec<ConversationSummary> = Vec::new();

        while let Some(row) = rows.next().await? {
            let peer: String = row.get(0)?;
            if seen.contains(&peer) {
                continue;
            }
            seen.insert(peer.clone());

            let content: String = row.get(1)?;
            let timestamp: i64 = row.get(2)?;

            // Count unread messages from this peer.
            let mut unread_stmt = self
                .conn
                .prepare(
                    "SELECT COUNT(*) FROM messages
                     WHERE sender = ?1 AND recipient = ?2 AND read = 0",
                )
                .await?;
            let mut unread_rows = unread_stmt.query((peer.as_str(), my_addr)).await?;
            let unread_count: u32 = match unread_rows.next().await? {
                Some(r) => r.get::<i64>(0)? as u32,
                None => 0,
            };

            // Try to get peer display name from peers table.
            let peer_name = self.get_peer_name(&peer).await?.unwrap_or_else(|| peer.clone());

            conversations.push(ConversationSummary {
                peer_addr: peer,
                peer_name,
                last_message: content,
                last_timestamp: timestamp,
                unread_count,
            });
        }

        Ok(conversations)
    }

    /// Get message statistics for a conversation between two users.
    pub async fn get_message_stats(
        &self,
        user_a: &str,
        user_b: &str,
    ) -> Result<MessageStats, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    COUNT(*) AS total,
                    COALESCE(SUM(CASE WHEN read = 0 AND sender = ?2 AND recipient = ?1 THEN 1 ELSE 0 END), 0) AS unread,
                    MIN(timestamp) AS first_ts,
                    MAX(timestamp) AS last_ts
                 FROM messages
                 WHERE (sender = ?1 AND recipient = ?2) OR (sender = ?2 AND recipient = ?1)",
            )
            .await?;

        let mut rows = stmt.query((user_a, user_b)).await?;

        match rows.next().await? {
            Some(row) => {
                let total: i64 = row.get(0)?;
                let unread: i64 = row.get(1)?;
                // MIN/MAX return NULL when no rows match; handle gracefully.
                let first_ts: Option<i64> = if total > 0 { row.get(2).ok() } else { None };
                let last_ts: Option<i64> = if total > 0 { row.get(3).ok() } else { None };

                Ok(MessageStats {
                    total: total as u64,
                    unread: unread as u64,
                    first_timestamp: first_ts,
                    last_timestamp: last_ts,
                })
            }
            None => Ok(MessageStats::default()),
        }
    }

    /// Get total unread message count for a user.
    pub async fn get_unread_count(&self, my_addr: &str) -> Result<u64, DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM messages WHERE recipient = ?1 AND read = 0",
            )
            .await?;
        let mut rows = stmt.query([my_addr]).await?;
        Ok(match rows.next().await? {
            Some(row) => row.get::<i64>(0)? as u64,
            None => 0,
        })
    }

    /// Get messages within a time range (useful for date-based navigation).
    pub async fn get_messages_by_time_range(
        &self,
        user_a: &str,
        user_b: &str,
        start_ts: i64,
        end_ts: i64,
        page: Page,
    ) -> Result<PagedResult<StoredMessage>, DbError> {
        let mut count_stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM messages
                 WHERE ((sender = ?1 AND recipient = ?2) OR (sender = ?2 AND recipient = ?1))
                   AND timestamp >= ?3 AND timestamp <= ?4",
            )
            .await?;
        let mut count_rows = count_stmt
            .query((user_a, user_b, start_ts, end_ts))
            .await?;
        let total = match count_rows.next().await? {
            Some(row) => row.get::<i64>(0)? as u64,
            None => 0,
        };

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sender, recipient, content, timestamp, read
                 FROM messages
                 WHERE ((sender = ?1 AND recipient = ?2) OR (sender = ?2 AND recipient = ?1))
                   AND timestamp >= ?3 AND timestamp <= ?4
                 ORDER BY timestamp ASC
                 LIMIT ?5 OFFSET ?6",
            )
            .await?;

        let mut rows = stmt
            .query((user_a, user_b, start_ts, end_ts, page.limit, page.offset))
            .await?;
        let mut messages = Vec::new();

        while let Some(row) = rows.next().await? {
            messages.push(row_to_message(&row)?);
        }

        Ok(PagedResult {
            items: messages,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    // ─── Peer Management ────────────────────────────────────────────────────

    /// Insert or update a peer record.
    pub async fn upsert_peer(&self, peer: &StoredPeer) -> Result<(), DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO peers (addr, name, host, grp, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(addr) DO UPDATE SET
                    name = ?2, host = ?3, grp = ?4, last_seen = ?5",
            )
            .await?;
        stmt.execute((
            peer.addr.as_str(),
            peer.name.as_str(),
            peer.host.as_str(),
            peer.group.as_deref().unwrap_or(""),
            peer.last_seen,
        ))
        .await?;
        Ok(())
    }

    /// Get all known peers.
    pub async fn get_peers(&self) -> Result<Vec<StoredPeer>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT addr, name, host, grp, last_seen FROM peers ORDER BY name")
            .await?;

        let mut rows = stmt.query(()).await?;
        let mut peers = Vec::new();

        while let Some(row) = rows.next().await? {
            let grp: String = row.get(3)?;
            peers.push(StoredPeer {
                addr: row.get::<String>(0)?,
                name: row.get::<String>(1)?,
                host: row.get::<String>(2)?,
                group: if grp.is_empty() { None } else { Some(grp) },
                last_seen: row.get::<i64>(4)?,
            });
        }

        Ok(peers)
    }

    /// Delete a peer by address.
    pub async fn delete_peer(&self, addr: &str) -> Result<bool, DbError> {
        let affected = self
            .conn
            .execute("DELETE FROM peers WHERE addr = ?1", [addr])
            .await?;
        Ok(affected > 0)
    }

    /// Get peer display name by address.
    async fn get_peer_name(&self, addr: &str) -> Result<Option<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM peers WHERE addr = ?1")
            .await?;
        let mut rows = stmt.query([addr]).await?;
        Ok(match rows.next().await? {
            Some(row) => Some(row.get::<String>(0)?),
            None => None,
        })
    }

    /// Update a peer's last_seen timestamp.
    pub async fn touch_peer(&self, addr: &str, timestamp: i64) -> Result<(), DbError> {
        self.conn
            .execute(
                "UPDATE peers SET last_seen = ?1 WHERE addr = ?2",
                (timestamp, addr),
            )
            .await?;
        Ok(())
    }
}

/// Helper to convert a libsql Row into a StoredMessage.
fn row_to_message(row: &libsql::Row) -> Result<StoredMessage, DbError> {
    Ok(StoredMessage {
        id: row.get::<String>(0)?,
        sender: row.get::<String>(1)?,
        recipient: row.get::<String>(2)?,
        content: row.get::<String>(3)?,
        timestamp: row.get::<i64>(4)?,
        read: row.get::<i64>(5)? != 0,
    })
}
