//! T2-MI payload type 0x21: Individual addressing — §5.2.8.
//!
//! Carries per-transmitter addressing data: an outer loop of transmitter
//! entries, each containing a `transmitter_identifier` and an inner loop of
//! typed function entries (ACE-PAPR, MISO group, Frequency, etc.).
//!
//! # Wire layout (ETSI TS 102 773 §5.2.8.1, Fig 11)
//!
//! ```text
//! rfu(8) · individual_addressing_length(8) · transmitter_loop()
//!
//! transmitter_loop() = for each transmitter:
//!   transmitter_identifier(16) · function_loop_length(8) · function()…
//!
//! function() = function_tag(8) · function_length(8) · function_body(function_length bytes)
//! ```
//!
//! `individual_addressing_length` counts the bytes of the transmitter loop
//! (everything after the 2-byte header).  `function_loop_length` counts the
//! bytes of the function loop within one transmitter entry (tag + length +
//! body of every function).  `function_length` counts only the body bytes.
//!
//! # RFU policy (individual-addressing exception)
//!
//! This payload is the crate's documented exception: non-zero RFU bits are
//! **preserved verbatim** rather than rejected, so gateway streams round-trip
//! byte-exact.  This applies to the top-level `rfu` byte and to every RFU
//! field inside the typed function bodies.

use std::fmt;

use num_enum::TryFromPrimitive;

use dvb_common::{Parse, Serialize};

/// Function tags per §5.2.8.2 Tables 5 & 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum AddressingFunctionTag {
    /// Transmitter time offset.
    TimeOffset = 0x00,
    /// Transmitter frequency offset.
    FrequencyOffset = 0x01,
    /// Transmitter power.
    Power = 0x02,
    /// Private data.
    PrivateData = 0x03,
    /// Cell ID.
    CellId = 0x04,
    /// Enable.
    Enable = 0x05,
    /// Bandwidth (not applicable for T2).
    Bandwidth = 0x06,
    /// ACE-PAPR reduction (T2-specific).
    AcePapr = 0x10,
    /// MISO group (T2-specific).
    MisoGroup = 0x11,
    /// TR-PAPR reduction (T2-specific).
    TrPapr = 0x12,
    /// L1-ACE-PAPR (T2-specific).
    L1AcePapr = 0x13,
    /// TX-SIG FEF sequence number (T2-specific).
    TxSigFefSeqNum = 0x15,
    /// TX-SIG auxiliary stream TX ID (T2-specific).
    TxSigAuxStreamTxId = 0x16,
    /// Frequency (T2-specific).
    Frequency = 0x17,
}

impl From<AddressingFunctionTag> for u8 {
    fn from(tag: AddressingFunctionTag) -> Self {
        tag as u8
    }
}

impl fmt::Display for AddressingFunctionTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

// ── Typed function bodies (§5.2.8.2 Tables 7–12b) ──────────────────────────

/// ACE-PAPR function body per §5.2.8.2.2, Table 7.
///
/// Layout (16 bits = 2 bytes):
/// - byte 0 `[7:3]`: ACE_gain (5 bits)
/// - byte 0 `[2:0]`: ACE_maximal_extension (3 bits)
/// - byte 1 `[7:1]`: ACE_clipping_threshold (7 bits)
/// - byte 1 `[0]`: rfu (1 bit) — preserved per individual-addressing exception
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AcePaprBody {
    /// ACE gain `[7:3]` (5 bits).
    pub ace_gain: u8,
    /// ACE maximal extension `[2:0]` (3 bits).
    pub ace_maximal_extension: u8,
    /// ACE clipping threshold `[7:1]` (7 bits).
    pub ace_clipping_threshold: u8,
    /// Reserved-for-future-use bit `[0]` — preserved verbatim.
    pub rfu: bool,
}

/// MISO group function body per §5.2.8.2.2, Table 8.
///
/// Layout (8 bits = 1 byte):
/// - `[7]`: MISO_group (1 bit)
/// - `[6:0]`: rfu (7 bits) — preserved per individual-addressing exception
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MisoGroupBody {
    /// MISO group flag `[7]` (1 bit).
    pub miso_group: bool,
    /// Reserved-for-future-use `[6:0]` (7 bits) — preserved verbatim.
    pub rfu: u8,
}

/// TR-PAPR function body per §5.2.8.2.4, Table 9.
///
/// Layout (40 bits = 5 bytes):
/// - byte 0 `[7:4]`: rfu1 (4 bits) — preserved
/// - byte 0 `[3:0]` + byte 1: TR_clipping_threshold (12 bits)
/// - byte 2 + byte 3 `[7:2]`: rfu2 (14 bits) — preserved
/// - byte 3 `[1:0]` + byte 4: number_of_iterations (10 bits)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrPaprBody {
    /// Reserved-for-future-use `[7:4]` (4 bits) — preserved verbatim.
    pub rfu1: u8,
    /// TR clipping threshold (12 bits).
    pub tr_clipping_threshold: u16,
    /// Reserved-for-future-use (14 bits) — preserved verbatim.
    pub rfu2: u16,
    /// Number of iterations (10 bits).
    pub number_of_iterations: u16,
}

/// L1-ACE-PAPR function body per §5.2.8.2.5, Table 10.
///
/// Layout (32 bits = 4 bytes):
/// - bytes 0-1: L1_ACE_max_correction (16 bits)
/// - bytes 2-3: rfu (16 bits) — preserved per individual-addressing exception
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct L1AcePaprBody {
    /// L1 ACE max correction (16 bits).
    pub l1_ace_max_correction: u16,
    /// Reserved-for-future-use (16 bits) — preserved verbatim.
    pub rfu: u16,
}

/// TX-SIG FEF sequence numbers function body per §5.2.8.2.5, Table 11.
///
/// Layout (40 bits = 5 bytes):
/// - byte 0 `[7:3]`: rfu1 (5 bits) — preserved
/// - byte 0 `[2:0]`: TX_SIG_FEF_SEQ_NUM_1 (3 bits)
/// - byte 1 `[7:3]`: rfu2 (5 bits) — preserved
/// - byte 1 `[2:0]`: TX_SIG_FEF_SEQ_NUM_2 (3 bits)
/// - bytes 2-4: rfu3 (24 bits) — preserved
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TxSigFefSeqNumBody {
    /// Reserved-for-future-use `[7:3]` (5 bits) — preserved verbatim.
    pub rfu1: u8,
    /// TX-SIG FEF sequence number 1 `[2:0]` (3 bits).
    pub seq_num_1: u8,
    /// Reserved-for-future-use `[7:3]` (5 bits) — preserved verbatim.
    pub rfu2: u8,
    /// TX-SIG FEF sequence number 2 `[2:0]` (3 bits).
    pub seq_num_2: u8,
    /// Reserved-for-future-use (24 bits) — preserved verbatim.
    pub rfu3: u32,
}

/// TX-SIG auxiliary stream transmitter ID function body per §5.2.9, Table 12a.
///
/// Layout (32 bits = 4 bytes):
/// - byte 0 + byte 1 `[7:4]`: TX_SIG_AUX_TX_ID (12 bits)
/// - byte 1 `[3:0]` + bytes 2-3: rfu (20 bits) — preserved
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TxSigAuxStreamTxIdBody {
    /// TX-SIG auxiliary stream transmitter ID (12 bits).
    pub tx_sig_aux_tx_id: u16,
    /// Reserved-for-future-use (20 bits) — preserved verbatim.
    pub rfu: u32,
}

/// Frequency function body per §5.2.9, Table 12b.
///
/// Layout (40 bits = 5 bytes):
/// - byte 0 `[7:5]`: rf_idx (3 bits)
/// - byte 0 `[4:0]`: frequency `[31:27]`
/// - bytes 1-3: frequency `[26:3]`
/// - byte 4 `[7:5]`: frequency `[2:0]`
/// - byte 4 `[4:0]`: rfu (5 bits) — preserved
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FrequencyBody {
    /// RF index `[7:5]` (3 bits).
    pub rf_idx: u8,
    /// Frequency in Hz (32 bits).
    pub frequency: u32,
    /// Reserved-for-future-use `[4:0]` (5 bits) — preserved verbatim.
    pub rfu: u8,
}

// ── Body size constants ────────────────────────────────────────────────────

const ACE_PAPR_BODY_LEN: usize = 2;
const MISO_GROUP_BODY_LEN: usize = 1;
const TR_PAPR_BODY_LEN: usize = 5;
const L1_ACE_PAPR_BODY_LEN: usize = 4;
const TX_SIG_FEF_SEQ_NUM_BODY_LEN: usize = 5;
const TX_SIG_AUX_STREAM_TX_ID_BODY_LEN: usize = 4;
const FREQUENCY_BODY_LEN: usize = 5;

// ── FunctionBody ───────────────────────────────────────────────────────────

/// Parsed function body — typed for known tags, raw for unknown/reserved or
/// length-mismatched entries.
///
/// Typed variants correspond to the function bodies whose wire layouts are
/// defined in ETSI TS 102 773 §5.2.8.2 Tables 7–12b.  [`FunctionBody::Raw`]
/// is the escape for:
/// - tags not in [`AddressingFunctionTag`] (reserved / private),
/// - tags in [`AddressingFunctionTag`] whose body layout is not vendored
///   (e.g. 0x00–0x06), or
/// - a known tag whose `function_length` doesn't match the expected body size
///   (future extension).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FunctionBody<'a> {
    /// ACE-PAPR (tag 0x10) — §5.2.8.2.2, Table 7.
    AcePapr(AcePaprBody),
    /// MISO group (tag 0x11) — §5.2.8.2.2, Table 8.
    MisoGroup(MisoGroupBody),
    /// TR-PAPR (tag 0x12) — §5.2.8.2.4, Table 9.
    TrPapr(TrPaprBody),
    /// L1-ACE-PAPR (tag 0x13) — §5.2.8.2.5, Table 10.
    L1AcePapr(L1AcePaprBody),
    /// TX-SIG FEF sequence numbers (tag 0x15) — §5.2.8.2.5, Table 11.
    TxSigFefSeqNum(TxSigFefSeqNumBody),
    /// TX-SIG auxiliary stream transmitter ID (tag 0x16) — §5.2.9, Table 12a.
    TxSigAuxStreamTxId(TxSigAuxStreamTxIdBody),
    /// Frequency (tag 0x17) — §5.2.9, Table 12b.
    Frequency(FrequencyBody),
    /// Unknown/reserved tag or known tag with unexpected `function_length`.
    /// `body` is the raw function body bytes (after tag + length, exactly
    /// `function_length` bytes).
    Raw(&'a [u8]),
}

// ── TransmitterEntry ───────────────────────────────────────────────────────

/// One transmitter entry in the individual-addressing loop (§5.2.8.1).
///
/// Wire layout within the transmitter loop:
/// `transmitter_identifier(16) · function_loop_length(8) · function()…`
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TransmitterEntry<'a> {
    /// Transmitter identifier (16 bits).
    pub transmitter_id: u16,
    /// Function entries within this transmitter.
    pub functions: Vec<FunctionEntry<'a>>,
}

// ── FunctionEntry ───────────────────────────────────────────────────────────

/// One function entry within a transmitter's function loop (§5.2.8.2).
///
/// Wire layout: `function_tag(8) · function_length(8) · function_body(function_length bytes)`
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct FunctionEntry<'a> {
    /// Raw function tag byte (8 bits). Use [`FunctionEntry::addressing_tag`]
    /// to convert to [`AddressingFunctionTag`] when the tag is known.
    pub tag: u8,
    /// Parsed function body — typed for known tags, raw otherwise.
    pub body: FunctionBody<'a>,
}

impl<'a> FunctionEntry<'a> {
    /// Convert the raw `tag` byte to [`AddressingFunctionTag`], if it is a
    /// known value.
    #[must_use]
    pub fn addressing_tag(&self) -> Option<AddressingFunctionTag> {
        AddressingFunctionTag::try_from(self.tag).ok()
    }
}

// ── IndividualAddressingPayload ─────────────────────────────────────────────

/// Individual addressing payload (type 0x21) per ETSI TS 102 773 §5.2.8.1, Fig 11.
///
/// Top-level layout:
/// - byte 0: rfu (8 bits) — preserved verbatim (individual-addressing RFU exception)
/// - byte 1: individual_addressing_length (8 bits) — length of the transmitter loop
/// - bytes 2..: transmitter loop — fully typed as [`TransmitterEntry`] vector
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct IndividualAddressingPayload<'a> {
    /// Reserved-for-future-use byte (byte 0); preserved verbatim for round-trip.
    pub rfu: u8,
    /// Typed transmitter entries.  The 8-bit `individual_addressing_length`
    /// field is derived from the serialized size of this vector on serialize.
    pub transmitters: Vec<TransmitterEntry<'a>>,
}

// ── Wire constants ─────────────────────────────────────────────────────────

const HEADER_LEN: usize = 2;
const TX_HEADER_LEN: usize = 3;
const FUNC_HEADER_LEN: usize = 2;

// ── Private body parse helpers ──────────────────────────────────────────────

fn parse_ace_papr(body: &[u8]) -> Result<AcePaprBody, crate::Error> {
    if body.len() < ACE_PAPR_BODY_LEN {
        return Err(crate::Error::BufferTooShort {
            need: ACE_PAPR_BODY_LEN,
            have: body.len(),
            what: "ACE-PAPR function body",
        });
    }
    Ok(AcePaprBody {
        ace_gain: (body[0] >> 3) & 0x1F,
        ace_maximal_extension: body[0] & 0x07,
        ace_clipping_threshold: (body[1] >> 1) & 0x7F,
        rfu: body[1] & 0x01 != 0,
    })
}

fn parse_miso_group(body: &[u8]) -> Result<MisoGroupBody, crate::Error> {
    if body.len() < MISO_GROUP_BODY_LEN {
        return Err(crate::Error::BufferTooShort {
            need: MISO_GROUP_BODY_LEN,
            have: body.len(),
            what: "MISO group function body",
        });
    }
    Ok(MisoGroupBody {
        miso_group: body[0] & 0x80 != 0,
        rfu: body[0] & 0x7F,
    })
}

fn parse_tr_papr(body: &[u8]) -> Result<TrPaprBody, crate::Error> {
    if body.len() < TR_PAPR_BODY_LEN {
        return Err(crate::Error::BufferTooShort {
            need: TR_PAPR_BODY_LEN,
            have: body.len(),
            what: "TR-PAPR function body",
        });
    }
    Ok(TrPaprBody {
        rfu1: (body[0] >> 4) & 0x0F,
        tr_clipping_threshold: ((body[0] as u16 & 0x0F) << 8) | body[1] as u16,
        rfu2: ((body[2] as u16) << 6) | ((body[3] as u16) >> 2),
        number_of_iterations: ((body[3] as u16 & 0x03) << 8) | body[4] as u16,
    })
}

fn parse_l1_ace_papr(body: &[u8]) -> Result<L1AcePaprBody, crate::Error> {
    if body.len() < L1_ACE_PAPR_BODY_LEN {
        return Err(crate::Error::BufferTooShort {
            need: L1_ACE_PAPR_BODY_LEN,
            have: body.len(),
            what: "L1-ACE-PAPR function body",
        });
    }
    Ok(L1AcePaprBody {
        l1_ace_max_correction: u16::from_be_bytes([body[0], body[1]]),
        rfu: u16::from_be_bytes([body[2], body[3]]),
    })
}

fn parse_tx_sig_fef_seq_num(body: &[u8]) -> Result<TxSigFefSeqNumBody, crate::Error> {
    if body.len() < TX_SIG_FEF_SEQ_NUM_BODY_LEN {
        return Err(crate::Error::BufferTooShort {
            need: TX_SIG_FEF_SEQ_NUM_BODY_LEN,
            have: body.len(),
            what: "TX-SIG FEF sequence numbers function body",
        });
    }
    Ok(TxSigFefSeqNumBody {
        rfu1: (body[0] >> 3) & 0x1F,
        seq_num_1: body[0] & 0x07,
        rfu2: (body[1] >> 3) & 0x1F,
        seq_num_2: body[1] & 0x07,
        rfu3: (body[2] as u32) << 16 | (body[3] as u32) << 8 | body[4] as u32,
    })
}

fn parse_tx_sig_aux_stream_tx_id(body: &[u8]) -> Result<TxSigAuxStreamTxIdBody, crate::Error> {
    if body.len() < TX_SIG_AUX_STREAM_TX_ID_BODY_LEN {
        return Err(crate::Error::BufferTooShort {
            need: TX_SIG_AUX_STREAM_TX_ID_BODY_LEN,
            have: body.len(),
            what: "TX-SIG aux stream TX ID function body",
        });
    }
    Ok(TxSigAuxStreamTxIdBody {
        tx_sig_aux_tx_id: ((body[0] as u16) << 4) | ((body[1] as u16) >> 4),
        rfu: ((body[1] as u32 & 0x0F) << 16) | (body[2] as u32) << 8 | body[3] as u32,
    })
}

fn parse_frequency(body: &[u8]) -> Result<FrequencyBody, crate::Error> {
    if body.len() < FREQUENCY_BODY_LEN {
        return Err(crate::Error::BufferTooShort {
            need: FREQUENCY_BODY_LEN,
            have: body.len(),
            what: "Frequency function body",
        });
    }
    Ok(FrequencyBody {
        rf_idx: (body[0] >> 5) & 0x07,
        frequency: ((body[0] as u32 & 0x1F) << 27)
            | (body[1] as u32) << 19
            | (body[2] as u32) << 11
            | (body[3] as u32) << 3
            | ((body[4] >> 5) as u32 & 0x07),
        rfu: body[4] & 0x1F,
    })
}

/// Try to parse a typed body for `tag` from `body_bytes`.
/// Returns `None` if the tag is known but `body_bytes.len()` doesn't match
/// the expected size (fall back to Raw).  Returns `None` for unknown tags.
/// (A length-matched body parse cannot fail for valid data; the per-helper
/// `BufferTooShort` checks are defensive and unreachable at this call site.)
fn try_parse_typed_body(
    tag: u8,
    body_bytes: &[u8],
) -> Option<Result<FunctionBody<'_>, crate::Error>> {
    match tag {
        t if t == AddressingFunctionTag::AcePapr as u8 && body_bytes.len() == ACE_PAPR_BODY_LEN => {
            Some(parse_ace_papr(body_bytes).map(FunctionBody::AcePapr))
        }
        t if t == AddressingFunctionTag::MisoGroup as u8
            && body_bytes.len() == MISO_GROUP_BODY_LEN =>
        {
            Some(parse_miso_group(body_bytes).map(FunctionBody::MisoGroup))
        }
        t if t == AddressingFunctionTag::TrPapr as u8 && body_bytes.len() == TR_PAPR_BODY_LEN => {
            Some(parse_tr_papr(body_bytes).map(FunctionBody::TrPapr))
        }
        t if t == AddressingFunctionTag::L1AcePapr as u8
            && body_bytes.len() == L1_ACE_PAPR_BODY_LEN =>
        {
            Some(parse_l1_ace_papr(body_bytes).map(FunctionBody::L1AcePapr))
        }
        t if t == AddressingFunctionTag::TxSigFefSeqNum as u8
            && body_bytes.len() == TX_SIG_FEF_SEQ_NUM_BODY_LEN =>
        {
            Some(parse_tx_sig_fef_seq_num(body_bytes).map(FunctionBody::TxSigFefSeqNum))
        }
        t if t == AddressingFunctionTag::TxSigAuxStreamTxId as u8
            && body_bytes.len() == TX_SIG_AUX_STREAM_TX_ID_BODY_LEN =>
        {
            Some(parse_tx_sig_aux_stream_tx_id(body_bytes).map(FunctionBody::TxSigAuxStreamTxId))
        }
        t if t == AddressingFunctionTag::Frequency as u8
            && body_bytes.len() == FREQUENCY_BODY_LEN =>
        {
            Some(parse_frequency(body_bytes).map(FunctionBody::Frequency))
        }
        _ => None,
    }
}

// ── Private body serialize helpers ─────────────────────────────────────────

fn serialize_ace_papr(body: &AcePaprBody, buf: &mut [u8]) {
    buf[0] = (body.ace_gain & 0x1F) << 3 | (body.ace_maximal_extension & 0x07);
    buf[1] = (body.ace_clipping_threshold & 0x7F) << 1 | if body.rfu { 1 } else { 0 };
}

fn serialize_miso_group(body: &MisoGroupBody, buf: &mut [u8]) {
    buf[0] = if body.miso_group { 0x80 } else { 0x00 } | (body.rfu & 0x7F);
}

fn serialize_tr_papr(body: &TrPaprBody, buf: &mut [u8]) {
    buf[0] = (body.rfu1 & 0x0F) << 4 | ((body.tr_clipping_threshold >> 8) as u8 & 0x0F);
    buf[1] = (body.tr_clipping_threshold & 0xFF) as u8;
    buf[2] = (body.rfu2 >> 6) as u8;
    buf[3] = ((body.rfu2 & 0x3F) as u8) << 2 | ((body.number_of_iterations >> 8) as u8 & 0x03);
    buf[4] = (body.number_of_iterations & 0xFF) as u8;
}

fn serialize_l1_ace_papr(body: &L1AcePaprBody, buf: &mut [u8]) {
    buf[0..2].copy_from_slice(&body.l1_ace_max_correction.to_be_bytes());
    buf[2..4].copy_from_slice(&body.rfu.to_be_bytes());
}

fn serialize_tx_sig_fef_seq_num(body: &TxSigFefSeqNumBody, buf: &mut [u8]) {
    buf[0] = (body.rfu1 & 0x1F) << 3 | (body.seq_num_1 & 0x07);
    buf[1] = (body.rfu2 & 0x1F) << 3 | (body.seq_num_2 & 0x07);
    buf[2] = ((body.rfu3 >> 16) & 0xFF) as u8;
    buf[3] = ((body.rfu3 >> 8) & 0xFF) as u8;
    buf[4] = (body.rfu3 & 0xFF) as u8;
}

fn serialize_tx_sig_aux_stream_tx_id(body: &TxSigAuxStreamTxIdBody, buf: &mut [u8]) {
    buf[0] = ((body.tx_sig_aux_tx_id >> 4) & 0xFF) as u8;
    buf[1] = ((body.tx_sig_aux_tx_id & 0x0F) as u8) << 4 | ((body.rfu >> 16) & 0x0F) as u8;
    buf[2] = ((body.rfu >> 8) & 0xFF) as u8;
    buf[3] = (body.rfu & 0xFF) as u8;
}

fn serialize_frequency(body: &FrequencyBody, buf: &mut [u8]) {
    buf[0] = (body.rf_idx & 0x07) << 5 | ((body.frequency >> 27) & 0x1F) as u8;
    buf[1] = ((body.frequency >> 19) & 0xFF) as u8;
    buf[2] = ((body.frequency >> 11) & 0xFF) as u8;
    buf[3] = ((body.frequency >> 3) & 0xFF) as u8;
    buf[4] = ((body.frequency & 0x07) as u8) << 5 | (body.rfu & 0x1F);
}

fn body_serialized_len(body: &FunctionBody<'_>) -> usize {
    match body {
        FunctionBody::AcePapr(_) => ACE_PAPR_BODY_LEN,
        FunctionBody::MisoGroup(_) => MISO_GROUP_BODY_LEN,
        FunctionBody::TrPapr(_) => TR_PAPR_BODY_LEN,
        FunctionBody::L1AcePapr(_) => L1_ACE_PAPR_BODY_LEN,
        FunctionBody::TxSigFefSeqNum(_) => TX_SIG_FEF_SEQ_NUM_BODY_LEN,
        FunctionBody::TxSigAuxStreamTxId(_) => TX_SIG_AUX_STREAM_TX_ID_BODY_LEN,
        FunctionBody::Frequency(_) => FREQUENCY_BODY_LEN,
        FunctionBody::Raw(bytes) => bytes.len(),
    }
}

fn serialize_body_into(body: &FunctionBody<'_>, buf: &mut [u8]) -> usize {
    match body {
        FunctionBody::AcePapr(b) => {
            serialize_ace_papr(b, buf);
            ACE_PAPR_BODY_LEN
        }
        FunctionBody::MisoGroup(b) => {
            serialize_miso_group(b, buf);
            MISO_GROUP_BODY_LEN
        }
        FunctionBody::TrPapr(b) => {
            serialize_tr_papr(b, buf);
            TR_PAPR_BODY_LEN
        }
        FunctionBody::L1AcePapr(b) => {
            serialize_l1_ace_papr(b, buf);
            L1_ACE_PAPR_BODY_LEN
        }
        FunctionBody::TxSigFefSeqNum(b) => {
            serialize_tx_sig_fef_seq_num(b, buf);
            TX_SIG_FEF_SEQ_NUM_BODY_LEN
        }
        FunctionBody::TxSigAuxStreamTxId(b) => {
            serialize_tx_sig_aux_stream_tx_id(b, buf);
            TX_SIG_AUX_STREAM_TX_ID_BODY_LEN
        }
        FunctionBody::Frequency(b) => {
            serialize_frequency(b, buf);
            FREQUENCY_BODY_LEN
        }
        FunctionBody::Raw(bytes) => {
            buf[..bytes.len()].copy_from_slice(bytes);
            bytes.len()
        }
    }
}

fn function_entry_serialized_len(entry: &FunctionEntry<'_>) -> usize {
    FUNC_HEADER_LEN + body_serialized_len(&entry.body)
}

fn transmitter_entry_serialized_len(entry: &TransmitterEntry<'_>) -> usize {
    let func_loop_len: usize = entry
        .functions
        .iter()
        .map(function_entry_serialized_len)
        .sum();
    TX_HEADER_LEN + func_loop_len
}

// ── Parse ──────────────────────────────────────────────────────────────────

impl<'a> Parse<'a> for IndividualAddressingPayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < HEADER_LEN {
            return Err(crate::Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "IndividualAddressingPayload header",
            });
        }

        let rfu = bytes[0];
        let individual_addressing_length = bytes[1] as usize;
        let need = HEADER_LEN + individual_addressing_length;
        if bytes.len() < need {
            return Err(crate::Error::BufferTooShort {
                need,
                have: bytes.len(),
                what: "IndividualAddressingPayload data",
            });
        }

        let data = &bytes[HEADER_LEN..need];
        let mut pos: usize = 0;
        let max_tx = individual_addressing_length / TX_HEADER_LEN + 1;
        let mut transmitters = Vec::with_capacity(max_tx.min(individual_addressing_length));

        while pos < individual_addressing_length {
            if pos + TX_HEADER_LEN > individual_addressing_length {
                return Err(crate::Error::BufferTooShort {
                    need: pos + TX_HEADER_LEN,
                    have: individual_addressing_length,
                    what: "transmitter entry header",
                });
            }

            let transmitter_id = u16::from_be_bytes([data[pos], data[pos + 1]]);
            let function_loop_length = data[pos + 2] as usize;
            pos += TX_HEADER_LEN;

            let func_end = pos + function_loop_length;
            if func_end > individual_addressing_length {
                return Err(crate::Error::BufferTooShort {
                    need: func_end,
                    have: individual_addressing_length,
                    what: "function loop",
                });
            }

            let max_funcs = function_loop_length / FUNC_HEADER_LEN + 1;
            let mut functions = Vec::with_capacity(max_funcs.min(function_loop_length));

            while pos < func_end {
                if pos + FUNC_HEADER_LEN > func_end {
                    return Err(crate::Error::BufferTooShort {
                        need: pos + FUNC_HEADER_LEN,
                        have: func_end,
                        what: "function entry header",
                    });
                }

                let tag = data[pos];
                let function_length = data[pos + 1] as usize;
                pos += FUNC_HEADER_LEN;

                let body_end = pos + function_length;
                if body_end > func_end {
                    return Err(crate::Error::BufferTooShort {
                        need: body_end,
                        have: func_end,
                        what: "function body",
                    });
                }

                let body_bytes = &data[pos..body_end];
                pos = body_end;

                let body = match try_parse_typed_body(tag, body_bytes) {
                    Some(Ok(typed)) => typed,
                    Some(Err(e)) => return Err(e),
                    None => FunctionBody::Raw(body_bytes),
                };

                functions.push(FunctionEntry { tag, body });
            }

            transmitters.push(TransmitterEntry {
                transmitter_id,
                functions,
            });
        }

        Ok(IndividualAddressingPayload { rfu, transmitters })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for IndividualAddressingPayload<'a> {
    const PACKET_TYPE: u8 = 0x21;
    const NAME: &'static str = "INDIVIDUAL_ADDRESSING";
}

// ── Serialize ──────────────────────────────────────────────────────────────

impl Serialize for IndividualAddressingPayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self
                .transmitters
                .iter()
                .map(transmitter_entry_serialized_len)
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        let data_len: usize = self
            .transmitters
            .iter()
            .map(transmitter_entry_serialized_len)
            .sum();
        let total = HEADER_LEN + data_len;

        if buf.len() < total {
            return Err(crate::Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }

        if data_len > u8::MAX as usize {
            return Err(crate::Error::ReservedBitsViolation {
                field: "individual_addressing_length",
                reason: "transmitter loop exceeds 255 bytes (8-bit length field)",
            });
        }

        buf[0] = self.rfu;
        buf[1] = data_len as u8;

        let mut pos: usize = HEADER_LEN;

        for tx in &self.transmitters {
            let func_loop_len: usize = tx.functions.iter().map(function_entry_serialized_len).sum();
            if func_loop_len > u8::MAX as usize {
                return Err(crate::Error::ReservedBitsViolation {
                    field: "function_loop_length",
                    reason: "function loop exceeds 255 bytes (8-bit length field)",
                });
            }

            buf[pos] = (tx.transmitter_id >> 8) as u8;
            buf[pos + 1] = (tx.transmitter_id & 0xFF) as u8;
            buf[pos + 2] = func_loop_len as u8;
            pos += TX_HEADER_LEN;

            for func in &tx.functions {
                let body_len = body_serialized_len(&func.body);
                if body_len > u8::MAX as usize {
                    return Err(crate::Error::ReservedBitsViolation {
                        field: "function_length",
                        reason: "function body exceeds 255 bytes (8-bit length field)",
                    });
                }

                buf[pos] = func.tag;
                buf[pos + 1] = body_len as u8;
                pos += FUNC_HEADER_LEN;

                let written = serialize_body_into(&func.body, &mut buf[pos..]);
                debug_assert_eq!(written, body_len);
                pos += body_len;
            }
        }

        debug_assert_eq!(pos, total);
        Ok(total)
    }
}

// ── Display ────────────────────────────────────────────────────────────────

impl fmt::Display for IndividualAddressingPayload<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IndividualAddressing {{ rfu: 0x{:02X}, tx_count: {} }}",
            self.rfu,
            self.transmitters.len()
        )
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addressing_function_tag_try_from_valid() {
        assert_eq!(
            AddressingFunctionTag::try_from(0x10),
            Ok(AddressingFunctionTag::AcePapr)
        );
        assert_eq!(
            AddressingFunctionTag::try_from(0x17),
            Ok(AddressingFunctionTag::Frequency)
        );
    }

    #[test]
    fn addressing_function_tag_try_from_rejects_unknown() {
        assert!(AddressingFunctionTag::try_from(0x14).is_err());
        assert!(AddressingFunctionTag::try_from(0xFF).is_err());
    }

    #[test]
    fn exhaustive_byte_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = AddressingFunctionTag::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 14, "expected 14 matched variants");
    }

    #[test]
    fn address_function_tag_display() {
        assert_eq!(AddressingFunctionTag::AcePapr.to_string(), "AcePapr");
    }

    #[test]
    fn parse_empty_data_loop() {
        let buf = [0x00u8, 0x00];
        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(result.rfu, 0x00);
        assert!(result.transmitters.is_empty());
    }

    #[test]
    fn parse_preserves_rfu_byte() {
        let buf = [0xFFu8, 0x00];
        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(result.rfu, 0xFF);
        assert!(result.transmitters.is_empty());
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(IndividualAddressingPayload::parse(&[0x00]).is_err());
    }

    #[test]
    fn parse_rejects_truncated_data() {
        assert!(IndividualAddressingPayload::parse(&[0x00, 0x04, 0xAA, 0xBB]).is_err());
    }

    #[test]
    fn parse_single_transmitter_single_function_ace_papr() {
        let func_body = [0xA8u8, 0x54];
        let func_loop_len = (FUNC_HEADER_LEN + func_body.len()) as u8;
        let tx_loop = [
            0x00,
            0x05,
            func_loop_len,
            0x10,
            ACE_PAPR_BODY_LEN as u8,
            func_body[0],
            func_body[1],
        ];
        let mut buf = vec![0x00u8, tx_loop.len() as u8];
        buf.extend_from_slice(&tx_loop);

        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(result.transmitters.len(), 1);
        assert_eq!(result.transmitters[0].transmitter_id, 0x0005);
        assert_eq!(result.transmitters[0].functions.len(), 1);

        let func = &result.transmitters[0].functions[0];
        assert_eq!(func.tag, 0x10);
        assert_eq!(func.addressing_tag(), Some(AddressingFunctionTag::AcePapr));

        match &func.body {
            FunctionBody::AcePapr(body) => {
                assert_eq!(body.ace_gain, 0x15);
                assert_eq!(body.ace_maximal_extension, 0x00);
                assert_eq!(body.ace_clipping_threshold, 0x2A);
                assert!(!body.rfu);
            }
            other => panic!("expected AcePapr, got {other:?}"),
        }
    }

    #[test]
    fn parse_single_transmitter_single_function_miso_group() {
        let func_body = [0x85u8];
        let func_loop_len = (FUNC_HEADER_LEN + func_body.len()) as u8;
        let tx_loop = [
            0x00,
            0x0A,
            func_loop_len,
            0x11,
            MISO_GROUP_BODY_LEN as u8,
            func_body[0],
        ];
        let mut buf = vec![0x00u8, tx_loop.len() as u8];
        buf.extend_from_slice(&tx_loop);

        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(result.transmitters.len(), 1);
        assert_eq!(result.transmitters[0].transmitter_id, 0x000A);

        let func = &result.transmitters[0].functions[0];
        assert_eq!(func.tag, 0x11);
        match &func.body {
            FunctionBody::MisoGroup(body) => {
                assert!(body.miso_group);
                assert_eq!(body.rfu, 0x05);
            }
            other => panic!("expected MisoGroup, got {other:?}"),
        }
    }

    #[test]
    fn parse_single_transmitter_single_function_frequency() {
        let freq_body = FrequencyBody {
            rf_idx: 3,
            frequency: 0x80000000,
            rfu: 0,
        };
        let mut func_bytes = [0u8; FREQUENCY_BODY_LEN];
        serialize_frequency(&freq_body, &mut func_bytes);

        let func_loop_len = (FUNC_HEADER_LEN + func_bytes.len()) as u8;
        let mut tx_loop = vec![0x00, 0x07, func_loop_len, 0x17, FREQUENCY_BODY_LEN as u8];
        tx_loop.extend_from_slice(&func_bytes);

        let mut buf = vec![0x00u8, tx_loop.len() as u8];
        buf.extend_from_slice(&tx_loop);

        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        let func = &result.transmitters[0].functions[0];
        assert_eq!(func.tag, 0x17);
        match &func.body {
            FunctionBody::Frequency(body) => {
                assert_eq!(body.rf_idx, 3);
                assert_eq!(body.frequency, 0x80000000);
                assert_eq!(body.rfu, 0);
            }
            other => panic!("expected Frequency, got {other:?}"),
        }
    }

    #[test]
    fn parse_unknown_tag_produces_raw_body() {
        let func_body = [0xDE, 0xAD, 0xBE];
        let func_loop_len = (FUNC_HEADER_LEN + func_body.len()) as u8;
        let tx_loop = [
            0x00,
            0x01,
            func_loop_len,
            0x14,
            func_body.len() as u8,
            func_body[0],
            func_body[1],
            func_body[2],
        ];
        let mut buf = vec![0x00u8, tx_loop.len() as u8];
        buf.extend_from_slice(&tx_loop);

        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        let func = &result.transmitters[0].functions[0];
        assert_eq!(func.tag, 0x14);
        assert_eq!(func.addressing_tag(), None);
        match &func.body {
            FunctionBody::Raw(bytes) => assert_eq!(*bytes, &[0xDE, 0xAD, 0xBE]),
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    #[test]
    fn parse_known_tag_wrong_length_falls_back_to_raw() {
        let func_body = [0xAA, 0xBB, 0xCC];
        let func_loop_len = (FUNC_HEADER_LEN + func_body.len()) as u8;
        let tx_loop = [
            0x00,
            0x01,
            func_loop_len,
            0x10,
            func_body.len() as u8,
            func_body[0],
            func_body[1],
            func_body[2],
        ];
        let mut buf = vec![0x00u8, tx_loop.len() as u8];
        buf.extend_from_slice(&tx_loop);

        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        let func = &result.transmitters[0].functions[0];
        assert_eq!(func.tag, 0x10);
        assert_eq!(func.addressing_tag(), Some(AddressingFunctionTag::AcePapr));
        match &func.body {
            FunctionBody::Raw(bytes) => assert_eq!(*bytes, &[0xAA, 0xBB, 0xCC]),
            other => panic!("expected Raw fallback for length mismatch, got {other:?}"),
        }
    }

    #[test]
    fn parse_truncated_transmitter_header() {
        let buf = [0x00u8, 0x02, 0x00];
        assert!(IndividualAddressingPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_truncated_function_header() {
        let tx_loop = [0x00, 0x01, 0x02, 0x10];
        let buf = [0x00u8, tx_loop.len() as u8, 0x00, 0x01, 0x02, 0x10];
        assert!(IndividualAddressingPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_function_body_exceeds_function_loop() {
        let tx_loop = [0x00, 0x01, 0x04, 0x10, 0xFF, 0xAA, 0xBB];
        let mut buf = vec![0x00u8, tx_loop.len() as u8];
        buf.extend_from_slice(&tx_loop);
        assert!(IndividualAddressingPayload::parse(&buf).is_err());
    }

    #[test]
    fn round_trip_two_transmitters_mixed_bodies() {
        let ace_body = AcePaprBody {
            ace_gain: 0x0A,
            ace_maximal_extension: 0x03,
            ace_clipping_threshold: 0x5A,
            rfu: true,
        };
        let raw_body: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];

        let orig = IndividualAddressingPayload {
            rfu: 0xAB,
            transmitters: vec![
                TransmitterEntry {
                    transmitter_id: 0x0005,
                    functions: vec![
                        FunctionEntry {
                            tag: 0x10,
                            body: FunctionBody::AcePapr(ace_body.clone()),
                        },
                        FunctionEntry {
                            tag: 0x14,
                            body: FunctionBody::Raw(raw_body),
                        },
                    ],
                },
                TransmitterEntry {
                    transmitter_id: 0x00FF,
                    functions: vec![FunctionEntry {
                        tag: 0x11,
                        body: FunctionBody::MisoGroup(MisoGroupBody {
                            miso_group: true,
                            rfu: 0x42,
                        }),
                    }],
                },
            ],
        };

        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();

        assert_eq!(buf[0], 0xAB);

        let parsed = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn round_trip_all_typed_bodies() {
        let orig = IndividualAddressingPayload {
            rfu: 0x00,
            transmitters: vec![
                TransmitterEntry {
                    transmitter_id: 0x0001,
                    functions: vec![
                        FunctionEntry {
                            tag: 0x10,
                            body: FunctionBody::AcePapr(AcePaprBody {
                                ace_gain: 0x1F,
                                ace_maximal_extension: 0x07,
                                ace_clipping_threshold: 0x7F,
                                rfu: true,
                            }),
                        },
                        FunctionEntry {
                            tag: 0x11,
                            body: FunctionBody::MisoGroup(MisoGroupBody {
                                miso_group: false,
                                rfu: 0x7F,
                            }),
                        },
                        FunctionEntry {
                            tag: 0x12,
                            body: FunctionBody::TrPapr(TrPaprBody {
                                rfu1: 0x0A,
                                tr_clipping_threshold: 0xABC,
                                rfu2: 0x1FFF,
                                number_of_iterations: 0x1FF,
                            }),
                        },
                    ],
                },
                TransmitterEntry {
                    transmitter_id: 0x0002,
                    functions: vec![
                        FunctionEntry {
                            tag: 0x13,
                            body: FunctionBody::L1AcePapr(L1AcePaprBody {
                                l1_ace_max_correction: 0x1234,
                                rfu: 0x5678,
                            }),
                        },
                        FunctionEntry {
                            tag: 0x15,
                            body: FunctionBody::TxSigFefSeqNum(TxSigFefSeqNumBody {
                                rfu1: 0x1F,
                                seq_num_1: 0x07,
                                rfu2: 0x00,
                                seq_num_2: 0x05,
                                rfu3: 0xABCDEF,
                            }),
                        },
                        FunctionEntry {
                            tag: 0x16,
                            body: FunctionBody::TxSigAuxStreamTxId(TxSigAuxStreamTxIdBody {
                                tx_sig_aux_tx_id: 0xFFF,
                                rfu: 0x0000F,
                            }),
                        },
                        FunctionEntry {
                            tag: 0x17,
                            body: FunctionBody::Frequency(FrequencyBody {
                                rf_idx: 0x05,
                                frequency: 0x87654321,
                                rfu: 0x1F,
                            }),
                        },
                    ],
                },
            ],
        };

        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn serialize_empty_data() {
        let orig = IndividualAddressingPayload {
            rfu: 0x00,
            transmitters: vec![],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [0x00, 0x00]);
    }

    #[test]
    fn serialize_detects_data_loop_overflow() {
        let mut functions = Vec::new();
        for _ in 0..100 {
            functions.push(FunctionEntry {
                tag: AddressingFunctionTag::MisoGroup as u8,
                body: FunctionBody::MisoGroup(MisoGroupBody {
                    miso_group: false,
                    rfu: 0,
                }),
            });
        }
        let payload = IndividualAddressingPayload {
            rfu: 0,
            transmitters: vec![TransmitterEntry {
                transmitter_id: 0,
                functions,
            }],
        };
        let mut buf = vec![0u8; payload.serialized_len()];
        let result = payload.serialize_into(&mut buf);
        assert!(
            matches!(
                result.unwrap_err(),
                crate::Error::ReservedBitsViolation { .. }
            ),
            "expected ReservedBitsViolation for overflowing length field"
        );
    }

    #[test]
    fn body_parse_round_trip_ace_papr() {
        let body = AcePaprBody {
            ace_gain: 0x0A,
            ace_maximal_extension: 0x05,
            ace_clipping_threshold: 0x41,
            rfu: true,
        };
        let mut buf = [0u8; ACE_PAPR_BODY_LEN];
        serialize_ace_papr(&body, &mut buf);
        let parsed = parse_ace_papr(&buf).unwrap();
        assert_eq!(body, parsed);
    }

    #[test]
    fn body_parse_round_trip_tr_papr() {
        let body = TrPaprBody {
            rfu1: 0x0C,
            tr_clipping_threshold: 0xFFF,
            rfu2: 0x0ABC,
            number_of_iterations: 0x3FF,
        };
        let mut buf = [0u8; TR_PAPR_BODY_LEN];
        serialize_tr_papr(&body, &mut buf);
        let parsed = parse_tr_papr(&buf).unwrap();
        assert_eq!(body, parsed);
    }

    #[test]
    fn body_parse_round_trip_frequency() {
        let body = FrequencyBody {
            rf_idx: 0x07,
            frequency: 0xFFFFFFFF,
            rfu: 0x1F,
        };
        let mut buf = [0u8; FREQUENCY_BODY_LEN];
        serialize_frequency(&body, &mut buf);
        let parsed = parse_frequency(&buf).unwrap();
        assert_eq!(body, parsed);
    }

    #[test]
    fn body_parse_round_trip_tx_sig_aux_stream_tx_id() {
        let body = TxSigAuxStreamTxIdBody {
            tx_sig_aux_tx_id: 0xFFF,
            rfu: 0xFFFFF,
        };
        let mut buf = [0u8; TX_SIG_AUX_STREAM_TX_ID_BODY_LEN];
        serialize_tx_sig_aux_stream_tx_id(&body, &mut buf);
        let parsed = parse_tx_sig_aux_stream_tx_id(&buf).unwrap();
        assert_eq!(body, parsed);
    }

    #[test]
    fn function_entry_addressing_tag_method() {
        let entry_known = FunctionEntry {
            tag: 0x10,
            body: FunctionBody::AcePapr(AcePaprBody {
                ace_gain: 0,
                ace_maximal_extension: 0,
                ace_clipping_threshold: 0,
                rfu: false,
            }),
        };
        assert_eq!(
            entry_known.addressing_tag(),
            Some(AddressingFunctionTag::AcePapr)
        );

        let entry_unknown = FunctionEntry {
            tag: 0xFF,
            body: FunctionBody::Raw(&[]),
        };
        assert_eq!(entry_unknown.addressing_tag(), None);
    }
}
