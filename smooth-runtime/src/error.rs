//! The engine's fatal outcome: [`SmoothError`].

use thiserror::Error;

use crate::client::ManifestFault;

/// Why a [`crate::SmoothClient`] stopped for good.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SmoothError {
    /// The manifest declares a `Protection` element (PlayReady/PIFF sample
    /// encryption); decrypting it is out of scope (MS-SSTR §2.2.2.2).
    #[error("manifest declares a Protection element; protected content is not supported")]
    ManifestProtected,
    /// A fragment carries PIFF/CENC sample-encryption boxes.
    #[error("fragment carries sample-encryption boxes; protected content is not supported")]
    FragmentEncrypted,
    /// The manifest could not be used and the retry budget is spent.
    #[error("manifest unusable after retries: {0}")]
    Manifest(ManifestFault),
    /// The manifest has no Video or Audio `StreamIndex` with a `QualityLevel`.
    #[error("manifest resolves no usable video or audio stream")]
    NoUsableStream,
    /// A stream's chunk timeline could not be enumerated.
    #[error("chunk timeline of stream {stream} is unusable: {reason}")]
    Timeline {
        /// Index of the stream in the manifest's stream order.
        stream: usize,
        /// The manifest parser's error, rendered.
        reason: String,
    },
    /// A fragment kept failing on a static presentation.
    #[error("fragment fetch failed (status {status:?})")]
    Fragment {
        /// The final HTTP status, when the failure was an HTTP one.
        status: Option<u16>,
    },
    /// A track could not be described or its init segment could not be built.
    #[error("track setup failed: {reason}")]
    TrackSetup {
        /// The codec-glue error, rendered.
        reason: String,
    },
    /// The configuration's retry schedule is not one a client can run with.
    #[error("invalid retry backoff")]
    InvalidBackoff(broadcast_common::backoff::InvalidBackoff),
}
