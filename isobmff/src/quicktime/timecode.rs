//! The QuickTime timecode sample entry (`tmcd`) and its samples — read and written.
//!
//! Source: `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`,
//! "Timecode sample description" and "Timecode media"; the sample-data record
//! layout is in `private/specs-verified/apple/isobmff/quicktime-file-format-2001.md`,
//! "Timecode Sample Data".

use alloc::vec::Vec;
use broadcast_common::wire::{u16_at, u32_at};
use broadcast_core::Parse;
use broadcast_core::Serialize;
use broadcast_core::len::{fit_u16, need_in, need_out};

use crate::box_support::OpaqueBox;
use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, BoxIter};
use crate::error::{Error, Result};

/// Four-CC of the timecode sample entry.
pub const TMCD: [u8; BOX_TYPE_SIZE] = *b"tmcd";
/// Four-CC of the optional source-tape name atom.
pub const NAME: [u8; BOX_TYPE_SIZE] = *b"name";

/// The SampleEntry `reserved[6]` width.
const SAMPLE_ENTRY_RESERVED_LEN: usize = 6;
/// Offset of the `data_reference_index` field.
const DATA_REFERENCE_INDEX_OFFSET: usize = 6;
/// Offset of the reserved 32-bit field.
const RESERVED_1_OFFSET: usize = 8;
/// Bytes of a `tmcd` body through `number_of_frames`.
const TMCD_FIXED_BODY_LEN: usize = 25;
/// Bytes of a `tmcd` body including the 24-bit reserved field.
const TMCD_RESERVED_BODY_LEN: usize = 28;
/// Offset of the flags field inside a `tmcd` body.
const FLAGS_OFFSET: usize = 12;
/// Offset of the time scale field inside a `tmcd` body.
const TIMESCALE_OFFSET: usize = 16;
/// Offset of the frame duration field inside a `tmcd` body.
const FRAME_DURATION_OFFSET: usize = 20;
/// Offset of the number-of-frames field inside a `tmcd` body.
const NUMBER_OF_FRAMES_OFFSET: usize = 24;
/// The `Counter` flag (0x0008).
pub const COUNTER_FLAG: u32 = 0x0008;
/// The `Drop frame` flag (0x0001).
pub const DROP_FRAME_FLAG: u32 = 0x0001;
/// Minutes per hour.
const MINUTES_PER_HOUR: u64 = 60;
/// Seconds per minute.
const SECONDS_PER_MINUTE: u64 = 60;
/// The sign bit of the minutes byte in the record form.
const NEGATIVE_BIT: u8 = 0x80;
/// The minutes mask in the record form.
const MINUTES_MASK: u8 = 0x7F;
/// The largest valid seconds value in the record form.
const MAX_SECONDS: u8 = 59;
/// The largest valid minutes value in the record form.
const MAX_MINUTES: u8 = 59;
/// Bytes of one decoded timecode sample.
const TMCD_SAMPLE_LEN: usize = 4;
/// Byte offset of the seconds field.
const SECONDS_BYTE_OFFSET: usize = 2;
/// Byte offset of the frames field.
const FRAMES_BYTE_OFFSET: usize = 3;
/// Bytes of a `name` atom's string length prefix.
const LENGTH_PREFIX_LEN: usize = 2;

/// A QuickTime timecode sample description.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TimecodeEntry {
    /// The SampleEntry `reserved[6]` bytes, kept verbatim.
    pub reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    /// `data_reference_index`.
    pub data_reference_index: u16,
    /// The reserved 32-bit field before `flags`.
    pub reserved_1: u32,
    /// The timecode characteristics flags.
    pub flags: u32,
    /// The timescale the frame duration is expressed in.
    pub timescale: u32,
    /// How long one frame lasts, in `timescale` units.
    pub frame_duration: u32,
    /// Frames per second of the timecode format.
    pub number_of_frames: u8,
    /// The reserved bytes between `number_of_frames` and the first child.
    /// QuickTime writers disagree on the count, so the wire length is kept.
    pub reserved_tail: Vec<u8>,
    /// The optional `name` atom's string bytes.
    pub name: Option<Vec<u8>>,
    /// Every other child, verbatim.
    pub others: Vec<OpaqueBox>,
}

impl TimecodeEntry {
    /// Whether the samples are 32-bit frame counters.
    #[must_use]
    pub const fn counter(&self) -> bool {
        self.flags & COUNTER_FLAG != 0
    }

    /// Whether the timecode is drop-frame.
    #[must_use]
    pub const fn drop_frame(&self) -> bool {
        self.flags & DROP_FRAME_FLAG != 0
    }

    /// Frames per second, from the time scale and frame duration.
    #[must_use]
    pub fn frames_per_second(&self) -> Option<u32> {
        if self.frame_duration == 0 {
            return None;
        }
        Some(self.timescale / self.frame_duration)
    }
}

impl<'a> Parse<'a> for TimecodeEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE + TMCD_FIXED_BODY_LEN, "tmcd")?;
        let body = crate::box_types::box_body(bytes, TMCD_FIXED_BODY_LEN, "tmcd")?;
        let mut reserved = [0u8; SAMPLE_ENTRY_RESERVED_LEN];
        reserved.copy_from_slice(&body[..SAMPLE_ENTRY_RESERVED_LEN]);
        let mut name = None;
        let mut others = Vec::new();
        let child_at = TMCD_RESERVED_BODY_LEN.min(body.len());
        let reserved_tail = body
            .get(TMCD_FIXED_BODY_LEN..child_at)
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        for item in BoxIter::clamped(body.get(child_at..).unwrap_or(&[])) {
            let Ok(item) = item else {
                break;
            };
            if item.four_cc() == NAME {
                name = Some(pascal_or_short_string(item.payload()));
            } else {
                others.push(OpaqueBox {
                    box_type: item.four_cc(),
                    data: item.payload().to_vec(),
                    to_end: item.to_end(),
                    largesize: item.has_largesize(),
                });
            }
        }
        Ok(Self {
            reserved,
            data_reference_index: u16_at(body, DATA_REFERENCE_INDEX_OFFSET),
            reserved_1: u32_at(body, RESERVED_1_OFFSET),
            flags: u32_at(body, FLAGS_OFFSET),
            timescale: u32_at(body, TIMESCALE_OFFSET),
            frame_duration: u32_at(body, FRAME_DURATION_OFFSET),
            number_of_frames: body[NUMBER_OF_FRAMES_OFFSET],
            reserved_tail,
            name,
            others,
        })
    }
}

impl Serialize for TimecodeEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + TMCD_FIXED_BODY_LEN + self.reserved_tail.len();
        if let Some(name) = &self.name {
            n += BOX_HEADER_MIN_SIZE + LENGTH_PREFIX_LEN + name.len();
        }
        for other in &self.others {
            n += other.serialized_len();
        }
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_box_header(buf, &TMCD, need)?;
        let u16_len = core::mem::size_of::<u16>();
        let u32_len = core::mem::size_of::<u32>();
        buf[c..c + SAMPLE_ENTRY_RESERVED_LEN].copy_from_slice(&self.reserved);
        c += SAMPLE_ENTRY_RESERVED_LEN;
        buf[c..c + u16_len].copy_from_slice(&self.data_reference_index.to_be_bytes());
        c += u16_len;
        buf[c..c + u32_len].copy_from_slice(&self.reserved_1.to_be_bytes());
        c += u32_len;
        buf[c..c + u32_len].copy_from_slice(&self.flags.to_be_bytes());
        c += u32_len;
        buf[c..c + u32_len].copy_from_slice(&self.timescale.to_be_bytes());
        c += u32_len;
        buf[c..c + u32_len].copy_from_slice(&self.frame_duration.to_be_bytes());
        c += u32_len;
        buf[c] = self.number_of_frames;
        c += 1;
        buf[c..c + self.reserved_tail.len()].copy_from_slice(&self.reserved_tail);
        c += self.reserved_tail.len();
        if let Some(name) = &self.name {
            let child_len = BOX_HEADER_MIN_SIZE + LENGTH_PREFIX_LEN + name.len();
            c += crate::box_types::write_box_header(&mut buf[c..], &NAME, child_len)?;
            buf[c..c + u16_len]
                .copy_from_slice(&fit_u16(name.len(), "tmcd name length")?.to_be_bytes());
            c += u16_len;
            buf[c..c + name.len()].copy_from_slice(name);
            c += name.len();
        }
        for other in &self.others {
            c += other.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

/// One decoded timecode value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Timecode {
    /// Hours.
    pub hours: u8,
    /// Minutes.
    pub minutes: u8,
    /// Seconds.
    pub seconds: u8,
    /// Frames.
    pub frames: u8,
    /// Whether the value is negative.
    pub negative: bool,
    /// The entry's drop-frame flag. Drop-frame *numbering* is not applied.
    pub drop_frame: bool,
}

/// Decode a `tmcd` sample against its sample description.
///
/// ffmpeg writes the counter form while leaving the `Counter` flag clear, so a
/// four-byte record whose fields are impossible (`seconds > 59` or
/// `frames >= number_of_frames`) is read as the counter form too. The counter
/// is a count of frames; `HH:MM:SS:FF` is derived by the entry's frame rate.
/// Drop-frame renumbering is **not** implemented (the flag is reported and the
/// value is the non-drop derivation), per the design's no-guessing rule.
pub fn timecode_from_sample(entry: &TimecodeEntry, sample: &[u8]) -> Result<Timecode> {
    need_in(sample, TMCD_SAMPLE_LEN, "tmcd sample")?;
    let record_is_valid = sample[SECONDS_BYTE_OFFSET] <= MAX_SECONDS
        && (sample[1] & MINUTES_MASK) <= MAX_MINUTES
        && (entry.number_of_frames == 0 || sample[FRAMES_BYTE_OFFSET] < entry.number_of_frames);
    if entry.counter() || !record_is_valid {
        return counter_timecode(entry, u32_at(sample, 0));
    }
    Ok(Timecode {
        hours: sample[0],
        minutes: sample[1] & MINUTES_MASK,
        seconds: sample[SECONDS_BYTE_OFFSET],
        frames: sample[FRAMES_BYTE_OFFSET],
        negative: sample[1] & NEGATIVE_BIT != 0,
        drop_frame: entry.drop_frame(),
    })
}

fn counter_timecode(entry: &TimecodeEntry, value: u32) -> Result<Timecode> {
    let fps = entry
        .frames_per_second()
        .ok_or(Error::InvalidInput("tmcd frame duration is zero"))?;
    if fps == 0 {
        return Err(Error::InvalidInput("tmcd frame rate is zero"));
    }
    let fps = u64::from(fps);
    let total = u64::from(value);
    let frames = total % fps;
    let seconds_total = total / fps;
    let seconds = seconds_total % SECONDS_PER_MINUTE;
    let minutes_total = seconds_total / SECONDS_PER_MINUTE;
    let minutes = minutes_total % MINUTES_PER_HOUR;
    let hours = minutes_total / MINUTES_PER_HOUR;
    let field = |value: u64| {
        u8::try_from(value)
            .map_err(|_| Error::InvalidInput("timecode counter leaves the 8-bit time fields"))
    };
    Ok(Timecode {
        hours: field(hours)?,
        minutes: field(minutes)?,
        seconds: field(seconds)?,
        frames: field(frames)?,
        negative: false,
        drop_frame: entry.drop_frame(),
    })
}

/// A `name` atom string: a 16-bit length then the bytes.
fn pascal_or_short_string(payload: &[u8]) -> Vec<u8> {
    if payload.len() < LENGTH_PREFIX_LEN {
        return Vec::new();
    }
    let len = usize::from(u16_at(payload, 0));
    payload
        .get(LENGTH_PREFIX_LEN..LENGTH_PREFIX_LEN + len)
        .map(<[u8]>::to_vec)
        .unwrap_or_default()
}
