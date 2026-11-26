//! Why a profile was refused as belonging to another codec.

use core::fmt;

use crate::codec::CodecId;
use crate::profile::CodecProfile;

/// Why a builder that takes a [`CodecProfile`] refused it.
///
/// A profile is a key for one codec only, so a request for one codec's
/// parameters, descriptor, configuration or preference cannot carry another
/// codec's profile.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    /// The profile belongs to a codec other than the one being described.
    Mismatch {
        /// The profile that was handed over.
        profile: CodecProfile,
        /// The codec the profile would have been recorded against.
        codec: CodecId,
    },
}

impl ProfileError {
    pub(crate) fn mismatch(profile: CodecProfile, codec: CodecId) -> Self {
        Self::Mismatch { profile, codec }
    }
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mismatch { profile, codec } => write!(
                f,
                "profile `{}` is not a profile of {}",
                profile.token(),
                codec.name()
            ),
        }
    }
}

impl core::error::Error for ProfileError {}
