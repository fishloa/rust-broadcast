//! Hint-track boxes — ISO/IEC 14496-12:2026 §9.1, §9.3, §9.4.
//!
//! Transcription: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`.
//!
//! | Box | Type | Clause | Container |
//! |-----|------|--------|-----------|
//! | [`HintInformationBox`] | `hnti` | §9.1.4 | `udta` (movie and track) |
//! | [`RtpHintInformation`] | `rtp ` | §9.1.4.2 | `hnti` (movie) |
//! | [`SdpHintInformation`] | `sdp ` | §9.1.4.3 | `hnti` (track) |
//! | [`HintStatisticsBox`] | `hinf` | §9.1.5 | `udta` (track) |
//! | [`TimescaleEntryBox`] | `tims` | §9.1.2.1 | RTP sample entry |
//! | [`TimeOffsetBox`] | `tsro` | §9.1.2.1 | RTP sample entry |
//! | [`SequenceOffsetBox`] | `snro` | §9.1.2.1 | RTP sample entry |
//! | [`TimestampSynchronyBox`] | `tssy` | §9.4.1.2 | RTP reception entry |
//! | [`ReceivedSsrcBox`] | `rssr` | §9.4.1.2 | RTP reception entry |
//! | [`SrtpProcessBox`] | `srpp` | §9.1.2.2 | SRTP sample entry |
//! | [`RtpHdrExtBox`] | `rtpx` | §9.4.1.4 | `extra_data_tlv` |
//! | [`ReceivedCsrcBox`] | `rcsr` | §9.4.1.4 | `extra_data_tlv` |
//! | [`PatBox`] | `tPAT` | §9.3.3.2 | MPEG-2 TS sample entry |
//! | [`PmtBox`] | `tPMT` | §9.3.3.2 | MPEG-2 TS sample entry |
//! | [`TsTimingBox`] | `tsti` | §9.3.3.2 | MPEG-2 TS sample entry |
//! | [`InitialSampleTimeBox`] | `istm` | §9.3.3.2 | MPEG-2 TS sample entry |
//!
//! The sample entries themselves ([`HintSampleEntry`]) parse and serialize the
//! per-format fixed fields and keep every child in wire order. One hint
//! sample's bytes are typed by [`RtpSample`] (server and reception tracks share
//! the format, §9.1.3); the reader reports samples as opaque bytes and never
//! interprets them.

use alloc::vec::Vec;

use broadcast_core::len::{fit_u16, fit_u32, need_in, need_out};
use broadcast_core::wire::{be_u16, be_u32, be_u64, u24_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK, box_len, parse_box,
    write_box_header, write_full_box_header,
};
use crate::error::{Error, Result};
use broadcast_core::error::BufferTooShort;

/// Four-CC of the Hint Information Box (`hnti`, §9.1.4).
pub const HNTI: [u8; BOX_TYPE_SIZE] = *b"hnti";
/// Four-CC of the RTP information box (`rtp `, §9.1.4.2).
pub const RTP: [u8; BOX_TYPE_SIZE] = *b"rtp ";
/// Four-CC of the SDP information box (`sdp `, §9.1.4.3).
pub const SDP: [u8; BOX_TYPE_SIZE] = *b"sdp ";
/// Four-CC of the Hint Statistics Box (`hinf`, §9.1.5).
pub const HINF: [u8; BOX_TYPE_SIZE] = *b"hinf";
/// Four-CC of the Max Rate Box (`maxr`, §9.1.5).
pub const MAXR: [u8; BOX_TYPE_SIZE] = *b"maxr";
/// Four-CC of the Payload Type Box (`payt`, §9.1.5).
pub const PAYT: [u8; BOX_TYPE_SIZE] = *b"payt";
/// Four-CC of the Timescale Entry Box (`tims`, §9.1.2).
pub const TIMS: [u8; BOX_TYPE_SIZE] = *b"tims";
/// Four-CC of the Time Offset Box (`tsro`, §9.1.2).
pub const TSRO: [u8; BOX_TYPE_SIZE] = *b"tsro";
/// Four-CC of the Sequence Offset Box (`snro`, §9.1.2).
pub const SNRO: [u8; BOX_TYPE_SIZE] = *b"snro";
/// Four-CC of the Timestamp Synchrony Box (`tssy`, §9.4.1.2).
pub const TSSY: [u8; BOX_TYPE_SIZE] = *b"tssy";
/// Four-CC of the Received SSRC Box (`rssr`, §9.4.1.2).
pub const RSSR: [u8; BOX_TYPE_SIZE] = *b"rssr";
/// Four-CC of the SRTP Process Box (`srpp`, §9.1.2.2).
pub const SRPP: [u8; BOX_TYPE_SIZE] = *b"srpp";
/// Four-CC of the RTP Header Extension TLV (`rtpx`, §9.4.1.4).
pub const RTPX: [u8; BOX_TYPE_SIZE] = *b"rtpx";
/// Four-CC of the Received CSRC Box (`rcsr`, §9.4.1.4).
pub const RCSR: [u8; BOX_TYPE_SIZE] = *b"rcsr";
/// Four-CC of the PAT Box (`tPAT`, §9.3.3.2).
pub const TPAT: [u8; BOX_TYPE_SIZE] = *b"tPAT";
/// Four-CC of the PMT Box (`tPMT`, §9.3.3.2).
pub const TPMT: [u8; BOX_TYPE_SIZE] = *b"tPMT";
/// Four-CC of the TS Timing Box (`tsti`, §9.3.3.2).
pub const TSTI: [u8; BOX_TYPE_SIZE] = *b"tsti";
/// Four-CC of the Initial Sample Time Box (`istm`, §9.3.3.2).
pub const ISTM: [u8; BOX_TYPE_SIZE] = *b"istm";

const U8_LEN: usize = 1;
const U16_LEN: usize = core::mem::size_of::<u16>();
const U32_LEN: usize = core::mem::size_of::<u32>();
const U64_LEN: usize = core::mem::size_of::<u64>();
const PID_MASK: u16 = 0x1FFF;
const RESERVED3_MASK: u16 = 0xE000;
const RESERVED3_SHIFT: u32 = 13;
const TSTI_METHOD_SHIFT: u32 = 15;
const TSTI_RESERVED_MASK: u16 = 0x6000;
const TSTI_RESERVED_SHIFT: u32 = 13;
const TSSY_SYNC_MASK: u8 = 0x03;
const RTP_SAMPLE_ENTRY_FIXED_LEN: usize = U16_LEN + U16_LEN + U32_LEN;
const MPEG2TS_SAMPLE_ENTRY_FIXED_LEN: usize = U16_LEN + U16_LEN + U8_LEN + U8_LEN + U8_LEN;
const MPEG2TS_PRECOMPUTED_ONLY: u8 = 0x80;
const MPEG2TS_RESERVED_MASK: u8 = 0x7F;
const FD_HINT_SAMPLE_ENTRY_FIXED_LEN: usize = U16_LEN + U16_LEN + U16_LEN + U16_LEN;
const SRPP_FIXED_LEN: usize = 4 * U32_LEN;

fn opaque_from_child(child: &crate::box_types::BoxItem<'_>) -> OpaqueBox {
    OpaqueBox {
        box_type: child.four_cc(),
        data: child.payload().to_vec(),
        to_end: child.to_end(),
        largesize: child.has_largesize(),
    }
}

fn require_type(bytes: &[u8], fourcc: &[u8; BOX_TYPE_SIZE], what: &'static str) -> Result<()> {
    let item = parse_box(bytes)?.0;
    if item.four_cc() != *fourcc {
        return Err(Error::InvalidInput(what));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// hnti — Hint Information Box (§9.1.4)
// ---------------------------------------------------------------------------

/// The movie-level RTP hint information box (`rtp `, §9.1.4.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RtpHintInformation {
    description_format: [u8; BOX_TYPE_SIZE],
    sdp_text: Vec<u8>,
}

impl RtpHintInformation {
    /// Build one from its description format and SDP text.
    #[must_use]
    pub fn new(description_format: [u8; BOX_TYPE_SIZE], sdp_text: &[u8]) -> Self {
        Self {
            description_format,
            sdp_text: sdp_text.to_vec(),
        }
    }

    /// `descriptionformat`, which §9.1.4.2 fixes to `sdp `.
    #[must_use]
    pub const fn description_format(&self) -> [u8; BOX_TYPE_SIZE] {
        self.description_format
    }

    /// The `sdptext` bytes, each line `<crlf>`-terminated.
    #[must_use]
    pub fn sdp_text(&self) -> &[u8] {
        &self.sdp_text
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, BOX_TYPE_SIZE, "rtp sdp descriptionformat")?;
        let mut description_format = [0u8; BOX_TYPE_SIZE];
        description_format.copy_from_slice(&body[..BOX_TYPE_SIZE]);
        Ok(Self {
            description_format,
            sdp_text: body[BOX_TYPE_SIZE..].to_vec(),
        })
    }
}

impl<'a> Parse<'a> for RtpHintInformation {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &RTP, "expected rtp")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for RtpHintInformation {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + BOX_TYPE_SIZE + self.sdp_text.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &RTP, need)?;
        buf[cursor..cursor + BOX_TYPE_SIZE].copy_from_slice(&self.description_format);
        cursor += BOX_TYPE_SIZE;
        buf[cursor..cursor + self.sdp_text.len()].copy_from_slice(&self.sdp_text);
        Ok(cursor + self.sdp_text.len())
    }
}

/// The track-level SDP hint information box (`sdp `, §9.1.4.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SdpHintInformation {
    sdp_text: Vec<u8>,
}

impl SdpHintInformation {
    /// Build one from its SDP text.
    #[must_use]
    pub fn new(sdp_text: &[u8]) -> Self {
        Self {
            sdp_text: sdp_text.to_vec(),
        }
    }

    /// The `sdptext` bytes.
    #[must_use]
    pub fn sdp_text(&self) -> &[u8] {
        &self.sdp_text
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        Ok(Self {
            sdp_text: body.to_vec(),
        })
    }
}

impl<'a> Parse<'a> for SdpHintInformation {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &SDP, "expected sdp")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for SdpHintInformation {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.sdp_text.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let cursor = write_box_header(buf, &SDP, need)?;
        buf[cursor..cursor + self.sdp_text.len()].copy_from_slice(&self.sdp_text);
        Ok(cursor + self.sdp_text.len())
    }
}

/// One child of the hint information box, in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HintInformationChild {
    /// The movie-level `rtp ` box (§9.1.4.2).
    Rtp(RtpHintInformation),
    /// The track-level `sdp ` box (§9.1.4.3).
    Sdp(SdpHintInformation),
    /// Any other child, verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for HintInformationChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Rtp(inner) => inner.serialized_len(),
            Self::Sdp(inner) => inner.serialized_len(),
            Self::Opaque(opaque) => opaque.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Rtp(inner) => inner.serialize_into(buf),
            Self::Sdp(inner) => inner.serialize_into(buf),
            Self::Opaque(opaque) => opaque.serialize_into(buf),
        }
    }
}

/// The Hint Information Box (`hnti`, §9.1.4) — movie and track level.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HintInformationBox {
    children: Vec<HintInformationChild>,
}

impl HintInformationBox {
    /// An empty container.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every child, in wire order.
    #[must_use]
    pub fn children(&self) -> &[HintInformationChild] {
        &self.children
    }

    /// The movie-level RTP information, when present.
    #[must_use]
    pub fn rtp(&self) -> Option<&RtpHintInformation> {
        self.children.iter().find_map(|child| match child {
            HintInformationChild::Rtp(rtp) => Some(rtp),
            _ => None,
        })
    }

    /// The track-level SDP information, when present.
    #[must_use]
    pub fn sdp(&self) -> Option<&SdpHintInformation> {
        self.children.iter().find_map(|child| match child {
            HintInformationChild::Sdp(sdp) => Some(sdp),
            _ => None,
        })
    }

    /// Append a child.
    pub fn push(&mut self, child: HintInformationChild) {
        self.children.push(child);
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut children = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            children.push(if child.four_cc() == RTP {
                HintInformationChild::Rtp(RtpHintInformation::parse_body(child.payload())?)
            } else if child.four_cc() == SDP {
                HintInformationChild::Sdp(SdpHintInformation::parse_body(child.payload())?)
            } else {
                HintInformationChild::Opaque(opaque_from_child(&child))
            });
        }
        Ok(Self { children })
    }
}

impl<'a> Parse<'a> for HintInformationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &HNTI, "expected hnti")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for HintInformationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + self
                    .children
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &HNTI, need)?;
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// hinf — Hint Statistics Box (§9.1.5)
// ---------------------------------------------------------------------------

macro_rules! stat_u64_box {
    ($( $ty:ident , $fourcc:expr , $accessor:ident , $doc:literal ),* $(,)?) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize))]
            #[non_exhaustive]
            pub struct $ty {
                version: u8,
                flags: u32,
                value: u64,
            }

            impl $ty {
                /// Build one with the given 64-bit value.
                #[must_use]
                pub const fn new(value: u64) -> Self {
                    Self { version: 0, flags: 0, value }
                }

                /// The FullBox version.
                #[must_use]
                pub const fn version(&self) -> u8 {
                    self.version
                }

                /// The FullBox flags.
                #[must_use]
                pub const fn flags(&self) -> u32 {
                    self.flags
                }

                /// The 64-bit counter.
                #[must_use]
                pub const fn $accessor(&self) -> u64 {
                    self.value
                }

                /// Parse the payload after the 8-byte box header.
                pub fn parse_body(body: &[u8]) -> Result<Self> {
                    need_in(body, FULLBOX_EXTRA_SIZE + U64_LEN, stringify!($ty))?;
                    Ok(Self {
                        version: body[0],
                        flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
                        value: be_u64(body, FULLBOX_EXTRA_SIZE, stringify!($ty))?,
                    })
                }
            }

            impl<'a> Parse<'a> for $ty {
                type Error = Error;

                fn parse(bytes: &'a [u8]) -> Result<Self> {
                    require_type(bytes, &$fourcc, concat!("expected ", stringify!($ty)))?;
                    Self::parse_body(parse_box(bytes)?.0.payload())
                }
            }

            impl Serialize for $ty {
                type Error = Error;

                fn serialized_len(&self) -> usize {
                    box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U64_LEN)
                }

                fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                    let need = self.serialized_len();
                    need_out(buf, need)?;
                    let mut cursor =
                        write_full_box_header(buf, &$fourcc, need, self.version, self.flags, false)?;
                    buf[cursor..cursor + U64_LEN].copy_from_slice(&self.value.to_be_bytes());
                    cursor += U64_LEN;
                    Ok(cursor)
                }
            }
        )*
    };
}

stat_u64_box! {
    TrpyBox, *b"trpy", bytes_sent, "Total bytes sent including the 12-byte RTP headers (`trpy`, §9.1.5).",
    NumpBox, *b"nump", packets_sent, "Total packets sent (`nump`, §9.1.5).",
    TpylBox, *b"tpyl", bytes_sent, "Total bytes sent excluding the RTP headers (`tpyl`, §9.1.5).",
    DmedBox, *b"dmed", bytes_sent, "Total bytes sent from media tracks (`dmed`, §9.1.5).",
    DimmBox, *b"dimm", bytes_sent, "Total bytes sent in immediate mode (`dimm`, §9.1.5).",
    DrepBox, *b"drep", bytes_sent, "Total bytes in repeated packets (`drep`, §9.1.5).",
}

macro_rules! stat_u32_box {
    ($( $ty:ident , $fourcc:expr , $accessor:ident , $doc:literal ),* $(,)?) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize))]
            #[non_exhaustive]
            pub struct $ty {
                version: u8,
                flags: u32,
                value: u32,
            }

            impl $ty {
                /// Build one with the given 32-bit value.
                #[must_use]
                pub const fn new(value: u32) -> Self {
                    Self { version: 0, flags: 0, value }
                }

                /// The FullBox version.
                #[must_use]
                pub const fn version(&self) -> u8 {
                    self.version
                }

                /// The FullBox flags.
                #[must_use]
                pub const fn flags(&self) -> u32 {
                    self.flags
                }

                /// The 32-bit value.
                #[must_use]
                pub const fn $accessor(&self) -> u32 {
                    self.value
                }

                /// Parse the payload after the 8-byte box header.
                pub fn parse_body(body: &[u8]) -> Result<Self> {
                    need_in(body, FULLBOX_EXTRA_SIZE + U32_LEN, stringify!($ty))?;
                    Ok(Self {
                        version: body[0],
                        flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
                        value: be_u32(body, FULLBOX_EXTRA_SIZE, stringify!($ty))?,
                    })
                }
            }

            impl<'a> Parse<'a> for $ty {
                type Error = Error;

                fn parse(bytes: &'a [u8]) -> Result<Self> {
                    require_type(bytes, &$fourcc, concat!("expected ", stringify!($ty)))?;
                    Self::parse_body(parse_box(bytes)?.0.payload())
                }
            }

            impl Serialize for $ty {
                type Error = Error;

                fn serialized_len(&self) -> usize {
                    box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U32_LEN)
                }

                fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                    let need = self.serialized_len();
                    need_out(buf, need)?;
                    let mut cursor =
                        write_full_box_header(buf, &$fourcc, need, self.version, self.flags, false)?;
                    buf[cursor..cursor + U32_LEN].copy_from_slice(&self.value.to_be_bytes());
                    cursor += U32_LEN;
                    Ok(cursor)
                }
            }
        )*
    };
}

stat_u32_box! {
    TotlBox, *b"totl", bytes_sent, "Total bytes sent including the RTP headers as a 32-bit count (`totl`, §9.1.5).",
    NpckBox, *b"npck", packets_sent, "Total packets sent as a 32-bit count (`npck`, §9.1.5).",
    TpayBox, *b"tpay", bytes_sent, "Total bytes sent excluding the RTP headers as a 32-bit count (`tpay`, §9.1.5).",
    PmaxBox, *b"pmax", bytes, "Largest packet sent including the RTP header (`pmax`, §9.1.5).",
    DmaxBox, *b"dmax", time, "Longest packet duration in milliseconds (`dmax`, §9.1.5).",
}

macro_rules! stat_i32_box {
    ($( $ty:ident , $fourcc:expr , $accessor:ident , $doc:literal ),* $(,)?) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize))]
            #[non_exhaustive]
            pub struct $ty {
                version: u8,
                flags: u32,
                value: i32,
            }

            impl $ty {
                /// Build one with the given signed value.
                #[must_use]
                pub const fn new(value: i32) -> Self {
                    Self { version: 0, flags: 0, value }
                }

                /// The FullBox version.
                #[must_use]
                pub const fn version(&self) -> u8 {
                    self.version
                }

                /// The FullBox flags.
                #[must_use]
                pub const fn flags(&self) -> u32 {
                    self.flags
                }

                /// The signed value.
                #[must_use]
                pub const fn $accessor(&self) -> i32 {
                    self.value
                }

                /// Parse the payload after the 8-byte box header.
                pub fn parse_body(body: &[u8]) -> Result<Self> {
                    need_in(body, FULLBOX_EXTRA_SIZE + U32_LEN, stringify!($ty))?;
                    Ok(Self {
                        version: body[0],
                        flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
                        value: be_u32(body, FULLBOX_EXTRA_SIZE, stringify!($ty))? as i32,
                    })
                }
            }

            impl<'a> Parse<'a> for $ty {
                type Error = Error;

                fn parse(bytes: &'a [u8]) -> Result<Self> {
                    require_type(bytes, &$fourcc, concat!("expected ", stringify!($ty)))?;
                    Self::parse_body(parse_box(bytes)?.0.payload())
                }
            }

            impl Serialize for $ty {
                type Error = Error;

                fn serialized_len(&self) -> usize {
                    box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U32_LEN)
                }

                fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                    let need = self.serialized_len();
                    need_out(buf, need)?;
                    let mut cursor =
                        write_full_box_header(buf, &$fourcc, need, self.version, self.flags, false)?;
                    buf[cursor..cursor + U32_LEN].copy_from_slice(&self.value.to_be_bytes());
                    cursor += U32_LEN;
                    Ok(cursor)
                }
            }
        )*
    };
}

stat_i32_box! {
    TminBox, *b"tmin", time, "Smallest relative transmission time in milliseconds (`tmin`, §9.1.5).",
    TmaxBox, *b"tmax", time, "Largest relative transmission time in milliseconds (`tmax`, §9.1.5).",
}

/// Maximum data rate over a period (`maxr`, §9.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MaxrBox {
    version: u8,
    flags: u32,
    period: u32,
    bytes: u32,
}

impl MaxrBox {
    /// Build one from a period and its byte count.
    #[must_use]
    pub const fn new(period: u32, bytes: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            period,
            bytes,
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The period in milliseconds.
    #[must_use]
    pub const fn period(&self) -> u32 {
        self.period
    }

    /// The maximum bytes sent in any period.
    #[must_use]
    pub const fn bytes(&self) -> u32 {
        self.bytes
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + U32_LEN + U32_LEN, "maxr")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            period: be_u32(body, FULLBOX_EXTRA_SIZE, "maxr period")?,
            bytes: be_u32(body, FULLBOX_EXTRA_SIZE + U32_LEN, "maxr bytes")?,
        })
    }
}

impl<'a> Parse<'a> for MaxrBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &MAXR, "expected maxr")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for MaxrBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U32_LEN + U32_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &MAXR, need, self.version, self.flags, false)?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.period.to_be_bytes());
        cursor += U32_LEN;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.bytes.to_be_bytes());
        cursor += U32_LEN;
        Ok(cursor)
    }
}

/// The payload type used in RTP packets (`payt`, §9.1.5).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PaytBox {
    version: u8,
    flags: u32,
    payload_id: u32,
    rtpmap_string: Vec<u8>,
}

impl PaytBox {
    /// Build one from a payload identifier and its `rtpmap` string.
    #[must_use]
    pub fn new(payload_id: u32, rtpmap_string: &[u8]) -> Self {
        Self {
            version: 0,
            flags: 0,
            payload_id,
            rtpmap_string: rtpmap_string.to_vec(),
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The payload identifier.
    #[must_use]
    pub const fn payload_id(&self) -> u32 {
        self.payload_id
    }

    /// The `rtpmap_string` bytes.
    #[must_use]
    pub fn rtpmap_string(&self) -> &[u8] {
        &self.rtpmap_string
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + U32_LEN + U8_LEN, "payt")?;
        let count = usize::from(body[FULLBOX_EXTRA_SIZE + U32_LEN]);
        let start = FULLBOX_EXTRA_SIZE + U32_LEN + U8_LEN;
        need_in(body, start + count, "payt rtpmap_string")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            payload_id: be_u32(body, FULLBOX_EXTRA_SIZE, "payt payloadID")?,
            rtpmap_string: body[start..start + count].to_vec(),
        })
    }
}

impl<'a> Parse<'a> for PaytBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &PAYT, "expected payt")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for PaytBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + U32_LEN + U8_LEN + self.rtpmap_string.len(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &PAYT, need, self.version, self.flags, false)?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.payload_id.to_be_bytes());
        cursor += U32_LEN;
        let count = broadcast_core::len::fit_u8(self.rtpmap_string.len(), "payt rtpmap_string")?;
        buf[cursor] = count;
        cursor += U8_LEN;
        buf[cursor..cursor + self.rtpmap_string.len()].copy_from_slice(&self.rtpmap_string);
        cursor += self.rtpmap_string.len();
        Ok(cursor)
    }
}

/// One child of the hint statistics box, in wire order (§9.1.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HintStatisticsChild {
    /// Total bytes sent, including RTP headers (`trpy`).
    Trpy(TrpyBox),
    /// Total packets sent (`nump`).
    Nump(NumpBox),
    /// Total bytes sent, excluding RTP headers (`tpyl`).
    Tpyl(TpylBox),
    /// 32-bit total bytes sent, including RTP headers (`totl`).
    Totl(TotlBox),
    /// 32-bit total packets sent (`npck`).
    Npck(NpckBox),
    /// 32-bit total bytes sent, excluding RTP headers (`tpay`).
    Tpay(TpayBox),
    /// Maximum data rate over a period (`maxr`).
    Maxr(MaxrBox),
    /// Total bytes sent from media tracks (`dmed`).
    Dmed(DmedBox),
    /// Total bytes sent in immediate mode (`dimm`).
    Dimm(DimmBox),
    /// Total bytes in repeated packets (`drep`).
    Drep(DrepBox),
    /// Smallest relative transmission time (`tmin`).
    Tmin(TminBox),
    /// Largest relative transmission time (`tmax`).
    Tmax(TmaxBox),
    /// Largest packet sent (`pmax`).
    Pmax(PmaxBox),
    /// Longest packet duration (`dmax`).
    Dmax(DmaxBox),
    /// The RTP payload type (`payt`).
    Payt(PaytBox),
    /// Any other child, verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for HintStatisticsChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Trpy(b) => b.serialized_len(),
            Self::Nump(b) => b.serialized_len(),
            Self::Tpyl(b) => b.serialized_len(),
            Self::Totl(b) => b.serialized_len(),
            Self::Npck(b) => b.serialized_len(),
            Self::Tpay(b) => b.serialized_len(),
            Self::Maxr(b) => b.serialized_len(),
            Self::Dmed(b) => b.serialized_len(),
            Self::Dimm(b) => b.serialized_len(),
            Self::Drep(b) => b.serialized_len(),
            Self::Tmin(b) => b.serialized_len(),
            Self::Tmax(b) => b.serialized_len(),
            Self::Pmax(b) => b.serialized_len(),
            Self::Dmax(b) => b.serialized_len(),
            Self::Payt(b) => b.serialized_len(),
            Self::Opaque(o) => o.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Trpy(b) => b.serialize_into(buf),
            Self::Nump(b) => b.serialize_into(buf),
            Self::Tpyl(b) => b.serialize_into(buf),
            Self::Totl(b) => b.serialize_into(buf),
            Self::Npck(b) => b.serialize_into(buf),
            Self::Tpay(b) => b.serialize_into(buf),
            Self::Maxr(b) => b.serialize_into(buf),
            Self::Dmed(b) => b.serialize_into(buf),
            Self::Dimm(b) => b.serialize_into(buf),
            Self::Drep(b) => b.serialize_into(buf),
            Self::Tmin(b) => b.serialize_into(buf),
            Self::Tmax(b) => b.serialize_into(buf),
            Self::Pmax(b) => b.serialize_into(buf),
            Self::Dmax(b) => b.serialize_into(buf),
            Self::Payt(b) => b.serialize_into(buf),
            Self::Opaque(o) => o.serialize_into(buf),
        }
    }
}

/// The Hint Statistics Box (`hinf`, §9.1.5) — a container of counters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HintStatisticsBox {
    children: Vec<HintStatisticsChild>,
}

impl HintStatisticsBox {
    /// An empty container.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every child, in wire order.
    #[must_use]
    pub fn children(&self) -> &[HintStatisticsChild] {
        &self.children
    }

    /// Append a child.
    pub fn push(&mut self, child: HintStatisticsChild) {
        self.children.push(child);
    }

    /// The 64-bit total bytes sent, when present.
    #[must_use]
    pub fn bytes_sent(&self) -> Option<u64> {
        self.children.iter().find_map(|child| match child {
            HintStatisticsChild::Trpy(b) => Some(b.bytes_sent()),
            _ => None,
        })
    }

    /// The 64-bit total packets sent, when present.
    #[must_use]
    pub fn packets_sent(&self) -> Option<u64> {
        self.children.iter().find_map(|child| match child {
            HintStatisticsChild::Nump(b) => Some(b.packets_sent()),
            _ => None,
        })
    }

    /// The largest packet sent, when present.
    #[must_use]
    pub fn largest_packet(&self) -> Option<u32> {
        self.children.iter().find_map(|child| match child {
            HintStatisticsChild::Pmax(b) => Some(b.bytes()),
            _ => None,
        })
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut children = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            let payload = child.payload();
            children.push(match &child.four_cc() {
                b"trpy" => HintStatisticsChild::Trpy(TrpyBox::parse_body(payload)?),
                b"nump" => HintStatisticsChild::Nump(NumpBox::parse_body(payload)?),
                b"tpyl" => HintStatisticsChild::Tpyl(TpylBox::parse_body(payload)?),
                b"totl" => HintStatisticsChild::Totl(TotlBox::parse_body(payload)?),
                b"npck" => HintStatisticsChild::Npck(NpckBox::parse_body(payload)?),
                b"tpay" => HintStatisticsChild::Tpay(TpayBox::parse_body(payload)?),
                b"maxr" => HintStatisticsChild::Maxr(MaxrBox::parse_body(payload)?),
                b"dmed" => HintStatisticsChild::Dmed(DmedBox::parse_body(payload)?),
                b"dimm" => HintStatisticsChild::Dimm(DimmBox::parse_body(payload)?),
                b"drep" => HintStatisticsChild::Drep(DrepBox::parse_body(payload)?),
                b"tmin" => HintStatisticsChild::Tmin(TminBox::parse_body(payload)?),
                b"tmax" => HintStatisticsChild::Tmax(TmaxBox::parse_body(payload)?),
                b"pmax" => HintStatisticsChild::Pmax(PmaxBox::parse_body(payload)?),
                b"dmax" => HintStatisticsChild::Dmax(DmaxBox::parse_body(payload)?),
                b"payt" => HintStatisticsChild::Payt(PaytBox::parse_body(payload)?),
                _ => HintStatisticsChild::Opaque(opaque_from_child(&child)),
            });
        }
        Ok(Self { children })
    }
}

impl<'a> Parse<'a> for HintStatisticsBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &HINF, "expected hinf")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for HintStatisticsBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + self
                    .children
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &HINF, need)?;
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// §9.1.2 / §9.4.1 entry children
// ---------------------------------------------------------------------------

/// The timescale of an RTP hint track (`tims`, §9.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TimescaleEntryBox {
    timescale: u32,
}

impl TimescaleEntryBox {
    /// Build one from its timescale.
    #[must_use]
    pub const fn new(timescale: u32) -> Self {
        Self { timescale }
    }

    /// The RTP timestamp clock frequency.
    #[must_use]
    pub const fn timescale(&self) -> u32 {
        self.timescale
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        crate::box_support::reject_past_fields(body, U32_LEN, "tims extra bytes")?;
        Ok(Self {
            timescale: be_u32(body, 0, "tims timescale")?,
        })
    }
}

impl<'a> Parse<'a> for TimescaleEntryBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &TIMS, "expected tims")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for TimescaleEntryBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + U32_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &TIMS, need)?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.timescale.to_be_bytes());
        cursor += U32_LEN;
        Ok(cursor)
    }
}

macro_rules! signed_offset_box {
    ($( $ty:ident , $fourcc:expr , $accessor:ident , $doc:literal ),* $(,)?) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize))]
            #[non_exhaustive]
            pub struct $ty {
                offset: i32,
            }

            impl $ty {
                /// Build one from its signed offset.
                #[must_use]
                pub const fn new(offset: i32) -> Self {
                    Self { offset }
                }

                /// The signed offset.
                #[must_use]
                pub const fn $accessor(&self) -> i32 {
                    self.offset
                }

                /// Parse the payload after the 8-byte box header.
                pub fn parse_body(body: &[u8]) -> Result<Self> {
                    crate::box_support::reject_past_fields(
                        body,
                        U32_LEN,
                        concat!(stringify!($ty), " extra bytes"),
                    )?;
                    Ok(Self {
                        offset: be_u32(body, 0, stringify!($ty))? as i32,
                    })
                }
            }

            impl<'a> Parse<'a> for $ty {
                type Error = Error;

                fn parse(bytes: &'a [u8]) -> Result<Self> {
                    require_type(bytes, &$fourcc, concat!("expected ", stringify!($ty)))?;
                    Self::parse_body(parse_box(bytes)?.0.payload())
                }
            }

            impl Serialize for $ty {
                type Error = Error;

                fn serialized_len(&self) -> usize {
                    box_len(BOX_HEADER_MIN_SIZE + U32_LEN)
                }

                fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                    let need = self.serialized_len();
                    need_out(buf, need)?;
                    let mut cursor = write_box_header(buf, &$fourcc, need)?;
                    buf[cursor..cursor + U32_LEN].copy_from_slice(&self.offset.to_be_bytes());
                    cursor += U32_LEN;
                    Ok(cursor)
                }
            }
        )*
    };
}

signed_offset_box! {
    TimeOffsetBox, TSRO, offset, "The RTP timestamp offset (`tsro`, §9.1.2).",
    SequenceOffsetBox, SNRO, offset, "The RTP sequence-number offset (`snro`, §9.1.2).",
}

/// RTP timestamp synchrony (`tssy`, §9.4.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TimestampSynchronyBox {
    reserved: u8,
    timestamp_sync: u8,
}

impl TimestampSynchronyBox {
    /// Build one from its `timestamp_sync` value, reserved bits zero.
    #[must_use]
    pub const fn new(timestamp_sync: u8) -> Self {
        Self {
            reserved: 0,
            timestamp_sync,
        }
    }

    /// The six reserved bits, carried as read.
    #[must_use]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }

    /// `timestamp_sync` (0, 1 or 2; 3 is reserved).
    #[must_use]
    pub const fn timestamp_sync(&self) -> u8 {
        self.timestamp_sync
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, U8_LEN, "tssy")?;
        Ok(Self {
            reserved: body[0] >> TSSY_RESERVED_SHIFT,
            timestamp_sync: body[0] & TSSY_SYNC_MASK,
        })
    }
}

impl<'a> Parse<'a> for TimestampSynchronyBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &TSSY, "expected tssy")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for TimestampSynchronyBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + U8_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &TSSY, need)?;
        let reserved = self.reserved << TSSY_RESERVED_SHIFT;
        buf[cursor] = reserved | (self.timestamp_sync & TSSY_SYNC_MASK);
        cursor += U8_LEN;
        Ok(cursor)
    }
}

/// The SSRC of a received RTP stream (`rssr`, §9.4.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ReceivedSsrcBox {
    ssrc: u32,
}

impl ReceivedSsrcBox {
    /// Build one from its SSRC.
    #[must_use]
    pub const fn new(ssrc: u32) -> Self {
        Self { ssrc }
    }

    /// The received SSRC.
    #[must_use]
    pub const fn ssrc(&self) -> u32 {
        self.ssrc
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        Ok(Self {
            ssrc: be_u32(body, 0, "rssr SSRC")?,
        })
    }
}

impl<'a> Parse<'a> for ReceivedSsrcBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &RSSR, "expected rssr")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for ReceivedSsrcBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + U32_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &RSSR, need)?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.ssrc.to_be_bytes());
        cursor += U32_LEN;
        Ok(cursor)
    }
}

/// A raw RTP header extension (`rtpx`, §9.4.1.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RtpHdrExtBox {
    data: Vec<u8>,
}

impl RtpHdrExtBox {
    /// Build one from the raw extension bytes.
    #[must_use]
    pub fn new(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
        }
    }

    /// The application-specific extension bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        Ok(Self {
            data: body.to_vec(),
        })
    }
}

impl<'a> Parse<'a> for RtpHdrExtBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &RTPX, "expected rtpx")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for RtpHdrExtBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.data.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &RTPX, need)?;
        buf[cursor..cursor + self.data.len()].copy_from_slice(&self.data);
        cursor += self.data.len();
        Ok(cursor)
    }
}

/// The received CSRC list (`rcsr`, §9.4.1.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ReceivedCsrcBox {
    csrc: Vec<u32>,
}

impl ReceivedCsrcBox {
    /// Build one from the CSRC values.
    #[must_use]
    pub fn new(csrc: Vec<u32>) -> Self {
        Self { csrc }
    }

    /// The CSRC values, in header order.
    #[must_use]
    pub fn csrc(&self) -> &[u32] {
        &self.csrc
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        if !body.len().is_multiple_of(U32_LEN) {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: body.len().next_multiple_of(U32_LEN),
                have: body.len(),
                what: "rcsr CSRC list",
            }));
        }
        let mut csrc = Vec::with_capacity(body.len() / U32_LEN);
        for chunk in body.chunks_exact(U32_LEN) {
            let mut word = [0u8; U32_LEN];
            word.copy_from_slice(chunk);
            csrc.push(u32::from_be_bytes(word));
        }
        Ok(Self { csrc })
    }
}

impl<'a> Parse<'a> for ReceivedCsrcBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &RCSR, "expected rcsr")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for ReceivedCsrcBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.csrc.len() * U32_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &RCSR, need)?;
        for value in &self.csrc {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&value.to_be_bytes());
            cursor += U32_LEN;
        }
        Ok(cursor)
    }
}

/// One child of an SRTP process box, in wire order (§9.1.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SrtpProcessChild {
    /// The scheme type box (`schm`, §13.4.5).
    SchemeType(crate::cenc::SchemeTypeBox),
    /// The scheme information box (`schi`, §13.4.7).
    SchemeInformation(crate::cenc::SchemeInformationBox),
    /// Any other child, verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for SrtpProcessChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::SchemeType(b) => b.serialized_len(),
            Self::SchemeInformation(b) => b.serialized_len(),
            Self::Opaque(o) => o.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::SchemeType(b) => b.serialize_into(buf),
            Self::SchemeInformation(b) => b.serialize_into(buf),
            Self::Opaque(o) => o.serialize_into(buf),
        }
    }
}

/// The SRTP process box (`srpp`, §9.1.2.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SrtpProcessBox {
    version: u8,
    flags: u32,
    encryption_algorithm_rtp: u32,
    encryption_algorithm_rtcp: u32,
    integrity_algorithm_rtp: u32,
    integrity_algorithm_rtcp: u32,
    children: Vec<SrtpProcessChild>,
}

impl SrtpProcessBox {
    /// Build one from its four algorithm identifiers, with no scheme boxes.
    #[must_use]
    pub const fn new(
        encryption_algorithm_rtp: u32,
        encryption_algorithm_rtcp: u32,
        integrity_algorithm_rtp: u32,
        integrity_algorithm_rtcp: u32,
    ) -> Self {
        Self {
            version: 0,
            flags: 0,
            encryption_algorithm_rtp,
            encryption_algorithm_rtcp,
            integrity_algorithm_rtp,
            integrity_algorithm_rtcp,
            children: Vec::new(),
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `encryption_algorithm_rtp`.
    #[must_use]
    pub const fn encryption_algorithm_rtp(&self) -> u32 {
        self.encryption_algorithm_rtp
    }

    /// `encryption_algorithm_rtcp`.
    #[must_use]
    pub const fn encryption_algorithm_rtcp(&self) -> u32 {
        self.encryption_algorithm_rtcp
    }

    /// `integrity_algorithm_rtp`.
    #[must_use]
    pub const fn integrity_algorithm_rtp(&self) -> u32 {
        self.integrity_algorithm_rtp
    }

    /// `integrity_algorithm_rtcp`.
    #[must_use]
    pub const fn integrity_algorithm_rtcp(&self) -> u32 {
        self.integrity_algorithm_rtcp
    }

    /// The scheme type box, when present.
    #[must_use]
    pub fn scheme_type(&self) -> Option<&crate::cenc::SchemeTypeBox> {
        self.children.iter().find_map(|child| match child {
            SrtpProcessChild::SchemeType(b) => Some(b),
            _ => None,
        })
    }

    /// The scheme information box, when present.
    #[must_use]
    pub fn scheme_information(&self) -> Option<&crate::cenc::SchemeInformationBox> {
        self.children.iter().find_map(|child| match child {
            SrtpProcessChild::SchemeInformation(b) => Some(b),
            _ => None,
        })
    }

    /// Every child, in wire order.
    #[must_use]
    pub fn children(&self) -> &[SrtpProcessChild] {
        &self.children
    }

    /// Append a child.
    pub fn push(&mut self, child: SrtpProcessChild) {
        self.children.push(child);
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + SRPP_FIXED_LEN, "srpp")?;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let encryption_algorithm_rtp = be_u32(body, cursor, "srpp encryption_algorithm_rtp")?;
        cursor += U32_LEN;
        let encryption_algorithm_rtcp = be_u32(body, cursor, "srpp encryption_algorithm_rtcp")?;
        cursor += U32_LEN;
        let integrity_algorithm_rtp = be_u32(body, cursor, "srpp integrity_algorithm_rtp")?;
        cursor += U32_LEN;
        let integrity_algorithm_rtcp = be_u32(body, cursor, "srpp integrity_algorithm_rtcp")?;
        cursor += U32_LEN;
        let mut children = Vec::new();
        for child in crate::box_types::BoxIter::new(&body[cursor..]) {
            let child = child?;
            children.push(if child.four_cc() == *b"schm" {
                let payload = child.payload();
                need_in(payload, FULLBOX_EXTRA_SIZE, "srpp schm")?;
                SrtpProcessChild::SchemeType(crate::cenc::SchemeTypeBox::parse_body(
                    &payload[FULLBOX_EXTRA_SIZE..],
                    payload[0],
                    u24_at(payload, 1) & FULLBOX_FLAGS_MASK,
                )?)
            } else if child.four_cc() == *b"schi" {
                SrtpProcessChild::SchemeInformation(crate::cenc::SchemeInformationBox::parse(
                    child.bytes,
                )?)
            } else {
                SrtpProcessChild::Opaque(opaque_from_child(&child))
            });
        }
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            encryption_algorithm_rtp,
            encryption_algorithm_rtcp,
            integrity_algorithm_rtp,
            integrity_algorithm_rtcp,
            children,
        })
    }
}

impl<'a> Parse<'a> for SrtpProcessBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &SRPP, "expected srpp")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for SrtpProcessBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + SRPP_FIXED_LEN
                + self
                    .children
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &SRPP, need, self.version, self.flags, false)?;
        for value in [
            self.encryption_algorithm_rtp,
            self.encryption_algorithm_rtcp,
            self.integrity_algorithm_rtp,
            self.integrity_algorithm_rtcp,
        ] {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&value.to_be_bytes());
            cursor += U32_LEN;
        }
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// §9.3.3 MPEG-2 TS entry children
// ---------------------------------------------------------------------------

macro_rules! ts_section_box {
    ($( $ty:ident , $fourcc:expr , $doc:literal ),* $(,)?) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Clone, PartialEq, Eq, Default)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize))]
            #[non_exhaustive]
            pub struct $ty {
                reserved: u8,
                pid: u16,
                section_data: Vec<u8>,
            }

            impl $ty {
                /// Build one from a PID and its table sections.
                #[must_use]
                pub fn new(pid: u16, section_data: &[u8]) -> Self {
                    Self {
                        reserved: 0,
                        pid,
                        section_data: section_data.to_vec(),
                    }
                }

                /// The three reserved bits, carried as read.
                #[must_use]
                pub const fn reserved(&self) -> u8 {
                    self.reserved
                }

                /// The PID the sections were extracted from.
                #[must_use]
                pub const fn pid(&self) -> u16 {
                    self.pid
                }

                /// The complete table sections, concatenated.
                #[must_use]
                pub fn section_data(&self) -> &[u8] {
                    &self.section_data
                }

                /// Parse the payload after the 8-byte box header.
                pub fn parse_body(body: &[u8]) -> Result<Self> {
                    need_in(body, U16_LEN, stringify!($ty))?;
                    let word = be_u16(body, 0, stringify!($ty))?;
                    Ok(Self {
                        reserved: ((word & RESERVED3_MASK) >> RESERVED3_SHIFT) as u8,
                        pid: word & PID_MASK,
                        section_data: body[U16_LEN..].to_vec(),
                    })
                }
            }

            impl<'a> Parse<'a> for $ty {
                type Error = Error;

                fn parse(bytes: &'a [u8]) -> Result<Self> {
                    require_type(bytes, &$fourcc, concat!("expected ", stringify!($ty)))?;
                    Self::parse_body(parse_box(bytes)?.0.payload())
                }
            }

            impl Serialize for $ty {
                type Error = Error;

                fn serialized_len(&self) -> usize {
                    box_len(BOX_HEADER_MIN_SIZE + U16_LEN + self.section_data.len())
                }

                fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                    let need = self.serialized_len();
                    need_out(buf, need)?;
                    let mut cursor = write_box_header(buf, &$fourcc, need)?;
                    let reserved = fit_u16(usize::from(self.reserved), "reserved")?;
                    let word = ((reserved << RESERVED3_SHIFT) & RESERVED3_MASK)
                        | (self.pid & PID_MASK);
                    buf[cursor..cursor + U16_LEN].copy_from_slice(&word.to_be_bytes());
                    cursor += U16_LEN;
                    buf[cursor..cursor + self.section_data.len()]
                        .copy_from_slice(&self.section_data);
                    cursor += self.section_data.len();
                    Ok(cursor)
                }
            }
        )*
    };
}

ts_section_box! {
    PatBox, TPAT, "The PAT sections a MPEG-2 TS hint track describes (`tPAT`, §9.3.3.2).",
    PmtBox, TPMT, "One PMT's sections a MPEG-2 TS hint track describes (`tPMT`, §9.3.3.2).",
    ObjectDescriptorBox, TOD, "The MPEG-2 OD sections a hint track describes (`tOD `, §9.3.3.2).",
}

/// Four-CC of the MPEG-2 OD box (`tOD `, §9.3.3.2).
pub const TOD: [u8; BOX_TYPE_SIZE] = *b"tOD ";
/// Four-CC of the RTP timestamp offset TLV (`rtpo`, §9.1.3.2).
pub const RTPO: [u8; BOX_TYPE_SIZE] = *b"rtpo";
/// Four-CC of the SRTP rollover counter box (`sroc`, §9.4.3.2.3).
pub const SROC: [u8; BOX_TYPE_SIZE] = *b"sroc";

/// The RTP timestamp offset TLV (`rtpo`) — §9.1.3.2.
///
/// The clause states the single field ("a 32-bit signed integer offset to the
/// actual RTP timestamp") without a class block; the box carries exactly that
/// `int(32)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RtpoOffsetBox {
    offset: i32,
}

impl RtpoOffsetBox {
    /// Build one from the signed offset.
    #[must_use]
    pub const fn new(offset: i32) -> Self {
        Self { offset }
    }

    /// The signed RTP timestamp offset.
    #[must_use]
    pub const fn offset(self) -> i32 {
        self.offset
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, U32_LEN, "rtpo")?;
        Ok(Self {
            offset: be_u32(body, 0, "rtpo")? as i32,
        })
    }

    /// Parse the whole box.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != RTPO {
            return Err(Error::UnexpectedBox { expected: "rtpo" });
        }
        Self::parse_body(item.body)
    }
}

impl<'a> Parse<'a> for RtpoOffsetBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for RtpoOffsetBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + U32_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &RTPO, need)?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.offset.to_be_bytes());
        cursor += U32_LEN;
        Ok(cursor)
    }
}

/// The SRTP rollover counter box (`sroc`) — §9.4.3.2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RolloverCounterBox {
    rollover_counter: u32,
}

impl RolloverCounterBox {
    /// Build one from the ROC value.
    #[must_use]
    pub const fn new(rollover_counter: u32) -> Self {
        Self { rollover_counter }
    }

    /// The non-zero ROC value.
    #[must_use]
    pub const fn rollover_counter(self) -> u32 {
        self.rollover_counter
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, U32_LEN, "sroc")?;
        Ok(Self {
            rollover_counter: be_u32(body, 0, "sroc")?,
        })
    }

    /// Parse the whole box.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != SROC {
            return Err(Error::UnexpectedBox { expected: "sroc" });
        }
        Self::parse_body(item.body)
    }
}

impl<'a> Parse<'a> for RolloverCounterBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for RolloverCounterBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + U32_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &SROC, need)?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.rollover_counter.to_be_bytes());
        cursor += U32_LEN;
        Ok(cursor)
    }
}

/// The sample-time derivation method for one PID (`tsti`, §9.3.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TsTimingBox {
    timing_derivation_method: u8,
    reserved: u8,
    pid: u16,
}

impl TsTimingBox {
    /// Build one from its method and PID, reserved bits zero.
    #[must_use]
    pub const fn new(timing_derivation_method: u8, pid: u16) -> Self {
        Self {
            timing_derivation_method,
            reserved: 0,
            pid,
        }
    }

    /// 0 for reception time, 1 for piecewise linearity between PCRs.
    #[must_use]
    pub const fn timing_derivation_method(&self) -> u8 {
        self.timing_derivation_method
    }

    /// The two reserved bits, carried as read.
    #[must_use]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }

    /// The PID the method applies to.
    #[must_use]
    pub const fn pid(&self) -> u16 {
        self.pid
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, U16_LEN, "tsti")?;
        let word = be_u16(body, 0, "tsti")?;
        Ok(Self {
            timing_derivation_method: (word >> TSTI_METHOD_SHIFT) as u8,
            reserved: ((word & TSTI_RESERVED_MASK) >> TSTI_RESERVED_SHIFT) as u8,
            pid: word & PID_MASK,
        })
    }
}

impl<'a> Parse<'a> for TsTimingBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &TSTI, "expected tsti")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for TsTimingBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + U16_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &TSTI, need)?;
        let method = fit_u16(
            usize::from(self.timing_derivation_method),
            "timing_derivation_method",
        )?;
        let reserved = fit_u16(usize::from(self.reserved), "reserved")?;
        let word = ((method << TSTI_METHOD_SHIFT) & TSTI_METHOD_MASK)
            | ((reserved << TSTI_RESERVED_SHIFT) & TSTI_RESERVED_MASK)
            | (self.pid & PID_MASK);
        buf[cursor..cursor + U16_LEN].copy_from_slice(&word.to_be_bytes());
        cursor += U16_LEN;
        Ok(cursor)
    }
}

/// The initial sample time of a MPEG-2 TS hint track (`istm`, §9.3.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct InitialSampleTimeBox {
    initial_sample_time: u32,
    reserved: u32,
}

impl InitialSampleTimeBox {
    /// Build one from its initial sample time, reserved zero.
    #[must_use]
    pub const fn new(initial_sample_time: u32) -> Self {
        Self {
            initial_sample_time,
            reserved: 0,
        }
    }

    /// `initialsampletime`.
    #[must_use]
    pub const fn initial_sample_time(&self) -> u32 {
        self.initial_sample_time
    }

    /// The reserved word, carried as read.
    #[must_use]
    pub const fn reserved(&self) -> u32 {
        self.reserved
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, U32_LEN + U32_LEN, "istm")?;
        Ok(Self {
            initial_sample_time: be_u32(body, 0, "istm initialsampletime")?,
            reserved: be_u32(body, U32_LEN, "istm reserved")?,
        })
    }
}

impl<'a> Parse<'a> for InitialSampleTimeBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        require_type(bytes, &ISTM, "expected istm")?;
        Self::parse_body(parse_box(bytes)?.0.payload())
    }
}

impl Serialize for InitialSampleTimeBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + U32_LEN + U32_LEN)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &ISTM, need)?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.initial_sample_time.to_be_bytes());
        cursor += U32_LEN;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.reserved.to_be_bytes());
        cursor += U32_LEN;
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// Hint sample entries (§9.1.2, §9.2.3, §9.3.3, §9.4.1–9.4.4)
// ---------------------------------------------------------------------------

/// A child of an RTP/SRTP/reception hint sample entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RtpEntryChild {
    /// `tims` — the RTP clock frequency (§9.1.2).
    Timescale(TimescaleEntryBox),
    /// `tsro` — a timestamp offset (§9.1.2).
    TimeOffset(TimeOffsetBox),
    /// `snro` — a sequence-number offset (§9.1.2).
    SequenceOffset(SequenceOffsetBox),
    /// `tssy` — the reception track synchrony (§9.4.1.2).
    TimestampSynchrony(TimestampSynchronyBox),
    /// `rssr` — the received SSRC (§9.4.1.2).
    ReceivedSsrc(ReceivedSsrcBox),
    /// `srpp` — the SRTP algorithms (§9.1.2.2).
    SrtpProcess(SrtpProcessBox),
    /// `sinf` — the protection scheme of a protected entry (§9.4.5, §9.3.5).
    SchemeInformation(crate::cenc::ProtectionSchemeInfoBox),
    /// Any other child, verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for RtpEntryChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Timescale(b) => b.serialized_len(),
            Self::TimeOffset(b) => b.serialized_len(),
            Self::SequenceOffset(b) => b.serialized_len(),
            Self::TimestampSynchrony(b) => b.serialized_len(),
            Self::ReceivedSsrc(b) => b.serialized_len(),
            Self::SrtpProcess(b) => b.serialized_len(),
            Self::SchemeInformation(b) => b.serialized_len(),
            Self::Opaque(o) => o.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Timescale(b) => b.serialize_into(buf),
            Self::TimeOffset(b) => b.serialize_into(buf),
            Self::SequenceOffset(b) => b.serialize_into(buf),
            Self::TimestampSynchrony(b) => b.serialize_into(buf),
            Self::ReceivedSsrc(b) => b.serialize_into(buf),
            Self::SrtpProcess(b) => b.serialize_into(buf),
            Self::SchemeInformation(b) => b.serialize_into(buf),
            Self::Opaque(o) => o.serialize_into(buf),
        }
    }
}

/// An RTP, SRTP, RTCP or SRTP-reception hint sample entry (§9.1.2, §9.4.1–9.4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RtpSampleEntry {
    format: [u8; BOX_TYPE_SIZE],
    reserved: [u8; crate::meta_track::SAMPLE_ENTRY_RESERVED_LEN],
    data_reference_index: u16,
    hint_track_version: u16,
    highest_compatible_version: u16,
    max_packet_size: u32,
    children: Vec<RtpEntryChild>,
}

impl RtpSampleEntry {
    /// Build one for `format` with the given maximum packet size.
    #[must_use]
    pub fn new(format: [u8; BOX_TYPE_SIZE], max_packet_size: u32) -> Self {
        Self {
            format,
            reserved: [0u8; crate::meta_track::SAMPLE_ENTRY_RESERVED_LEN],
            data_reference_index: 1,
            hint_track_version: HINT_TRACK_VERSION,
            highest_compatible_version: HINT_TRACK_VERSION,
            max_packet_size,
            children: Vec::new(),
        }
    }

    /// The entry's four-CC (`rtp `, `srtp`, `rrtp`, `rsrp`, `rtcp`, `stcp`,
    /// `prtp`).
    #[must_use]
    pub const fn format(&self) -> [u8; BOX_TYPE_SIZE] {
        self.format
    }

    /// The entry's `data_reference_index` (§8.5.2.2).
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// `hinttrackversion`.
    #[must_use]
    pub const fn hint_track_version(&self) -> u16 {
        self.hint_track_version
    }

    /// `highestcompatibleversion`.
    #[must_use]
    pub const fn highest_compatible_version(&self) -> u16 {
        self.highest_compatible_version
    }

    /// `maxpacketsize`.
    #[must_use]
    pub const fn max_packet_size(&self) -> u32 {
        self.max_packet_size
    }

    /// Every child, in wire order.
    #[must_use]
    pub fn children(&self) -> &[RtpEntryChild] {
        &self.children
    }

    /// The `tims` timescale, when present.
    #[must_use]
    pub fn timescale(&self) -> Option<u32> {
        self.children.iter().find_map(|child| match child {
            RtpEntryChild::Timescale(b) => Some(b.timescale()),
            _ => None,
        })
    }

    /// The `tsro` timestamp offset, when present.
    #[must_use]
    pub fn time_offset(&self) -> Option<i32> {
        self.children.iter().find_map(|child| match child {
            RtpEntryChild::TimeOffset(b) => Some(b.offset()),
            _ => None,
        })
    }

    /// The `snro` sequence-number offset, when present.
    #[must_use]
    pub fn sequence_offset(&self) -> Option<i32> {
        self.children.iter().find_map(|child| match child {
            RtpEntryChild::SequenceOffset(b) => Some(b.offset()),
            _ => None,
        })
    }

    /// The `tssy` synchrony value, when present.
    #[must_use]
    pub fn timestamp_sync(&self) -> Option<u8> {
        self.children.iter().find_map(|child| match child {
            RtpEntryChild::TimestampSynchrony(b) => Some(b.timestamp_sync()),
            _ => None,
        })
    }

    /// The `rssr` received SSRC, when present.
    #[must_use]
    pub fn received_ssrc(&self) -> Option<u32> {
        self.children.iter().find_map(|child| match child {
            RtpEntryChild::ReceivedSsrc(b) => Some(b.ssrc()),
            _ => None,
        })
    }

    /// The `srpp` SRTP algorithms, when present.
    #[must_use]
    pub fn srtp_process(&self) -> Option<&SrtpProcessBox> {
        self.children.iter().find_map(|child| match child {
            RtpEntryChild::SrtpProcess(b) => Some(b),
            _ => None,
        })
    }

    /// Append a child.
    pub fn push(&mut self, child: RtpEntryChild) {
        self.children.push(child);
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        let format = item.four_cc();
        let (reserved, data_reference_index, rest) =
            crate::meta_track::parse_sample_entry_base(bytes, "hint sample entry")?;
        need_in(rest, RTP_SAMPLE_ENTRY_FIXED_LEN, "RTP hint sample entry")?;
        let hint_track_version = be_u16(rest, 0, "hinttrackversion")?;
        let highest_compatible_version = be_u16(rest, U16_LEN, "highestcompatibleversion")?;
        let max_packet_size = be_u32(rest, U16_LEN + U16_LEN, "maxpacketsize")?;
        let children = parse_rtp_children(&rest[RTP_SAMPLE_ENTRY_FIXED_LEN..])?;
        Ok(Self {
            format,
            reserved,
            data_reference_index,
            hint_track_version,
            highest_compatible_version,
            max_packet_size,
            children,
        })
    }
}

fn parse_rtp_children(body: &[u8]) -> Result<Vec<RtpEntryChild>> {
    let mut children = Vec::new();
    for child in crate::box_types::BoxIter::new(body) {
        let child = child?;
        children.push(match child.four_cc() {
            TIMS => RtpEntryChild::Timescale(TimescaleEntryBox::parse_body(child.payload())?),
            TSRO => RtpEntryChild::TimeOffset(TimeOffsetBox::parse_body(child.payload())?),
            SNRO => RtpEntryChild::SequenceOffset(SequenceOffsetBox::parse_body(child.payload())?),
            TSSY => RtpEntryChild::TimestampSynchrony(TimestampSynchronyBox::parse_body(
                child.payload(),
            )?),
            RSSR => RtpEntryChild::ReceivedSsrc(ReceivedSsrcBox::parse_body(child.payload())?),
            SRPP => RtpEntryChild::SrtpProcess(SrtpProcessBox::parse(child.bytes)?),
            fourcc if fourcc == *b"sinf" => RtpEntryChild::SchemeInformation(
                crate::cenc::ProtectionSchemeInfoBox::parse(child.bytes)?,
            ),
            _ => RtpEntryChild::Opaque(opaque_from_child(&child)),
        });
    }
    Ok(children)
}

impl<'a> Parse<'a> for RtpSampleEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for RtpSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + crate::meta_track::SAMPLE_ENTRY_FIXED_LEN
                + RTP_SAMPLE_ENTRY_FIXED_LEN
                + self
                    .children
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        let mut cursor = crate::meta_track::write_sample_entry_base(
            buf,
            &self.format,
            need,
            &self.reserved,
            self.data_reference_index,
        )?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.hint_track_version.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN]
            .copy_from_slice(&self.highest_compatible_version.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.max_packet_size.to_be_bytes());
        cursor += U32_LEN;
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// A child of an MPEG-2 TS hint sample entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Mpeg2TsEntryChild {
    /// `tPAT` — the PAT sections (§9.3.3.2).
    Pat(PatBox),
    /// `tPMT` — one PMT's sections (§9.3.3.2).
    Pmt(PmtBox),
    /// `tOD ` — the MPEG-2 OD sections (§9.3.3.2).
    ObjectDescriptor(ObjectDescriptorBox),
    /// `tsti` — the sample-time derivation method (§9.3.3.2).
    TsTiming(TsTimingBox),
    /// `istm` — the initial sample time (§9.3.3.2).
    InitialSampleTime(InitialSampleTimeBox),
    /// `sinf` — the protection scheme of a `pm2t` entry (§9.3.5).
    SchemeInformation(crate::cenc::ProtectionSchemeInfoBox),
    /// Any other child, verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for Mpeg2TsEntryChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Pat(b) => b.serialized_len(),
            Self::Pmt(b) => b.serialized_len(),
            Self::ObjectDescriptor(b) => b.serialized_len(),
            Self::TsTiming(b) => b.serialized_len(),
            Self::InitialSampleTime(b) => b.serialized_len(),
            Self::SchemeInformation(b) => b.serialized_len(),
            Self::Opaque(o) => o.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Pat(b) => b.serialize_into(buf),
            Self::Pmt(b) => b.serialize_into(buf),
            Self::ObjectDescriptor(b) => b.serialize_into(buf),
            Self::TsTiming(b) => b.serialize_into(buf),
            Self::InitialSampleTime(b) => b.serialize_into(buf),
            Self::SchemeInformation(b) => b.serialize_into(buf),
            Self::Opaque(o) => o.serialize_into(buf),
        }
    }
}

/// An MPEG-2 TS hint sample entry (`rm2t`, `sm2t` or `pm2t`, §9.3.3, §9.3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Mpeg2TsHintSampleEntry {
    format: [u8; BOX_TYPE_SIZE],
    reserved: [u8; crate::meta_track::SAMPLE_ENTRY_RESERVED_LEN],
    data_reference_index: u16,
    hint_track_version: u16,
    highest_compatible_version: u16,
    preceding_bytes_len: u8,
    trailing_bytes_len: u8,
    precomputed_only_flag: bool,
    reserved_bits: u8,
    children: Vec<Mpeg2TsEntryChild>,
}

impl Mpeg2TsHintSampleEntry {
    /// Build one for `format` with no preceding or trailing bytes.
    #[must_use]
    pub fn new(format: [u8; BOX_TYPE_SIZE]) -> Self {
        Self {
            format,
            reserved: [0u8; crate::meta_track::SAMPLE_ENTRY_RESERVED_LEN],
            data_reference_index: 1,
            hint_track_version: HINT_TRACK_VERSION,
            highest_compatible_version: HINT_TRACK_VERSION,
            preceding_bytes_len: 0,
            trailing_bytes_len: 0,
            precomputed_only_flag: false,
            reserved_bits: 0,
            children: Vec::new(),
        }
    }

    /// The entry's four-CC (`rm2t`, `sm2t`, `pm2t`).
    #[must_use]
    pub const fn format(&self) -> [u8; BOX_TYPE_SIZE] {
        self.format
    }

    /// The entry's `data_reference_index` (§8.5.2.2).
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// `hinttrackversion`.
    #[must_use]
    pub const fn hint_track_version(&self) -> u16 {
        self.hint_track_version
    }

    /// `highestcompatibleversion`.
    #[must_use]
    pub const fn highest_compatible_version(&self) -> u16 {
        self.highest_compatible_version
    }

    /// Bytes preceding each TS packet in a sample.
    #[must_use]
    pub const fn preceding_bytes_len(&self) -> u8 {
        self.preceding_bytes_len
    }

    /// Bytes following each TS packet in a sample.
    #[must_use]
    pub const fn trailing_bytes_len(&self) -> u8 {
        self.trailing_bytes_len
    }

    /// Whether every sample is precomputed.
    #[must_use]
    pub const fn precomputed_only_flag(&self) -> bool {
        self.precomputed_only_flag
    }

    /// The seven reserved bits after the flag, carried as read.
    #[must_use]
    pub const fn reserved_bits(&self) -> u8 {
        self.reserved_bits
    }

    /// Every child, in wire order.
    #[must_use]
    pub fn children(&self) -> &[Mpeg2TsEntryChild] {
        &self.children
    }

    /// The `tPAT` box, when present.
    #[must_use]
    pub fn pat(&self) -> Option<&PatBox> {
        self.children.iter().find_map(|child| match child {
            Mpeg2TsEntryChild::Pat(b) => Some(b),
            _ => None,
        })
    }

    /// Every `tPMT` box, in wire order.
    #[must_use]
    pub fn pmts(&self) -> Vec<&PmtBox> {
        self.children
            .iter()
            .filter_map(|child| match child {
                Mpeg2TsEntryChild::Pmt(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    /// The `tsti` box, when present.
    #[must_use]
    pub fn ts_timing(&self) -> Option<&TsTimingBox> {
        self.children.iter().find_map(|child| match child {
            Mpeg2TsEntryChild::TsTiming(b) => Some(b),
            _ => None,
        })
    }

    /// The `istm` box, when present.
    #[must_use]
    pub fn initial_sample_time(&self) -> Option<&InitialSampleTimeBox> {
        self.children.iter().find_map(|child| match child {
            Mpeg2TsEntryChild::InitialSampleTime(b) => Some(b),
            _ => None,
        })
    }

    /// Append a child.
    pub fn push(&mut self, child: Mpeg2TsEntryChild) {
        self.children.push(child);
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        let format = item.four_cc();
        let (reserved, data_reference_index, rest) =
            crate::meta_track::parse_sample_entry_base(bytes, "mpeg-2 ts hint sample entry")?;
        need_in(
            rest,
            MPEG2TS_SAMPLE_ENTRY_FIXED_LEN,
            "MPEG-2 TS hint sample entry",
        )?;
        let hint_track_version = be_u16(rest, 0, "hinttrackversion")?;
        let highest_compatible_version = be_u16(rest, U16_LEN, "highestcompatibleversion")?;
        let preceding_bytes_len = rest[MPEG2TS_FIELDS_OFFSET];
        let trailing_bytes_len = rest[MPEG2TS_TRAILING_OFFSET];
        let flags = rest[MPEG2TS_FLAGS_OFFSET];
        let mut children = Vec::new();
        for child in crate::box_types::BoxIter::new(&rest[MPEG2TS_SAMPLE_ENTRY_FIXED_LEN..]) {
            let child = child?;
            children.push(match child.four_cc() {
                TPAT => Mpeg2TsEntryChild::Pat(PatBox::parse_body(child.payload())?),
                TPMT => Mpeg2TsEntryChild::Pmt(PmtBox::parse_body(child.payload())?),
                TOD => Mpeg2TsEntryChild::ObjectDescriptor(ObjectDescriptorBox::parse_body(
                    child.payload(),
                )?),
                TSTI => Mpeg2TsEntryChild::TsTiming(TsTimingBox::parse_body(child.payload())?),
                ISTM => Mpeg2TsEntryChild::InitialSampleTime(InitialSampleTimeBox::parse_body(
                    child.payload(),
                )?),
                fourcc if fourcc == *b"sinf" => Mpeg2TsEntryChild::SchemeInformation(
                    crate::cenc::ProtectionSchemeInfoBox::parse(child.bytes)?,
                ),
                _ => Mpeg2TsEntryChild::Opaque(opaque_from_child(&child)),
            });
        }
        Ok(Self {
            format,
            reserved,
            data_reference_index,
            hint_track_version,
            highest_compatible_version,
            preceding_bytes_len,
            trailing_bytes_len,
            precomputed_only_flag: (flags & MPEG2TS_PRECOMPUTED_ONLY) != 0,
            reserved_bits: flags & MPEG2TS_RESERVED_MASK,
            children,
        })
    }
}

impl<'a> Parse<'a> for Mpeg2TsHintSampleEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for Mpeg2TsHintSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + crate::meta_track::SAMPLE_ENTRY_FIXED_LEN
                + MPEG2TS_SAMPLE_ENTRY_FIXED_LEN
                + self
                    .children
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        let mut cursor = crate::meta_track::write_sample_entry_base(
            buf,
            &self.format,
            need,
            &self.reserved,
            self.data_reference_index,
        )?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.hint_track_version.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN]
            .copy_from_slice(&self.highest_compatible_version.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor] = self.preceding_bytes_len;
        cursor += U8_LEN;
        buf[cursor] = self.trailing_bytes_len;
        cursor += U8_LEN;
        let flag = if self.precomputed_only_flag {
            MPEG2TS_PRECOMPUTED_ONLY
        } else {
            0
        };
        buf[cursor] = flag | (self.reserved_bits & MPEG2TS_RESERVED_MASK);
        cursor += U8_LEN;
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// A child of the FD hint sample entry (§9.2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FdHintEntryChild {
    /// `tims` — the packet timing clock (§9.2.3).
    Timescale(TimescaleEntryBox),
    /// Any other child, verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for FdHintEntryChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Timescale(b) => b.serialized_len(),
            Self::Opaque(o) => o.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Timescale(b) => b.serialize_into(buf),
            Self::Opaque(o) => o.serialize_into(buf),
        }
    }
}

/// The FD hint sample entry (`fdp `, §9.2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FdHintSampleEntry {
    reserved: [u8; crate::meta_track::SAMPLE_ENTRY_RESERVED_LEN],
    data_reference_index: u16,
    hint_track_version: u16,
    highest_compatible_version: u16,
    partition_entry_id: u16,
    fec_overhead: u16,
    children: Vec<FdHintEntryChild>,
}

impl FdHintSampleEntry {
    /// Build one from its partition entry identifier and FEC overhead.
    #[must_use]
    pub const fn new(partition_entry_id: u16, fec_overhead: u16) -> Self {
        Self {
            reserved: [0u8; crate::meta_track::SAMPLE_ENTRY_RESERVED_LEN],
            data_reference_index: 1,
            hint_track_version: HINT_TRACK_VERSION,
            highest_compatible_version: HINT_TRACK_VERSION,
            partition_entry_id,
            fec_overhead,
            children: Vec::new(),
        }
    }

    /// The entry's four-CC (`fdp `).
    #[must_use]
    pub const fn format(&self) -> [u8; BOX_TYPE_SIZE] {
        FD_HINT_SAMPLE_ENTRY
    }

    /// The entry's `data_reference_index` (§8.5.2.2).
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// `hinttrackversion`.
    #[must_use]
    pub const fn hint_track_version(&self) -> u16 {
        self.hint_track_version
    }

    /// `highestcompatibleversion`.
    #[must_use]
    pub const fn highest_compatible_version(&self) -> u16 {
        self.highest_compatible_version
    }

    /// `partition_entry_ID`, one-based into the `fiin` array.
    #[must_use]
    pub const fn partition_entry_id(&self) -> u16 {
        self.partition_entry_id
    }

    /// `FEC_overhead`, a fixed 8.8 percentage.
    #[must_use]
    pub const fn fec_overhead(&self) -> u16 {
        self.fec_overhead
    }

    /// The `tims` timescale, when present.
    #[must_use]
    pub fn timescale(&self) -> Option<u32> {
        self.children.iter().find_map(|child| match child {
            FdHintEntryChild::Timescale(b) => Some(b.timescale()),
            _ => None,
        })
    }

    /// Every child, in wire order.
    #[must_use]
    pub fn children(&self) -> &[FdHintEntryChild] {
        &self.children
    }

    /// Append a child.
    pub fn push(&mut self, child: FdHintEntryChild) {
        self.children.push(child);
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let (reserved, data_reference_index, rest) =
            crate::meta_track::parse_sample_entry_base(bytes, "fdp sample entry")?;
        need_in(rest, FD_HINT_SAMPLE_ENTRY_FIXED_LEN, "fdp sample entry")?;
        let hint_track_version = be_u16(rest, 0, "hinttrackversion")?;
        let highest_compatible_version = be_u16(rest, U16_LEN, "highestcompatibleversion")?;
        let partition_entry_id = be_u16(rest, FD_PARTITION_ID_OFFSET, "partition_entry_ID")?;
        let fec_overhead = be_u16(rest, FD_OVERHEAD_OFFSET, "FEC_overhead")?;
        let mut children = Vec::new();
        for child in crate::box_types::BoxIter::new(&rest[FD_HINT_SAMPLE_ENTRY_FIXED_LEN..]) {
            let child = child?;
            children.push(if child.four_cc() == TIMS {
                FdHintEntryChild::Timescale(TimescaleEntryBox::parse_body(child.payload())?)
            } else {
                FdHintEntryChild::Opaque(opaque_from_child(&child))
            });
        }
        Ok(Self {
            reserved,
            data_reference_index,
            hint_track_version,
            highest_compatible_version,
            partition_entry_id,
            fec_overhead,
            children,
        })
    }
}

impl<'a> Parse<'a> for FdHintSampleEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for FdHintSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + crate::meta_track::SAMPLE_ENTRY_FIXED_LEN
                + FD_HINT_SAMPLE_ENTRY_FIXED_LEN
                + self
                    .children
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        let mut cursor = crate::meta_track::write_sample_entry_base(
            buf,
            &FD_HINT_SAMPLE_ENTRY,
            need,
            &self.reserved,
            self.data_reference_index,
        )?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.hint_track_version.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN]
            .copy_from_slice(&self.highest_compatible_version.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.partition_entry_id.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.fec_overhead.to_be_bytes());
        cursor += U16_LEN;
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// Any hint sample entry this crate types.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HintSampleEntry {
    /// `rtp `, `srtp`, `rrtp`, `rsrp`, `rtcp`, `stcp` or `prtp`.
    Rtp(RtpSampleEntry),
    /// `rm2t`, `sm2t` or `pm2t`.
    Mpeg2Ts(Mpeg2TsHintSampleEntry),
    /// `fdp `.
    Fd(FdHintSampleEntry),
}

impl HintSampleEntry {
    /// The entry's four-CC.
    #[must_use]
    pub const fn format(&self) -> [u8; BOX_TYPE_SIZE] {
        match self {
            Self::Rtp(entry) => entry.format(),
            Self::Mpeg2Ts(entry) => entry.format(),
            Self::Fd(entry) => entry.format(),
        }
    }

    /// Parse an entry, dispatching on its four-CC.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let fourcc = parse_box(bytes)?.0.four_cc();
        if matches!(
            &fourcc,
            b"rtp " | b"srtp" | b"rrtp" | b"rsrp" | b"rtcp" | b"stcp" | b"prtp"
        ) {
            Ok(Self::Rtp(RtpSampleEntry::parse_box(bytes)?))
        } else if matches!(&fourcc, b"rm2t" | b"sm2t" | b"pm2t") {
            Ok(Self::Mpeg2Ts(Mpeg2TsHintSampleEntry::parse_box(bytes)?))
        } else if &fourcc == b"fdp " {
            Ok(Self::Fd(FdHintSampleEntry::parse_box(bytes)?))
        } else {
            Err(Error::InvalidInput("not a hint sample entry"))
        }
    }
}

impl Serialize for HintSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Rtp(entry) => entry.serialized_len(),
            Self::Mpeg2Ts(entry) => entry.serialized_len(),
            Self::Fd(entry) => entry.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Rtp(entry) => entry.serialize_into(buf),
            Self::Mpeg2Ts(entry) => entry.serialize_into(buf),
            Self::Fd(entry) => entry.serialize_into(buf),
        }
    }
}

/// The `hinttrackversion` this crate writes (§9.1.2, §9.2.3, §9.3.3).
pub const HINT_TRACK_VERSION: u16 = 1;
/// Four-CC of the FD hint sample entry (`fdp `, §9.2.3).
pub const FD_HINT_SAMPLE_ENTRY: [u8; BOX_TYPE_SIZE] = *b"fdp ";

const RTP_PACKET_FIXED_LEN: usize = U32_LEN + U16_LEN + U16_LEN + U16_LEN + U16_LEN;
const RTP_CONSTRUCTOR_LEN: usize = 16;
const RTP_VERSION_MASK: u8 = 0xC0;
const RTP_VERSION_SHIFT: u32 = 6;
const RTP_PADDING_BIT: u8 = 0x20;
const RTP_EXTENSION_BIT: u8 = 0x10;
const RTP_CSRC_MASK: u8 = 0x0F;
const RTP_MARKER_BIT: u8 = 0x80;
const RTP_PAYLOAD_MASK: u8 = 0x7F;
const RTP_EXTRA_FLAG: u16 = 0x0004;
const RTP_BFRAME_FLAG: u16 = 0x0002;
const RTP_REPEAT_FLAG: u16 = 0x0001;
const RTP_RESERVED_FLAGS_MASK: u16 = 0xFFF8;
const RTP_RESERVED_FLAGS_SHIFT: u32 = 3;
const RTP_IMMEDIATE_DATA_LEN: usize = 14;
const RTP_TRACK_REF_OFFSET: usize = 1;
const RTP_LENGTH_OFFSET: usize = 2;
const RTP_SAMPLE_NUMBER_OFFSET: usize = 4;
const RTP_SAMPLE_OFFSET: usize = 8;
const RTP_BYTES_PER_BLOCK_OFFSET: usize = 12;
const RTP_SAMPLES_PER_BLOCK_OFFSET: usize = 14;
const RTP_DESCRIPTION_INDEX_OFFSET: usize = 4;
const RTP_DESCRIPTION_OFFSET: usize = 8;
const RTP_DESCRIPTION_RESERVED_OFFSET: usize = 12;
const TSSY_RESERVED_SHIFT: u32 = 2;
const TSTI_METHOD_MASK: u16 = 0x8000;
const MPEG2TS_FIELDS_OFFSET: usize = 2 * U16_LEN;
const MPEG2TS_TRAILING_OFFSET: usize = MPEG2TS_FIELDS_OFFSET + U8_LEN;
const MPEG2TS_FLAGS_OFFSET: usize = MPEG2TS_FIELDS_OFFSET + 2 * U8_LEN;
const FD_PARTITION_ID_OFFSET: usize = 2 * U16_LEN;
const FD_OVERHEAD_OFFSET: usize = 3 * U16_LEN;
const RTP_SAMPLE_CONSTRUCTOR_TYPE: u8 = 2;
const RTP_DESCRIPTION_CONSTRUCTOR_TYPE: u8 = 3;

/// One RTP constructor (`RTPconstructor`, §9.1.3.3). Every form is 16 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RtpConstructor {
    /// `type` 0: nothing to insert.
    Noop {
        /// The 15 padding bytes, carried as read.
        pad: [u8; RTP_CONSTRUCTOR_LEN - 1],
    },
    /// `type` 1: inline payload bytes.
    Immediate {
        /// `count`, the number of meaningful payload bytes.
        count: u8,
        /// The 14 bytes after `count`: `data[count]` then padding.
        data: [u8; RTP_IMMEDIATE_DATA_LEN],
    },
    /// `type` 2: bytes copied from a media sample.
    Sample {
        /// `trackrefindex` (§9.1.3.3: 1-based, or 0 for the associated track,
        /// -1 for the hint track itself).
        track_ref_index: i8,
        /// `length` bytes to copy.
        length: u16,
        /// `samplenumber` in the referenced track.
        sample_number: u32,
        /// `sampleoffset` within that sample.
        sample_offset: u32,
        /// `bytesperblock`, always 1 in MP4 files.
        bytes_per_block: u16,
        /// `samplesperblock`, always 1 in MP4 files.
        samples_per_block: u16,
    },
    /// `type` 3: bytes copied from a sample description entry.
    SampleDescription {
        /// `trackrefindex`.
        track_ref_index: i8,
        /// `length` bytes to copy.
        length: u16,
        /// Index into the referenced track's `stsd`.
        sample_description_index: u32,
        /// Offset within that sample entry.
        sample_description_offset: u32,
        /// The trailing reserved word, carried as read.
        reserved: u32,
    },
}

impl RtpConstructor {
    /// Parse one 16-byte constructor.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, RTP_CONSTRUCTOR_LEN, "RTP constructor")?;
        let kind = bytes[0];
        Ok(match kind {
            0 => {
                let mut pad = [0u8; RTP_CONSTRUCTOR_LEN - 1];
                pad.copy_from_slice(&bytes[1..RTP_CONSTRUCTOR_LEN]);
                Self::Noop { pad }
            }
            1 => {
                let count = bytes[1];
                let mut data = [0u8; RTP_IMMEDIATE_DATA_LEN];
                data.copy_from_slice(&bytes[2..RTP_CONSTRUCTOR_LEN]);
                Self::Immediate { count, data }
            }
            RTP_SAMPLE_CONSTRUCTOR_TYPE => Self::Sample {
                track_ref_index: bytes[RTP_TRACK_REF_OFFSET] as i8,
                length: be_u16(bytes, RTP_LENGTH_OFFSET, "RTP constructor length")?,
                sample_number: be_u32(
                    bytes,
                    RTP_SAMPLE_NUMBER_OFFSET,
                    "RTP constructor samplenumber",
                )?,
                sample_offset: be_u32(bytes, RTP_SAMPLE_OFFSET, "RTP constructor sampleoffset")?,
                bytes_per_block: be_u16(
                    bytes,
                    RTP_BYTES_PER_BLOCK_OFFSET,
                    "RTP constructor bytesperblock",
                )?,
                samples_per_block: be_u16(
                    bytes,
                    RTP_SAMPLES_PER_BLOCK_OFFSET,
                    "RTP constructor samplesperblock",
                )?,
            },
            RTP_DESCRIPTION_CONSTRUCTOR_TYPE => Self::SampleDescription {
                track_ref_index: bytes[RTP_TRACK_REF_OFFSET] as i8,
                length: be_u16(bytes, RTP_LENGTH_OFFSET, "RTP constructor length")?,
                sample_description_index: be_u32(
                    bytes,
                    RTP_DESCRIPTION_INDEX_OFFSET,
                    "RTP constructor sampledescriptionindex",
                )?,
                sample_description_offset: be_u32(
                    bytes,
                    RTP_DESCRIPTION_OFFSET,
                    "RTP constructor sampledescriptionoffset",
                )?,
                reserved: be_u32(
                    bytes,
                    RTP_DESCRIPTION_RESERVED_OFFSET,
                    "RTP constructor reserved",
                )?,
            },
            _ => return Err(Error::InvalidInput("unknown RTP constructor type")),
        })
    }

    /// The constructor's union discriminator.
    #[must_use]
    pub const fn constructor_type(&self) -> u8 {
        match self {
            Self::Noop { .. } => 0,
            Self::Immediate { .. } => 1,
            Self::Sample { .. } => RTP_SAMPLE_CONSTRUCTOR_TYPE,
            Self::SampleDescription { .. } => RTP_DESCRIPTION_CONSTRUCTOR_TYPE,
        }
    }

    /// Serialize one constructor into exactly 16 bytes.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, RTP_CONSTRUCTOR_LEN)?;
        buf[0] = self.constructor_type();
        match self {
            Self::Noop { pad } => buf[1..RTP_CONSTRUCTOR_LEN].copy_from_slice(pad),
            Self::Immediate { count, data } => {
                buf[1] = *count;
                buf[2..RTP_CONSTRUCTOR_LEN].copy_from_slice(data);
            }
            Self::Sample {
                track_ref_index,
                length,
                sample_number,
                sample_offset,
                bytes_per_block,
                samples_per_block,
            } => {
                buf[1] = *track_ref_index as u8;
                buf[2..4].copy_from_slice(&length.to_be_bytes());
                buf[4..8].copy_from_slice(&sample_number.to_be_bytes());
                buf[8..12].copy_from_slice(&sample_offset.to_be_bytes());
                buf[12..14].copy_from_slice(&bytes_per_block.to_be_bytes());
                buf[14..16].copy_from_slice(&samples_per_block.to_be_bytes());
            }
            Self::SampleDescription {
                track_ref_index,
                length,
                sample_description_index,
                sample_description_offset,
                reserved,
            } => {
                buf[1] = *track_ref_index as u8;
                buf[2..4].copy_from_slice(&length.to_be_bytes());
                buf[4..8].copy_from_slice(&sample_description_index.to_be_bytes());
                buf[8..12].copy_from_slice(&sample_description_offset.to_be_bytes());
                buf[12..16].copy_from_slice(&reserved.to_be_bytes());
            }
        }
        Ok(RTP_CONSTRUCTOR_LEN)
    }
}

/// One RTP packet's composition instructions (`RTPpacket`, §9.1.3.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RtpPacket {
    relative_time: i32,
    rtp_version: u8,
    padding: bool,
    extension: bool,
    csrc_count: u8,
    marker: bool,
    payload_type: u8,
    sequence_seed: u16,
    reserved: u16,
    extra_flag: bool,
    bframe_flag: bool,
    repeat_flag: bool,
    extra_information: Vec<u8>,
    constructors: Vec<RtpConstructor>,
}

impl RtpPacket {
    /// Build one with no extra information and no constructors.
    #[must_use]
    pub const fn new(
        relative_time: i32,
        payload_type: u8,
        sequence_seed: u16,
        csrc_count: u8,
    ) -> Self {
        Self {
            relative_time,
            rtp_version: 0,
            padding: false,
            extension: false,
            csrc_count,
            marker: false,
            payload_type,
            sequence_seed,
            reserved: 0,
            extra_flag: false,
            bframe_flag: false,
            repeat_flag: false,
            extra_information: Vec::new(),
            constructors: Vec::new(),
        }
    }

    /// `relative_time`, which warps the packet's transmission time.
    #[must_use]
    pub const fn relative_time(&self) -> i32 {
        self.relative_time
    }

    /// `RTP_version` (2 bits).
    #[must_use]
    pub const fn rtp_version(&self) -> u8 {
        self.rtp_version
    }

    /// `P_bit`.
    #[must_use]
    pub const fn padding(&self) -> bool {
        self.padding
    }

    /// `X_bit`.
    #[must_use]
    pub const fn extension(&self) -> bool {
        self.extension
    }

    /// `CSRC_count` (4 bits).
    #[must_use]
    pub const fn csrc_count(&self) -> u8 {
        self.csrc_count
    }

    /// `M_bit`.
    #[must_use]
    pub const fn marker(&self) -> bool {
        self.marker
    }

    /// `payload_type` (7 bits).
    #[must_use]
    pub const fn payload_type(&self) -> u8 {
        self.payload_type
    }

    /// `RTPsequenceseed`.
    #[must_use]
    pub const fn sequence_seed(&self) -> u16 {
        self.sequence_seed
    }

    /// The 13 reserved bits, carried as read.
    #[must_use]
    pub const fn reserved(&self) -> u16 {
        self.reserved
    }

    /// `extra_flag`.
    #[must_use]
    pub const fn extra_flag(&self) -> bool {
        self.extra_flag
    }

    /// `bframe_flag` (reserved in reception tracks).
    #[must_use]
    pub const fn bframe_flag(&self) -> bool {
        self.bframe_flag
    }

    /// `repeat_flag` (reserved in reception tracks).
    #[must_use]
    pub const fn repeat_flag(&self) -> bool {
        self.repeat_flag
    }

    /// The extra-information bytes after the length field, carried verbatim.
    #[must_use]
    pub fn extra_information(&self) -> &[u8] {
        &self.extra_information
    }

    /// The constructors, in wire order.
    #[must_use]
    pub fn constructors(&self) -> &[RtpConstructor] {
        &self.constructors
    }

    /// Append a constructor.
    pub fn push_constructor(&mut self, constructor: RtpConstructor) {
        self.constructors.push(constructor);
    }

    fn parse(cursor: &mut usize, bytes: &[u8]) -> Result<Self> {
        need_in(bytes, *cursor + RTP_PACKET_FIXED_LEN, "RTP packet")?;
        let relative_time = be_u32(bytes, *cursor, "RTP relative_time")? as i32;
        *cursor += U32_LEN;
        let first = bytes[*cursor];
        let second = bytes[*cursor + U8_LEN];
        *cursor += U16_LEN;
        let rtp_version = (first & RTP_VERSION_MASK) >> RTP_VERSION_SHIFT;
        let padding = (first & RTP_PADDING_BIT) != 0;
        let extension = (first & RTP_EXTENSION_BIT) != 0;
        let csrc_count = first & RTP_CSRC_MASK;
        let marker = (second & RTP_MARKER_BIT) != 0;
        let payload_type = second & RTP_PAYLOAD_MASK;
        let sequence_seed = be_u16(bytes, *cursor, "RTP sequence seed")?;
        *cursor += U16_LEN;
        let flags = be_u16(bytes, *cursor, "RTP packet flags")?;
        *cursor += U16_LEN;
        let reserved = (flags & RTP_RESERVED_FLAGS_MASK) >> RTP_RESERVED_FLAGS_SHIFT;
        let extra_flag = (flags & RTP_EXTRA_FLAG) != 0;
        let bframe_flag = (flags & RTP_BFRAME_FLAG) != 0;
        let repeat_flag = (flags & RTP_REPEAT_FLAG) != 0;
        let entry_count = be_u16(bytes, *cursor, "RTP entrycount")?;
        *cursor += U16_LEN;
        let mut extra_information = Vec::new();
        if extra_flag {
            need_in(bytes, *cursor + U32_LEN, "RTP extra_information_length")?;
            let length = be_u32(bytes, *cursor, "RTP extra_information_length")? as usize;
            *cursor += U32_LEN;
            if length < U32_LEN {
                return Err(Error::InvalidInput("RTP extra_information_length"));
            }
            let body_len = length - U32_LEN;
            need_in(bytes, *cursor + body_len, "RTP extra information")?;
            extra_information.extend_from_slice(&bytes[*cursor..*cursor + body_len]);
            *cursor += body_len;
        }
        let constructor_bytes = usize::from(entry_count)
            .checked_mul(RTP_CONSTRUCTOR_LEN)
            .ok_or(Error::InvalidInput("RTP constructor count"))?;
        need_in(bytes, *cursor + constructor_bytes, "RTP constructors")?;
        let mut constructors = Vec::with_capacity(usize::from(entry_count));
        for _ in 0..entry_count {
            constructors.push(RtpConstructor::parse(&bytes[*cursor..])?);
            *cursor += RTP_CONSTRUCTOR_LEN;
        }
        Ok(Self {
            relative_time,
            rtp_version,
            padding,
            extension,
            csrc_count,
            marker,
            payload_type,
            sequence_seed,
            reserved,
            extra_flag,
            bframe_flag,
            repeat_flag,
            extra_information,
            constructors,
        })
    }

    fn serialized_len(&self) -> usize {
        RTP_PACKET_FIXED_LEN
            + if self.extra_flag {
                U32_LEN + self.extra_information.len()
            } else {
                0
            }
            + self.constructors.len() * RTP_CONSTRUCTOR_LEN
    }

    fn serialize_into(&self, buf: &mut [u8], cursor: &mut usize) -> Result<()> {
        need_out(buf, *cursor + self.serialized_len())?;
        buf[*cursor..*cursor + U32_LEN].copy_from_slice(&self.relative_time.to_be_bytes());
        *cursor += U32_LEN;
        let first = ((self.rtp_version << RTP_VERSION_SHIFT) & RTP_VERSION_MASK)
            | if self.padding { RTP_PADDING_BIT } else { 0 }
            | if self.extension { RTP_EXTENSION_BIT } else { 0 }
            | (self.csrc_count & RTP_CSRC_MASK);
        let second =
            if self.marker { RTP_MARKER_BIT } else { 0 } | (self.payload_type & RTP_PAYLOAD_MASK);
        buf[*cursor] = first;
        buf[*cursor + U8_LEN] = second;
        *cursor += U16_LEN;
        buf[*cursor..*cursor + U16_LEN].copy_from_slice(&self.sequence_seed.to_be_bytes());
        *cursor += U16_LEN;
        let flags = ((self.reserved << RTP_RESERVED_FLAGS_SHIFT) & RTP_RESERVED_FLAGS_MASK)
            | if self.extra_flag { RTP_EXTRA_FLAG } else { 0 }
            | if self.bframe_flag { RTP_BFRAME_FLAG } else { 0 }
            | if self.repeat_flag { RTP_REPEAT_FLAG } else { 0 };
        buf[*cursor..*cursor + U16_LEN].copy_from_slice(&flags.to_be_bytes());
        *cursor += U16_LEN;
        let entry_count = fit_u16(self.constructors.len(), "RTP entrycount")?;
        buf[*cursor..*cursor + U16_LEN].copy_from_slice(&entry_count.to_be_bytes());
        *cursor += U16_LEN;
        if self.extra_flag {
            let length = fit_u32(
                U32_LEN + self.extra_information.len(),
                "RTP extra_information_length",
            )?;
            buf[*cursor..*cursor + U32_LEN].copy_from_slice(&length.to_be_bytes());
            *cursor += U32_LEN;
            buf[*cursor..*cursor + self.extra_information.len()]
                .copy_from_slice(&self.extra_information);
            *cursor += self.extra_information.len();
        }
        for constructor in &self.constructors {
            constructor.serialize_into(&mut buf[*cursor..])?;
            *cursor += RTP_CONSTRUCTOR_LEN;
        }
        Ok(())
    }
}

/// One RTP hint sample (`RTPsample`, §9.1.3.1) — the sample's bytes typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RtpSample {
    reserved: u16,
    packets: Vec<RtpPacket>,
    extra_data: Vec<u8>,
}

impl RtpSample {
    /// An empty sample.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The 16-bit reserved field, carried as read.
    #[must_use]
    pub const fn reserved(&self) -> u16 {
        self.reserved
    }

    /// The packets this sample composes.
    #[must_use]
    pub fn packets(&self) -> &[RtpPacket] {
        &self.packets
    }

    /// The bytes after the last packet (`extradata`).
    #[must_use]
    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    /// Append a packet.
    pub fn push_packet(&mut self, packet: RtpPacket) {
        self.packets.push(packet);
    }

    /// Parse one sample's bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, U16_LEN + U16_LEN, "RTP sample")?;
        let packet_count = be_u16(bytes, 0, "RTP packetcount")?;
        let reserved = be_u16(bytes, U16_LEN, "RTP sample reserved")?;
        let mut cursor = U16_LEN + U16_LEN;
        let mut packets = Vec::with_capacity(usize::from(packet_count));
        for _ in 0..packet_count {
            packets.push(RtpPacket::parse(&mut cursor, bytes)?);
        }
        Ok(Self {
            reserved,
            packets,
            extra_data: bytes[cursor..].to_vec(),
        })
    }
}

impl Serialize for RtpSample {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        U16_LEN
            + U16_LEN
            + self
                .packets
                .iter()
                .map(RtpPacket::serialized_len)
                .sum::<usize>()
            + self.extra_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let packet_count = fit_u16(self.packets.len(), "RTP packetcount")?;
        buf[..U16_LEN].copy_from_slice(&packet_count.to_be_bytes());
        buf[U16_LEN..U16_LEN + U16_LEN].copy_from_slice(&self.reserved.to_be_bytes());
        let mut cursor = U16_LEN + U16_LEN;
        for packet in &self.packets {
            packet.serialize_into(buf, &mut cursor)?;
        }
        buf[cursor..cursor + self.extra_data.len()].copy_from_slice(&self.extra_data);
        cursor += self.extra_data.len();
        Ok(cursor)
    }
}
