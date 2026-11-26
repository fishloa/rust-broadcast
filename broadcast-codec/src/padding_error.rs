//! Why a padding request was refused.

use core::fmt;

/// Why a padding builder refused its request.
///
/// Padding counts decoded audio samples, so a [`CodecParameters`] that
/// describes video cannot carry it.
///
/// [`CodecParameters`]: crate::CodecParameters
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaddingError {
    /// The parameters describe video, and padding is audio-only.
    Video,
}

impl fmt::Display for PaddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Video => f.write_str("padding is audio-only and these parameters describe video"),
        }
    }
}

impl core::error::Error for PaddingError {}
