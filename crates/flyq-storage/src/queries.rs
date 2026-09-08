//! SQL query helpers with pagination and full-text search.

use crate::db::{Database, DbError};
use crate::models::{
    ConversationSummary, Group, GroupSummary, MessageStats, Page, PagedResult, SearchResult,
    StoredMessage, StoredPeer,
};

impl Database {
    // ─── Message CRUD ───────────────────────────────────────────────────────

    /// Insert a message into the database.
    pub async fn insert_message(&self, msg: &StoredMessage) -> Result<(), DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO messages (id, sender, recipient, content, timestamp, read, packet_no, group_id, media_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .await?;
        let pkt: Option<i64> = msg.packet_no.map(|n| n as i64);
        stmt.execute((
            msg.id.as_str(),
            msg.sender.as_str(),
            msg.recipient.as_str(),
            msg.content.as_str(),
            msg.timestamp,
            msg.read as i64,
            pkt,
            msg.group_id.as_deref(),
            msg.media_type as i64,
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

    /// Delete a message by the sender's original packet_no.
    ///
    /// Used for the IPMsg DelMsg command: the sender asks us to delete the
    /// message identified by their `packet_no`. The `sender_addr` parameter
    /// is the peer's `ip:port` string, ensuring we only delete messages
    /// received from that specific peer.
    pub async fn delete_message_by_packet_no(
        &self,
        sender_addr: &str,
        packet_no: u32,
    ) -> Result<bool, DbError> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM messages WHERE sender = ?1 AND packet_no = ?2",
                (sender_addr, packet_no as i64),
            )
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
                "SELECT id, sender, recipient, content, timestamp, read, packet_no, group_id, media_type
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
                "SELECT id, sender, recipient, content, timestamp, read, packet_no, group_id, media_type
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
                    packet_no: None,
                    group_id: None,
                    media_type: 0,
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
                    packet_no: None,
                    group_id: None,
                    media_type: 0,
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
                "SELECT id, sender, recipient, content, timestamp, read, packet_no, group_id, media_type
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
    ///
    /// On conflict (peer already exists), updates name/host/grp/last_seen but
    /// preserves any user-set `remark_name` and `avatar_path`.
    pub async fn upsert_peer(&self, peer: &StoredPeer) -> Result<(), DbError> {
        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO peers (addr, name, host, grp, last_seen, remark_name, avatar_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
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
            peer.remark_name.as_deref(),
            peer.avatar_path.as_deref(),
        ))
        .await?;
        Ok(())
    }

    /// Get all known peers.
    pub async fn get_peers(&self) -> Result<Vec<StoredPeer>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT addr, name, host, grp, last_seen, remark_name, avatar_path FROM peers ORDER BY name")
            .await?;

        let mut rows = stmt.query(()).await?;
        let mut peers = Vec::new();

        while let Some(row) = rows.next().await? {
            let grp: String = row.get(3)?;
            let remark: Option<String> = row.get(5)?;
            let avatar: Option<String> = row.get(6)?;
            peers.push(StoredPeer {
                addr: row.get::<String>(0)?,
                name: row.get::<String>(1)?,
                host: row.get::<String>(2)?,
                group: if grp.is_empty() { None } else { Some(grp) },
                last_seen: row.get::<i64>(4)?,
                remark_name: remark.filter(|s| !s.is_empty()),
                avatar_path: avatar.filter(|s| !s.is_empty()),
            });
        }

        Ok(peers)
    }

    /// Set (or clear) a user-defined remark name for a peer.
    ///
    /// Pass `None` or an empty string to clear the alias and revert to the
    /// peer's broadcast name.
    pub async fn set_remark_name(&self, addr: &str, remark: Option<&str>) -> Result<(), DbError> {
        let remark_value = remark.filter(|s| !s.is_empty());
        self.conn
            .execute(
                "UPDATE peers SET remark_name = ?1 WHERE addr = ?2",
                (remark_value, addr),
            )
            .await?;
        Ok(())
    }

    /// Set (or clear) the avatar image path for a peer.
    pub async fn set_avatar_path(&self, addr: &str, path: Option<&str>) -> Result<(), DbError> {
        let path_value = path.filter(|s| !s.is_empty());
        self.conn
            .execute(
                "UPDATE peers SET avatar_path = ?1 WHERE addr = ?2",
                (path_value, addr),
            )
            .await?;
        Ok(())
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

    // ─── Group Management ────────────────────────────────────────────────────

    /// Insert a new group. Members are stored as a JSON array string.
    pub async fn insert_group(&self, group: &Group) -> Result<(), DbError> {
        let members_json = serde_json::to_string(&group.members)
            .map_err(|e| DbError::Migration(e.to_string()))?;
        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO groups (id, name, members, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .await?;
        stmt.execute((
            group.id.as_str(),
            group.name.as_str(),
            members_json.as_str(),
            group.created_at,
        ))
        .await?;
        Ok(())
    }

    /// Get all groups ordered by creation time (newest first).
    pub async fn get_groups(&self) -> Result<Vec<Group>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, members, created_at FROM groups ORDER BY created_at DESC")
            .await?;

        let mut rows = stmt.query(()).await?;
        let mut groups = Vec::new();

        while let Some(row) = rows.next().await? {
            let members_json: String = row.get(2)?;
            let members: Vec<String> = serde_json::from_str(&members_json).unwrap_or_default();
            groups.push(Group {
                id: row.get::<String>(0)?,
                name: row.get::<String>(1)?,
                members,
                created_at: row.get::<i64>(3)?,
            });
        }

        Ok(groups)
    }

    /// Find a group by its display name.
    pub async fn get_group_by_name(&self, name: &str) -> Result<Option<Group>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, members, created_at FROM groups WHERE name = ?1")
            .await?;

        let mut rows = stmt.query([name]).await?;
        match rows.next().await? {
            Some(row) => {
                let members_json: String = row.get(2)?;
                let members: Vec<String> = serde_json::from_str(&members_json).unwrap_or_default();
                Ok(Some(Group {
                    id: row.get::<String>(0)?,
                    name: row.get::<String>(1)?,
                    members,
                    created_at: row.get::<i64>(3)?,
                }))
            }
            None => Ok(None),
        }
    }

    /// Update a group's member list.
    pub async fn update_group_members(
        &self,
        group_id: &str,
        members: &[String],
    ) -> Result<(), DbError> {
        let members_json = serde_json::to_string(members)
            .map_err(|e| DbError::Migration(e.to_string()))?;
        self.conn
            .execute(
                "UPDATE groups SET members = ?1 WHERE id = ?2",
                (members_json.as_str(), group_id),
            )
            .await?;
        Ok(())
    }

    /// Delete a group by ID. Does not delete associated messages.
    pub async fn delete_group(&self, group_id: &str) -> Result<bool, DbError> {
        let affected = self
            .conn
            .execute("DELETE FROM groups WHERE id = ?1", [group_id])
            .await?;
        Ok(affected > 0)
    }

    // ─── Group Messages ──────────────────────────────────────────────────────

    /// Get messages in a group conversation with pagination.
    pub async fn get_group_messages_paged(
        &self,
        group_id: &str,
        page: Page,
    ) -> Result<PagedResult<StoredMessage>, DbError> {
        let mut count_stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM messages WHERE group_id = ?1")
            .await?;
        let mut count_rows = count_stmt.query([group_id]).await?;
        let total = match count_rows.next().await? {
            Some(row) => row.get::<i64>(0)? as u64,
            None => 0,
        };

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sender, recipient, content, timestamp, read, packet_no, group_id, media_type
                 FROM messages
                 WHERE group_id = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .await?;

        let mut rows = stmt.query((group_id, page.limit, page.offset)).await?;
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

    /// Mark all messages in a group as read.
    pub async fn mark_group_conversation_read(
        &self,
        group_id: &str,
    ) -> Result<u64, DbError> {
        let affected = self
            .conn
            .execute(
                "UPDATE messages SET read = 1 WHERE group_id = ?1 AND read = 0",
                [group_id],
            )
            .await?;
        Ok(affected)
    }

    /// Get group summaries for the sidebar (last message + unread count per group).
    pub async fn get_group_summaries(&self) -> Result<Vec<GroupSummary>, DbError> {
        let groups = self.get_groups().await?;
        let mut summaries = Vec::new();

        for group in &groups {
            // Get last message in this group.
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT content, timestamp FROM messages
                     WHERE group_id = ?1
                     ORDER BY timestamp DESC LIMIT 1",
                )
                .await?;
            let mut rows = stmt.query([group.id.as_str()]).await?;

            let (last_message, last_timestamp) = match rows.next().await? {
                Some(row) => (
                    row.get::<String>(0).unwrap_or_default(),
                    row.get::<i64>(1).unwrap_or(0),
                ),
                None => continue, // skip groups with no messages
            };

            // Count unread.
            let mut unread_stmt = self
                .conn
                .prepare(
                    "SELECT COUNT(*) FROM messages WHERE group_id = ?1 AND read = 0",
                )
                .await?;
            let mut unread_rows = unread_stmt.query([group.id.as_str()]).await?;
            let unread_count: u32 = match unread_rows.next().await? {
                Some(r) => r.get::<i64>(0)? as u32,
                None => 0,
            };

            summaries.push(GroupSummary {
                group_id: group.id.clone(),
                group_name: group.name.clone(),
                last_message,
                last_timestamp,
                unread_count,
                member_count: group.members.len(),
            });
        }

        // Sort by most recent activity.
        summaries.sort_by(|a, b| b.last_timestamp.cmp(&a.last_timestamp));

        Ok(summaries)
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
        packet_no: row.get::<Option<i64>>(6)?.map(|n| n as u32),
        group_id: row.get::<Option<String>>(7)?,
        media_type: row.get::<Option<i64>>(8)?.unwrap_or(0) as u8,
    })
}
