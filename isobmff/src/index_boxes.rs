//! Indexing and level boxes — ISO/IEC 14496-12:2026 §8.14.4, §8.8.13, §8.1.3,
//! §8.8.15.
//!
//! | Box | Type | Clause | Container |
//! |-----|------|--------|-----------|
//! | [`SubsegmentIndexBox`] | `ssix` | §8.14.4 | File |
//! | [`LevelAssignmentBox`] | `leva` | §8.8.13 | `mvex` |
//! | [`ProgressiveDownloadInfoBox`] | `pdin` | §8.1.3 | File |
//! | [`TrackExtensionPropertiesBox`] | `trep` | §8.8.15 | `mvex` |

use alloc::vec::Vec;

use broadcast_core::len::{fit_u8, need_in, need_out};
use broadcast_core::wire::{be_u32, u24_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK,
    box_len, write_full_box_header,
};
use crate::error::{Error, Result};

/// Four-CC of the Subsegment Index Box (`ssix`, ISO/IEC 14496-12:2026 §8.14.4).
pub const SSIX: [u8; BOX_TYPE_SIZE] = *b"ssix";
/// Four-CC of the Level Assignment Box (`leva`, §8.8.13).
pub const LEVA: [u8; BOX_TYPE_SIZE] = *b"leva";
/// Four-CC of the Progressive Download Information Box (`pdin`, §8.1.3).
pub const PDIN: [u8; BOX_TYPE_SIZE] = *b"pdin";
/// Four-CC of the Track Extension Properties Box (`trep`, §8.8.15).
pub const TREP: [u8; BOX_TYPE_SIZE] = *b"trep";

/// Width of a `range_size` field in bytes (`unsigned int(24)`).
const RANGE_SIZE_LEN: usize = 3;
/// Width of one `ssix` range record: `level`(8) + `range_size`(24).
const SSIX_RANGE_LEN: usize = 1 + RANGE_SIZE_LEN;
/// Fixed part of one `leva` entry: `track_ID` plus the packed flag byte.
const LEVA_LEVEL_HEADER_LEN: usize = core::mem::size_of::<u32>() + 1;
/// Width of one `pdin` `(rate, initial_delay)` pair.
const PDIN_ENTRY_LEN: usize = core::mem::size_of::<u32>() * 2;
/// Bits of `assignment_type` in `leva`.
const LEVA_ASSIGNMENT_BITS: u32 = 7;
/// `assignment_type` that maps a sample group to levels.
const LEVA_TYPE_GROUPING: u8 = 0;
/// `assignment_type` that maps a parameterised sample group to levels.
const LEVA_TYPE_GROUPING_PARAMETER: u8 = 1;
/// `assignment_type` for a track-level assignment.
const LEVA_TYPE_TRACK: u8 = 2;
/// The second `assignment_type` for a track-level assignment.
const LEVA_TYPE_TRACK_ALT: u8 = 3;
/// `assignment_type` for a sub-track assignment.
const LEVA_TYPE_SUB_TRACK: u8 = 4;

/// One byte-range assignment within a [`SubsegmentIndexBox`] (§8.14.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SubsegmentRange {
    level: u8,
    range_size: u32,
}

impl SubsegmentRange {
    /// Build one range.
    #[must_use]
    pub const fn new(level: u8, range_size: u32) -> Self {
        Self { level, range_size }
    }

    /// The level this range is assigned to.
    #[must_use]
    pub const fn level(&self) -> u8 {
        self.level
    }

    /// The range's size in bytes (`0` in the last entry means "to the end").
    #[must_use]
    pub const fn range_size(&self) -> u32 {
        self.range_size
    }
}

/// One subsegment of a [`SubsegmentIndexBox`] (§8.14.4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SubsegmentIndexEntry {
    ranges: Vec<SubsegmentRange>,
}

impl SubsegmentIndexEntry {
    /// Build one subsegment entry.
    #[must_use]
    pub fn new(ranges: Vec<SubsegmentRange>) -> Self {
        Self { ranges }
    }

    /// The partial-subsegment ranges, in wire order.
    #[must_use]
    pub fn ranges(&self) -> &[SubsegmentRange] {
        &self.ranges
    }
}

/// Subsegment Index Box (`ssix`) — ISO/IEC 14496-12:2026 §8.14.4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SubsegmentIndexBox {
    version: u8,
    flags: u32,
    subsegments: Vec<SubsegmentIndexEntry>,
}

impl SubsegmentIndexBox {
    /// Build one from its entries.
    #[must_use]
    pub fn new(subsegments: Vec<SubsegmentIndexEntry>) -> Self {
        Self {
            version: 0,
            flags: 0,
            subsegments,
        }
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The indexed subsegments, in wire order.
    #[must_use]
    pub fn subsegments(&self) -> &[SubsegmentIndexEntry] {
        &self.subsegments
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "ssix body",
        )?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let count = be_u32(body, c, "ssix subsegment_count")? as usize;
        c += core::mem::size_of::<u32>();
        let mut subsegments = Vec::with_capacity(crate::box_support::bounded_entry_count(
            body.len().saturating_sub(c),
            core::mem::size_of::<u32>(),
            count,
        ));
        for _ in 0..count {
            need_in(body, c + core::mem::size_of::<u32>(), "ssix range_count")?;
            let range_count = be_u32(body, c, "ssix range_count")? as usize;
            c += core::mem::size_of::<u32>();
            let mut ranges = Vec::with_capacity(crate::box_support::bounded_entry_count(
                body.len().saturating_sub(c),
                SSIX_RANGE_LEN,
                range_count,
            ));
            for _ in 0..range_count {
                need_in(body, c + SSIX_RANGE_LEN, "ssix range")?;
                let level = body[c];
                let range_size = u24_at(body, c + 1);
                c += SSIX_RANGE_LEN;
                ranges.push(SubsegmentRange { level, range_size });
            }
            subsegments.push(SubsegmentIndexEntry { ranges });
        }
        Ok(Self {
            version,
            flags,
            subsegments,
        })
    }
}

impl<'a> Parse<'a> for SubsegmentIndexBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "ssix box")? != u32::from_be_bytes(SSIX) {
            return Err(Error::InvalidInput("expected ssix"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "ssix box",
        )?)
    }
}

impl Serialize for SubsegmentIndexBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        for subsegment in &self.subsegments {
            n += core::mem::size_of::<u32>() + subsegment.ranges.len() * SSIX_RANGE_LEN;
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &SSIX, need, self.version, self.flags, false)?;
        let count = broadcast_core::len::fit_u32(self.subsegments.len(), "subsegment_count")?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&count.to_be_bytes());
        c += core::mem::size_of::<u32>();
        for subsegment in &self.subsegments {
            let range_count = broadcast_core::len::fit_u32(subsegment.ranges.len(), "range_count")?;
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&range_count.to_be_bytes());
            c += core::mem::size_of::<u32>();
            for range in &subsegment.ranges {
                buf[c] = range.level;
                buf[c + 1..c + 1 + RANGE_SIZE_LEN]
                    .copy_from_slice(&range.range_size.to_be_bytes()[1..]);
                c += SSIX_RANGE_LEN;
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// LevelAssignmentBox — leva (§8.8.13)
// ---------------------------------------------------------------------------

/// The `assignment_type` of one [`LevelAssignmentEntry`] (§8.8.13.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LevelAssignmentType {
    /// `assignment_type == 0`: a sample group maps samples to levels.
    SampleGroup,
    /// `assignment_type == 1`: a parameterised sample group maps samples.
    ParameterizedSampleGroup,
    /// `assignment_type == 2`: assignment by track.
    Track,
    /// `assignment_type == 3`: the other track-level assignment.
    TrackAlternative,
    /// `assignment_type == 4`: assignment by sub-track.
    SubTrack,
    /// Any other value (reserved by the standard).
    Reserved(u8),
}

impl LevelAssignmentType {
    /// Spec token for this assignment type.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::SampleGroup => "sample-group",
            Self::ParameterizedSampleGroup => "parameterized-sample-group",
            Self::Track => "track",
            Self::TrackAlternative => "track-alternative",
            Self::SubTrack => "sub-track",
            Self::Reserved(_) => "reserved",
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::SampleGroup => LEVA_TYPE_GROUPING,
            Self::ParameterizedSampleGroup => LEVA_TYPE_GROUPING_PARAMETER,
            Self::Track => LEVA_TYPE_TRACK,
            Self::TrackAlternative => LEVA_TYPE_TRACK_ALT,
            Self::SubTrack => LEVA_TYPE_SUB_TRACK,
            Self::Reserved(code) => *code,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            LEVA_TYPE_GROUPING => Self::SampleGroup,
            LEVA_TYPE_GROUPING_PARAMETER => Self::ParameterizedSampleGroup,
            LEVA_TYPE_TRACK => Self::Track,
            LEVA_TYPE_TRACK_ALT => Self::TrackAlternative,
            LEVA_TYPE_SUB_TRACK => Self::SubTrack,
            other => Self::Reserved(other),
        }
    }
}

broadcast_core::impl_spec_display!(LevelAssignmentType, Reserved);

/// Mask of the `padding_flag` bit in the packed `leva` entry byte.
const LEVA_PADDING_MASK: u8 = 1 << LEVA_ASSIGNMENT_BITS;
/// Mask of the `assignment_type` bits in the packed `leva` entry byte.
const LEVA_ASSIGNMENT_MASK: u8 = !LEVA_PADDING_MASK;

/// One level of a [`LevelAssignmentBox`] (§8.8.13.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LevelAssignmentEntry {
    track_id: u32,
    padding_flag: bool,
    assignment: LevelAssignmentType,
    grouping_type: Option<u32>,
    grouping_type_parameter: Option<u32>,
    sub_track_id: Option<u32>,
}

impl LevelAssignmentEntry {
    /// Build one entry for the given track and assignment type.
    #[must_use]
    pub fn new(track_id: u32, padding_flag: bool, assignment: LevelAssignmentType) -> Self {
        Self {
            track_id,
            padding_flag,
            assignment,
            grouping_type: None,
            grouping_type_parameter: None,
            sub_track_id: None,
        }
    }

    /// Set the sample-group keys for a grouping assignment.
    #[must_use]
    pub fn with_grouping(mut self, grouping_type: u32, parameter: Option<u32>) -> Self {
        self.grouping_type = Some(grouping_type);
        self.grouping_type_parameter = parameter;
        self
    }

    /// Set the `sub_track_ID` for a sub-track assignment.
    #[must_use]
    pub fn with_sub_track_id(mut self, sub_track_id: u32) -> Self {
        self.sub_track_id = Some(sub_track_id);
        self
    }

    /// The track this level is assigned to.
    #[must_use]
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// The `padding_flag`.
    #[must_use]
    pub const fn padding_flag(&self) -> bool {
        self.padding_flag
    }

    /// The assignment type.
    #[must_use]
    pub const fn assignment(&self) -> LevelAssignmentType {
        self.assignment
    }

    /// The `grouping_type` for a grouping assignment.
    #[must_use]
    pub const fn grouping_type(&self) -> Option<u32> {
        self.grouping_type
    }

    /// The `grouping_type_parameter` for a parameterised grouping assignment.
    #[must_use]
    pub const fn grouping_type_parameter(&self) -> Option<u32> {
        self.grouping_type_parameter
    }

    /// The `sub_track_ID` for a sub-track assignment.
    #[must_use]
    pub const fn sub_track_id(&self) -> Option<u32> {
        self.sub_track_id
    }
}

/// Level Assignment Box (`leva`) — ISO/IEC 14496-12:2026 §8.8.13.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LevelAssignmentBox {
    version: u8,
    flags: u32,
    levels: Vec<LevelAssignmentEntry>,
}

impl LevelAssignmentBox {
    /// Build one from its level entries.
    #[must_use]
    pub fn new(levels: Vec<LevelAssignmentEntry>) -> Self {
        Self {
            version: 0,
            flags: 0,
            levels,
        }
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The level entries, in wire order.
    #[must_use]
    pub fn levels(&self) -> &[LevelAssignmentEntry] {
        &self.levels
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + 1, "leva body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let count = body[c] as usize;
        c += 1;
        let mut levels = Vec::with_capacity(count);
        for _ in 0..count {
            need_in(body, c + LEVA_LEVEL_HEADER_LEN, "leva level entry")?;
            let track_id = be_u32(body, c, "leva track_ID")?;
            c += core::mem::size_of::<u32>();
            let padding_flag = body[c] & LEVA_PADDING_MASK != 0;
            let assignment = LevelAssignmentType::from_code(body[c] & LEVA_ASSIGNMENT_MASK);
            c += 1;
            let mut entry = LevelAssignmentEntry {
                track_id,
                padding_flag,
                assignment,
                grouping_type: None,
                grouping_type_parameter: None,
                sub_track_id: None,
            };
            match assignment {
                LevelAssignmentType::SampleGroup => {
                    need_in(body, c + core::mem::size_of::<u32>(), "leva grouping_type")?;
                    entry.grouping_type = Some(be_u32(body, c, "leva grouping_type")?);
                    c += core::mem::size_of::<u32>();
                }
                LevelAssignmentType::ParameterizedSampleGroup => {
                    need_in(body, c + PDIN_ENTRY_LEN, "leva grouping_type_parameter")?;
                    entry.grouping_type = Some(be_u32(body, c, "leva grouping_type")?);
                    c += core::mem::size_of::<u32>();
                    entry.grouping_type_parameter = Some(be_u32(body, c, "leva parameter")?);
                    c += core::mem::size_of::<u32>();
                }
                LevelAssignmentType::SubTrack => {
                    need_in(body, c + core::mem::size_of::<u32>(), "leva sub_track_ID")?;
                    entry.sub_track_id = Some(be_u32(body, c, "leva sub_track_ID")?);
                    c += core::mem::size_of::<u32>();
                }
                _ => {}
            }
            levels.push(entry);
        }
        Ok(Self {
            version,
            flags,
            levels,
        })
    }
}

impl<'a> Parse<'a> for LevelAssignmentBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "leva box")? != u32::from_be_bytes(LEVA) {
            return Err(Error::InvalidInput("expected leva"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + 1,
            "leva box",
        )?)
    }
}

impl Serialize for LevelAssignmentBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + 1;
        for level in &self.levels {
            n += LEVA_LEVEL_HEADER_LEN;
            match level.assignment {
                LevelAssignmentType::SampleGroup => n += core::mem::size_of::<u32>(),
                LevelAssignmentType::ParameterizedSampleGroup => n += PDIN_ENTRY_LEN,
                LevelAssignmentType::SubTrack => n += core::mem::size_of::<u32>(),
                _ => {}
            }
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &LEVA, need, self.version, self.flags, false)?;
        let count = fit_u8(self.levels.len(), "level_count")?;
        buf[c] = count;
        c += 1;
        for level in &self.levels {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&level.track_id.to_be_bytes());
            c += core::mem::size_of::<u32>();
            let mut first = level.assignment.code();
            if level.padding_flag {
                first |= LEVA_PADDING_MASK;
            }
            buf[c] = first;
            c += 1;
            match level.assignment {
                LevelAssignmentType::SampleGroup => {
                    let grouping_type = level
                        .grouping_type
                        .ok_or(Error::InvalidInput("leva missing grouping_type"))?;
                    buf[c..c + core::mem::size_of::<u32>()]
                        .copy_from_slice(&grouping_type.to_be_bytes());
                    c += core::mem::size_of::<u32>();
                }
                LevelAssignmentType::ParameterizedSampleGroup => {
                    let grouping_type = level
                        .grouping_type
                        .ok_or(Error::InvalidInput("leva missing grouping_type"))?;
                    buf[c..c + core::mem::size_of::<u32>()]
                        .copy_from_slice(&grouping_type.to_be_bytes());
                    c += core::mem::size_of::<u32>();
                    let parameter = level
                        .grouping_type_parameter
                        .ok_or(Error::InvalidInput("leva missing grouping_type_parameter"))?;
                    buf[c..c + core::mem::size_of::<u32>()]
                        .copy_from_slice(&parameter.to_be_bytes());
                    c += core::mem::size_of::<u32>();
                }
                LevelAssignmentType::SubTrack => {
                    let sub_track_id = level
                        .sub_track_id
                        .ok_or(Error::InvalidInput("leva missing sub_track_ID"))?;
                    buf[c..c + core::mem::size_of::<u32>()]
                        .copy_from_slice(&sub_track_id.to_be_bytes());
                    c += core::mem::size_of::<u32>();
                }
                _ => {}
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// ProgressiveDownloadInfoBox — pdin (§8.1.3)
// ---------------------------------------------------------------------------

/// One `(rate, initial_delay)` pair of a [`ProgressiveDownloadInfoBox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ProgressiveDownloadEntry {
    rate: u32,
    initial_delay: u32,
}

impl ProgressiveDownloadEntry {
    /// Build one entry.
    #[must_use]
    pub const fn new(rate: u32, initial_delay: u32) -> Self {
        Self {
            rate,
            initial_delay,
        }
    }

    /// The download rate in bytes per second.
    #[must_use]
    pub const fn rate(&self) -> u32 {
        self.rate
    }

    /// The suggested initial delay in milliseconds.
    #[must_use]
    pub const fn initial_delay(&self) -> u32 {
        self.initial_delay
    }
}

/// Progressive Download Information Box (`pdin`) — ISO/IEC 14496-12:2026 §8.1.3.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ProgressiveDownloadInfoBox {
    version: u8,
    flags: u32,
    entries: Vec<ProgressiveDownloadEntry>,
}

impl ProgressiveDownloadInfoBox {
    /// Build one from its entries.
    #[must_use]
    pub fn new(entries: Vec<ProgressiveDownloadEntry>) -> Self {
        Self {
            version: 0,
            flags: 0,
            entries,
        }
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The `(rate, initial_delay)` pairs.
    #[must_use]
    pub fn entries(&self) -> &[ProgressiveDownloadEntry] {
        &self.entries
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "pdin body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let rest = &body[FULLBOX_EXTRA_SIZE..];
        let mut entries = Vec::with_capacity(rest.len() / PDIN_ENTRY_LEN);
        for chunk in rest.chunks_exact(PDIN_ENTRY_LEN) {
            entries.push(ProgressiveDownloadEntry {
                rate: be_u32(chunk, 0, "pdin rate")?,
                initial_delay: be_u32(chunk, core::mem::size_of::<u32>(), "pdin initial_delay")?,
            });
        }
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl<'a> Parse<'a> for ProgressiveDownloadInfoBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "pdin box")? != u32::from_be_bytes(PDIN) {
            return Err(Error::InvalidInput("expected pdin"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "pdin box",
        )?)
    }
}

impl Serialize for ProgressiveDownloadInfoBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.entries.len() * PDIN_ENTRY_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &PDIN, need, self.version, self.flags, false)?;
        for entry in &self.entries {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&entry.rate.to_be_bytes());
            buf[c + core::mem::size_of::<u32>()..c + PDIN_ENTRY_LEN]
                .copy_from_slice(&entry.initial_delay.to_be_bytes());
            c += PDIN_ENTRY_LEN;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// TrackExtensionPropertiesBox — trep (§8.8.15)
// ---------------------------------------------------------------------------

/// Track Extension Properties Box (`trep`) — ISO/IEC 14496-12:2026 §8.8.15.
///
/// `track_ID` followed by any number of child boxes, which this crate carries
/// opaque so a private property box round-trips.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TrackExtensionPropertiesBox {
    version: u8,
    flags: u32,
    track_id: u32,
    children: Vec<OpaqueBox>,
}

impl TrackExtensionPropertiesBox {
    /// Build one for `track_id`.
    #[must_use]
    pub fn new(track_id: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            track_id,
            children: Vec::new(),
        }
    }

    /// Append one opaque child.
    #[must_use]
    pub fn with_child(mut self, child: OpaqueBox) -> Self {
        self.children.push(child);
        self
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The track these properties describe.
    #[must_use]
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// The child boxes.
    #[must_use]
    pub fn children(&self) -> &[OpaqueBox] {
        &self.children
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "trep body",
        )?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let track_id = be_u32(body, FULLBOX_EXTRA_SIZE, "trep track_ID")?;
        let mut children = Vec::new();
        for child in crate::box_types::BoxIter::new(&body[FULLBOX_EXTRA_SIZE + 4..]) {
            let child = child?;
            children.push(OpaqueBox {
                box_type: child.four_cc(),
                data: child.payload().to_vec(),
                to_end: child.to_end(),
                largesize: child.has_largesize(),
            });
        }
        Ok(Self {
            version,
            flags,
            track_id,
            children,
        })
    }
}

impl<'a> Parse<'a> for TrackExtensionPropertiesBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "trep box")? != u32::from_be_bytes(TREP) {
            return Err(Error::InvalidInput("expected trep"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "trep box",
        )?)
    }
}

impl Serialize for TrackExtensionPropertiesBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        for child in &self.children {
            n += child.serialized_len();
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &TREP, need, self.version, self.flags, false)?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.track_id.to_be_bytes());
        c += core::mem::size_of::<u32>();
        for child in &self.children {
            c += child.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}
