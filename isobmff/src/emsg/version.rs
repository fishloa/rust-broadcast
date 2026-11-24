//! The `emsg` `FullBox` version field — DASH-IF IOP Part 10 §6.1 / Table 6-2.
//!
//! Per Table 6-2 (`emsg.version`): "version 0 is used for segment relative
//! timing, version 1 for representation relative timing." The two versions
//! carry the same logical field set except for the presentation-time field:
//!
//! - **version 0** — `presentation_time_delta` (u32), relative to the
//!   segment's earliest presentation time;
//! - **version 1** — `presentation_time` (u64), relative to `Period@start`.
//!
//! The box body field *ordering* also differs between the two versions — see
//! the crate root caveat and [`crate::EmsgBox`].

/// The `version` byte of the `'emsg'` `FullBox` (DASH-IF Part 10 Table 6-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EmsgVersion {
    /// Version 0 — segment-relative timing; carries `presentation_time_delta`
    /// (u32). Strings precede the integer fields on the wire.
    SegmentRelative,
    /// Version 1 — representation/`Period@start`-relative timing; carries
    /// `presentation_time` (u64). Integer fields precede the strings on the
    /// wire.
    RepresentationRelative,
}

/// The wire value of `version` for [`EmsgVersion::SegmentRelative`].
pub const VERSION_0: u8 = 0;
/// The wire value of `version` for [`EmsgVersion::RepresentationRelative`].
pub const VERSION_1: u8 = 1;

impl EmsgVersion {
    /// Decode the `FullBox` `version` byte; only 0 and 1 are defined.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            VERSION_0 => Some(EmsgVersion::SegmentRelative),
            VERSION_1 => Some(EmsgVersion::RepresentationRelative),
            _ => None,
        }
    }

    /// The `version` byte as it appears on the wire.
    pub fn to_u8(self) -> u8 {
        match self {
            EmsgVersion::SegmentRelative => VERSION_0,
            EmsgVersion::RepresentationRelative => VERSION_1,
        }
    }

    /// Spec label for this version.
    pub fn name(&self) -> &'static str {
        match self {
            EmsgVersion::SegmentRelative => "segment relative (v0)",
            EmsgVersion::RepresentationRelative => "representation relative (v1)",
        }
    }
}

broadcast_common::impl_spec_display!(EmsgVersion);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn round_trips() {
        assert_eq!(EmsgVersion::from_u8(0), Some(EmsgVersion::SegmentRelative));
        assert_eq!(
            EmsgVersion::from_u8(1),
            Some(EmsgVersion::RepresentationRelative)
        );
        assert_eq!(EmsgVersion::from_u8(2), None);
        assert_eq!(EmsgVersion::SegmentRelative.to_u8(), 0);
        assert_eq!(EmsgVersion::RepresentationRelative.to_u8(), 1);
    }

    #[test]
    fn display_uses_name() {
        assert_eq!(
            EmsgVersion::SegmentRelative.to_string(),
            "segment relative (v0)"
        );
    }
}
