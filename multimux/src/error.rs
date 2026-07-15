//! Error type for multimux.

use thiserror::Error;

/// Errors from configuration, RTSP ingest, segmentation, or the HTTP origin.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MultimuxError {
    /// Config file could not be read or parsed.
    #[error("config: {0}")]
    Config(String),
    /// An RTSP/transport/SDP failure while pulling a source.
    #[error("source: {0}")]
    Source(String),
    /// A `transmux` segmentation/depayload error.
    #[error("transmux: {0}")]
    Transmux(#[from] transmux::Error),
    /// An I/O error (socket, bind).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// multimux result alias.
pub type Result<T> = core::result::Result<T, MultimuxError>;
