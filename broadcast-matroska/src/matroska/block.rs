//! `SimpleBlock` and `Block` payloads (RFC 9559 §12).
//!
//! Both elements carry the same layout: a track-number VINT, a signed 16-bit
//! timestamp relative to the Cluster's `Timestamp`, one flags octet, and the
//! frame data, laced in one of the three forms. The laced frame sizes are
//! re-encoded at the widths the file used, so a block round-trips byte for
//! byte.

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::ebml::VintSize;
use crate::ebml::vint::VintWidth;
use crate::error::{Error, Result};
use crate::matroska::types::LacingKind;

/// Keyframe flag, `SimpleBlock` only (RFC 9559 §12).
pub const FLAG_KEYFRAME: u8 = 0x80;
/// Invisible flag.
pub const FLAG_INVISIBLE: u8 = 0x08;
/// Lacing mask.
pub const FLAG_LACING_MASK: u8 = 0x06;
/// No lacing.
pub const FLAG_LACING_NONE: u8 = 0x00;
/// Xiph lacing.
pub const FLAG_LACING_XIPH: u8 = 0x02;
/// Fixed-size lacing.
pub const FLAG_LACING_FIXED: u8 = 0x04;
/// EBML lacing.
pub const FLAG_LACING_EBML: u8 = 0x06;
/// Discardable flag.
pub const FLAG_DISCARDABLE: u8 = 0x01;
/// Xiph lacing's continuation byte.
pub const XIPH_CONTINUATION: u8 = 0xFF;
/// Octets in the relative timestamp.
pub const TIMESTAMP_OCTETS: usize = 2;
/// Octets in the flags field.
pub const FLAGS_OCTETS: usize = 1;
/// Octets in a lace-count byte.
pub const LACE_COUNT_OCTETS: usize = 1;
/// Bits of VINT data per octet.
const VINT_DATA_BITS: u32 = 7;
/// One lace; the byte counts frames minus one.
const LACE_COUNT_BIAS: usize = 1;
/// Fewest frames a laced block can carry.
const MIN_LACED_FRAMES: usize = 2;
/// The frame pairs a size delta is taken between.
const DELTA_WINDOW: usize = 2;

/// A Block's fixed header (RFC 9559 §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BlockHeader {
    track: u64,
    track_width: VintWidth,
    relative_timestamp: i16,
    flags: u8,
}

impl BlockHeader {
    /// Build a header from its fields.
    #[must_use]
    pub const fn new(
        track: u64,
        track_width: VintWidth,
        relative_timestamp: i16,
        flags: u8,
    ) -> Self {
        Self {
            track,
            track_width,
            relative_timestamp,
            flags,
        }
    }

    /// Build a header at the narrowest track-number width.
    pub fn with_values(track: u64, relative_timestamp: i16, flags: u8) -> Result<Self> {
        if track == 0 {
            return Err(Error::InvalidBlock {
                what: "track number is zero",
            });
        }
        let size = VintSize::minimal(track)?;
        Ok(Self {
            track,
            track_width: size.width(),
            relative_timestamp,
            flags,
        })
    }

    /// Parse the header from the front of a Block payload, returning it and the
    /// octets consumed.
    pub fn parse(bytes: &[u8]) -> Result<(Self, usize)> {
        let (track_size, track_used) = VintSize::parse(bytes)?;
        let track = track_size.value().ok_or(Error::InvalidBlock {
            what: "track number has the unknown size",
        })?;
        if track == 0 {
            return Err(Error::InvalidBlock {
                what: "track number is zero",
            });
        }
        let rest = bytes.get(track_used..).ok_or(Error::Truncated {
            what: "Block header",
            need: track_used,
            have: bytes.len(),
        })?;
        let fixed = TIMESTAMP_OCTETS + FLAGS_OCTETS;
        let taken = rest.get(..fixed).ok_or(Error::Truncated {
            what: "Block header",
            need: track_used + fixed,
            have: bytes.len(),
        })?;
        let timestamp = i16::from_be_bytes([taken[0], taken[1]]);
        Ok((
            Self {
                track,
                track_width: track_size.width(),
                relative_timestamp: timestamp,
                flags: taken[TIMESTAMP_OCTETS],
            },
            track_used + fixed,
        ))
    }

    /// The track number.
    #[must_use]
    pub const fn track(&self) -> u64 {
        self.track
    }

    /// The timestamp relative to the Cluster's `Timestamp`, in Matroska Ticks.
    #[must_use]
    pub const fn relative_timestamp(&self) -> i16 {
        self.relative_timestamp
    }

    /// The raw flags octet, preserved so its reserved bits round-trip.
    #[must_use]
    pub const fn flags(&self) -> u8 {
        self.flags
    }

    /// The keyframe flag (`SimpleBlock` only; a `Block` states its keyframe by
    /// omitting `ReferenceBlock`, RFC 9559 §5.1.3.5.5).
    #[must_use]
    pub const fn keyframe_flag(&self) -> bool {
        self.flags & FLAG_KEYFRAME != 0
    }

    /// The invisible flag.
    #[must_use]
    pub const fn invisible(&self) -> bool {
        self.flags & FLAG_INVISIBLE != 0
    }

    /// The discardable flag.
    #[must_use]
    pub const fn discardable(&self) -> bool {
        self.flags & FLAG_DISCARDABLE != 0
    }

    /// The lacing form the flags select.
    #[must_use]
    pub const fn lacing_kind(&self) -> LacingKind {
        match self.flags & FLAG_LACING_MASK {
            FLAG_LACING_XIPH => LacingKind::Xiph,
            FLAG_LACING_FIXED => LacingKind::Fixed,
            FLAG_LACING_EBML => LacingKind::Ebml,
            _ => LacingKind::None,
        }
    }

    /// The encoded header length.
    #[must_use]
    pub const fn serialized_len(&self) -> usize {
        self.track_width.get() + TIMESTAMP_OCTETS + FLAGS_OCTETS
    }

    /// Write the header, returning the octets written.
    pub fn write(&self, buf: &mut [u8]) -> Result<usize> {
        let size = VintSize::Known {
            value: self.track,
            width: self.track_width,
        };
        let used = size.write(buf)?;
        let have = buf.len();
        let rest = buf
            .get_mut(used..used + TIMESTAMP_OCTETS + FLAGS_OCTETS)
            .ok_or(Error::OutputBufferTooSmall(
                broadcast_common::error::OutputBufferTooSmall {
                    need: used + TIMESTAMP_OCTETS + FLAGS_OCTETS,
                    have,
                },
            ))?;
        rest[..TIMESTAMP_OCTETS].copy_from_slice(&self.relative_timestamp.to_be_bytes());
        rest[TIMESTAMP_OCTETS] = self.flags;
        Ok(used + TIMESTAMP_OCTETS + FLAGS_OCTETS)
    }
}

/// The lacing form of a Block and its frames (RFC 9559 §12).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Lacing<'a> {
    /// One frame, no lace-count byte.
    None {
        /// The frame.
        frame: &'a [u8],
    },
    /// Xiph lacing: sizes in 255-octet continuation chains.
    Xiph {
        /// The laced frames.
        frames: Vec<&'a [u8]>,
    },
    /// EBML lacing: the first size as a VINT, the rest as signed VINT deltas
    /// at one fixed width.
    Ebml {
        /// The laced frames.
        frames: Vec<&'a [u8]>,
        /// Width of the first frame's size VINT.
        first_width: VintWidth,
        /// Width of each delta VINT.
        delta_width: VintWidth,
    },
    /// Fixed-size lacing: every frame is the same length.
    Fixed {
        /// The laced frames.
        frames: Vec<&'a [u8]>,
    },
}

impl<'a> Lacing<'a> {
    /// The lacing form.
    #[must_use]
    pub const fn kind(&self) -> LacingKind {
        match self {
            Self::None { .. } => LacingKind::None,
            Self::Xiph { .. } => LacingKind::Xiph,
            Self::Ebml { .. } => LacingKind::Ebml,
            Self::Fixed { .. } => LacingKind::Fixed,
        }
    }

    /// How many frames the lace carries.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::None { .. } => 1,
            Self::Xiph { frames } | Self::Ebml { frames, .. } | Self::Fixed { frames } => {
                frames.len()
            }
        }
    }

    /// Whether the lace carries no frames, which is never valid.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The frame at `index`.
    #[must_use]
    pub fn frame(&self, index: usize) -> Option<&'a [u8]> {
        match self {
            Self::None { frame } => (index == 0).then_some(*frame),
            Self::Xiph { frames } | Self::Ebml { frames, .. } | Self::Fixed { frames } => {
                frames.get(index).copied()
            }
        }
    }

    /// The encoded length.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        match self {
            Self::None { frame } => frame.len(),
            Self::Xiph { frames } => {
                LACE_COUNT_OCTETS
                    + xiph_sizes(frames)
                        .iter()
                        .map(|n| xiph_len(*n))
                        .sum::<usize>()
                    + frames.iter().map(|frame| frame.len()).sum::<usize>()
            }
            Self::Ebml {
                frames,
                first_width,
                delta_width,
            } => {
                LACE_COUNT_OCTETS
                    + first_width.get()
                    + delta_width.get() * frames.len().saturating_sub(2)
                    + frames.iter().map(|frame| frame.len()).sum::<usize>()
            }
            Self::Fixed { frames } => {
                LACE_COUNT_OCTETS + frames.iter().map(|frame| frame.len()).sum::<usize>()
            }
        }
    }

    /// Write the lace, returning the octets written.
    pub fn write(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::None { frame } => copy_into(buf, frame),
            Self::Xiph { frames } => write_xiph(frames, buf),
            Self::Ebml {
                frames,
                first_width,
                delta_width,
            } => write_ebml(frames, *first_width, *delta_width, buf),
            Self::Fixed { frames } => write_fixed(frames, buf),
        }
    }
}

/// The frame-size table at the front of a laced Block payload (RFC 9559 §12).
///
/// The reader needs each frame's byte range without holding the frame data, so
/// it reads only the table and computes the ranges from it. The table is the
/// same syntax [`Lacing`] parses; this type exposes it without the frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaceHeader {
    data_offset: usize,
    explicit: Vec<u64>,
    frame_count: usize,
    fixed: bool,
}

impl LaceHeader {
    /// Parse the frame-size table for a Block whose flags select `kind`.
    ///
    /// `max_frames` is the caller's hostile-input cap on the lace count.
    pub fn parse(payload: &[u8], kind: LacingKind, max_frames: usize) -> Result<Self> {
        if kind == LacingKind::None {
            return Ok(Self {
                data_offset: 0,
                explicit: Vec::new(),
                frame_count: 1,
                fixed: false,
            });
        }
        let (count, data) = parse_lace_count(payload)?;
        if count < MIN_LACED_FRAMES {
            return Err(Error::InvalidBlock {
                what: "lacing set with one frame",
            });
        }
        if count > max_frames {
            return Err(Error::LimitExceeded {
                limit: "max_lace_frames",
                value: u64::try_from(count).unwrap_or(u64::MAX),
            });
        }
        match kind {
            LacingKind::Xiph => {
                let mut position = 0usize;
                let mut explicit = Vec::with_capacity(count - 1);
                for _ in 0..count - 1 {
                    let mut size = 0u64;
                    loop {
                        let octet = *data.get(position).ok_or(Error::Truncated {
                            what: "Xiph lace size",
                            need: position + 1,
                            have: data.len(),
                        })?;
                        position += 1;
                        size = size
                            .checked_add(u64::from(octet))
                            .ok_or(Error::SizeOverflow)?;
                        if octet != XIPH_CONTINUATION {
                            break;
                        }
                    }
                    explicit.push(size);
                }
                Ok(Self {
                    data_offset: LACE_COUNT_OCTETS
                        .checked_add(position)
                        .ok_or(Error::SizeOverflow)?,
                    explicit,
                    frame_count: count,
                    fixed: false,
                })
            }
            LacingKind::Ebml => {
                let (first, first_used) = VintSize::parse(data)?;
                let first = first.value().ok_or(Error::InvalidBlock {
                    what: "first EBML lace size is unknown",
                })?;
                let mut position = first_used;
                let mut explicit = Vec::with_capacity(count - 1);
                explicit.push(first);
                let mut delta_width: Option<VintWidth> = None;
                for _ in 0..count - MIN_LACED_FRAMES {
                    let slice = data.get(position..).ok_or(Error::Truncated {
                        what: "EBML lace delta",
                        need: position,
                        have: data.len(),
                    })?;
                    let (delta, width) = parse_signed_vint(slice)?;
                    match delta_width {
                        Some(previous) if previous != width => {
                            return Err(Error::InvalidBlock {
                                what: "EBML lace deltas differ in width",
                            });
                        }
                        Some(_) => {}
                        None => delta_width = Some(width),
                    }
                    position = position
                        .checked_add(width.get())
                        .ok_or(Error::SizeOverflow)?;
                    let previous = *explicit.last().ok_or(Error::InvalidBlock {
                        what: "EBML lace with no previous size",
                    })?;
                    let next = i64::try_from(previous).map_err(|_| Error::SizeOverflow)? + delta;
                    let next = u64::try_from(next).map_err(|_| Error::InvalidBlock {
                        what: "EBML lace size is negative",
                    })?;
                    explicit.push(next);
                }
                Ok(Self {
                    data_offset: LACE_COUNT_OCTETS
                        .checked_add(position)
                        .ok_or(Error::SizeOverflow)?,
                    explicit,
                    frame_count: count,
                    fixed: false,
                })
            }
            LacingKind::Fixed => Ok(Self {
                data_offset: LACE_COUNT_OCTETS,
                explicit: Vec::new(),
                frame_count: count,
                fixed: true,
            }),
            LacingKind::None => Ok(Self {
                data_offset: 0,
                explicit: Vec::new(),
                frame_count: 1,
                fixed: false,
            }),
        }
    }

    /// Octets from the Block payload start to the first frame.
    #[must_use]
    pub const fn data_offset(&self) -> usize {
        self.data_offset
    }

    /// The number of frames the table describes.
    #[must_use]
    pub const fn frame_count(&self) -> usize {
        self.frame_count
    }

    /// Every frame's size, the last one taking the remainder of `payload_len`.
    ///
    /// `payload_len` is the whole Block payload length, so a caller that read
    /// only a prefix still gets the exact ranges. A zero-length frame is
    /// rejected: a hostile count byte would otherwise turn a few bytes into
    /// hundreds of empty samples.
    pub fn frame_sizes(&self, payload_len: u64) -> Result<Vec<u64>> {
        let data_start = u64::try_from(self.data_offset).map_err(|_| Error::SizeOverflow)?;
        let available = payload_len
            .checked_sub(data_start)
            .ok_or(Error::InvalidBlock {
                what: "lace header runs past the block",
            })?;
        if self.fixed {
            let count = u64::try_from(self.frame_count).map_err(|_| Error::SizeOverflow)?;
            if count == 0 || available % count != 0 {
                return Err(Error::InvalidBlock {
                    what: "fixed lace frames are not equal",
                });
            }
            let each = available / count;
            if each == 0 {
                return Err(Error::InvalidBlock {
                    what: "laced frame has zero length",
                });
            }
            return Ok(alloc::vec![each; self.frame_count]);
        }
        let explicit: u64 = self.explicit.iter().try_fold(0u64, |total, size| {
            total.checked_add(*size).ok_or(Error::SizeOverflow)
        })?;
        if explicit > available {
            return Err(Error::InvalidBlock {
                what: "laced frame sizes exceed the block",
            });
        }
        let mut sizes = self.explicit.clone();
        sizes.push(available - explicit);
        if sizes.contains(&0) {
            return Err(Error::InvalidBlock {
                what: "laced frame has zero length",
            });
        }
        Ok(sizes)
    }
}

/// The full `SimpleBlock`/`Block` payload (RFC 9559 §12).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Block<'a> {
    header: BlockHeader,
    lacing: Lacing<'a>,
}

impl<'a> Block<'a> {
    /// Build one from a parsed header and lacing.
    #[must_use]
    pub const fn from_parts(header: BlockHeader, lacing: Lacing<'a>) -> Self {
        Self { header, lacing }
    }

    /// The fixed header.
    #[must_use]
    pub const fn header(&self) -> BlockHeader {
        self.header
    }

    /// The lacing and its frames.
    #[must_use]
    pub const fn lacing(&self) -> &Lacing<'a> {
        &self.lacing
    }

    /// How many frames the block carries.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.lacing.len()
    }

    /// The frame at `index`.
    #[must_use]
    pub fn frame(&self, index: usize) -> Option<&'a [u8]> {
        self.lacing.frame(index)
    }
}

impl<'a> Parse<'a> for Block<'a> {
    type Error = Error;

    fn parse(payload: &'a [u8]) -> Result<Self> {
        let (header, used) = BlockHeader::parse(payload)?;
        let rest = payload.get(used..).ok_or(Error::Truncated {
            what: "Block lacing",
            need: used,
            have: payload.len(),
        })?;
        let lacing = parse_lacing(rest, header.lacing_kind())?;
        Ok(Self::from_parts(header, lacing))
    }
}

impl Serialize for Block<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.header.serialized_len() + self.lacing.serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let used = self.header.write(buf)?;
        let have = buf.len();
        let rest = buf.get_mut(used..).ok_or(Error::OutputBufferTooSmall(
            broadcast_common::error::OutputBufferTooSmall {
                need: used + self.lacing.serialized_len(),
                have,
            },
        ))?;
        Ok(used + self.lacing.write(rest)?)
    }
}

/// Parse the lace after a Block header.
fn parse_lacing<'a>(payload: &'a [u8], kind: LacingKind) -> Result<Lacing<'a>> {
    match kind {
        LacingKind::None => Ok(Lacing::None { frame: payload }),
        LacingKind::Xiph => parse_xiph(payload),
        LacingKind::Ebml => parse_ebml(payload),
        LacingKind::Fixed => parse_fixed(payload),
    }
}

/// Read the lace-count byte as a frame count.
fn parse_lace_count(payload: &[u8]) -> Result<(usize, &[u8])> {
    let first = *payload.first().ok_or(Error::Truncated {
        what: "lace count",
        need: LACE_COUNT_OCTETS,
        have: 0,
    })?;
    let count = usize::from(first) + LACE_COUNT_BIAS;
    let rest = payload.get(LACE_COUNT_OCTETS..).ok_or(Error::Truncated {
        what: "lace count",
        need: LACE_COUNT_OCTETS,
        have: payload.len(),
    })?;
    Ok((count, rest))
}

/// Split frame data by its sizes, the last size taking the remainder.
fn split_frames(data: &[u8], mut sizes: Vec<usize>) -> Result<Vec<&[u8]>> {
    let explicit: usize = sizes.iter().sum();
    let remainder = data
        .len()
        .checked_sub(explicit)
        .ok_or(Error::InvalidBlock {
            what: "laced frame sizes exceed the block",
        })?;
    sizes.push(remainder);
    let mut frames = Vec::with_capacity(sizes.len());
    let mut position = 0usize;
    for size in sizes {
        let end = position.checked_add(size).ok_or(Error::SizeOverflow)?;
        let frame = data.get(position..end).ok_or(Error::InvalidBlock {
            what: "laced frame past the block",
        })?;
        frames.push(frame);
        position = end;
    }
    Ok(frames)
}

/// Parse Xiph lacing (RFC 9559 §12).
fn parse_xiph<'a>(payload: &'a [u8]) -> Result<Lacing<'a>> {
    let (count, data) = parse_lace_count(payload)?;
    if count < MIN_LACED_FRAMES {
        return Err(Error::InvalidBlock {
            what: "lacing set with one frame",
        });
    }
    let mut position = 0usize;
    let mut sizes = Vec::with_capacity(count - 1);
    for _ in 0..count - 1 {
        let mut size = 0usize;
        loop {
            let octet = *data.get(position).ok_or(Error::Truncated {
                what: "Xiph lace size",
                need: position + 1,
                have: data.len(),
            })?;
            position += 1;
            size = size
                .checked_add(usize::from(octet))
                .ok_or(Error::SizeOverflow)?;
            if octet != XIPH_CONTINUATION {
                break;
            }
        }
        sizes.push(size);
    }
    let frames_data = data.get(position..).ok_or(Error::Truncated {
        what: "Xiph frame data",
        need: position,
        have: data.len(),
    })?;
    Ok(Lacing::Xiph {
        frames: split_frames(frames_data, sizes)?,
    })
}

/// Parse EBML lacing (RFC 9559 §12).
fn parse_ebml<'a>(payload: &'a [u8]) -> Result<Lacing<'a>> {
    let (count, rest) = parse_lace_count(payload)?;
    if count < MIN_LACED_FRAMES {
        return Err(Error::InvalidBlock {
            what: "lacing set with one frame",
        });
    }
    let (first, first_used) = VintSize::parse(rest)?;
    let first_width = first.width();
    let first = first.value().ok_or(Error::InvalidBlock {
        what: "first EBML lace size is unknown",
    })?;
    let mut position = first_used;
    let mut sizes = Vec::with_capacity(count - 1);
    sizes.push(usize::try_from(first).map_err(|_| Error::SizeOverflow)?);
    let mut delta_width: Option<VintWidth> = None;
    for _ in 0..count - MIN_LACED_FRAMES {
        let slice = rest.get(position..).ok_or(Error::Truncated {
            what: "EBML lace delta",
            need: position,
            have: rest.len(),
        })?;
        let (delta, width) = parse_signed_vint(slice)?;
        match delta_width {
            Some(previous_width) if previous_width != width => {
                return Err(Error::InvalidBlock {
                    what: "EBML lace deltas differ in width",
                });
            }
            Some(_) => {}
            None => delta_width = Some(width),
        }
        position = position
            .checked_add(width.get())
            .ok_or(Error::SizeOverflow)?;
        let previous = *sizes.last().ok_or(Error::InvalidBlock {
            what: "EBML lace with no previous size",
        })?;
        let next = i64::try_from(previous).map_err(|_| Error::SizeOverflow)? + delta;
        let next = usize::try_from(next).map_err(|_| Error::InvalidBlock {
            what: "EBML lace size is negative",
        })?;
        sizes.push(next);
    }
    let frames_data = rest.get(position..).ok_or(Error::Truncated {
        what: "EBML frame data",
        need: position,
        have: rest.len(),
    })?;
    Ok(Lacing::Ebml {
        frames: split_frames(frames_data, sizes)?,
        first_width,
        delta_width: delta_width.unwrap_or(VintWidth::MIN),
    })
}

/// Parse fixed-size lacing (RFC 9559 §12).
fn parse_fixed<'a>(payload: &'a [u8]) -> Result<Lacing<'a>> {
    let (count, data) = parse_lace_count(payload)?;
    if count < MIN_LACED_FRAMES {
        return Err(Error::InvalidBlock {
            what: "lacing set with one frame",
        });
    }
    if data.len() % count != 0 {
        return Err(Error::InvalidBlock {
            what: "fixed lace frames are not equal",
        });
    }
    let size = data.len() / count;
    let mut frames = Vec::with_capacity(count);
    for index in 0..count {
        let start = index * size;
        frames.push(&data[start..start + size]);
    }
    Ok(Lacing::Fixed { frames })
}

/// A signed EBML-lacing size: a VINT whose data is biased.
fn parse_signed_vint(bytes: &[u8]) -> Result<(i64, VintWidth)> {
    let (size, _) = VintSize::parse(bytes)?;
    let width = size.width();
    let value = size.value().ok_or(Error::InvalidBlock {
        what: "EBML lace delta is unknown",
    })?;
    let bias = signed_bias(width);
    let delta = i64::try_from(value).map_err(|_| Error::SizeOverflow)? - bias;
    Ok((delta, width))
}

/// The bias a signed lacing VINT of `width` subtracts.
fn signed_bias(width: VintWidth) -> i64 {
    (1i64 << (width.data_bits() - 1)) - 1
}

/// The sizes of the first `n-1` Xiph frames.
fn xiph_sizes(frames: &[&[u8]]) -> Vec<usize> {
    frames
        .iter()
        .take(frames.len().saturating_sub(1))
        .map(|frame| frame.len())
        .collect()
}

/// Octets a Xiph lace size needs, continuation chains included.
fn xiph_len(size: usize) -> usize {
    size / usize::from(XIPH_CONTINUATION) + 1
}

/// Write Xiph lacing.
fn write_xiph(frames: &[&[u8]], buf: &mut [u8]) -> Result<usize> {
    let have = buf.len();
    if frames.len() < MIN_LACED_FRAMES {
        return Err(Error::InvalidBlock {
            what: "Xiph lacing needs at least two frames",
        });
    }
    let mut used = 0usize;
    let count = u8::try_from(frames.len() - LACE_COUNT_BIAS).map_err(|_| Error::InvalidBlock {
        what: "lace count exceeds 256",
    })?;
    write_byte(buf, &mut used, count)?;
    for size in xiph_sizes(frames) {
        let mut remaining = size;
        loop {
            if remaining >= usize::from(XIPH_CONTINUATION) {
                write_byte(buf, &mut used, XIPH_CONTINUATION)?;
                remaining -= usize::from(XIPH_CONTINUATION);
            } else {
                let octet = u8::try_from(remaining).map_err(|_| Error::SizeOverflow)?;
                write_byte(buf, &mut used, octet)?;
                break;
            }
        }
    }
    for frame in frames {
        used += copy_into(
            buf.get_mut(used..).ok_or(Error::OutputBufferTooSmall(
                broadcast_common::error::OutputBufferTooSmall {
                    need: used + frame.len(),
                    have,
                },
            ))?,
            frame,
        )?;
    }
    Ok(used)
}

/// Write EBML lacing.
fn write_ebml(
    frames: &[&[u8]],
    first_width: VintWidth,
    delta_width: VintWidth,
    buf: &mut [u8],
) -> Result<usize> {
    let have = buf.len();
    if frames.len() < MIN_LACED_FRAMES {
        return Err(Error::InvalidBlock {
            what: "EBML lacing needs at least two frames",
        });
    }
    let mut used = 0usize;
    let count = u8::try_from(frames.len() - LACE_COUNT_BIAS).map_err(|_| Error::InvalidBlock {
        what: "lace count exceeds 256",
    })?;
    write_byte(buf, &mut used, count)?;
    let first = VintSize::Known {
        value: u64::try_from(frames[0].len()).map_err(|_| Error::SizeOverflow)?,
        width: first_width,
    };
    used += first.write(buf.get_mut(used..).ok_or(Error::OutputBufferTooSmall(
        broadcast_common::error::OutputBufferTooSmall {
            need: used + first_width.get(),
            have,
        },
    ))?)?;
    for pair in frames[..frames.len() - 1].windows(DELTA_WINDOW) {
        let delta = i64::try_from(pair[1].len()).map_err(|_| Error::SizeOverflow)?
            - i64::try_from(pair[0].len()).map_err(|_| Error::SizeOverflow)?;
        let biased = delta + signed_bias(delta_width);
        let value = u64::try_from(biased).map_err(|_| Error::InvalidBlock {
            what: "EBML lace delta does not fit its width",
        })?;
        let size = VintSize::Known {
            value,
            width: delta_width,
        };
        if value == mask_for(delta_width) {
            return Err(Error::InvalidBlock {
                what: "EBML lace delta is the reserved unknown value",
            });
        }
        used += size.write(buf.get_mut(used..).ok_or(Error::OutputBufferTooSmall(
            broadcast_common::error::OutputBufferTooSmall {
                need: used + delta_width.get(),
                have,
            },
        ))?)?;
    }
    for frame in frames {
        used += copy_into(
            buf.get_mut(used..).ok_or(Error::OutputBufferTooSmall(
                broadcast_common::error::OutputBufferTooSmall {
                    need: used + frame.len(),
                    have,
                },
            ))?,
            frame,
        )?;
    }
    Ok(used)
}

/// Write fixed-size lacing.
fn write_fixed(frames: &[&[u8]], buf: &mut [u8]) -> Result<usize> {
    let have = buf.len();
    if frames.len() < MIN_LACED_FRAMES {
        return Err(Error::InvalidBlock {
            what: "fixed lacing needs at least two frames",
        });
    }
    let size = frames[0].len();
    if frames.iter().any(|frame| frame.len() != size) {
        return Err(Error::InvalidBlock {
            what: "fixed lace frames differ in length",
        });
    }
    let mut used = 0usize;
    let count = u8::try_from(frames.len() - LACE_COUNT_BIAS).map_err(|_| Error::InvalidBlock {
        what: "lace count exceeds 256",
    })?;
    write_byte(buf, &mut used, count)?;
    for frame in frames {
        used += copy_into(
            buf.get_mut(used..).ok_or(Error::OutputBufferTooSmall(
                broadcast_common::error::OutputBufferTooSmall {
                    need: used + frame.len(),
                    have,
                },
            ))?,
            frame,
        )?;
    }
    Ok(used)
}

/// Write one octet and advance `used`.
fn write_byte(buf: &mut [u8], used: &mut usize, octet: u8) -> Result<()> {
    let have = buf.len();
    let slot = buf.get_mut(*used).ok_or(Error::OutputBufferTooSmall(
        broadcast_common::error::OutputBufferTooSmall {
            need: *used + 1,
            have,
        },
    ))?;
    *slot = octet;
    *used += 1;
    Ok(())
}

/// Copy `bytes` into the front of `buf`.
fn copy_into(buf: &mut [u8], bytes: &[u8]) -> Result<usize> {
    let have = buf.len();
    let target = buf
        .get_mut(..bytes.len())
        .ok_or(Error::OutputBufferTooSmall(
            broadcast_common::error::OutputBufferTooSmall {
                need: bytes.len(),
                have,
            },
        ))?;
    target.copy_from_slice(bytes);
    Ok(bytes.len())
}

/// The all-ones VINT_DATA mask for `width`.
fn mask_for(width: VintWidth) -> u64 {
    (1u64 << (VINT_DATA_BITS * width.get() as u32)) - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_laced_block_round_trips() {
        let payload = [0x81u8, 0x00, 0x00, 0x80, 0xAA, 0xBB];
        let block = Block::parse(&payload).unwrap();
        assert_eq!(block.header().track(), 1);
        assert!(block.header().keyframe_flag());
        assert_eq!(block.frame(0), Some(&[0xAA, 0xBB][..]));
        assert_eq!(block.try_to_bytes().unwrap(), payload);
    }

    #[test]
    fn xiph_lacing_round_trips() {
        let payload = [0x81u8, 0x00, 0x00, 0x02, 0x01, 0x02, 0xAA, 0xBB, 0xCC, 0xDD];
        let block = Block::parse(&payload).unwrap();
        assert_eq!(block.frame_count(), 2);
        assert_eq!(block.frame(0), Some(&[0xAA, 0xBB][..]));
        assert_eq!(block.frame(1), Some(&[0xCC, 0xDD][..]));
        assert_eq!(block.try_to_bytes().unwrap(), payload);
    }

    #[test]
    fn ebml_lacing_round_trips() {
        let payload = [
            0x81u8, 0x00, 0x00, 0x06, 0x02, 0x83, 0xC0, 0xAA, 0xAA, 0xAA, 0xBB, 0xBB, 0xBB, 0xBB,
            0xCC,
        ];
        let block = Block::parse(&payload).unwrap();
        assert_eq!(block.frame_count(), 3);
        assert_eq!(block.frame(0), Some(&[0xAA, 0xAA, 0xAA][..]));
        assert_eq!(block.frame(1), Some(&[0xBB, 0xBB, 0xBB, 0xBB][..]));
        assert_eq!(block.frame(2), Some(&[0xCC][..]));
        assert_eq!(block.try_to_bytes().unwrap(), payload);
    }

    #[test]
    fn ebml_lacing_negative_delta_round_trips() {
        let payload = [0x81u8, 0x00, 0x00, 0x06, 0x02, 0x82, 0xBD, 0xAA, 0xBB, 0xCC];
        let block = Block::parse(&payload).unwrap();
        assert_eq!(block.frame_count(), 3);
        assert_eq!(block.frame(0), Some(&[0xAA, 0xBB][..]));
        assert_eq!(block.frame(1), Some(&[][..]));
        assert_eq!(block.frame(2), Some(&[0xCC][..]));
        assert_eq!(block.try_to_bytes().unwrap(), payload);
    }

    #[test]
    fn fixed_lacing_round_trips() {
        let payload = [
            0x81u8, 0x00, 0x00, 0x04, 0x01, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        ];
        let block = Block::parse(&payload).unwrap();
        assert_eq!(block.frame_count(), 2);
        assert_eq!(block.frame(0), Some(&[0xAA, 0xBB, 0xCC][..]));
        assert_eq!(block.frame(1), Some(&[0xDD, 0xEE, 0xFF][..]));
        assert_eq!(block.try_to_bytes().unwrap(), payload);
    }

    #[test]
    fn malformed_blocks_are_refused() {
        assert!(Block::parse(&[0x81, 0x00, 0x00, 0x02, 0x01, 0x05, 0xAA, 0xBB]).is_err());
        assert!(Block::parse(&[0x81, 0x00, 0x00, 0x04, 0x00, 0xAA]).is_err());
        assert!(Block::parse(&[0xFF, 0x00]).is_err());
        assert!(Block::parse(&[0x00, 0x00, 0x00]).is_err());
    }
    /// The Block header of these fixtures is four octets; `rest` is the lace
    /// section (`frame_sizes` takes its length, not the whole Block's).
    fn lace(rest: &[u8], kind: LacingKind) -> Result<LaceHeader> {
        LaceHeader::parse(rest, kind, 256)
    }

    fn sizes(rest: &[u8], kind: LacingKind) -> Result<Vec<u64>> {
        lace(rest, kind)?.frame_sizes(u64::try_from(rest.len()).unwrap())
    }

    /// webm_demux's `ebml_lacing_negative_running_size_is_rejected`.
    #[test]
    fn ebml_lacing_negative_running_size_is_rejected() {
        let rest = [0x02u8, 0x82, 0xBC];
        assert!(matches!(
            lace(&rest, LacingKind::Ebml),
            Err(Error::InvalidBlock {
                what: "EBML lace size is negative"
            })
        ));
    }

    /// webm_demux's `ebml_lacing_zero_running_size_reaches_the_zero_length_rule`.
    #[test]
    fn ebml_lacing_zero_running_size_reaches_the_zero_length_rule() {
        let rest = [0x02u8, 0x82, 0xBD, 0xAA, 0xBB, 0xCC];
        assert!(matches!(
            sizes(&rest, LacingKind::Ebml),
            Err(Error::InvalidBlock {
                what: "laced frame has zero length"
            })
        ));
    }

    /// webm_demux's `fixed_lacing_needs_a_whole_multiple`.
    #[test]
    fn fixed_lacing_needs_a_whole_multiple() {
        let rest = [0x01u8, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        assert!(matches!(
            sizes(&rest, LacingKind::Fixed),
            Err(Error::InvalidBlock {
                what: "fixed lace frames are not equal"
            })
        ));
    }

    /// webm_demux's `zero_length_laced_frames_are_rejected`.
    #[test]
    fn zero_length_laced_frames_are_rejected() {
        let rest = [0xFFu8];
        assert!(matches!(
            sizes(&rest, LacingKind::Fixed),
            Err(Error::InvalidBlock {
                what: "laced frame has zero length"
            })
        ));
    }

    /// webm_demux's `xiph_laced_zero_length_frame_is_rejected`.
    #[test]
    fn xiph_laced_zero_length_frame_is_rejected() {
        let rest = [0x01u8, 0x00, 0xAA];
        assert!(matches!(
            sizes(&rest, LacingKind::Xiph),
            Err(Error::InvalidBlock {
                what: "laced frame has zero length"
            })
        ));
    }

    /// webm_demux's `a_full_lace_count_is_accepted_and_the_cap_is_the_format_maximum`.
    #[test]
    fn a_full_lace_count_is_accepted_and_the_cap_is_the_format_maximum() {
        assert_eq!(
            crate::limits::DEFAULT_MAX_LACE_FRAMES,
            u32::from(u8::MAX) + 1,
            "the cap is the format's own maximum frame count"
        );
        let mut rest = alloc::vec![0xFFu8];
        rest.extend(::core::iter::repeat_n(0xAAu8, 256));
        let header = lace(&rest, LacingKind::Fixed).expect("256 fixed frames");
        assert_eq!(header.frame_count(), 256);
        let sizes = header
            .frame_sizes(u64::try_from(rest.len()).unwrap())
            .expect("each frame one byte");
        assert_eq!(sizes.len(), 256);
        assert!(sizes.iter().all(|size| *size == 1));
    }

    /// webm_demux's `ebml_laced_block_reads_a_width_8_first_lace_size`: the
    /// width-8 size is decoded, so the block reaches the zero-length rule.
    #[test]
    fn ebml_laced_block_reads_a_width_8_first_lace_size() {
        let rest = [0x01u8, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xAA];
        assert!(matches!(
            sizes(&rest, LacingKind::Ebml),
            Err(Error::InvalidBlock {
                what: "laced frame has zero length"
            })
        ));
    }
}
