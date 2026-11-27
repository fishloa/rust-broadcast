//! What a tolerant bitstream parser skipped, in the workspace's one damage
//! vocabulary.
//!
//! A parser in this crate that can continue past a broken structure does — it
//! drops the structure and keeps walking, so a damaged capture still plays.
//! `docs/API-CONVENTIONS.md`'s best-effort-reading rule makes a silent skip a
//! defect, so every such parser is handed a [`Findings`] collector and records
//! one [`ParseFinding`] per skip. A [`ParseFinding`] carries what was skipped
//! and, where the parser knows it, its offset within the bytes it was given;
//! [`From<ParseFinding> for Damage`] turns it into the caller's one report.
//!
//! The collector is a value, so the crate stays stateless: a caller that does
//! not want the findings passes a collector it drops.

use alloc::vec::Vec;
use broadcast_core::damage::{ByteRange, Damage, DamageCause, DamageKind, Severity};
use core::fmt;

use crate::error::Error;

/// One structure a tolerant parser skipped.
///
/// `#[non_exhaustive]`: a new tolerated parse failure adds a variant, not a
/// breaking change.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseFinding {
    /// A parameter set (SPS/PPS/VPS/slice header) was rejected: the reader
    /// could not turn it into codec configuration.
    ParameterSet {
        /// The structure in the codec's own terms, e.g. `"SPS"`.
        what: &'static str,
        /// The shared failure kind, when the parser reported one of them.
        cause: Option<DamageCause>,
    },
    /// A SEI NAL unit's messages could not be walked to its end.
    MalformedSei {
        /// Offset of the broken message within the bytes the parser was given.
        offset: usize,
    },
    /// HDR metadata (static `mdcv`/`clli` or HDR10+) did not parse.
    HdrMetadata {
        /// Offset of the broken payload within the bytes the parser was given.
        offset: usize,
    },
    /// The access-unit splitter could not delimit an access unit.
    AccessUnit {
        /// Offset of the bytes it dropped.
        offset: usize,
    },
    /// An audio frame header (ADTS, AC-3, E-AC-3, MPEG audio, DTS) did not
    /// parse, so the frame it belonged to was skipped.
    AudioFrameHeader {
        /// Offset of the first skipped byte within the bytes the parser was
        /// given.
        offset: usize,
    },
    /// A LAME (Xing/Info) tag did not parse.
    LameTag {
        /// Offset of the tag within the bytes the parser was given.
        offset: usize,
    },
}

impl ParseFinding {
    /// The finding a rejected parameter set produces.
    #[must_use]
    pub fn parameter_set(what: &'static str, error: &Error) -> Self {
        let cause = match error {
            Error::BufferTooShort(e) => Some(DamageCause::BufferTooShort(*e)),
            Error::InvalidValue(e) => Some(DamageCause::InvalidValue(*e)),
            Error::FieldOverflow(e) => Some(DamageCause::FieldOverflow(*e)),
            Error::OutputBufferTooSmall(_) => None,
            #[cfg(feature = "isobmff")]
            Error::Isobmff(_) => None,
            Error::InvalidInput(_) => None,
        };
        Self::ParameterSet { what, cause }
    }

    /// The finding a malformed SEI walk produces.
    #[must_use]
    pub const fn malformed_sei(offset: usize) -> Self {
        Self::MalformedSei { offset }
    }

    /// The finding an unparsable HDR metadata payload produces.
    #[must_use]
    pub const fn hdr_metadata(offset: usize) -> Self {
        Self::HdrMetadata { offset }
    }

    /// The finding an access unit the splitter cannot delimit produces.
    #[must_use]
    pub const fn access_unit(offset: usize) -> Self {
        Self::AccessUnit { offset }
    }

    /// The finding an unparsable audio frame header produces.
    #[must_use]
    pub const fn audio_frame_header(offset: usize) -> Self {
        Self::AudioFrameHeader { offset }
    }

    /// The finding an unparsable LAME tag produces.
    #[must_use]
    pub const fn lame_tag(offset: usize) -> Self {
        Self::LameTag { offset }
    }

    /// A short, stable label for this finding — used by `Display`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::ParameterSet { .. } => "parameter set",
            Self::MalformedSei { .. } => "malformed SEI",
            Self::HdrMetadata { .. } => "HDR metadata",
            Self::AccessUnit { .. } => "access unit",
            Self::AudioFrameHeader { .. } => "audio frame header",
            Self::LameTag { .. } => "LAME tag",
        }
    }

    /// Offset of the skipped structure within the bytes the parser was given,
    /// when the parser knows it.
    #[must_use]
    pub const fn offset(&self) -> Option<usize> {
        match self {
            Self::ParameterSet { .. } => None,
            Self::MalformedSei { offset }
            | Self::HdrMetadata { offset }
            | Self::AccessUnit { offset }
            | Self::AudioFrameHeader { offset }
            | Self::LameTag { offset } => Some(*offset),
        }
    }
}

/// A [`ParseFinding`] as the caller's one report
/// (`broadcast_core::damage::Damage`). A caller above this crate adds the
/// track and the absolute input location it knows; the finding supplies the
/// kind, the severity, the structure and the offset within its own bytes.
impl From<ParseFinding> for Damage {
    fn from(finding: ParseFinding) -> Self {
        let at = |offset: usize| ByteRange::new(offset as u64, 0);
        match finding {
            ParseFinding::ParameterSet { what, cause } => {
                let mut damage = Damage::new(DamageKind::MetadataSkipped, Severity::Degraded, what);
                if let Some(cause) = cause {
                    damage = damage.with_cause(cause);
                }
                damage
            }
            ParseFinding::MalformedSei { offset } => {
                Damage::new(DamageKind::MetadataSkipped, Severity::Informational, "SEI")
                    .at_byte_range(at(offset))
            }
            ParseFinding::HdrMetadata { offset } => Damage::new(
                DamageKind::MetadataSkipped,
                Severity::Informational,
                "HDR metadata",
            )
            .at_byte_range(at(offset)),
            ParseFinding::AccessUnit { offset } => Damage::new(
                DamageKind::Resynchronised,
                Severity::Degraded,
                "access unit",
            )
            .at_byte_range(at(offset)),
            ParseFinding::AudioFrameHeader { offset } => Damage::new(
                DamageKind::MediaUnitDropped,
                Severity::Degraded,
                "audio frame",
            )
            .at_byte_range(at(offset)),
            ParseFinding::LameTag { offset } => Damage::new(
                DamageKind::MetadataSkipped,
                Severity::Informational,
                "LAME tag",
            )
            .at_byte_range(at(offset)),
        }
    }
}

impl fmt::Display for ParseFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParameterSet { what, .. } => write!(f, "{} {what}", self.name()),
            Self::MalformedSei { offset }
            | Self::HdrMetadata { offset }
            | Self::AccessUnit { offset }
            | Self::AudioFrameHeader { offset }
            | Self::LameTag { offset } => write!(f, "{} at byte {offset}", self.name()),
        }
    }
}

/// The findings one parser call produced.
///
/// A caller that wants them reads [`items`](Self::items)/[`into_vec`](Self::into_vec);
/// one that does not passes a collector it drops.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Findings {
    items: Vec<ParseFinding>,
}

impl Findings {
    /// No findings.
    #[must_use]
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Record one finding.
    pub fn push(&mut self, finding: ParseFinding) {
        self.items.push(finding);
    }

    /// Whether nothing was skipped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many structures were skipped.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// The findings, in the order they were made.
    #[must_use]
    pub fn items(&self) -> &[ParseFinding] {
        &self.items
    }

    /// Take the findings out.
    #[must_use]
    pub fn into_vec(self) -> Vec<ParseFinding> {
        self.items
    }

    /// Append every finding of `other`, in order.
    pub fn absorb(&mut self, other: Self) {
        self.items.extend(other.items);
    }
}
