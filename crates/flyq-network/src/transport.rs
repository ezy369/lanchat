//! TCP transport for reliable message delivery and file transfer.

use thiserror::Error;
use tokio::net::TcpListener;
use std::net::SocketAddr;
use tracing::info;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Connection closed")]
    ConnectionClosed,
}

/// TCP transport layer for file transfers and reliable messaging.
pub struct Transport {
    listener: TcpListener,
}

impl Transport {
    /// Bind a TCP listener on the given address.
    pub async fn bind(addr: SocketAddr) -> Result<Self, TransportError> {
        let listener = TcpListener::bind(addr).await?;
        info!("TCP transport listening on {}", addr);
        Ok(Self { listener })
    }

    /// Accept an incoming connection.
    pub async fn accept(&self) -> Result<(tokio::net::TcpStream, SocketAddr), TransportError> {
        let (stream, addr) = self.listener.accept().await?;
        Ok((stream, addr))
    }

    /// Connect to a remote peer.
    pub async fn connect(addr: SocketAddr) -> Result<tokio::net::TcpStream, TransportError> {
        let stream = tokio::net::TcpStream::connect(addr).await?;
        Ok(stream)
    }
}
