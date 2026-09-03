//! Database connection and initialization.

use libsql::Connection;
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database error: {0}")]
    Libsql(#[from] libsql::Error),
    #[error("Migration error: {0}")]
    Migration(String),
}

/// Database handle wrapping a libSQL connection.
#[derive(Clone)]
pub struct Database {
    pub(crate) conn: Connection,
}

impl Database {
    /// Open or create a database at the given path.
    pub async fn open(path: &str) -> Result<Self, DbError> {
        let conn = libsql::Builder::new_local(path)
            .build()
            .await?
            .connect()?;
        info!("Database opened at {}", path);
        Ok(Self { conn })
    }

    /// Run database migrations to create required tables.
    pub async fn migrate(&self) -> Result<(), DbError> {
        self.conn
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS messages (
                    id TEXT PRIMARY KEY,
                    sender TEXT NOT NULL,
                    recipient TEXT NOT NULL,
                    content TEXT NOT NULL,
                    timestamp INTEGER NOT NULL,
                    read INTEGER NOT NULL DEFAULT 0
                );

                CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender);
                CREATE INDEX IF NOT EXISTS idx_messages_recipient ON messages(recipient);
                CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
                CREATE INDEX IF NOT EXISTS idx_messages_conversation
                    ON messages(sender, recipient, timestamp);

                CREATE TABLE IF NOT EXISTS peers (
                    addr TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    host TEXT NOT NULL,
                    grp TEXT,
                    last_seen INTEGER NOT NULL
                );

                -- FTS5 virtual table for full-text search on message content.
                -- Uses unicode61 tokenizer which handles CJK characters by splitting
                -- on each character boundary (suitable for Chinese/Japanese/Korean).
                CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                    content,
                    sender UNINDEXED,
                    recipient UNINDEXED,
                    timestamp UNINDEXED,
                    msg_id UNINDEXED,
                    tokenize='unicode61'
                );

                -- Triggers to keep FTS index in sync with messages table.
                CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
                    INSERT INTO messages_fts(content, sender, recipient, timestamp, msg_id)
                    VALUES (NEW.content, NEW.sender, NEW.recipient, NEW.timestamp, NEW.id);
                END;

                CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
                    DELETE FROM messages_fts WHERE msg_id = OLD.id;
                END;

                CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE OF content ON messages BEGIN
                    DELETE FROM messages_fts WHERE msg_id = OLD.id;
                    INSERT INTO messages_fts(content, sender, recipient, timestamp, msg_id)
                    VALUES (NEW.content, NEW.sender, NEW.recipient, NEW.timestamp, NEW.id);
                END;
                ",
            )
            .await?;
        info!("Database migrations completed");
        Ok(())
    }

    /// Rebuild the FTS index from the messages table.
    ///
    /// Useful after bulk imports or if the index gets out of sync.
    pub async fn rebuild_fts_index(&self) -> Result<(), DbError> {
        self.conn
            .execute_batch(
                "
                DELETE FROM messages_fts;
                INSERT INTO messages_fts(content, sender, recipient, timestamp, msg_id)
                SELECT content, sender, recipient, timestamp, id FROM messages;
                ",
            )
            .await?;
        info!("FTS index rebuilt");
        Ok(())
    }

    /// Get a reference to the underlying connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}
