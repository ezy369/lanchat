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
use std::sync::atomic::{AtomicU64, Ordering};
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
    ///
    /// Directories are detected and advertised with `file_type = 2` and a size
    /// equal to the recursive total byte count of their contents, so the
    /// receiving UI can show a meaningful number before pulling the listing.
    pub async fn from_path(file_id: u32, path: &Path) -> Result<Self, TransportError> {
        let metadata = tokio::fs::metadata(path).await?;
        let is_dir = metadata.is_dir();
        let mtime = mtime_secs(&metadata);

        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let size = if is_dir {
            dir_total_size(path).await.unwrap_or(0)
        } else {
            metadata.len()
        };

        Ok(Self {
            file_id,
            filename,
            path: path.to_path_buf(),
            size,
            mtime,
            file_type: if is_dir { 2 } else { 1 },
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
        let file_id = self.alloc_id();
        let offer = FileOffer::from_path(file_id, path).await?;
        let mut files = self.files.write().await;
        files.insert(file_id, offer.clone());
        debug!("Registered file {} (id={})", offer.filename, file_id);
        Ok(offer)
    }

    /// Allocate the next unique file id.
    fn alloc_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Recursively register every entry under a previously-registered directory
    /// offer (`dir_id`) and return the resulting listing.
    ///
    /// Each entry's `filename` is its path relative to the directory root using
    /// `/` separators; directories carry `file_type == 2` and `size == 0`, files
    /// carry `file_type == 1` and their real byte size. Every entry is inserted
    /// into the registry with a fresh id so the receiver can subsequently pull
    /// each file via `GetFileData`.
    pub async fn register_dir_recursive(&self, dir_id: u32) -> Result<Vec<FileOffer>, TransportError> {
        let root = self
            .get(dir_id)
            .await
            .ok_or_else(|| TransportError::FileNotRegistered(dir_id.to_string()))?;
        if root.file_type != 2 {
            return Err(TransportError::InvalidRequest(format!(
                "id {} is not a directory",
                dir_id
            )));
        }
        let mut entries = Vec::new();
        self.walk_and_register(&root.path, "", &mut entries).await?;
        debug!(
            "Registered directory {} recursively ({} entries)",
            root.filename,
            entries.len()
        );
        Ok(entries)
    }

    /// Recursive worker for [`FileRegistry::register_dir_recursive`].
    ///
    /// `rel_prefix` is the `/`-separated path of `abs` relative to the transfer
    /// root (empty for the root's immediate children).
    async fn walk_and_register(
        &self,
        abs: &Path,
        rel_prefix: &str,
        out: &mut Vec<FileOffer>,
    ) -> Result<(), TransportError> {
        let mut rd = tokio::fs::read_dir(abs).await?;
        let mut children = Vec::new();
        while let Some(entry) = rd.next_entry().await? {
            children.push(entry);
        }
        // Deterministic order so listings are stable across runs.
        children.sort_by_key(|e| e.file_name());

        for entry in children {
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = if rel_prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", rel_prefix, name)
            };
            let path = entry.path();
            let ft = entry.file_type().await?;

            if ft.is_dir() {
                let m = entry.metadata().await.ok();
                let offer = FileOffer {
                    file_id: self.alloc_id(),
                    filename: rel.clone(),
                    path: path.clone(),
                    size: 0,
                    mtime: m.as_ref().map(mtime_secs).unwrap_or(0),
                    file_type: 2,
                };
                self.register_with_id(offer.clone()).await;
                out.push(offer);
                // Async recursion requires boxing the future.
                Box::pin(self.walk_and_register(&path, &rel, out)).await?;
            } else if ft.is_file() {
                let m = entry.metadata().await?;
                let offer = FileOffer {
                    file_id: self.alloc_id(),
                    filename: rel,
                    path,
                    size: m.len(),
                    mtime: mtime_secs(&m),
                    file_type: 1,
                };
                self.register_with_id(offer.clone()).await;
                out.push(offer);
            }
            // Symlinks and other special entries are intentionally skipped.
        }
        Ok(())
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

    // Directory listing request: recursively register the directory contents and
    // stream back one `fileId:relPath:sizeHex:mtimeHex:fileTypeHex` record per
    // line. The receiver then pulls each file individually via GetFileData.
    if packet.command == Command::GetDirFiles {
        info!(
            "Serving directory listing for {} to {}",
            offer.filename, peer_addr
        );
        let listing = registry.register_dir_recursive(file_id).await?;
        let mut body = String::new();
        for entry in &listing {
            body.push_str(&entry.to_wire_record());
            body.push('\n');
        }
        reader.write_all(body.as_bytes()).await?;
        reader.flush().await?;
        info!(
            "Directory listing sent: {} entries ({}) to {}",
            listing.len(),
            offer.filename,
            peer_addr
        );
        return Ok(());
    }

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

/// Extract a metadata's modification time as unix seconds (0 if unavailable).
fn mtime_secs(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Recursively sum the byte size of every file under `dir`.
///
/// Uses an explicit stack instead of recursion to avoid deep-nesting blowups.
async fn dir_total_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&current).await?;
        while let Some(entry) = rd.next_entry().await? {
            let ft = entry.file_type().await?;
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                if let Ok(m) = entry.metadata().await {
                    total += m.len();
                }
            }
        }
    }
    Ok(total)
}

/// Safely join a `/`- or `\`-separated relative path onto `base`.
///
/// Returns `None` if the relative path attempts traversal (`..`) or contains a
/// drive/absolute component (`:`), preventing a malicious peer from writing
/// outside the intended destination directory.
pub fn safe_relative_join(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for comp in rel.split(['/', '\\']) {
        match comp {
            "" | "." => continue,
            ".." => return None,
            seg => {
                if seg.contains(':') {
                    return None;
                }
                out.push(seg);
            }
        }
    }
    Some(out)
}

/// Summary of a completed directory download.
#[derive(Debug, Clone, Copy, Default)]
pub struct DirDownloadSummary {
    /// Number of files written.
    pub files: usize,
    /// Number of directories created.
    pub dirs: usize,
    /// Total bytes received across all files.
    pub bytes: u64,
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

    /// Fetch the recursive file listing for a directory offer.
    ///
    /// Connects to the peer's TCP port, sends a `GetDirFiles` request for
    /// `dir_file_id`, and parses the newline-separated wire records returned.
    /// Each returned offer's `filename` is the entry's path relative to the
    /// directory root (`/`-separated); its `path` field is not meaningful here.
    pub async fn fetch_dir_listing(
        peer_addr: SocketAddr,
        dir_file_id: u32,
        sender_name: &str,
        sender_host: &str,
    ) -> Result<Vec<FileOffer>, TransportError> {
        let mut stream = TcpStream::connect(peer_addr).await?;
        debug!(
            "Connected to {} for directory listing (id={})",
            peer_addr, dir_file_id
        );

        let request = format!(
            "1:{}:{}:{}:{}:{}\n",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as u32,
            sender_name,
            sender_host,
            Command::GetDirFiles.to_raw(),
            dir_file_id,
        );
        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;

        // The sender writes the whole listing then closes; read until EOF.
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        let text = String::from_utf8_lossy(&buf);

        let mut listing = Vec::new();
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            if let Some(offer) = FileOffer::from_wire_record(line, Path::new("")) {
                listing.push(offer);
            }
        }
        debug!(
            "Directory listing: {} entries from {}",
            listing.len(),
            peer_addr
        );
        Ok(listing)
    }

    /// Download an entire directory tree from a peer.
    ///
    /// Pulls the recursive listing via [`FileDownloader::fetch_dir_listing`],
    /// recreates the directory structure under `dest_base`, and downloads each
    /// file with an individual `GetFileData` connection. The `progress`
    /// callback, if given, receives aggregated `(bytes_received, total_bytes)`
    /// figures across the whole tree rather than per file.
    ///
    /// Relative paths from the listing are validated with [`safe_relative_join`]
    /// so a malicious peer cannot write outside `dest_base`.
    pub async fn download_dir(
        peer_addr: SocketAddr,
        dir_file_id: u32,
        dest_base: &Path,
        sender_name: &str,
        sender_host: &str,
        progress: Option<ProgressCallback>,
    ) -> Result<DirDownloadSummary, TransportError> {
        let listing =
            Self::fetch_dir_listing(peer_addr, dir_file_id, sender_name, sender_host).await?;

        tokio::fs::create_dir_all(dest_base).await?;

        let grand_total: u64 = listing
            .iter()
            .filter(|o| o.file_type != 2)
            .map(|o| o.size)
            .sum();

        let mut received_total: u64 = 0;
        let mut files = 0usize;
        let mut dirs = 0usize;

        // Bytes completed before the current file, so the per-file callback can
        // report an aggregated (prior_bytes + this_file_received) figure.
        let base = Arc::new(AtomicU64::new(0));

        for entry in &listing {
            let dest = safe_relative_join(dest_base, &entry.filename).ok_or_else(|| {
                TransportError::InvalidRequest(format!("unsafe path in listing: {}", entry.filename))
            })?;

            if entry.file_type == 2 {
                tokio::fs::create_dir_all(&dest).await?;
                dirs += 1;
                continue;
            }

            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            base.store(received_total, Ordering::Relaxed);
            let per_file: Option<ProgressCallback> = progress.as_ref().map(|cb| {
                let cb = cb.clone();
                let b = base.clone();
                let gt = grand_total;
                Arc::new(move |recv: u64, _this_total: u64| {
                    cb(b.load(Ordering::Relaxed) + recv, gt);
                }) as ProgressCallback
            });

            Self::download(
                peer_addr,
                entry.file_id,
                0,
                entry.size,
                &dest,
                sender_name,
                sender_host,
                per_file,
            )
            .await?;

            received_total += entry.size;
            files += 1;
        }

        // Report completion (100%) once at the end.
        if let Some(cb) = &progress {
            cb(received_total, grand_total);
        }

        info!(
            "Directory download complete: {} files, {} dirs, {} bytes to {:?}",
            files, dirs, received_total, dest_base
        );
        Ok(DirDownloadSummary {
            files,
            dirs,
            bytes: received_total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a unique temporary directory path (not yet created) for a test.
    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "flyq-transport-test-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn safe_relative_join_should_append_forward_slash_segments() {
        let base = Path::new("/dest");
        let joined = safe_relative_join(base, "sub/dir/file.txt").expect("valid rel path");
        assert_eq!(joined, Path::new("/dest/sub/dir/file.txt"));
    }

    #[test]
    fn safe_relative_join_should_append_backslash_segments() {
        let base = Path::new("/dest");
        let joined = safe_relative_join(base, "sub\\file.txt").expect("valid rel path");
        assert_eq!(joined, Path::new("/dest/sub/file.txt"));
    }

    #[test]
    fn safe_relative_join_should_ignore_empty_and_dot_segments() {
        let base = Path::new("/dest");
        let joined = safe_relative_join(base, "./sub//file.txt").expect("valid rel path");
        assert_eq!(joined, Path::new("/dest/sub/file.txt"));
    }

    #[test]
    fn safe_relative_join_should_reject_parent_traversal() {
        assert!(safe_relative_join(Path::new("/dest"), "../etc/passwd").is_none());
        assert!(safe_relative_join(Path::new("/dest"), "sub/../../x").is_none());
    }

    #[test]
    fn safe_relative_join_should_reject_drive_letter() {
        assert!(safe_relative_join(Path::new("/dest"), "C:/Windows/x").is_none());
        assert!(safe_relative_join(Path::new("/dest"), "sub/C:x").is_none());
    }

    #[test]
    fn file_offer_wire_record_should_roundtrip() {
        let offer = FileOffer {
            file_id: 42,
            filename: "report.pdf".to_string(),
            path: PathBuf::from("/tmp/report.pdf"),
            size: 4096,
            mtime: 1_700_000_000,
            file_type: 1,
        };
        let wire = offer.to_wire_record();
        let parsed = FileOffer::from_wire_record(&wire, Path::new("/base")).expect("valid record");
        assert_eq!(parsed.file_id, 42);
        assert_eq!(parsed.filename, "report.pdf");
        assert_eq!(parsed.size, 4096);
        assert_eq!(parsed.mtime, 1_700_000_000);
        assert_eq!(parsed.file_type, 1);
    }

    #[test]
    fn file_offer_wire_record_should_reject_malformed() {
        assert!(FileOffer::from_wire_record("1:only:three", Path::new("/b")).is_none());
        assert!(FileOffer::from_wire_record("x:name:1:2:1", Path::new("/b")).is_none());
    }

    #[tokio::test]
    async fn from_path_should_detect_directory_and_sum_recursive_size() {
        let root = unique_temp_dir("from-path");
        tokio::fs::create_dir_all(root.join("nested"))
            .await
            .expect("create tree");
        // 10 bytes at top level + 5 bytes nested = 15 bytes total.
        tokio::fs::write(root.join("a.txt"), b"0123456789")
            .await
            .expect("write a");
        tokio::fs::write(root.join("nested/b.txt"), b"01234")
            .await
            .expect("write b");

        let offer = FileOffer::from_path(7, &root).await.expect("probe dir");
        assert_eq!(offer.file_type, 2, "directories advertise file_type 2");
        assert_eq!(offer.size, 15, "size is the recursive byte total");

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn register_dir_recursive_should_list_nested_tree_with_relative_paths() {
        let root = unique_temp_dir("register-dir");
        tokio::fs::create_dir_all(root.join("sub/deeper"))
            .await
            .expect("create tree");
        tokio::fs::write(root.join("top.txt"), b"hello")
            .await
            .expect("write top");
        tokio::fs::write(root.join("sub/mid.txt"), b"world!")
            .await
            .expect("write mid");
        tokio::fs::write(root.join("sub/deeper/leaf.txt"), b"x")
            .await
            .expect("write leaf");

        let registry = FileRegistry::new();
        let dir_offer = registry.register(&root).await.expect("register root dir");
        assert_eq!(dir_offer.file_type, 2);

        let listing = registry
            .register_dir_recursive(dir_offer.file_id)
            .await
            .expect("recursive listing");

        // Collect (relative path, file_type, size) for assertions.
        let find = |name: &str| listing.iter().find(|e| e.filename == name).cloned();

        let sub = find("sub").expect("sub dir listed");
        assert_eq!(sub.file_type, 2);
        assert_eq!(sub.size, 0, "directory entries carry size 0");

        let top = find("top.txt").expect("top file listed");
        assert_eq!(top.file_type, 1);
        assert_eq!(top.size, 5);

        let mid = find("sub/mid.txt").expect("nested file listed with rel path");
        assert_eq!(mid.file_type, 1);
        assert_eq!(mid.size, 6);

        let deeper = find("sub/deeper").expect("deeper dir listed");
        assert_eq!(deeper.file_type, 2);

        let leaf = find("sub/deeper/leaf.txt").expect("deep leaf listed");
        assert_eq!(leaf.file_type, 1);
        assert_eq!(leaf.size, 1);

        // Every listed entry must have been inserted into the registry so the
        // receiver can subsequently pull each file via GetFileData.
        for entry in &listing {
            assert!(
                registry.get(entry.file_id).await.is_some(),
                "entry {} (id={}) not registered",
                entry.filename,
                entry.file_id
            );
        }

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn register_dir_recursive_should_reject_non_directory_id() {
        let root = unique_temp_dir("register-file-as-dir");
        tokio::fs::create_dir_all(&root).await.expect("create root");
        let file_path = root.join("solo.txt");
        tokio::fs::write(&file_path, b"data")
            .await
            .expect("write file");

        let registry = FileRegistry::new();
        let file_offer = registry.register(&file_path).await.expect("register file");
        assert_eq!(file_offer.file_type, 1);

        let err = registry
            .register_dir_recursive(file_offer.file_id)
            .await
            .expect_err("must reject a regular file id");
        assert!(matches!(err, TransportError::InvalidRequest(_)));

        tokio::fs::remove_dir_all(&root).await.ok();
    }
}
