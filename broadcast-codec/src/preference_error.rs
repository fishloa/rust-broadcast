//! Why a preference's text form was refused.

use alloc::string::String;
use core::fmt;

/// Why a [`Preference`](crate::Preference) text form was refused.
///
/// Every variant names the token it refused, so a command-line error says
/// which part of which preference is wrong.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreferenceError {
    /// A preference has no `=`: `aac`.
    MissingEquals {
        /// The whole preference text.
        text: String,
    },
    /// A direction prefix is neither `dec` nor `enc`: `decx:aac=libav`.
    UnknownDirection {
        /// The prefix.
        prefix: String,
    },
    /// A codec token names no codec: `foo=libav`.
    UnknownCodec {
        /// The codec token.
        token: String,
    },
    /// The provider part is empty: `aac=`.
    EmptyProvider,
    /// The implementation part is empty: `aac=libav:`.
    EmptyImplementation,
    /// A character is not allowed in a name: `aac=lib av`.
    InvalidCharacter {
        /// The token the character is in.
        token: String,
        /// The character.
        character: char,
    },
    /// A comma-separated list has an empty entry.
    EmptyEntry {
        /// Zero-based position of the empty entry.
        entry: usize,
    },
}

impl PreferenceError {
    pub(crate) fn missing_equals(text: &str) -> Self {
        Self::MissingEquals { text: text.into() }
    }

    pub(crate) fn unknown_direction(prefix: &str) -> Self {
        Self::UnknownDirection {
            prefix: prefix.into(),
        }
    }

    pub(crate) fn unknown_codec(token: &str) -> Self {
        Self::UnknownCodec {
            token: token.into(),
        }
    }

    pub(crate) fn invalid_character(token: &str, character: char) -> Self {
        Self::InvalidCharacter {
            token: token.into(),
            character,
        }
    }
}

impl fmt::Display for PreferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEquals { text } => write!(f, "preference `{text}` has no `=`"),
            Self::UnknownDirection { prefix } => {
                write!(
                    f,
                    "preference direction `{prefix}` is neither `dec` nor `enc`"
                )
            }
            Self::UnknownCodec { token } => {
                write!(f, "preference codec token `{token}` names no codec")
            }
            Self::EmptyProvider => f.write_str("preference has an empty provider"),
            Self::EmptyImplementation => f.write_str("preference has an empty implementation"),
            Self::InvalidCharacter { token, character } => {
                write!(f, "character `{character}` is not allowed in `{token}`")
            }
            Self::EmptyEntry { entry } => {
                write!(f, "preference list entry {entry} is empty")
            }
        }
    }
}

impl core::error::Error for PreferenceError {}
