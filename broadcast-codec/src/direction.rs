//! Which way an implementation converts.

/// Which way an implementation converts.
///
/// A [`Preference`](crate::Preference) written as text may carry `dec:` or
/// `enc:` in front of its codec token to name one direction; without it the
/// preference applies to both.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Coded packets in, decoded frames out.
    Decode,
    /// Decoded frames in, coded packets out.
    Encode,
}

impl Direction {
    /// The preference prefix for [`Self::Decode`].
    const DECODE_PREFIX: &'static str = "dec";
    /// The preference prefix for [`Self::Encode`].
    const ENCODE_PREFIX: &'static str = "enc";

    /// Label of the direction.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Decode => "decode",
            Self::Encode => "encode",
        }
    }

    /// The direction a preference's `dec`/`enc` prefix names.
    pub(crate) fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            Self::DECODE_PREFIX => Some(Self::Decode),
            Self::ENCODE_PREFIX => Some(Self::Encode),
            _ => None,
        }
    }

    /// The prefix that names this direction.
    pub(crate) const fn prefix(&self) -> &'static str {
        match self {
            Self::Decode => Self::DECODE_PREFIX,
            Self::Encode => Self::ENCODE_PREFIX,
        }
    }
}

broadcast_core::impl_spec_display!(Direction);
