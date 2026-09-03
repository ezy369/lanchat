//! TCP file transfer for IPMsg/FeiQ protocol.
//!
//! In IPMsg, file transfers work as follows:
//! 1. Sender includes file metadata in a SendMsg packet (FILEATTACHOPT flag)
//! 2. Receiver connects to sender's TCP port 2425
//! 3. Receiver sends a GetFileData request line:
//!    `version:packet_no:sender:host:0x60:fileId=offset`
//! 4. Sender streams raw file bytes until complete, then closes connection
//!
//! This module implements both the server (file provider) and client (file
//! downloader) sides of the protocol.

use flyq_protocol::{Command, PacketParser};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Buffer size for file streaming (64 KB).
const TRANSFER_BUF_SIZE: usize = 65536;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Connection closed by peer")]
    ConnectionClosed,
    #[error("Invalid file request: {0}")]
    InvalidRequest(String),
    #[error("File not found in registry: {0}")]
    FileNotRegistered(String),
    #[error("Protocol error: {0}")]
    Protocol(String),
}

/// Metadata for a file offered for transfer.
#[derive(Debug, Clone)]
pub struct FileOffer {
    /// Unique file ID (used in the protocol).
    pub file_id: u32,
    /// Original filename.
    pub filename: String,
    /// Full path on disk.
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Modification time (unix timestamp, hex-encoded in protocol).
    pub mtime: u64,
    /// File type code (1=regular file, 2=directory, 3=binary file).
    pub file_type: u32,
}

impl FileOffer {
    /// Create a FileOffer from a path, probing size and mtime.
    pub async fn from_path(file_id: u32, path: &Path) -> Result<Self, TransportError> {
        let metadata = tokio::fs::metadata(path).await?;
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Ok(Self {
            file_id,
            filename,
            path: path.to_path_buf(),
            size: metadata.len(),
            mtime,
            file_type: 1, // Regular file.
        })
    }

    /// Format as FeiQ wire record: `fileId:filename:sizeHex:mtimeHex:fileTypeHex`
    pub fn to_wire_record(&self) -> String {
        format!(
            "{}:{}:{:X}:{:X}:{:X}",
            self.file_id, self.filename, self.size, self.mtime, self.file_type
        )
    }

    /// Parse a FeiQ wire record.
    pub fn from_wire_record(record: &str, base_dir: &Path) -> Option<Self> {
        let parts: Vec<&str> = record.split(':').collect();
        if parts.len() < 5 {
            return None;
        }
        let file_id = parts[0].parse::<u32>().ok()?;
        let filename = parts[1].to_string();
        let size = u64::from_str_radix(parts[2], 16).ok()?;
        let mtime = u64::from_str_radix(parts[3], 16).ok()?;
        let file_type = u32::from_str_radix(parts[4], 16).ok()?;
        let path = base_dir.join(&filename);

        Some(Self {
            file_id,
            filename,
            path,
            size,
            mtime,
            file_type,
        })
    }
}

/// Registry of files available for transfer.
///
/// When a message with attachments is sent, the files are registered here.
/// When a peer connects via TCP requesting a file, we look it up by ID.
#[derive(Clone)]
pub struct FileRegistry {
    files: Arc<RwLock<HashMap<u32, FileOffer>>>,
    next_id: Arc<std::sync::atomic::AtomicU32>,
}

impl FileRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            files: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(std::sync::atomic::AtomicU32::new(1)),
        }
    }

    /// Register a file for transfer, returning its assigned file_id.
    pub async fn register(&self, path: &Path) -> Result<FileOffer, TransportError> {
        let file_id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let offer = FileOffer::from_path(file_id, path).await?;
        let mut files = self.files.write().await;
        files.insert(file_id, offer.clone());
        debug!("Registered file {} (id={})", offer.filename, file_id);
        Ok(offer)
    }

    /// Register a file with a specific ID (for received file offers).
    pub async fn register_with_id(&self, offer: FileOffer) {
        let mut files = self.files.write().await;
        files.insert(offer.file_id, offer);
    }

    /// Look up a file by ID.
    pub async fn get(&self, file_id: u32) -> Option<FileOffer> {
        let files = self.files.read().await;
        files.get(&file_id).cloned()
    }

    /// Remove a file from the registry (after successful transfer).
    pub async fn remove(&self, file_id: u32) -> Option<FileOffer> {
        let mut files = self.files.write().await;
        files.remove(&file_id)
    }

    /// Get all registered file IDs.
    pub async fn ids(&self) -> Vec<u32> {
        let files = self.files.read().await;
        files.keys().copied().collect()
    }

    /// Clear all registered files.
    pub async fn clear(&self) {
        let mut files = self.files.write().await;
        files.clear();
    }
}

impl Default for FileRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Progress callback type for file transfers.
pub type ProgressCallback = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// TCP file transfer server.
///
/// Listens on the IPMsg TCP port (2425) and serves file download requests
/// from peers. Each incoming connection is handled in a spawned task.
pub struct Transport {
    listener: TcpListener,
    registry: FileRegistry,
}

impl Transport {
    /// Bind a TCP listener on the given address.
    pub async fn bind(addr: SocketAddr, registry: FileRegistry) -> Result<Self, TransportError> {
        let listener = TcpListener::bind(addr).await?;
        info!("TCP file transfer server listening on {}", addr);
        Ok(Self { listener, registry })
    }

    /// Get the local address this transport is bound to.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Run the file transfer server loop.
    ///
    /// Accepts connections and spawns a task for each one.
    /// Returns when the listener is closed or an unrecoverable error occurs.
    pub async fn serve(self, mut shutdown: tokio::sync::watch::Receiver<bool>) -> Result<(), TransportError> {
        loop {
            tokio::select! {
                accept_result = self.listener.accept() => {
                    match accept_result {
                        Ok((stream, peer_addr)) => {
                            info!("File transfer connection from {}", peer_addr);
                            let registry = self.registry.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_file_request(stream, peer_addr, registry).await {
                                    warn!("File transfer error for {}: {}", peer_addr, e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("TCP accept error: {}", e);
                            // Brief pause before retrying to avoid tight error loop.
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("File transfer server shutting down");
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// Accept a single incoming connection (for testing).
    pub async fn accept(&self) -> Result<(TcpStream, SocketAddr), TransportError> {
        let (stream, addr) = self.listener.accept().await?;
        Ok((stream, addr))
    }
}

/// Handle an incoming TCP file download request.
///
/// Protocol:
/// 1. Read the request line (terminated by \n or connection close)
/// 2. Parse as IPMsg packet with GetFileData command
/// 3. Extract fileId and offset from the extra field
/// 4. Look up file in registry
/// 5. Stream file bytes from offset to end
async fn handle_file_request(
    stream: TcpStream,
    peer_addr: SocketAddr,
    registry: FileRegistry,
) -> Result<(), TransportError> {
    let mut reader = BufReader::new(stream);

    // Read the request line (up to 1024 bytes or newline).
    let mut request_buf = vec![0u8; 1024];
    let mut request_len = 0;

    loop {
        let byte = {
            let mut b = [0u8; 1];
            match reader.read_exact(&mut b).await {
                Ok(_) => b[0],
                Err(_) => break,
            }
        };

        if byte == b'\n' || byte == b'\0' {
            break;
        }
        if request_len < request_buf.len() {
            request_buf[request_len] = byte;
            request_len += 1;
        }
    }

    if request_len == 0 {
        return Err(TransportError::InvalidRequest("empty request".to_string()));
    }

    let request_str = String::from_utf8_lossy(&request_buf[..request_len]);
    debug!("File request from {}: {}", peer_addr, request_str);

    // Parse the request as an IPMsg packet.
    let packet = PacketParser::parse(&request_str)
        .map_err(|e| TransportError::Protocol(format!("parse error: {}", e)))?;

    if packet.command != Command::GetFileData && packet.command != Command::GetDirFiles {
        return Err(TransportError::InvalidRequest(format!(
            "expected GetFileData, got {:?}",
            packet.command
        )));
    }

    // Parse extra field: "fileId=offset" or just "fileId"
    let extra = packet.extra.as_deref().unwrap_or("");
    let (file_id, offset) = parse_file_request(extra)?;

    // Look up the file.
    let offer = registry
        .get(file_id)
        .await
        .ok_or_else(|| TransportError::FileNotRegistered(file_id.to_string()))?;

    info!(
        "Serving file {} ({} bytes, offset {}) to {}",
        offer.filename, offer.size, offset, peer_addr
    );

    // Open and stream the file.
    let file = File::open(&offer.path).await?;
    let mut file_reader = BufReader::new(file);

    // Seek to offset if non-zero.
    if offset > 0 {
        use tokio::io::AsyncSeekExt;
        let mut file_reader = tokio::io::BufReader::new(File::open(&offer.path).await?);
        file_reader.seek(std::io::SeekFrom::Start(offset)).await?;
        stream_file(&mut file_reader, &mut reader, offer.size - offset).await?;
    } else {
        stream_file(&mut file_reader, &mut reader, offer.size).await?;
    }

    info!("File transfer complete: {} to {}", offer.filename, peer_addr);
    Ok(())
}

/// Stream file data over a TCP connection.
async fn stream_file<R: AsyncReadExt + Unpin, W: AsyncWriteExt + Unpin>(
    file: &mut R,
    conn: &mut W,
    total_size: u64,
) -> Result<(), TransportError> {
    let mut buf = vec![0u8; TRANSFER_BUF_SIZE];
    let mut sent: u64 = 0;

    while sent < total_size {
        let to_read = std::cmp::min(buf.len(), (total_size - sent) as usize);
        let n = file.read(&mut buf[..to_read]).await?;
        if n == 0 {
            break; // EOF
        }
        conn.write_all(&buf[..n]).await?;
        sent += n as u64;
    }

    conn.flush().await?;
    Ok(())
}

/// Parse the file request extra field: "fileId=offset" or "fileId".
fn parse_file_request(extra: &str) -> Result<(u32, u64), TransportError> {
    if extra.is_empty() {
        return Err(TransportError::InvalidRequest("empty file request".to_string()));
    }

    if let Some((id_str, offset_str)) = extra.split_once('=') {
        let file_id = id_str
            .parse::<u32>()
            .map_err(|_| TransportError::InvalidRequest(format!("bad file_id: {}", id_str)))?;
        let offset = offset_str
            .parse::<u64>()
            .map_err(|_| TransportError::InvalidRequest(format!("bad offset: {}", offset_str)))?;
        Ok((file_id, offset))
    } else {
        let file_id = extra
            .parse::<u32>()
            .map_err(|_| TransportError::InvalidRequest(format!("bad file_id: {}", extra)))?;
        Ok((file_id, 0))
    }
}

// ─── Client Side (File Download) ────────────────────────────────────────────

/// Client for downloading files from a peer via TCP.
pub struct FileDownloader;

impl FileDownloader {
    /// Download a file from a peer's TCP port.
    ///
    /// Connects to `peer_addr`, sends a GetFileData request for `file_id`
    /// starting at `offset`, and writes the received bytes to `dest_path`.
    ///
    /// If `progress` is provided, it's called with (bytes_received, total_size)
    /// periodically during the transfer.
    pub async fn download(
        peer_addr: SocketAddr,
        file_id: u32,
        offset: u64,
        expected_size: u64,
        dest_path: &Path,
        sender_name: &str,
        sender_host: &str,
        progress: Option<ProgressCallback>,
    ) -> Result<u64, TransportError> {
        // Connect to peer's TCP port.
        let mut stream = TcpStream::connect(peer_addr).await?;
        debug!("Connected to {} for file download (id={})", peer_addr, file_id);

        // Build and send the GetFileData request.
        let request = format!(
            "1:{}:{}:{}:{}:{}={}\n",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as u32,
            sender_name,
            sender_host,
            Command::GetFileData.to_raw(),
            file_id,
            offset,
        );

        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;

        // Receive file data.
        let mut dest = File::create(dest_path).await?;
        let mut buf = vec![0u8; TRANSFER_BUF_SIZE];
        let mut received: u64 = 0;
        let remaining = expected_size.saturating_sub(offset);

        while received < remaining {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                break; // Connection closed by sender.
            }
            dest.write_all(&buf[..n]).await?;
            received += n as u64;

            if let Some(ref cb) = progress {
                cb(received + offset, expected_size);
            }
        }

        dest.flush().await?;
        info!(
            "Downloaded {} bytes to {:?}",
            received, dest_path
        );
        Ok(received)
    }

    /// Connect to a peer (for advanced use cases).
    pub async fn connect(peer_addr: SocketAddr) -> Result<TcpStream, TransportError> {
        let stream = TcpStream::connect(peer_addr).await?;
        Ok(stream)
    }
}
