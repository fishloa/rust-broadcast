//! Enumerations for EN 302 755 §7.2 L1-pre / L1-post signalling fields.
//!
//! Every enum has a `Reserved(u8)` catch-all so parsing never fails on
//! reserved/future values. Use `from_u8` / `to_u8` for lossless round-trips.
//! PAPR and PLP code-rate decoding are version/profile-dependent and are
//! handled as methods on `L1Pre` and `PlpConfig` (see `pre.rs` / `post.rs`).

/// Transmitter input stream types — EN 302 755 §7.2.2 Table 21 (8-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum TxInputStreamType {
    /// 0x00 — Transport Stream (TS) only.
    TsOnly,
    /// 0x01 — Generic Stream (GSE and/or GFPS and/or GCS) but not TS.
    GenericStream,
    /// 0x02 — Both TS and Generic Stream.
    Both,
    /// All other values — reserved.
    Reserved(u8),
}

impl TxInputStreamType {
    /// Decode from the 8-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::TsOnly,
            0x01 => Self::GenericStream,
            0x02 => Self::Both,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 8-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::TsOnly => 0x00,
            Self::GenericStream => 0x01,
            Self::Both => 0x02,
            Self::Reserved(v) => v,
        }
    }
}

/// Guard interval — EN 302 755 §7.2.2 Table 22 (3-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum GuardInterval {
    /// 000 — 1/32.
    G1_32,
    /// 001 — 1/16.
    G1_16,
    /// 010 — 1/8.
    G1_8,
    /// 011 — 1/4.
    G1_4,
    /// 100 — 1/128.
    G1_128,
    /// 101 — 19/128.
    G19_128,
    /// 110 — 19/256.
    G19_256,
    /// 111 — Reserved.
    Reserved(u8),
}

impl GuardInterval {
    /// Decode from the 3-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::G1_32,
            1 => Self::G1_16,
            2 => Self::G1_8,
            3 => Self::G1_4,
            4 => Self::G1_128,
            5 => Self::G19_128,
            6 => Self::G19_256,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 3-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::G1_32 => 0,
            Self::G1_16 => 1,
            Self::G1_8 => 2,
            Self::G1_4 => 3,
            Self::G1_128 => 4,
            Self::G19_128 => 5,
            Self::G19_256 => 6,
            Self::Reserved(v) => v,
        }
    }
}

/// L1-post constellation — EN 302 755 §7.2.2 Table 24 (4-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum L1Modulation {
    /// 0000 — BPSK.
    Bpsk,
    /// 0001 — QPSK.
    Qpsk,
    /// 0010 — 16-QAM.
    Qam16,
    /// 0011 — 64-QAM.
    Qam64,
    /// All other values — reserved.
    Reserved(u8),
}

impl L1Modulation {
    /// Decode from the 4-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            0 => Self::Bpsk,
            1 => Self::Qpsk,
            2 => Self::Qam16,
            3 => Self::Qam64,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 4-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Bpsk => 0,
            Self::Qpsk => 1,
            Self::Qam16 => 2,
            Self::Qam64 => 3,
            Self::Reserved(v) => v,
        }
    }
}

/// L1-post code rate — EN 302 755 §7.2.2 Table 25 (2-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum L1CodeRate {
    /// 00 — 1/2.
    R1_2,
    /// All other values — reserved.
    Reserved(u8),
}

impl L1CodeRate {
    /// Decode from the 2-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::R1_2,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 2-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::R1_2 => 0,
            Self::Reserved(v) => v,
        }
    }
}

/// L1-post FEC type — EN 302 755 §7.2.2 Table 26 (2-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum L1FecType {
    /// 00 — LDPC 16K.
    Ldpc16K,
    /// All other values — reserved.
    Reserved(u8),
}

impl L1FecType {
    /// Decode from the 2-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::Ldpc16K,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 2-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Ldpc16K => 0,
            Self::Reserved(v) => v,
        }
    }
}

/// Scattered pilot pattern — EN 302 755 §7.2.2 Table 27 (4-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PilotPattern {
    /// 0000 — PP1.
    Pp1,
    /// 0001 — PP2.
    Pp2,
    /// 0010 — PP3.
    Pp3,
    /// 0011 — PP4.
    Pp4,
    /// 0100 — PP5.
    Pp5,
    /// 0101 — PP6.
    Pp6,
    /// 0110 — PP7.
    Pp7,
    /// 0111 — PP8.
    Pp8,
    /// All other values — reserved.
    Reserved(u8),
}

impl PilotPattern {
    /// Decode from the 4-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            0 => Self::Pp1,
            1 => Self::Pp2,
            2 => Self::Pp3,
            3 => Self::Pp4,
            4 => Self::Pp5,
            5 => Self::Pp6,
            6 => Self::Pp7,
            7 => Self::Pp8,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 4-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Pp1 => 0,
            Self::Pp2 => 1,
            Self::Pp3 => 2,
            Self::Pp4 => 3,
            Self::Pp5 => 4,
            Self::Pp6 => 5,
            Self::Pp7 => 6,
            Self::Pp8 => 7,
            Self::Reserved(v) => v,
        }
    }
}

/// T2 version field — EN 302 755 §7.2.2 Table 28 (4-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum T2Version {
    /// 0000 — Specification version 1.1.1.
    V1_1_1,
    /// 0001 — Specification version 1.2.1.
    V1_2_1,
    /// 0010 — Specification version 1.3.1.
    V1_3_1,
    /// All other values — reserved.
    Reserved(u8),
}

impl T2Version {
    /// Decode from the 4-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            0 => Self::V1_1_1,
            1 => Self::V1_2_1,
            2 => Self::V1_3_1,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 4-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::V1_1_1 => 0,
            Self::V1_2_1 => 1,
            Self::V1_3_1 => 2,
            Self::Reserved(v) => v,
        }
    }
}

/// PLP type — EN 302 755 §7.2.3.1 Table 30 (3-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PlpType {
    /// 000 — Common PLP.
    Common,
    /// 001 — Data PLP Type 1.
    DataType1,
    /// 010 — Data PLP Type 2.
    DataType2,
    /// All other values — reserved.
    Reserved(u8),
}

impl PlpType {
    /// Decode from the 3-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::Common,
            1 => Self::DataType1,
            2 => Self::DataType2,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 3-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Common => 0,
            Self::DataType1 => 1,
            Self::DataType2 => 2,
            Self::Reserved(v) => v,
        }
    }
}

/// PLP payload type — EN 302 755 §7.2.3.1 Table 31 (5-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PlpPayloadType {
    /// 00000 — GFPS.
    Gfps,
    /// 00001 — GCS.
    Gcs,
    /// 00010 — GSE.
    Gse,
    /// 00011 — TS.
    Ts,
    /// All other values — reserved.
    Reserved(u8),
}

impl PlpPayloadType {
    /// Decode from the 5-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x1F {
            0 => Self::Gfps,
            1 => Self::Gcs,
            2 => Self::Gse,
            3 => Self::Ts,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 5-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Gfps => 0,
            Self::Gcs => 1,
            Self::Gse => 2,
            Self::Ts => 3,
            Self::Reserved(v) => v,
        }
    }
}

/// PLP modulation — EN 302 755 §7.2.3.1 Table 33 (3-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PlpModulation {
    /// 000 — QPSK.
    Qpsk,
    /// 001 — 16-QAM.
    Qam16,
    /// 010 — 64-QAM.
    Qam64,
    /// 011 — 256-QAM.
    Qam256,
    /// All other values — reserved.
    Reserved(u8),
}

impl PlpModulation {
    /// Decode from the 3-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::Qpsk,
            1 => Self::Qam16,
            2 => Self::Qam64,
            3 => Self::Qam256,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 3-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Qpsk => 0,
            Self::Qam16 => 1,
            Self::Qam64 => 2,
            Self::Qam256 => 3,
            Self::Reserved(v) => v,
        }
    }
}

/// PLP FEC type — EN 302 755 §7.2.3.1 Table 34 (2-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PlpFecType {
    /// 00 — 16K LDPC.
    Ldpc16K,
    /// 01 — 64K LDPC.
    Ldpc64K,
    /// All other values — reserved.
    Reserved(u8),
}

impl PlpFecType {
    /// Decode from the 2-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::Ldpc16K,
            1 => Self::Ldpc64K,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 2-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Ldpc16K => 0,
            Self::Ldpc64K => 1,
            Self::Reserved(v) => v,
        }
    }
}

/// PLP mode — EN 302 755 §7.2.3.1 Table 35 (2-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PlpMode {
    /// 00 — Not specified (valid only if T2_VERSION='0000').
    NotSpecified,
    /// 01 — Normal Mode.
    Normal,
    /// 10 — High Efficiency Mode.
    HighEfficiency,
    /// All other values — reserved.
    Reserved(u8),
}

impl PlpMode {
    /// Decode from the 2-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::NotSpecified,
            1 => Self::Normal,
            2 => Self::HighEfficiency,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 2-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::NotSpecified => 0,
            Self::Normal => 1,
            Self::HighEfficiency => 2,
            Self::Reserved(v) => v,
        }
    }
}

/// Auxiliary stream type — EN 302 755 §7.2.3.1 Table 36 (4-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum AuxStreamType {
    /// 0000 — TX-SIG (see ETSI TS 102 992).
    TxSig,
    /// All other values — reserved.
    Reserved(u8),
}

impl AuxStreamType {
    /// Decode from the 4-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            0 => Self::TxSig,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 4-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::TxSig => 0,
            Self::Reserved(v) => v,
        }
    }
}

/// PAPR reduction for T2_VERSION = '0000' — EN 302 755 §7.2.2 Table 23a (4-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PaprReductionV0 {
    /// 0000 — No PAPR reduction is used.
    NoReduction,
    /// 0001 — ACE-PAPR only is used.
    AceOnly,
    /// 0010 — TR-PAPR only is used.
    TrOnly,
    /// 0011 — Both ACE and TR are used.
    AceAndTr,
    /// All other values — reserved.
    Reserved(u8),
}

impl PaprReductionV0 {
    /// Decode from the 4-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            0 => Self::NoReduction,
            1 => Self::AceOnly,
            2 => Self::TrOnly,
            3 => Self::AceAndTr,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 4-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::NoReduction => 0,
            Self::AceOnly => 1,
            Self::TrOnly => 2,
            Self::AceAndTr => 3,
            Self::Reserved(v) => v,
        }
    }
}

/// PAPR reduction for T2_VERSION > '0000' — EN 302 755 §7.2.2 Table 23b (4-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PaprReductionVn {
    /// 0000 — L1-ACE is used and TR is used on P2 symbols only.
    L1AceTrOnP2,
    /// 0001 — L1-ACE and ACE only are used.
    L1AceAndAce,
    /// 0010 — L1-ACE and TR only are used.
    L1AceAndTr,
    /// 0011 — L1-ACE, ACE and TR are used.
    L1AceAceAndTr,
    /// All other values — reserved.
    Reserved(u8),
}

impl PaprReductionVn {
    /// Decode from the 4-bit wire value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            0 => Self::L1AceTrOnP2,
            1 => Self::L1AceAndAce,
            2 => Self::L1AceAndTr,
            3 => Self::L1AceAceAndTr,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 4-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::L1AceTrOnP2 => 0,
            Self::L1AceAndAce => 1,
            Self::L1AceAndTr => 2,
            Self::L1AceAceAndTr => 3,
            Self::Reserved(v) => v,
        }
    }
}

/// Version-tagged PAPR reduction result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PaprReduction {
    /// Decoded using Table 23a (T2_VERSION = '0000').
    V0(PaprReductionV0),
    /// Decoded using Table 23b (T2_VERSION > '0000').
    Vn(PaprReductionVn),
}

/// PLP code rate (T2-base profile column) — EN 302 755 §7.2.3.1 Table 32 (3-bit field).
///
/// The T2-Lite column differs (see Table 32); this enum decodes the T2-base column only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PlpCodeRate {
    /// 000 — 1/2.
    R1_2,
    /// 001 — 3/5.
    R3_5,
    /// 010 — 2/3.
    R2_3,
    /// 011 — 3/4.
    R3_4,
    /// 100 — 4/5.
    R4_5,
    /// 101 — 5/6.
    R5_6,
    /// All other values — reserved (T2-base; T2-Lite uses 110/111 differently).
    Reserved(u8),
}

impl PlpCodeRate {
    /// Decode from the 3-bit wire value (T2-base profile column, Table 32).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::R1_2,
            1 => Self::R3_5,
            2 => Self::R2_3,
            3 => Self::R3_4,
            4 => Self::R4_5,
            5 => Self::R5_6,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 3-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::R1_2 => 0,
            Self::R3_5 => 1,
            Self::R2_3 => 2,
            Self::R3_4 => 3,
            Self::R4_5 => 4,
            Self::R5_6 => 5,
            Self::Reserved(v) => v,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tx_input_stream_type_round_trip() {
        for v in 0u8..=2 {
            let e = TxInputStreamType::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, TxInputStreamType::Reserved(_)));
        }
        assert!(matches!(
            TxInputStreamType::from_u8(3),
            TxInputStreamType::Reserved(3)
        ));
        assert!(matches!(
            TxInputStreamType::from_u8(0xFF),
            TxInputStreamType::Reserved(0xFF)
        ));
    }

    #[test]
    fn guard_interval_round_trip() {
        for v in 0u8..=6 {
            let e = GuardInterval::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, GuardInterval::Reserved(_)));
        }
        assert!(matches!(
            GuardInterval::from_u8(7),
            GuardInterval::Reserved(7)
        ));
    }

    #[test]
    fn l1_modulation_round_trip() {
        for v in 0u8..=3 {
            let e = L1Modulation::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, L1Modulation::Reserved(_)));
        }
        assert!(matches!(
            L1Modulation::from_u8(0x0F),
            L1Modulation::Reserved(_)
        ));
    }

    #[test]
    fn l1_code_rate_round_trip() {
        assert_eq!(L1CodeRate::from_u8(0).to_u8(), 0);
        assert!(!matches!(L1CodeRate::from_u8(0), L1CodeRate::Reserved(_)));
        assert!(matches!(L1CodeRate::from_u8(1), L1CodeRate::Reserved(1)));
        assert!(matches!(L1CodeRate::from_u8(3), L1CodeRate::Reserved(3)));
    }

    #[test]
    fn l1_fec_type_round_trip() {
        assert_eq!(L1FecType::from_u8(0).to_u8(), 0);
        assert!(matches!(L1FecType::from_u8(1), L1FecType::Reserved(1)));
    }

    #[test]
    fn pilot_pattern_round_trip() {
        for v in 0u8..=7 {
            let e = PilotPattern::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, PilotPattern::Reserved(_)));
        }
        assert!(matches!(
            PilotPattern::from_u8(8),
            PilotPattern::Reserved(8)
        ));
    }

    #[test]
    fn t2_version_round_trip() {
        for v in 0u8..=2 {
            let e = T2Version::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, T2Version::Reserved(_)));
        }
        assert!(matches!(T2Version::from_u8(3), T2Version::Reserved(3)));
    }

    #[test]
    fn plp_type_round_trip() {
        for v in 0u8..=2 {
            let e = PlpType::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, PlpType::Reserved(_)));
        }
        assert!(matches!(PlpType::from_u8(7), PlpType::Reserved(7)));
    }

    #[test]
    fn plp_payload_type_round_trip() {
        for v in 0u8..=3 {
            let e = PlpPayloadType::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, PlpPayloadType::Reserved(_)));
        }
        assert!(matches!(
            PlpPayloadType::from_u8(0x1F),
            PlpPayloadType::Reserved(_)
        ));
    }

    #[test]
    fn plp_modulation_round_trip() {
        for v in 0u8..=3 {
            let e = PlpModulation::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, PlpModulation::Reserved(_)));
        }
        assert!(matches!(
            PlpModulation::from_u8(7),
            PlpModulation::Reserved(7)
        ));
    }

    #[test]
    fn plp_fec_type_round_trip() {
        for v in 0u8..=1 {
            let e = PlpFecType::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, PlpFecType::Reserved(_)));
        }
        assert!(matches!(PlpFecType::from_u8(2), PlpFecType::Reserved(2)));
    }

    #[test]
    fn plp_mode_round_trip() {
        for v in 0u8..=2 {
            let e = PlpMode::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, PlpMode::Reserved(_)));
        }
        assert!(matches!(PlpMode::from_u8(3), PlpMode::Reserved(3)));
    }

    #[test]
    fn aux_stream_type_round_trip() {
        assert_eq!(AuxStreamType::from_u8(0).to_u8(), 0);
        assert!(matches!(
            AuxStreamType::from_u8(0x0F),
            AuxStreamType::Reserved(_)
        ));
    }

    #[test]
    fn plp_code_rate_round_trip() {
        for v in 0u8..=5 {
            let e = PlpCodeRate::from_u8(v);
            assert_eq!(e.to_u8(), v);
            assert!(!matches!(e, PlpCodeRate::Reserved(_)));
        }
        assert!(matches!(PlpCodeRate::from_u8(6), PlpCodeRate::Reserved(6)));
        assert!(matches!(PlpCodeRate::from_u8(7), PlpCodeRate::Reserved(7)));
    }
}
