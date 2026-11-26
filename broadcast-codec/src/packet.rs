//! One coded access unit.

use alloc::borrow::Cow;
use alloc::vec::Vec;

/// One coded access unit.
///
/// A decoder borrows the bytes (`Packet::borrowed`); an encoder hands ownership
/// back (`Packet::owned`). `pts` and `dts` are in the stream's time base, and
/// `key` marks a random-access point.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet<'a> {
    data: Cow<'a, [u8]>,
    pts: Option<i64>,
    dts: Option<i64>,
    key: bool,
}

impl<'a> Packet<'a> {
    /// A packet borrowing `data`, with no timestamps and not marked key.
    #[must_use]
    pub const fn borrowed(data: &'a [u8]) -> Self {
        Self {
            data: Cow::Borrowed(data),
            pts: None,
            dts: None,
            key: false,
        }
    }

    /// A packet owning `data`, with no timestamps and not marked key.
    #[must_use]
    pub const fn owned(data: Vec<u8>) -> Self {
        Self {
            data: Cow::Owned(data),
            pts: None,
            dts: None,
            key: false,
        }
    }

    /// Attach the presentation timestamp.
    #[must_use]
    pub const fn with_pts(mut self, pts: i64) -> Self {
        self.pts = Some(pts);
        self
    }

    /// Attach the decode timestamp.
    #[must_use]
    pub const fn with_dts(mut self, dts: i64) -> Self {
        self.dts = Some(dts);
        self
    }

    /// Mark the packet as, or not as, a random-access point.
    #[must_use]
    pub const fn with_key(mut self, key: bool) -> Self {
        self.key = key;
        self
    }

    /// The coded bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Presentation timestamp in the stream's time base, when known.
    #[must_use]
    pub const fn pts(&self) -> Option<i64> {
        self.pts
    }

    /// Decode timestamp in the stream's time base, when known.
    #[must_use]
    pub const fn dts(&self) -> Option<i64> {
        self.dts
    }

    /// Whether this is a random-access point.
    #[must_use]
    pub const fn key(&self) -> bool {
        self.key
    }

    /// Take ownership of the coded bytes, dropping the borrow.
    #[must_use]
    pub fn into_owned(self) -> Packet<'static> {
        Packet {
            data: Cow::Owned(self.data.into_owned()),
            pts: self.pts,
            dts: self.dts,
            key: self.key,
        }
    }
}
