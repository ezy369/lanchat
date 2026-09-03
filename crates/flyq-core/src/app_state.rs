//! Application state management.

use flyq_network::PeerManager;
use flyq_storage::Database;

/// Central application state.
pub struct AppState {
    /// Our display name.
    pub username: String,
    /// Our hostname.
    pub hostname: String,
    /// Peer manager for tracking online users.
    pub peer_manager: PeerManager,
    /// Database handle (initialized lazily).
    pub db: Option<Database>,
}

impl AppState {
    /// Create a new application state with the given username.
    pub fn new(username: &str) -> Self {
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        Self {
            username: username.to_string(),
            hostname,
            peer_manager: PeerManager::new(),
            db: None,
        }
    }

    /// Initialize the database.
    pub async fn init_database(&mut self, path: &str) -> Result<(), flyq_storage::DbError> {
        let db = Database::open(path).await?;
        db.migrate().await?;
        self.db = Some(db);
        Ok(())
    }
}
