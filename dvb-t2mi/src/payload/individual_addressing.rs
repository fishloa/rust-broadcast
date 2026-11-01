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
//! function() = function_tag(8) · function_length(8) · function_body(function_length - 2 bytes)
//! ```
//!
//! `individual_addressing_length` counts the bytes of the transmitter loop
//! (everything after the 2-byte header).  `function_loop_length` counts the
//! bytes of the function loop within one transmitter entry (tag + length +
//! body of every function).  `function_length` counts the **whole**
//! `function()` including its 2-byte tag+length header (TS 102 773 §5.2.8.2.0:
//! "including the function_tag, function_length and function_body() fields"),
//! so the body is `function_length - 2` bytes; the real Colombia capture
//! (`00 04 ff 9c` = a 2-byte time offset) confirms this.
//!
//! Tags 0x00–0x06 are the TS 101 191 §6.1 functions TS 102 773 §5.2.8.1
//! defers to; tags 0x10–0x17 are the T2-specific functions of §5.2.8.2.
//!
//! # RFU policy (individual-addressing exception)
//!
//! This payload is the crate's documented exception: non-zero RFU bits are
//! **preserved verbatim** rather than rejected, so gateway streams round-trip
//! byte-exact.  This applies to the top-level `rfu` byte and to every RFU
//! field inside the typed function bodies.

use alloc::vec::Vec;
use core::fmt;

use num_enum::TryFromPrimitive;

use broadcast_common::{Parse, Serialize};

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

impl AddressingFunctionTag {
    /// Human-readable spec label (ETSI TS 102 773 §5.2.8.2 Tables 5 & 6).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::TimeOffset => "Transmitter time offset",
            Self::FrequencyOffset => "Transmitter frequency offset",
            Self::Power => "Transmitter power",
            Self::PrivateData => "Private data",
            Self::CellId => "Cell ID",
            Self::Enable => "Enable",
            Self::Bandwidth => "Bandwidth",
            Self::AcePapr => "ACE-PAPR reduction",
            Self::MisoGroup => "MISO group",
            Self::TrPapr => "TR-PAPR reduction",
            Self::L1AcePapr => "L1-ACE-PAPR",
            Self::TxSigFefSeqNum => "TX-SIG FEF sequence number",
            Self::TxSigAuxStreamTxId => "TX-SIG auxiliary stream TX ID",
            Self::Frequency => "Frequency",
        }
    }
}
broadcast_common::impl_spec_display!(AddressingFunctionTag);

// ── Typed function bodies (§5.2.8.2 Tables 7–12b) ──────────────────────────

/// ACE-PAPR function body per §5.2.8.2.1, Table 7.
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

// ── Existing TS 101 191 §6.1 function bodies (Tables 7–13) ─────────────────
//
// Referenced by TS 102 773 §5.2.8.1 / Table 5.  Transcribed and verified on
// the rendered pages in `docs/tables/ts_101_191/`.

/// Transmitter time offset function body — ETSI TS 101 191 §6.1.1, Table 7
/// (function_tag 0x00).
///
/// Layout (2 bytes): `time_offset` (16, `tcimsbf`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TimeOffsetBody {
    /// Deliberate time offset of the mega-frames, unit 100 ns
    /// (range -32 768..=32 767).
    pub time_offset: i16,
}

impl TimeOffsetBody {
    /// Build a body from its `time_offset` (units of 100 ns).
    #[must_use]
    pub fn new(time_offset: i16) -> Self {
        Self { time_offset }
    }
}

/// Transmitter frequency offset function body — ETSI TS 101 191 §6.1.2,
/// Table 8 (function_tag 0x01).
///
/// Layout (3 bytes): `frequency_offset` (24, `tcimsbf`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FrequencyOffsetBody {
    /// Deliberate frequency offset in Hz, a 24-bit two's-complement value
    /// (range -8 388 608..=8 388 607).
    pub frequency_offset: i32,
}

impl FrequencyOffsetBody {
    /// Build a body from its `frequency_offset` (Hz, 24-bit signed range).
    #[must_use]
    pub fn new(frequency_offset: i32) -> Self {
        Self { frequency_offset }
    }
}

/// Transmitter power function body — ETSI TS 101 191 §6.1.3, Table 9
/// (function_tag 0x02).
///
/// Layout (2 bytes): `tx_power` (16, `uimsbf`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PowerBody {
    /// Transmitter ERP, unit 0,1 dB.
    pub tx_power: u16,
}

impl PowerBody {
    /// Build a body from its `tx_power` (units of 0,1 dB).
    #[must_use]
    pub fn new(tx_power: u16) -> Self {
        Self { tx_power }
    }
}

/// Private data function body — ETSI TS 101 191 §6.1.4, Table 10
/// (function_tag 0x03).
///
/// Layout (N bytes): `private_data` (8 × N, `bsblf`).  The spec defines no
/// structure inside `private_data` ("can be used for proprietary functions"),
/// so it is the one field here that is opaque by specification.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PrivateDataBody<'a> {
    /// Proprietary bytes, borrowed from the input.
    pub private_data: &'a [u8],
}

impl<'a> PrivateDataBody<'a> {
    /// Build a body from its proprietary bytes.
    #[must_use]
    pub fn new(private_data: &'a [u8]) -> Self {
        Self { private_data }
    }
}

/// Cell id function body — ETSI TS 101 191 §6.1.5, Table 11
/// (function_tag 0x04).
///
/// Layout (3 bytes):
/// - bytes 0-1: `cell_id` (16)
/// - byte 2 `[7]`: `wait_for_enable_flag` (1)
/// - byte 2 `[6:0]`: `reserved_future_use` (7) — preserved per the
///   individual-addressing RFU exception
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CellIdBody {
    /// Cell identifier (EN 300 468 `cell_id`).
    pub cell_id: u16,
    /// `false`: insert the cell_id immediately; `true`: insert it after the
    /// corresponding enable function has been received.
    pub wait_for_enable_flag: bool,
    /// `reserved_future_use` `[6:0]` (7 bits) — preserved verbatim.
    pub rfu: u8,
}

impl CellIdBody {
    /// Build a body with zeroed RFU bits.
    #[must_use]
    pub fn new(cell_id: u16, wait_for_enable_flag: bool) -> Self {
        Self {
            cell_id,
            wait_for_enable_flag,
            rfu: 0,
        }
    }
}

/// Enable function body — ETSI TS 101 191 §6.1.6, Table 12
/// (function_tag 0x05).
///
/// Layout (N bytes): `enabled_function_tag` (8) × N.  Each value is a
/// `function_tag` per Table 6 (see [`AddressingFunctionTag`]); values outside
/// that table are kept as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct EnableBody {
    /// The `function_tag` values being enabled, in wire order.
    pub enabled_function_tags: Vec<u8>,
}

impl EnableBody {
    /// Build a body from the list of enabled `function_tag` values.
    #[must_use]
    pub fn new(enabled_function_tags: Vec<u8>) -> Self {
        Self {
            enabled_function_tags,
        }
    }
}

/// Bandwidth function body — ETSI TS 101 191 §6.1.7, Table 13
/// (function_tag 0x06; "not applicable" to DVB-T2 per TS 102 773 Table 5).
///
/// Layout (1 byte):
/// - `[7:1]`: `ch_bandwidth` (7) — Table 14: 0 = 5 MHz, 1..=127 reserved
/// - `[0]`: `wait_for_enable_flag` (1)
///
/// The spec prose calls `ch_bandwidth` an "8-bit field" while Table 13 gives
/// it 7 bits; the table width is implemented (see
/// `docs/tables/ts_101_191/`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct BandwidthBody {
    /// `ch_bandwidth` `[7:1]` (7 bits); see [`CH_BANDWIDTH_5_MHZ`].
    pub ch_bandwidth: u8,
    /// `false`: implement the bandwidth immediately; `true`: after the
    /// corresponding enable function.
    pub wait_for_enable_flag: bool,
}

/// `ch_bandwidth` value 0 = 5 MHz (TS 101 191 Table 14).
pub const CH_BANDWIDTH_5_MHZ: u8 = 0;

impl BandwidthBody {
    /// Build a body from its two fields.
    #[must_use]
    pub fn new(ch_bandwidth: u8, wait_for_enable_flag: bool) -> Self {
        Self {
            ch_bandwidth,
            wait_for_enable_flag,
        }
    }
}

// ── Body size constants ────────────────────────────────────────────────────

const TIME_OFFSET_BODY_LEN: usize = 2;
const FREQUENCY_OFFSET_BODY_LEN: usize = 3;
const POWER_BODY_LEN: usize = 2;
const CELL_ID_BODY_LEN: usize = 3;
const BANDWIDTH_BODY_LEN: usize = 1;
const FREQUENCY_OFFSET_MIN: i32 = -(1 << 23);
const FREQUENCY_OFFSET_MAX: i32 = (1 << 23) - 1;
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
/// Typed variants cover every function whose body is defined by the spec: tags
/// 0x00–0x06 per ETSI TS 101 191 §6.1 Tables 7–13 (which TS 102 773 §5.2.8.1
/// defers to) and tags 0x10–0x17 per TS 102 773 §5.2.8.2 Tables 7–12b.
/// [`FunctionBody::Raw`] is the escape for:
/// - tags not in [`AddressingFunctionTag`] (reserved / future use), or
/// - a known fixed-size tag whose `function_length` doesn't match the
///   expected body size (future extension).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FunctionBody<'a> {
    /// Transmitter time offset (tag 0x00) — TS 101 191 §6.1.1, Table 7.
    TimeOffset(TimeOffsetBody),
    /// Transmitter frequency offset (tag 0x01) — TS 101 191 §6.1.2, Table 8.
    FrequencyOffset(FrequencyOffsetBody),
    /// Transmitter power (tag 0x02) — TS 101 191 §6.1.3, Table 9.
    Power(PowerBody),
    /// Private data (tag 0x03) — TS 101 191 §6.1.4, Table 10.
    PrivateData(PrivateDataBody<'a>),
    /// Cell id (tag 0x04) — TS 101 191 §6.1.5, Table 11.
    CellId(CellIdBody),
    /// Enable (tag 0x05) — TS 101 191 §6.1.6, Table 12.
    Enable(EnableBody),
    /// Bandwidth (tag 0x06) — TS 101 191 §6.1.7, Table 13.
    Bandwidth(BandwidthBody),
    /// ACE-PAPR (tag 0x10) — §5.2.8.2.1, Table 7.
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
    /// `function_length - 2` bytes).
    Raw(&'a [u8]),
}

impl FunctionBody<'_> {
    /// Human-readable spec label (ETSI TS 102 773 §5.2.8.2 Tables 7–12b).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::TimeOffset(_) => "Transmitter time offset",
            Self::FrequencyOffset(_) => "Transmitter frequency offset",
            Self::Power(_) => "Transmitter power",
            Self::PrivateData(_) => "Private data",
            Self::CellId(_) => "Cell ID",
            Self::Enable(_) => "Enable",
            Self::Bandwidth(_) => "Bandwidth",
            Self::AcePapr(_) => "ACE-PAPR",
            Self::MisoGroup(_) => "MISO group",
            Self::TrPapr(_) => "TR-PAPR",
            Self::L1AcePapr(_) => "L1-ACE-PAPR",
            Self::TxSigFefSeqNum(_) => "TX-SIG FEF sequence numbers",
            Self::TxSigAuxStreamTxId(_) => "TX-SIG auxiliary stream TX ID",
            Self::Frequency(_) => "Frequency",
            Self::Raw(_) => "raw",
        }
    }
}
broadcast_common::impl_spec_display!(FunctionBody<'_>);

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
/// Wire layout: `function_tag(8) · function_length(8) · function_body(function_length - 2 bytes)`
/// — `function_length` counts the **whole** `function()` including the 2-byte
/// tag + length header, so the body is `function_length - 2` bytes.
///
/// `tag` and `body` must agree: the typed [`FunctionBody`] variants each belong
/// to exactly one tag, and [`FunctionBody::Raw`] is only for a tag/length that
/// does *not* parse as a typed body. [`Serialize`] rejects a mismatch with
/// [`crate::Error::FunctionTagBodyMismatch`] (parse always produces agreeing
/// pairs, so serialize -> parse is the identity).
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

impl FunctionEntry<'_> {
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

fn parse_time_offset(body: &[u8]) -> Result<TimeOffsetBody, crate::Error> {
    let (b, _) =
        body.split_first_chunk::<TIME_OFFSET_BODY_LEN>()
            .ok_or(crate::Error::BufferTooShort {
                need: TIME_OFFSET_BODY_LEN,
                have: body.len(),
                what: "time offset function body",
            })?;
    Ok(TimeOffsetBody {
        time_offset: i16::from_be_bytes(*b),
    })
}

fn parse_frequency_offset(body: &[u8]) -> Result<FrequencyOffsetBody, crate::Error> {
    let (b, _) = body
        .split_first_chunk::<FREQUENCY_OFFSET_BODY_LEN>()
        .ok_or(crate::Error::BufferTooShort {
            need: FREQUENCY_OFFSET_BODY_LEN,
            have: body.len(),
            what: "frequency offset function body",
        })?;
    // 24-bit two's complement: place in the top of an i32, then arithmetic-shift back.
    Ok(FrequencyOffsetBody {
        frequency_offset: i32::from_be_bytes([b[0], b[1], b[2], 0]) >> 8,
    })
}

fn parse_power(body: &[u8]) -> Result<PowerBody, crate::Error> {
    let (b, _) =
        body.split_first_chunk::<POWER_BODY_LEN>()
            .ok_or(crate::Error::BufferTooShort {
                need: POWER_BODY_LEN,
                have: body.len(),
                what: "power function body",
            })?;
    Ok(PowerBody {
        tx_power: u16::from_be_bytes(*b),
    })
}

fn parse_cell_id(body: &[u8]) -> Result<CellIdBody, crate::Error> {
    let (b, _) =
        body.split_first_chunk::<CELL_ID_BODY_LEN>()
            .ok_or(crate::Error::BufferTooShort {
                need: CELL_ID_BODY_LEN,
                have: body.len(),
                what: "cell id function body",
            })?;
    Ok(CellIdBody {
        cell_id: u16::from_be_bytes([b[0], b[1]]),
        wait_for_enable_flag: b[2] & 0x80 != 0,
        rfu: b[2] & 0x7F,
    })
}

fn parse_bandwidth(body: &[u8]) -> Result<BandwidthBody, crate::Error> {
    let b = *body.first().ok_or(crate::Error::BufferTooShort {
        need: BANDWIDTH_BODY_LEN,
        have: body.len(),
        what: "bandwidth function body",
    })?;
    Ok(BandwidthBody {
        ch_bandwidth: b >> 1,
        wait_for_enable_flag: b & 0x01 != 0,
    })
}

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
    let (b, _) =
        body.split_first_chunk::<L1_ACE_PAPR_BODY_LEN>()
            .ok_or(crate::Error::BufferTooShort {
                need: L1_ACE_PAPR_BODY_LEN,
                have: body.len(),
                what: "L1-ACE-PAPR function body",
            })?;
    Ok(L1AcePaprBody {
        l1_ace_max_correction: u16::from_be_bytes([b[0], b[1]]),
        rfu: u16::from_be_bytes([b[2], b[3]]),
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
        t if t == AddressingFunctionTag::TimeOffset as u8
            && body_bytes.len() == TIME_OFFSET_BODY_LEN =>
        {
            Some(parse_time_offset(body_bytes).map(FunctionBody::TimeOffset))
        }
        t if t == AddressingFunctionTag::FrequencyOffset as u8
            && body_bytes.len() == FREQUENCY_OFFSET_BODY_LEN =>
        {
            Some(parse_frequency_offset(body_bytes).map(FunctionBody::FrequencyOffset))
        }
        t if t == AddressingFunctionTag::Power as u8 && body_bytes.len() == POWER_BODY_LEN => {
            Some(parse_power(body_bytes).map(FunctionBody::Power))
        }
        // private_data and enabled_function_tag are N-byte loops: any length.
        t if t == AddressingFunctionTag::PrivateData as u8 => Some(Ok(FunctionBody::PrivateData(
            PrivateDataBody::new(body_bytes),
        ))),
        t if t == AddressingFunctionTag::CellId as u8 && body_bytes.len() == CELL_ID_BODY_LEN => {
            Some(parse_cell_id(body_bytes).map(FunctionBody::CellId))
        }
        t if t == AddressingFunctionTag::Enable as u8 => Some(Ok(FunctionBody::Enable(
            EnableBody::new(body_bytes.to_vec()),
        ))),
        t if t == AddressingFunctionTag::Bandwidth as u8
            && body_bytes.len() == BANDWIDTH_BODY_LEN =>
        {
            Some(parse_bandwidth(body_bytes).map(FunctionBody::Bandwidth))
        }
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

/// Range-check `value` against a `bits`-wide wire field before it is narrowed
/// and written, so a value that does not fit is rejected instead of silently
/// wrapped (see `dvb-t2mi/docs/README.md`).
fn check_bits(value: u64, bits: u32, field: &'static str) -> Result<(), crate::error::Error> {
    broadcast_common::len::fit_bits(value, bits, field)
        .map(|_| ())
        .map_err(|_| crate::Error::ReservedBitsViolation {
            field,
            reason: range_reason(bits),
        })
}

/// Static "why" text for [`check_bits`], keyed by field width — every width
/// this module's bit-packed bodies actually use.
const fn range_reason(bits: u32) -> &'static str {
    match bits {
        3 => "Must be in range 0..=7 (3-bit field)",
        4 => "Must be in range 0..=15 (4-bit field)",
        5 => "Must be in range 0..=31 (5-bit field)",
        7 => "Must be in range 0..=127 (7-bit field)",
        10 => "Must be in range 0..=1023 (10-bit field)",
        12 => "Must be in range 0..=4095 (12-bit field)",
        14 => "Must be in range 0..=16383 (14-bit field)",
        20 => "Must be in range 0..=1048575 (20-bit field)",
        24 => "Must be in range 0..=16777215 (24-bit field)",
        _ => "value exceeds field width",
    }
}

fn serialize_time_offset(body: &TimeOffsetBody, buf: &mut [u8]) {
    buf[..TIME_OFFSET_BODY_LEN].copy_from_slice(&body.time_offset.to_be_bytes());
}

fn serialize_frequency_offset(
    body: &FrequencyOffsetBody,
    buf: &mut [u8],
) -> Result<(), crate::error::Error> {
    if !(FREQUENCY_OFFSET_MIN..=FREQUENCY_OFFSET_MAX).contains(&body.frequency_offset) {
        return Err(crate::Error::ReservedBitsViolation {
            field: "frequency_offset",
            reason: "Must be in range -8388608..=8388607 (24-bit signed field)",
        });
    }
    let be = body.frequency_offset.to_be_bytes();
    buf[..FREQUENCY_OFFSET_BODY_LEN].copy_from_slice(&be[1..]);
    Ok(())
}

fn serialize_power(body: &PowerBody, buf: &mut [u8]) {
    buf[..POWER_BODY_LEN].copy_from_slice(&body.tx_power.to_be_bytes());
}

fn serialize_cell_id(body: &CellIdBody, buf: &mut [u8]) -> Result<(), crate::error::Error> {
    check_bits(u64::from(body.rfu), 7, "cell_id reserved_future_use")?;
    buf[..2].copy_from_slice(&body.cell_id.to_be_bytes());
    buf[2] = if body.wait_for_enable_flag {
        0x80
    } else {
        0x00
    } | body.rfu;
    Ok(())
}

fn serialize_bandwidth(body: &BandwidthBody, buf: &mut [u8]) -> Result<(), crate::error::Error> {
    check_bits(u64::from(body.ch_bandwidth), 7, "ch_bandwidth")?;
    buf[0] = body.ch_bandwidth << 1 | u8::from(body.wait_for_enable_flag);
    Ok(())
}

fn serialize_ace_papr(body: &AcePaprBody, buf: &mut [u8]) -> Result<(), crate::error::Error> {
    check_bits(u64::from(body.ace_gain), 5, "ace_gain")?;
    check_bits(
        u64::from(body.ace_maximal_extension),
        3,
        "ace_maximal_extension",
    )?;
    check_bits(
        u64::from(body.ace_clipping_threshold),
        7,
        "ace_clipping_threshold",
    )?;
    buf[0] = body.ace_gain << 3 | body.ace_maximal_extension;
    buf[1] = body.ace_clipping_threshold << 1 | if body.rfu { 1 } else { 0 };
    Ok(())
}

fn serialize_miso_group(body: &MisoGroupBody, buf: &mut [u8]) -> Result<(), crate::error::Error> {
    check_bits(u64::from(body.rfu), 7, "miso_group rfu")?;
    buf[0] = if body.miso_group { 0x80 } else { 0x00 } | body.rfu;
    Ok(())
}

fn serialize_tr_papr(body: &TrPaprBody, buf: &mut [u8]) -> Result<(), crate::error::Error> {
    check_bits(u64::from(body.rfu1), 4, "tr_papr rfu1")?;
    check_bits(
        u64::from(body.tr_clipping_threshold),
        12,
        "tr_clipping_threshold",
    )?;
    check_bits(u64::from(body.rfu2), 14, "tr_papr rfu2")?;
    check_bits(
        u64::from(body.number_of_iterations),
        10,
        "number_of_iterations",
    )?;
    buf[0] = (body.rfu1 << 4) | (body.tr_clipping_threshold >> 8) as u8;
    buf[1] = (body.tr_clipping_threshold & 0xFF) as u8;
    buf[2] = (body.rfu2 >> 6) as u8;
    buf[3] = ((body.rfu2 & 0x3F) as u8) << 2 | (body.number_of_iterations >> 8) as u8;
    buf[4] = (body.number_of_iterations & 0xFF) as u8;
    Ok(())
}

fn serialize_l1_ace_papr(body: &L1AcePaprBody, buf: &mut [u8]) {
    buf[0..2].copy_from_slice(&body.l1_ace_max_correction.to_be_bytes());
    buf[2..4].copy_from_slice(&body.rfu.to_be_bytes());
}

fn serialize_tx_sig_fef_seq_num(
    body: &TxSigFefSeqNumBody,
    buf: &mut [u8],
) -> Result<(), crate::error::Error> {
    check_bits(u64::from(body.rfu1), 5, "tx_sig_fef_seq_num rfu1")?;
    check_bits(u64::from(body.seq_num_1), 3, "seq_num_1")?;
    check_bits(u64::from(body.rfu2), 5, "tx_sig_fef_seq_num rfu2")?;
    check_bits(u64::from(body.seq_num_2), 3, "seq_num_2")?;
    check_bits(u64::from(body.rfu3), 24, "tx_sig_fef_seq_num rfu3")?;
    buf[0] = (body.rfu1 << 3) | body.seq_num_1;
    buf[1] = (body.rfu2 << 3) | body.seq_num_2;
    buf[2] = ((body.rfu3 >> 16) & 0xFF) as u8;
    buf[3] = ((body.rfu3 >> 8) & 0xFF) as u8;
    buf[4] = (body.rfu3 & 0xFF) as u8;
    Ok(())
}

fn serialize_tx_sig_aux_stream_tx_id(
    body: &TxSigAuxStreamTxIdBody,
    buf: &mut [u8],
) -> Result<(), crate::error::Error> {
    check_bits(u64::from(body.tx_sig_aux_tx_id), 12, "tx_sig_aux_tx_id")?;
    check_bits(u64::from(body.rfu), 20, "tx_sig_aux_stream_tx_id rfu")?;
    buf[0] = ((body.tx_sig_aux_tx_id >> 4) & 0xFF) as u8;
    buf[1] = ((body.tx_sig_aux_tx_id & 0x0F) as u8) << 4 | ((body.rfu >> 16) & 0x0F) as u8;
    buf[2] = ((body.rfu >> 8) & 0xFF) as u8;
    buf[3] = (body.rfu & 0xFF) as u8;
    Ok(())
}

fn serialize_frequency(body: &FrequencyBody, buf: &mut [u8]) -> Result<(), crate::error::Error> {
    check_bits(u64::from(body.rf_idx), 3, "rf_idx")?;
    check_bits(u64::from(body.rfu), 5, "frequency rfu")?;
    buf[0] = (body.rf_idx << 5) | ((body.frequency >> 27) & 0x1F) as u8;
    buf[1] = ((body.frequency >> 19) & 0xFF) as u8;
    buf[2] = ((body.frequency >> 11) & 0xFF) as u8;
    buf[3] = ((body.frequency >> 3) & 0xFF) as u8;
    buf[4] = ((body.frequency & 0x07) as u8) << 5 | body.rfu;
    Ok(())
}

fn body_serialized_len(body: &FunctionBody<'_>) -> usize {
    match body {
        FunctionBody::TimeOffset(_) => TIME_OFFSET_BODY_LEN,
        FunctionBody::FrequencyOffset(_) => FREQUENCY_OFFSET_BODY_LEN,
        FunctionBody::Power(_) => POWER_BODY_LEN,
        FunctionBody::PrivateData(b) => b.private_data.len(),
        FunctionBody::CellId(_) => CELL_ID_BODY_LEN,
        FunctionBody::Enable(b) => b.enabled_function_tags.len(),
        FunctionBody::Bandwidth(_) => BANDWIDTH_BODY_LEN,
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

fn serialize_body_into(
    body: &FunctionBody<'_>,
    buf: &mut [u8],
) -> Result<usize, crate::error::Error> {
    match body {
        FunctionBody::TimeOffset(b) => {
            serialize_time_offset(b, buf);
            Ok(TIME_OFFSET_BODY_LEN)
        }
        FunctionBody::FrequencyOffset(b) => {
            serialize_frequency_offset(b, buf)?;
            Ok(FREQUENCY_OFFSET_BODY_LEN)
        }
        FunctionBody::Power(b) => {
            serialize_power(b, buf);
            Ok(POWER_BODY_LEN)
        }
        FunctionBody::PrivateData(b) => {
            let n = b.private_data.len();
            buf[..n].copy_from_slice(b.private_data);
            Ok(n)
        }
        FunctionBody::CellId(b) => {
            serialize_cell_id(b, buf)?;
            Ok(CELL_ID_BODY_LEN)
        }
        FunctionBody::Enable(b) => {
            let n = b.enabled_function_tags.len();
            buf[..n].copy_from_slice(&b.enabled_function_tags);
            Ok(n)
        }
        FunctionBody::Bandwidth(b) => {
            serialize_bandwidth(b, buf)?;
            Ok(BANDWIDTH_BODY_LEN)
        }
        FunctionBody::AcePapr(b) => {
            serialize_ace_papr(b, buf)?;
            Ok(ACE_PAPR_BODY_LEN)
        }
        FunctionBody::MisoGroup(b) => {
            serialize_miso_group(b, buf)?;
            Ok(MISO_GROUP_BODY_LEN)
        }
        FunctionBody::TrPapr(b) => {
            serialize_tr_papr(b, buf)?;
            Ok(TR_PAPR_BODY_LEN)
        }
        FunctionBody::L1AcePapr(b) => {
            serialize_l1_ace_papr(b, buf);
            Ok(L1_ACE_PAPR_BODY_LEN)
        }
        FunctionBody::TxSigFefSeqNum(b) => {
            serialize_tx_sig_fef_seq_num(b, buf)?;
            Ok(TX_SIG_FEF_SEQ_NUM_BODY_LEN)
        }
        FunctionBody::TxSigAuxStreamTxId(b) => {
            serialize_tx_sig_aux_stream_tx_id(b, buf)?;
            Ok(TX_SIG_AUX_STREAM_TX_ID_BODY_LEN)
        }
        FunctionBody::Frequency(b) => {
            serialize_frequency(b, buf)?;
            Ok(FREQUENCY_BODY_LEN)
        }
        FunctionBody::Raw(bytes) => {
            buf[..bytes.len()].copy_from_slice(bytes);
            Ok(bytes.len())
        }
    }
}

/// The `function_tag` a typed body belongs to; `None` for [`FunctionBody::Raw`].
fn typed_body_tag(body: &FunctionBody<'_>) -> Option<u8> {
    let tag = match body {
        FunctionBody::TimeOffset(_) => AddressingFunctionTag::TimeOffset,
        FunctionBody::FrequencyOffset(_) => AddressingFunctionTag::FrequencyOffset,
        FunctionBody::Power(_) => AddressingFunctionTag::Power,
        FunctionBody::PrivateData(_) => AddressingFunctionTag::PrivateData,
        FunctionBody::CellId(_) => AddressingFunctionTag::CellId,
        FunctionBody::Enable(_) => AddressingFunctionTag::Enable,
        FunctionBody::Bandwidth(_) => AddressingFunctionTag::Bandwidth,
        FunctionBody::AcePapr(_) => AddressingFunctionTag::AcePapr,
        FunctionBody::MisoGroup(_) => AddressingFunctionTag::MisoGroup,
        FunctionBody::TrPapr(_) => AddressingFunctionTag::TrPapr,
        FunctionBody::L1AcePapr(_) => AddressingFunctionTag::L1AcePapr,
        FunctionBody::TxSigFefSeqNum(_) => AddressingFunctionTag::TxSigFefSeqNum,
        FunctionBody::TxSigAuxStreamTxId(_) => AddressingFunctionTag::TxSigAuxStreamTxId,
        FunctionBody::Frequency(_) => AddressingFunctionTag::Frequency,
        FunctionBody::Raw(_) => return None,
    };
    Some(tag as u8)
}

/// Reject an entry whose `tag` and `body` could not survive serialize -> parse.
fn check_tag_matches_body(entry: &FunctionEntry<'_>) -> Result<(), crate::Error> {
    match typed_body_tag(&entry.body) {
        Some(expected) if expected != entry.tag => Err(crate::Error::FunctionTagBodyMismatch {
            tag: entry.tag,
            body: entry.body.name(),
            reason: "a typed body is only valid under its own function_tag",
        }),
        Some(_) => Ok(()),
        None => {
            let FunctionBody::Raw(bytes) = &entry.body else {
                return Ok(());
            };
            if try_parse_typed_body(entry.tag, bytes).is_some() {
                Err(crate::Error::FunctionTagBodyMismatch {
                    tag: entry.tag,
                    body: entry.body.name(),
                    reason: "this tag/length parses as a typed body; use the typed variant",
                })
            } else {
                Ok(())
            }
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

            let (tx_id_bytes, _) = data
                .get(pos..)
                .and_then(|s| s.split_first_chunk::<2>())
                .ok_or(crate::Error::BufferTooShort {
                    need: pos + 2,
                    have: individual_addressing_length,
                    what: "transmitter entry header",
                })?;
            let transmitter_id = u16::from_be_bytes(*tx_id_bytes);
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
                // `function_length` counts the whole function() including the
                // 2-byte tag+length header (TS 102 773 §5.2.8.2.0, confirmed by
                // the real capture), so the body is `function_length - 2`.
                let function_length = data[pos + 1] as usize;
                let body_len = function_length.checked_sub(FUNC_HEADER_LEN).ok_or(
                    crate::Error::InvalidFunctionLength {
                        function_length: data[pos + 1],
                    },
                )?;
                pos += FUNC_HEADER_LEN;

                let body_end = pos + body_len;
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

        // Validate every entry before writing a byte: tag and body must agree,
        // or serialize -> parse would not be the identity.
        for func in self.transmitters.iter().flat_map(|tx| tx.functions.iter()) {
            check_tag_matches_body(func)?;
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
                // function_length counts the whole function() incl. its header.
                let function_length =
                    broadcast_common::len::fit_u8(FUNC_HEADER_LEN + body_len, "function_length")
                        .map_err(|_| crate::Error::ReservedBitsViolation {
                            field: "function_length",
                            reason: "function exceeds 255 bytes (8-bit length field)",
                        })?;

                buf[pos] = func.tag;
                buf[pos + 1] = function_length;
                pos += FUNC_HEADER_LEN;

                let written = serialize_body_into(&func.body, &mut buf[pos..])?;
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
        // §204 convention: Display is the spec label (was Debug-style before).
        assert_eq!(
            AddressingFunctionTag::AcePapr.to_string(),
            "ACE-PAPR reduction"
        );
        assert_eq!(AddressingFunctionTag::AcePapr.name(), "ACE-PAPR reduction");
        assert_eq!(AddressingFunctionTag::CellId.to_string(), "Cell ID");
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
            (FUNC_HEADER_LEN + ACE_PAPR_BODY_LEN) as u8,
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
            (FUNC_HEADER_LEN + MISO_GROUP_BODY_LEN) as u8,
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
        serialize_frequency(&freq_body, &mut func_bytes).unwrap();

        let func_loop_len = (FUNC_HEADER_LEN + func_bytes.len()) as u8;
        let mut tx_loop = vec![
            0x00,
            0x07,
            func_loop_len,
            0x17,
            (FUNC_HEADER_LEN + FREQUENCY_BODY_LEN) as u8,
        ];
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
            (FUNC_HEADER_LEN + func_body.len()) as u8,
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
            (FUNC_HEADER_LEN + func_body.len()) as u8,
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
        serialize_ace_papr(&body, &mut buf).unwrap();
        let parsed = parse_ace_papr(&buf).unwrap();
        assert_eq!(body, parsed);
    }

    #[test]
    fn oversized_ace_gain_is_rejected_not_wrapped() {
        // Before the fix, 0x20 (exceeds 5 bits) silently wrapped to 0x00 in
        // the ace_gain field and returned Ok.
        let body = AcePaprBody {
            ace_gain: 0x20,
            ace_maximal_extension: 0,
            ace_clipping_threshold: 0,
            rfu: false,
        };
        let mut buf = [0u8; ACE_PAPR_BODY_LEN];
        assert!(matches!(
            serialize_ace_papr(&body, &mut buf),
            Err(crate::Error::ReservedBitsViolation { .. })
        ));
    }

    #[test]
    fn max_ace_gain_still_serializes_and_round_trips() {
        let body = AcePaprBody {
            ace_gain: 0x1F, // the 5-bit maximum
            ace_maximal_extension: 0,
            ace_clipping_threshold: 0,
            rfu: false,
        };
        let mut buf = [0u8; ACE_PAPR_BODY_LEN];
        serialize_ace_papr(&body, &mut buf).unwrap();
        assert_eq!(parse_ace_papr(&buf).unwrap(), body);
    }

    #[test]
    fn oversized_tr_clipping_threshold_is_rejected_not_wrapped() {
        // Before the fix, 0x1000 (exceeds 12 bits) silently wrapped to 0 in
        // the tr_clipping_threshold field and returned Ok.
        let body = TrPaprBody {
            rfu1: 0,
            tr_clipping_threshold: 0x1000,
            rfu2: 0,
            number_of_iterations: 0,
        };
        let mut buf = [0u8; TR_PAPR_BODY_LEN];
        assert!(matches!(
            serialize_tr_papr(&body, &mut buf),
            Err(crate::Error::ReservedBitsViolation { .. })
        ));
    }

    #[test]
    fn oversized_rf_idx_is_rejected_not_wrapped() {
        // Before the fix, 8 (exceeds 3 bits) silently wrapped to 0 in the
        // rf_idx field and returned Ok.
        let body = FrequencyBody {
            rf_idx: 8,
            frequency: 0,
            rfu: 0,
        };
        let mut buf = [0u8; FREQUENCY_BODY_LEN];
        assert!(matches!(
            serialize_frequency(&body, &mut buf),
            Err(crate::Error::ReservedBitsViolation { .. })
        ));
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
        serialize_tr_papr(&body, &mut buf).unwrap();
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
        serialize_frequency(&body, &mut buf).unwrap();
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
        serialize_tx_sig_aux_stream_tx_id(&body, &mut buf).unwrap();
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

    // ── TS 101 191 §6.1 functions (tags 0x00–0x06) ─────────────────────────
    //
    // Spec vectors: every byte below is hand-derived from the bit layout in
    // `docs/tables/ts_101_191/6-1-existing-individual-addressing-functions.md`.
    // The one real-capture vector lives in `tests/it/real_capture.rs`.

    /// Wrap one function (`tag`, `body`) in a one-transmitter payload
    /// (`tx_id` 0x1234).  `function_length` = 2 + body length.
    fn wrap_one(tag: u8, body: &[u8]) -> Vec<u8> {
        let function_length = (FUNC_HEADER_LEN + body.len()) as u8;
        let mut tx = vec![0x12, 0x34, function_length, tag, function_length];
        tx.extend_from_slice(body);
        let mut buf = vec![0x00, tx.len() as u8];
        buf.extend_from_slice(&tx);
        buf
    }

    /// Parse `wrap_one(tag, body)`, hand the typed body to `check`, and assert
    /// the payload re-serializes byte-identically.
    fn parse_and_round_trip(tag: u8, body: &[u8], check: impl FnOnce(&FunctionBody<'_>)) {
        let buf = wrap_one(tag, body);
        let parsed = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(parsed.transmitters[0].functions.len(), 1);
        check(&parsed.transmitters[0].functions[0].body);
        let mut out = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out).unwrap();
        assert_eq!(out, buf, "re-serialization must be byte-identical");
    }

    fn single(tag: u8, body: FunctionBody<'_>) -> IndividualAddressingPayload<'_> {
        IndividualAddressingPayload {
            rfu: 0,
            transmitters: vec![TransmitterEntry {
                transmitter_id: 1,
                functions: vec![FunctionEntry { tag, body }],
            }],
        }
    }

    fn assert_serialize_rejected(p: &IndividualAddressingPayload<'_>) {
        let mut buf = vec![0u8; p.serialized_len()];
        assert!(matches!(
            p.serialize_into(&mut buf),
            Err(crate::Error::ReservedBitsViolation { .. })
        ));
    }

    #[test]
    fn time_offset_vectors() {
        // 0xFF9C = -100 (two's complement), 0x8000 = i16::MIN, 0x7FFF = i16::MAX.
        for (bytes, want) in [
            ([0xFF, 0x9C], -100i16),
            ([0x80, 0x00], i16::MIN),
            ([0x7F, 0xFF], i16::MAX),
            ([0x00, 0x01], 1),
        ] {
            parse_and_round_trip(0x00, &bytes, |b| match b {
                FunctionBody::TimeOffset(t) => assert_eq!(t.time_offset, want),
                other => panic!("expected TimeOffset, got {other:?}"),
            });
        }
    }

    #[test]
    fn frequency_offset_vectors() {
        // 24-bit two's complement: 0xFFFF38 = -200, 0x800000 = -2^23,
        // 0x7FFFFF = 2^23-1, 0x000001 = 1.
        for (bytes, want) in [
            ([0xFF, 0xFF, 0x38], -200i32),
            ([0x80, 0x00, 0x00], -8_388_608),
            ([0x7F, 0xFF, 0xFF], 8_388_607),
            ([0x00, 0x00, 0x01], 1),
        ] {
            parse_and_round_trip(0x01, &bytes, |b| match b {
                FunctionBody::FrequencyOffset(f) => assert_eq!(f.frequency_offset, want),
                other => panic!("expected FrequencyOffset, got {other:?}"),
            });
        }
    }

    #[test]
    fn frequency_offset_out_of_24_bit_range_is_rejected() {
        for bad in [8_388_608, -8_388_609, i32::MAX, i32::MIN] {
            let p = single(
                0x01,
                FunctionBody::FrequencyOffset(FrequencyOffsetBody::new(bad)),
            );
            assert_serialize_rejected(&p);
        }
    }

    #[test]
    fn power_vector() {
        // tx_power 0x1234 (4660 x 0,1 dB), big-endian.
        parse_and_round_trip(0x02, &[0x12, 0x34], |b| match b {
            FunctionBody::Power(p) => assert_eq!(p.tx_power, 0x1234),
            other => panic!("expected Power, got {other:?}"),
        });
    }

    #[test]
    fn private_data_vectors() {
        parse_and_round_trip(0x03, &[0xDE, 0xAD, 0xBE, 0xEF], |b| match b {
            FunctionBody::PrivateData(p) => assert_eq!(p.private_data, &[0xDE, 0xAD, 0xBE, 0xEF]),
            other => panic!("expected PrivateData, got {other:?}"),
        });
        // N = 0 is a legal empty loop.
        parse_and_round_trip(0x03, &[], |b| match b {
            FunctionBody::PrivateData(p) => assert!(p.private_data.is_empty()),
            other => panic!("expected PrivateData, got {other:?}"),
        });
    }

    #[test]
    fn cell_id_bit_positions() {
        // byte 2 = wait_for_enable_flag[7] | reserved_future_use[6:0].
        // flag set, rfu 0x55: 0x80 | 0x55 = 0xD5.
        parse_and_round_trip(0x04, &[0x12, 0x34, 0xD5], |b| match b {
            FunctionBody::CellId(c) => {
                assert_eq!(c.cell_id, 0x1234);
                assert!(c.wait_for_enable_flag);
                assert_eq!(c.rfu, 0x55);
            }
            other => panic!("expected CellId, got {other:?}"),
        });
        // Only bit 7 -> only the flag; only bit 0 -> only the rfu LSB.
        parse_and_round_trip(0x04, &[0x00, 0x00, 0x80], |b| match b {
            FunctionBody::CellId(c) => {
                assert_eq!(c.cell_id, 0);
                assert!(c.wait_for_enable_flag);
                assert_eq!(c.rfu, 0);
            }
            other => panic!("expected CellId, got {other:?}"),
        });
        parse_and_round_trip(0x04, &[0xFF, 0xFF, 0x01], |b| match b {
            FunctionBody::CellId(c) => {
                assert_eq!(c.cell_id, 0xFFFF);
                assert!(!c.wait_for_enable_flag);
                assert_eq!(c.rfu, 0x01);
            }
            other => panic!("expected CellId, got {other:?}"),
        });
    }

    #[test]
    fn cell_id_oversized_rfu_is_rejected() {
        let mut body = CellIdBody::new(1, false);
        body.rfu = 0x80;
        assert_serialize_rejected(&single(0x04, FunctionBody::CellId(body)));
    }

    #[test]
    fn enable_vectors() {
        // Enables ACE-PAPR (0x10), time offset (0x00), and an unassigned tag.
        parse_and_round_trip(0x05, &[0x10, 0x00, 0x7E], |b| match b {
            FunctionBody::Enable(e) => assert_eq!(e.enabled_function_tags, vec![0x10, 0x00, 0x7E]),
            other => panic!("expected Enable, got {other:?}"),
        });
        parse_and_round_trip(0x05, &[], |b| match b {
            FunctionBody::Enable(e) => assert!(e.enabled_function_tags.is_empty()),
            other => panic!("expected Enable, got {other:?}"),
        });
    }

    #[test]
    fn bandwidth_bit_positions() {
        // byte = ch_bandwidth[7:1] | wait_for_enable_flag[0].
        for (byte, bw, flag) in [
            (0x00u8, 0u8, false),
            (0x01, 0, true),
            (0x02, 1, false),
            (0xFE, 127, false),
            (0xFF, 127, true),
        ] {
            parse_and_round_trip(0x06, &[byte], |b| match b {
                FunctionBody::Bandwidth(w) => {
                    assert_eq!(w.ch_bandwidth, bw);
                    assert_eq!(w.wait_for_enable_flag, flag);
                }
                other => panic!("expected Bandwidth, got {other:?}"),
            });
        }
        assert_eq!(CH_BANDWIDTH_5_MHZ, 0);
    }

    #[test]
    fn bandwidth_oversized_ch_bandwidth_is_rejected() {
        assert_serialize_rejected(&single(
            0x06,
            FunctionBody::Bandwidth(BandwidthBody::new(128, false)),
        ));
    }

    #[test]
    fn legacy_fixed_size_tags_with_wrong_length_stay_raw() {
        for (tag, body) in [
            (0x00u8, &[0x01u8][..]),
            (0x01, &[0x01, 0x02]),
            (0x02, &[0x01, 0x02, 0x03]),
            (0x04, &[0x01, 0x02]),
            (0x06, &[0x01, 0x02]),
        ] {
            parse_and_round_trip(tag, body, |b| match b {
                FunctionBody::Raw(r) => assert_eq!(*r, body),
                other => panic!("tag {tag:#04x}: expected Raw fallback, got {other:?}"),
            });
        }
    }

    #[test]
    fn function_length_counts_its_own_header() {
        // function_length 1 cannot even cover tag+length: reject, never wrap.
        let tx = [0x00, 0x01, 0x02, 0x00, 0x01];
        let mut buf = vec![0x00, tx.len() as u8];
        buf.extend_from_slice(&tx);
        assert_eq!(
            IndividualAddressingPayload::parse(&buf),
            Err(crate::Error::InvalidFunctionLength { function_length: 1 })
        );
        // function_length 0 likewise.
        let tx = [0x00, 0x01, 0x02, 0x00, 0x00];
        let mut buf = vec![0x00, tx.len() as u8];
        buf.extend_from_slice(&tx);
        assert_eq!(
            IndividualAddressingPayload::parse(&buf),
            Err(crate::Error::InvalidFunctionLength { function_length: 0 })
        );
        // function_length 2 = empty body.
        let tx = [0x00, 0x01, 0x02, 0x99, 0x02];
        let mut buf = vec![0x00, tx.len() as u8];
        buf.extend_from_slice(&tx);
        let parsed = IndividualAddressingPayload::parse(&buf).unwrap();
        assert!(matches!(
            parsed.transmitters[0].functions[0].body,
            FunctionBody::Raw(&[])
        ));
    }

    fn one_function(tag: u8, body: FunctionBody<'_>) -> IndividualAddressingPayload<'_> {
        IndividualAddressingPayload {
            rfu: 0,
            transmitters: vec![TransmitterEntry {
                transmitter_id: 1,
                functions: vec![FunctionEntry { tag, body }],
            }],
        }
    }

    fn serialize_err(p: &IndividualAddressingPayload<'_>) -> crate::Error {
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap_err()
    }

    /// A typed body under another tag, or a `Raw` body whose tag/length parses
    /// as a typed body, would not survive serialize -> parse: rejected.
    #[test]
    fn serialize_rejects_tag_body_mismatch() {
        // Typed body, wrong tag.
        let p = one_function(
            AddressingFunctionTag::Power as u8,
            FunctionBody::TimeOffset(TimeOffsetBody { time_offset: 7 }),
        );
        assert!(matches!(
            serialize_err(&p),
            crate::Error::FunctionTagBodyMismatch { tag, .. } if tag == AddressingFunctionTag::Power as u8
        ));
        // Raw under a known fixed-size tag with the matching length: would
        // re-parse as TimeOffset.
        let p = one_function(
            AddressingFunctionTag::TimeOffset as u8,
            FunctionBody::Raw(&[0x00, 0x07]),
        );
        assert!(matches!(
            serialize_err(&p),
            crate::Error::FunctionTagBodyMismatch { .. }
        ));
        // Raw under an any-length typed tag (private data, enable).
        for tag in [
            AddressingFunctionTag::PrivateData as u8,
            AddressingFunctionTag::Enable as u8,
        ] {
            let p = one_function(tag, FunctionBody::Raw(&[1, 2, 3]));
            assert!(matches!(
                serialize_err(&p),
                crate::Error::FunctionTagBodyMismatch { .. }
            ));
        }
    }

    /// The legitimate Raw uses still serialize and round-trip identically: an
    /// unknown tag, and a known fixed-size tag with a non-matching length.
    #[test]
    fn serialize_accepts_legitimate_raw_and_matching_typed() {
        let unknown = one_function(0xEE, FunctionBody::Raw(&[1, 2, 3]));
        let wrong_len = one_function(
            AddressingFunctionTag::TimeOffset as u8,
            FunctionBody::Raw(&[1, 2, 3]),
        );
        let typed = one_function(
            AddressingFunctionTag::TimeOffset as u8,
            FunctionBody::TimeOffset(TimeOffsetBody { time_offset: -5 }),
        );
        for p in [unknown, wrong_len, typed] {
            let mut out = vec![0u8; p.serialized_len()];
            p.serialize_into(&mut out).unwrap();
            assert_eq!(IndividualAddressingPayload::parse(&out).unwrap(), p);
        }
    }

    #[test]
    fn round_trip_all_legacy_bodies_in_one_payload() {
        let orig = IndividualAddressingPayload {
            rfu: 0xA5,
            transmitters: vec![TransmitterEntry {
                transmitter_id: 0xBEEF,
                functions: vec![
                    FunctionEntry {
                        tag: 0x00,
                        body: FunctionBody::TimeOffset(TimeOffsetBody::new(-1234)),
                    },
                    FunctionEntry {
                        tag: 0x01,
                        body: FunctionBody::FrequencyOffset(FrequencyOffsetBody::new(-50_000)),
                    },
                    FunctionEntry {
                        tag: 0x02,
                        body: FunctionBody::Power(PowerBody::new(0xFFFF)),
                    },
                    FunctionEntry {
                        tag: 0x03,
                        body: FunctionBody::PrivateData(PrivateDataBody::new(&[1, 2, 3])),
                    },
                    FunctionEntry {
                        tag: 0x04,
                        body: FunctionBody::CellId(CellIdBody::new(0x0102, true)),
                    },
                    FunctionEntry {
                        tag: 0x05,
                        body: FunctionBody::Enable(EnableBody::new(vec![0x04, 0x10])),
                    },
                    FunctionEntry {
                        tag: 0x06,
                        body: FunctionBody::Bandwidth(BandwidthBody::new(0, true)),
                    },
                ],
            }],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        assert_eq!(IndividualAddressingPayload::parse(&buf).unwrap(), orig);
    }
}
