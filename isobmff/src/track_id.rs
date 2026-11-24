//! The ISO/IEC 14496-12:2026 box-level track identity.
//!
//! [`TrackId`] is the `track_ID` the file carries (`tkhd`, `tfhd`, `trex`,
//! `prft`'s `reference_track_ID`). It is a thin newtype over the integer the
//! wire always carried, so a track id can no longer be passed where a sequence
//! number or timescale is expected; build one with [`TrackId::new`] and read it
//! back with [`TrackId::value`].
//!
//! `Debug` and `Display` render the bare number (no `TrackId(..)` wrapper) and
//! serde is `transparent`, so every golden `Debug` hash, JSON document, URL and
//! manifest that carries the integer stays byte-identical.

use core::fmt;

/// A track identifier: the ISO/IEC 14496-12:2026 `tkhd.track_ID` /
/// `tfhd.track_ID` (1-based, unique within a movie).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TrackId(u32);

impl TrackId {
    /// Wrap a raw track id.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The raw track id.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// The id following this one (`value() + 1`), for demuxers that hand out
    /// consecutive ids.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Debug for TrackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn render_as_bare_numbers() {
        assert_eq!(format!("{} {:?}", TrackId::new(7), TrackId::new(7)), "7 7");
        assert_eq!(format!("{:?}", Some(TrackId::new(3))), "Some(3)");
        assert_eq!(format!("{:04}", TrackId::new(5)), "0005");
        assert_eq!(TrackId::new(9).value(), 9);
        assert_eq!(TrackId::new(9).next(), TrackId::new(10));
    }
}
