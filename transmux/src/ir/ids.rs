//! Typed identifiers of the IR (issue #1159): [`TrackId`] and [`ProgramNumber`].
//!
//! Both are thin newtypes over the integer the wire/IR always carried, so a
//! track id can no longer be passed where a sequence number, timescale or
//! program number is expected. Neither has a silent `From<integer>`: build one
//! with `new` and read it back with `value`, so every crossing between the
//! typed and the raw world is visible.
//!
//! `Debug` and `Display` render the bare number (no `TrackId(..)` wrapper) and
//! serde is `transparent`, so every golden `Debug` hash, JSON document, URL and
//! manifest that used to carry the integer is byte-identical.

use core::fmt;

/// A track identifier: the ISO/IEC 14496-12 `tkhd.track_ID` /
/// `tfhd.track_ID` (1-based, unique within a movie), or the id a non-ISOBMFF
/// demuxer assigned to a track. Distinct from `media_plane`'s `ProgramId`.
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

/// An MPEG-2 TS `program_number` (ISO/IEC 13818-1 §2.4.4.3): the PAT/PMT
/// program identifier, 16 bits on the wire. Distinct from [`TrackId`] and from
/// `media_plane`'s `ProgramId`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct ProgramNumber(u16);

impl ProgramNumber {
    /// Wrap a raw `program_number`.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// The raw `program_number`.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

impl fmt::Debug for ProgramNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for ProgramNumber {
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
        assert_eq!(
            format!(
                "{} {:?}",
                ProgramNumber::new(0x401),
                ProgramNumber::new(0x401)
            ),
            "1025 1025"
        );
        assert_eq!(TrackId::new(9).value(), 9);
        assert_eq!(ProgramNumber::new(9).value(), 9);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_is_transparent() {
        assert_eq!(serde_json::to_string(&TrackId::new(4)).unwrap(), "4");
        assert_eq!(
            serde_json::from_str::<TrackId>("4").unwrap(),
            TrackId::new(4)
        );
        assert_eq!(serde_json::to_string(&ProgramNumber::new(1)).unwrap(), "1");
    }
}
