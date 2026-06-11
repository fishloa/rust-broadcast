//! SH Delivery System Descriptor — ETSI EN 300 468 §6.4.6.2 (tag_extension 0x05).
use super::*;

impl<'a> ExtensionBodyDef<'a> for ShDeliverySystem {
    const TAG_EXTENSION: u8 = 0x05;
    const NAME: &'static str = "SH_DELIVERY_SYSTEM";
}

// ---------------------------------------------------------------------------
//  SH-specific enums (Tables 120, 123-132)
// ---------------------------------------------------------------------------

/// Diversity mode — ETSI EN 300 468 Table 120.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ShDiversityMode {
    /// No diversity.
    NoDiversity,
    /// paTS only (0b1000).
    PaTsOnly,
    /// paTS + FEC diversity, FEC at link (0b1101).
    FecAtLink,
    /// paTS + FEC diversity, FEC at PHY (0b1110).
    FecAtPhy,
    /// paTS + FEC diversity, FEC at PHY and link (0b1111).
    FecAtPhyAndLink,
    /// Reserved / future use.
    Reserved(u8),
}

impl ShDiversityMode {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => ShDiversityMode::NoDiversity,
            0x08 => ShDiversityMode::PaTsOnly,
            0x0D => ShDiversityMode::FecAtLink,
            0x0E => ShDiversityMode::FecAtPhy,
            0x0F => ShDiversityMode::FecAtPhyAndLink,
            other => ShDiversityMode::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            ShDiversityMode::NoDiversity => 0x00,
            ShDiversityMode::PaTsOnly => 0x08,
            ShDiversityMode::FecAtLink => 0x0D,
            ShDiversityMode::FecAtPhy => 0x0E,
            ShDiversityMode::FecAtPhyAndLink => 0x0F,
            ShDiversityMode::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            ShDiversityMode::NoDiversity => "no diversity",
            ShDiversityMode::PaTsOnly => "paTS only",
            ShDiversityMode::FecAtLink => "paTS + FEC diversity, FEC at link",
            ShDiversityMode::FecAtPhy => "paTS + FEC diversity, FEC at PHY",
            ShDiversityMode::FecAtPhyAndLink => "paTS + FEC diversity, FEC at PHY and link",
            ShDiversityMode::Reserved(_) => "reserved",
        }
    }
}

/// Polarization for SH — ETSI EN 300 468 Table 123.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ShPolarization {
    /// Linear horizontal.
    LinearHorizontal,
    /// Linear vertical.
    LinearVertical,
    /// Circular left.
    CircularLeft,
    /// Circular right.
    CircularRight,
    /// Reserved / future use.
    Reserved(u8),
}

impl ShPolarization {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::LinearHorizontal,
            1 => Self::LinearVertical,
            2 => Self::CircularLeft,
            3 => Self::CircularRight,
            other => Self::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::LinearHorizontal => 0,
            Self::LinearVertical => 1,
            Self::CircularLeft => 2,
            Self::CircularRight => 3,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            Self::LinearHorizontal => "linear horizontal",
            Self::LinearVertical => "linear vertical",
            Self::CircularLeft => "circular left",
            Self::CircularRight => "circular right",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// Roll-off factor for SH — ETSI EN 300 468 Table 124.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ShRollOff {
    /// α = 0.35.
    Alpha035,
    /// α = 0.25.
    Alpha025,
    /// α = 0.15.
    Alpha015,
    /// Reserved / future use.
    Reserved(u8),
}

impl ShRollOff {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Alpha035,
            1 => Self::Alpha025,
            2 => Self::Alpha015,
            other => Self::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Alpha035 => 0,
            Self::Alpha025 => 1,
            Self::Alpha015 => 2,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            Self::Alpha035 => "α = 0.35",
            Self::Alpha025 => "α = 0.25",
            Self::Alpha015 => "α = 0.15",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// Modulation mode for TDM — ETSI EN 300 468 Table 125.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ShModulationModeType {
    /// QPSK.
    Qpsk,
    /// 8PSK.
    Psk8,
    /// 16APSK.
    Apsk16,
    /// Reserved / future use.
    Reserved(u8),
}

impl ShModulationModeType {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Qpsk,
            1 => Self::Psk8,
            2 => Self::Apsk16,
            other => Self::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Qpsk => 0,
            Self::Psk8 => 1,
            Self::Apsk16 => 2,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            Self::Qpsk => "QPSK",
            Self::Psk8 => "8PSK",
            Self::Apsk16 => "16APSK",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// Code rate for SH — ETSI EN 300 468 Table 126 (4 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ShCodeRate {
    /// 1/5 standard.
    Rate1_5Standard,
    /// 2/9 standard.
    Rate2_9Standard,
    /// 1/4 standard.
    Rate1_4Standard,
    /// 2/7 standard.
    Rate2_7Standard,
    /// 1/3 standard.
    Rate1_3Standard,
    /// 1/3 complementary.
    Rate1_3Complementary,
    /// 2/5 standard.
    Rate2_5Standard,
    /// 2/5 complementary.
    Rate2_5Complementary,
    /// 1/2 standard.
    Rate1_2Standard,
    /// 1/2 complementary.
    Rate1_2Complementary,
    /// 2/3 standard.
    Rate2_3Standard,
    /// 2/3 complementary.
    Rate2_3Complementary,
    /// Reserved / future use.
    Reserved(u8),
}

impl ShCodeRate {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => ShCodeRate::Rate1_5Standard,
            0x01 => ShCodeRate::Rate2_9Standard,
            0x02 => ShCodeRate::Rate1_4Standard,
            0x03 => ShCodeRate::Rate2_7Standard,
            0x04 => ShCodeRate::Rate1_3Standard,
            0x05 => ShCodeRate::Rate1_3Complementary,
            0x06 => ShCodeRate::Rate2_5Standard,
            0x07 => ShCodeRate::Rate2_5Complementary,
            0x08 => ShCodeRate::Rate1_2Standard,
            0x09 => ShCodeRate::Rate1_2Complementary,
            0x0A => ShCodeRate::Rate2_3Standard,
            0x0B => ShCodeRate::Rate2_3Complementary,
            other => ShCodeRate::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            ShCodeRate::Rate1_5Standard => 0x00,
            ShCodeRate::Rate2_9Standard => 0x01,
            ShCodeRate::Rate1_4Standard => 0x02,
            ShCodeRate::Rate2_7Standard => 0x03,
            ShCodeRate::Rate1_3Standard => 0x04,
            ShCodeRate::Rate1_3Complementary => 0x05,
            ShCodeRate::Rate2_5Standard => 0x06,
            ShCodeRate::Rate2_5Complementary => 0x07,
            ShCodeRate::Rate1_2Standard => 0x08,
            ShCodeRate::Rate1_2Complementary => 0x09,
            ShCodeRate::Rate2_3Standard => 0x0A,
            ShCodeRate::Rate2_3Complementary => 0x0B,
            ShCodeRate::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            ShCodeRate::Rate1_5Standard => "1/5 standard",
            ShCodeRate::Rate2_9Standard => "2/9 standard",
            ShCodeRate::Rate1_4Standard => "1/4 standard",
            ShCodeRate::Rate2_7Standard => "2/7 standard",
            ShCodeRate::Rate1_3Standard => "1/3 standard",
            ShCodeRate::Rate1_3Complementary => "1/3 complementary",
            ShCodeRate::Rate2_5Standard => "2/5 standard",
            ShCodeRate::Rate2_5Complementary => "2/5 complementary",
            ShCodeRate::Rate1_2Standard => "1/2 standard",
            ShCodeRate::Rate1_2Complementary => "1/2 complementary",
            ShCodeRate::Rate2_3Standard => "2/3 standard",
            ShCodeRate::Rate2_3Complementary => "2/3 complementary",
            ShCodeRate::Reserved(_) => "reserved",
        }
    }
}

/// Bandwidth for OFDM — ETSI EN 300 468 Table 128 (3 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ShBandwidth {
    /// 8 MHz.
    Mhz8,
    /// 7 MHz.
    Mhz7,
    /// 6 MHz.
    Mhz6,
    /// 5 MHz.
    Mhz5,
    /// 1.7 MHz.
    Mhz1_7,
    /// Reserved / future use.
    Reserved(u8),
}

impl ShBandwidth {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Mhz8,
            1 => Self::Mhz7,
            2 => Self::Mhz6,
            3 => Self::Mhz5,
            4 => Self::Mhz1_7,
            other => Self::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Mhz8 => 0,
            Self::Mhz7 => 1,
            Self::Mhz6 => 2,
            Self::Mhz5 => 3,
            Self::Mhz1_7 => 4,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mhz8 => "8 MHz",
            Self::Mhz7 => "7 MHz",
            Self::Mhz6 => "6 MHz",
            Self::Mhz5 => "5 MHz",
            Self::Mhz1_7 => "1.7 MHz",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// Constellation and hierarchy — ETSI EN 300 468 Table 130 (3 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ShConstellationAndHierarchy {
    /// QPSK.
    Qpsk,
    /// 16QAM, non-hierarchical.
    Qam16NonHierarchical,
    /// 16QAM, hierarchical, α = 1.
    Qam16Alpha1,
    /// 16QAM, hierarchical, α = 2.
    Qam16Alpha2,
    /// 16QAM, hierarchical, α = 3.
    Qam16Alpha3,
    /// Reserved / future use.
    Reserved(u8),
}

impl ShConstellationAndHierarchy {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Qpsk,
            1 => Self::Qam16NonHierarchical,
            2 => Self::Qam16Alpha1,
            3 => Self::Qam16Alpha2,
            4 => Self::Qam16Alpha3,
            other => Self::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Qpsk => 0,
            Self::Qam16NonHierarchical => 1,
            Self::Qam16Alpha1 => 2,
            Self::Qam16Alpha2 => 3,
            Self::Qam16Alpha3 => 4,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            Self::Qpsk => "QPSK",
            Self::Qam16NonHierarchical => "16QAM, non-hierarchical",
            Self::Qam16Alpha1 => "16QAM, α = 1",
            Self::Qam16Alpha2 => "16QAM, α = 2",
            Self::Qam16Alpha3 => "16QAM, α = 3",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// Guard interval for OFDM — ETSI EN 300 468 Table 131 (2 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ShGuardInterval {
    /// 1/32.
    G1_32,
    /// 1/16.
    G1_16,
    /// 1/8.
    G1_8,
    /// 1/4.
    G1_4,
    /// Reserved / future use.
    Reserved(u8),
}

impl ShGuardInterval {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::G1_32,
            1 => Self::G1_16,
            2 => Self::G1_8,
            3 => Self::G1_4,
            other => Self::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::G1_32 => 0,
            Self::G1_16 => 1,
            Self::G1_8 => 2,
            Self::G1_4 => 3,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            Self::G1_32 => "1/32",
            Self::G1_16 => "1/16",
            Self::G1_8 => "1/8",
            Self::G1_4 => "1/4",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// Transmission mode for OFDM — ETSI EN 300 468 Table 132 (2 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ShTransmissionMode {
    /// 1k mode.
    Mode1k,
    /// 2k mode.
    Mode2k,
    /// 4k mode.
    Mode4k,
    /// 8k mode.
    Mode8k,
    /// Reserved / future use.
    Reserved(u8),
}

impl ShTransmissionMode {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Mode1k,
            1 => Self::Mode2k,
            2 => Self::Mode4k,
            3 => Self::Mode8k,
            other => Self::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Mode1k => 0,
            Self::Mode2k => 1,
            Self::Mode4k => 2,
            Self::Mode8k => 3,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mode1k => "1k mode",
            Self::Mode2k => "2k mode",
            Self::Mode4k => "4k mode",
            Self::Mode8k => "8k mode",
            Self::Reserved(_) => "reserved",
        }
    }
}

// ---------------------------------------------------------------------------
//  Structs
// ---------------------------------------------------------------------------

/// SH_delivery_system body (Table 119, §6.4.6.2). The modulation loop is
/// unfolded; `modulation_type` (Table 121) selects Tdm/Ofdm,
/// `interleaver_presence` (Table 122) gates the interleaver, and
/// `interleaver_type` selects its layout. Diversity mode: Table 120.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ShDeliverySystem {
    /// `diversity_mode` — Table 120.
    pub diversity_mode: ShDiversityMode,
    /// Modulation entries (the loop to end of body).
    pub modulations: Vec<ShModulation>,
}

/// One modulation entry in the SH_delivery_system loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ShModulation {
    /// Modulation parameters; the variant encodes `modulation_type` (Table 121).
    pub modulation: ShModulationMode,
    /// Interleaver block; `Some` encodes `interleaver_presence==1`, the variant
    /// encodes `interleaver_type`.
    pub interleaver: Option<ShInterleaver>,
}

/// Modulation mode for an SH delivery system entry (Table 121).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ShModulationMode {
    /// `modulation_type == 0` — Time-Domain Multiplex.
    Tdm {
        /// polarization (2 bits) — Table 123.
        polarization: ShPolarization,
        /// roll_off (2 bits) — Table 124.
        roll_off: ShRollOff,
        /// modulation_mode (2 bits) — Table 125.
        modulation_mode: ShModulationModeType,
        /// code_rate (4 bits) — Table 126.
        code_rate: ShCodeRate,
        /// symbol_rate (5 bits) — Table 127 (raw).
        symbol_rate: u8,
    },
    /// `modulation_type == 1` — OFDM.
    Ofdm {
        /// bandwidth (3 bits) — Table 128.
        bandwidth: ShBandwidth,
        /// priority (1 bit) — Table 129.
        priority: bool,
        /// constellation_and_hierarchy (3 bits) — Table 130.
        constellation_and_hierarchy: ShConstellationAndHierarchy,
        /// code_rate (4 bits) — Table 126.
        code_rate: ShCodeRate,
        /// guard_interval (2 bits) — Table 131.
        guard_interval: ShGuardInterval,
        /// transmission_mode (2 bits) — Table 132.
        transmission_mode: ShTransmissionMode,
        /// common_frequency (1 bit).
        common_frequency: bool,
    },
}

/// Interleaver block for an SH modulation entry (Table 122).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ShInterleaver {
    /// `interleaver_type == 0` — full interleaver parameters.
    Type0 {
        /// common_multiplier (6 bits).
        common_multiplier: u8,
        /// nof_late_taps (6 bits).
        nof_late_taps: u8,
        /// nof_slices (6 bits).
        nof_slices: u8,
        /// slice_distance (8 bits).
        slice_distance: u8,
        /// non_late_increments (6 bits).
        non_late_increments: u8,
    },
    /// `interleaver_type == 1` — common_multiplier only.
    Type1 {
        /// common_multiplier (6 bits).
        common_multiplier: u8,
    },
}

// ---------------------------------------------------------------------------
//  Parse / Serialize
// ---------------------------------------------------------------------------

impl<'a> Parse<'a> for ShDeliverySystem {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: sel.len(),
                what: "SH_delivery_system body",
            });
        }
        let diversity_mode = ShDiversityMode::from_u8(sel[0] >> 4);
        let mut pos = 1;
        let mut modulations = Vec::new();
        while pos < sel.len() {
            if sel.len() - pos < 3 {
                return Err(Error::BufferTooShort {
                    need: pos + 3,
                    have: sel.len(),
                    what: "SH_delivery_system body",
                });
            }
            let flags = sel[pos];
            let modulation_type = (flags >> 7) & 0x01;
            let interleaver_presence = (flags >> 6) & 0x01;
            let interleaver_type = (flags >> 5) & 0x01;
            let mb0 = sel[pos + 1];
            let mb1 = sel[pos + 2];
            pos += 3;

            let modulation = if modulation_type == 0 {
                let polarization = ShPolarization::from_u8(mb0 >> 6);
                let roll_off = ShRollOff::from_u8((mb0 >> 4) & 0x03);
                let modulation_mode = ShModulationModeType::from_u8((mb0 >> 2) & 0x03);
                let code_rate_raw = ((mb0 & 0x03) << 2) | (mb1 >> 6);
                let code_rate = ShCodeRate::from_u8(code_rate_raw);
                let symbol_rate = (mb1 >> 1) & 0x1F;
                ShModulationMode::Tdm {
                    polarization,
                    roll_off,
                    modulation_mode,
                    code_rate,
                    symbol_rate,
                }
            } else {
                let bandwidth = ShBandwidth::from_u8(mb0 >> 5);
                let priority = ((mb0 >> 4) & 0x01) != 0;
                let constellation_and_hierarchy =
                    ShConstellationAndHierarchy::from_u8((mb0 >> 1) & 0x07);
                let code_rate_raw = ((mb0 & 0x01) << 3) | (mb1 >> 5);
                let code_rate = ShCodeRate::from_u8(code_rate_raw);
                let guard_interval = ShGuardInterval::from_u8((mb1 >> 3) & 0x03);
                let transmission_mode = ShTransmissionMode::from_u8((mb1 >> 1) & 0x03);
                let common_frequency = (mb1 & 0x01) != 0;
                ShModulationMode::Ofdm {
                    bandwidth,
                    priority,
                    constellation_and_hierarchy,
                    code_rate,
                    guard_interval,
                    transmission_mode,
                    common_frequency,
                }
            };

            let interleaver = if interleaver_presence == 1 {
                if interleaver_type == 0 {
                    if sel.len() - pos < 4 {
                        return Err(Error::BufferTooShort {
                            need: pos + 4,
                            have: sel.len(),
                            what: "SH_delivery_system body",
                        });
                    }
                    let b0 = sel[pos];
                    let b1 = sel[pos + 1];
                    let b2 = sel[pos + 2];
                    let b3 = sel[pos + 3];
                    let common_multiplier = b0 >> 2;
                    let nof_late_taps = ((b0 & 0x03) << 4) | (b1 >> 4);
                    let nof_slices = ((b1 & 0x0F) << 2) | (b2 >> 6);
                    let slice_distance = ((b2 & 0x3F) << 2) | (b3 >> 6);
                    let non_late_increments = b3 & 0x3F;
                    pos += 4;
                    Some(ShInterleaver::Type0 {
                        common_multiplier,
                        nof_late_taps,
                        nof_slices,
                        slice_distance,
                        non_late_increments,
                    })
                } else {
                    if sel.len() - pos < 1 {
                        return Err(Error::BufferTooShort {
                            need: pos + 1,
                            have: sel.len(),
                            what: "SH_delivery_system body",
                        });
                    }
                    let common_multiplier = sel[pos] >> 2;
                    pos += 1;
                    Some(ShInterleaver::Type1 { common_multiplier })
                }
            } else {
                None
            };

            modulations.push(ShModulation {
                modulation,
                interleaver,
            });
        }
        Ok(ShDeliverySystem {
            diversity_mode,
            modulations,
        })
    }
}

impl Serialize for ShDeliverySystem {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self
            .modulations
            .iter()
            .map(|m| {
                3 + match &m.interleaver {
                    None => 0,
                    Some(ShInterleaver::Type0 { .. }) => 4,
                    Some(ShInterleaver::Type1 { .. }) => 1,
                }
            })
            .sum::<usize>()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = (self.diversity_mode.to_u8() << 4) | 0x0F;
        let mut p = 1;
        for m in &self.modulations {
            let modulation_type_bit = matches!(m.modulation, ShModulationMode::Ofdm { .. }) as u8;
            let interleaver_presence_bit = m.interleaver.is_some() as u8;
            let interleaver_type_bit =
                matches!(m.interleaver, Some(ShInterleaver::Type1 { .. })) as u8;
            buf[p] = (modulation_type_bit << 7)
                | (interleaver_presence_bit << 6)
                | (interleaver_type_bit << 5)
                | 0x1F;
            p += 1;

            match &m.modulation {
                ShModulationMode::Tdm {
                    polarization,
                    roll_off,
                    modulation_mode,
                    code_rate,
                    symbol_rate,
                } => {
                    let cr = code_rate.to_u8();
                    buf[p] = (polarization.to_u8() << 6)
                        | ((roll_off.to_u8() & 0x03) << 4)
                        | ((modulation_mode.to_u8() & 0x03) << 2)
                        | ((cr >> 2) & 0x03);
                    buf[p + 1] = ((cr & 0x03) << 6) | ((symbol_rate & 0x1F) << 1) | 0x01;
                }
                ShModulationMode::Ofdm {
                    bandwidth,
                    priority,
                    constellation_and_hierarchy,
                    code_rate,
                    guard_interval,
                    transmission_mode,
                    common_frequency,
                } => {
                    let cr = code_rate.to_u8();
                    buf[p] = (bandwidth.to_u8() << 5)
                        | (u8::from(*priority) << 4)
                        | ((constellation_and_hierarchy.to_u8() & 0x07) << 1)
                        | ((cr >> 3) & 0x01);
                    buf[p + 1] = ((cr & 0x07) << 5)
                        | ((guard_interval.to_u8() & 0x03) << 3)
                        | ((transmission_mode.to_u8() & 0x03) << 1)
                        | u8::from(*common_frequency);
                }
            }
            p += 2;

            match &m.interleaver {
                Some(ShInterleaver::Type0 {
                    common_multiplier,
                    nof_late_taps,
                    nof_slices,
                    slice_distance,
                    non_late_increments,
                }) => {
                    let cm = common_multiplier & 0x3F;
                    let lt = nof_late_taps & 0x3F;
                    let ns = nof_slices & 0x3F;
                    let sd = slice_distance;
                    let nli = non_late_increments & 0x3F;
                    buf[p] = (cm << 2) | (lt >> 4);
                    buf[p + 1] = ((lt & 0x0F) << 4) | (ns >> 2);
                    buf[p + 2] = ((ns & 0x03) << 6) | (sd >> 2);
                    buf[p + 3] = ((sd & 0x03) << 6) | nli;
                    p += 4;
                }
                Some(ShInterleaver::Type1 { common_multiplier }) => {
                    buf[p] = ((common_multiplier & 0x3F) << 2) | 0x03;
                    p += 1;
                }
                None => {}
            }
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn sh_diversity_mode_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShDiversityMode::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn sh_polarization_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShPolarization::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn sh_roll_off_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShRollOff::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn sh_modulation_mode_type_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShModulationModeType::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn sh_code_rate_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShCodeRate::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn sh_bandwidth_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShBandwidth::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn sh_constellation_and_hierarchy_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShConstellationAndHierarchy::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn sh_guard_interval_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShGuardInterval::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn sh_transmission_mode_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShTransmissionMode::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn sh_enum_names() {
        assert_eq!(ShPolarization::LinearHorizontal.name(), "linear horizontal");
        assert_eq!(ShPolarization::CircularRight.name(), "circular right");
        assert_eq!(ShRollOff::Alpha035.name(), "α = 0.35");
        assert_eq!(ShModulationModeType::Psk8.name(), "8PSK");
        assert_eq!(ShBandwidth::Mhz1_7.name(), "1.7 MHz");
        assert_eq!(
            ShConstellationAndHierarchy::Qam16Alpha2.name(),
            "16QAM, α = 2"
        );
        assert_eq!(ShGuardInterval::G1_8.name(), "1/8");
        assert_eq!(ShTransmissionMode::Mode4k.name(), "4k mode");
        assert_eq!(ShPolarization::Reserved(5).name(), "reserved");
    }

    #[test]
    fn parse_sh_tdm_no_interleaver() {
        let sel = [0xD0, 0x00, 0x9E, 0xAA];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ShDeliverySystem));
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, ShDiversityMode::FecAtLink);
                assert_eq!(b.modulations.len(), 1);
                let m = &b.modulations[0];
                assert!(m.interleaver.is_none());
                match &m.modulation {
                    ShModulationMode::Tdm {
                        polarization,
                        roll_off,
                        modulation_mode,
                        code_rate,
                        symbol_rate,
                    } => {
                        assert_eq!(*polarization, ShPolarization::CircularLeft);
                        assert_eq!(*roll_off, ShRollOff::Alpha025);
                        assert_eq!(*modulation_mode, ShModulationModeType::Reserved(3));
                        assert_eq!(*code_rate, ShCodeRate::Rate2_3Standard);
                        assert_eq!(*symbol_rate, 21);
                    }
                    other => panic!("expected Tdm, got {other:?}"),
                }
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_ofdm_interleaver_type1() {
        let sel = [0x50, 0xE0, 0x35, 0x7D, 0x54];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, ShDiversityMode::Reserved(5));
                assert_eq!(b.modulations.len(), 1);
                let m = &b.modulations[0];
                match &m.modulation {
                    ShModulationMode::Ofdm {
                        bandwidth,
                        priority,
                        constellation_and_hierarchy,
                        code_rate,
                        guard_interval,
                        transmission_mode,
                        common_frequency,
                    } => {
                        assert_eq!(*bandwidth, ShBandwidth::Mhz7);
                        assert!(*priority);
                        assert_eq!(
                            *constellation_and_hierarchy,
                            ShConstellationAndHierarchy::Qam16Alpha1
                        );
                        assert_eq!(*code_rate, ShCodeRate::Rate2_3Complementary);
                        assert_eq!(*guard_interval, ShGuardInterval::G1_4);
                        assert_eq!(*transmission_mode, ShTransmissionMode::Mode4k);
                        assert!(*common_frequency);
                    }
                    other => panic!("expected Ofdm, got {other:?}"),
                }
                match &m.interleaver {
                    Some(ShInterleaver::Type1 { common_multiplier }) => {
                        assert_eq!(*common_multiplier, 21);
                    }
                    other => panic!("expected Type1 interleaver, got {other:?}"),
                }
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_tdm_interleaver_type0() {
        let sel = [0x80, 0x40, 0x35, 0x54, 0x29, 0x47, 0x99, 0x28];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, ShDiversityMode::PaTsOnly);
                assert_eq!(b.modulations.len(), 1);
                let m = &b.modulations[0];
                match &m.modulation {
                    ShModulationMode::Tdm {
                        polarization,
                        roll_off,
                        modulation_mode,
                        code_rate,
                        symbol_rate,
                    } => {
                        assert_eq!(*polarization, ShPolarization::LinearHorizontal);
                        assert_eq!(*roll_off, ShRollOff::Reserved(3));
                        assert_eq!(*modulation_mode, ShModulationModeType::Psk8);
                        assert_eq!(*code_rate, ShCodeRate::Rate1_3Complementary);
                        assert_eq!(*symbol_rate, 10);
                    }
                    other => panic!("expected Tdm, got {other:?}"),
                }
                match &m.interleaver {
                    Some(ShInterleaver::Type0 {
                        common_multiplier,
                        nof_late_taps,
                        nof_slices,
                        slice_distance,
                        non_late_increments,
                    }) => {
                        assert_eq!(*common_multiplier, 10);
                        assert_eq!(*nof_late_taps, 20);
                        assert_eq!(*nof_slices, 30);
                        assert_eq!(*slice_distance, 100);
                        assert_eq!(*non_late_increments, 40);
                    }
                    other => panic!("expected Type0 interleaver, got {other:?}"),
                }
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_two_entries_mixed() {
        let sel = [
            0xD0, 0x00, 0x9E, 0xAA, 0xC0, 0x8B, 0x2A, 0x3D, 0x98, 0xCC, 0xB7,
        ];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, ShDiversityMode::FecAtLink);
                assert_eq!(b.modulations.len(), 2);
                let m0 = &b.modulations[0];
                assert!(matches!(m0.modulation, ShModulationMode::Tdm { .. }));
                assert!(m0.interleaver.is_none());
                let m1 = &b.modulations[1];
                assert!(matches!(m1.modulation, ShModulationMode::Ofdm { .. }));
                match &m1.modulation {
                    ShModulationMode::Ofdm {
                        bandwidth,
                        priority,
                        constellation_and_hierarchy,
                        code_rate,
                        ..
                    } => {
                        assert_eq!(*bandwidth, ShBandwidth::Mhz1_7);
                        assert!(!priority);
                        assert_eq!(
                            *constellation_and_hierarchy,
                            ShConstellationAndHierarchy::Reserved(5)
                        );
                        assert_eq!(*code_rate, ShCodeRate::Rate1_2Complementary);
                    }
                    _ => unreachable!(),
                }
                assert!(matches!(m1.interleaver, Some(ShInterleaver::Type0 { .. })));
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_rejects_partial_entry() {
        let sel = [0xD0, 0x00, 0x9E, 0xAA, 0x00];
        let bytes = wrap(0x05, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_sh_single_diversity_byte() {
        let sel = [0xD0];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, ShDiversityMode::FecAtLink);
                assert!(b.modulations.is_empty());
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_rejects_empty_selector() {
        let bytes = wrap(0x05, &[]);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn tsduck_sh_round_trips() {
        let vectors: [(&str, u8); 2] =
            [("7f02055f", 0x05), ("7f0d05afff94ac175f68831d8d99ad", 0x05)];
        for (hex, _ext) in vectors {
            let bytes = from_hex(hex);
            let d =
                ExtensionDescriptor::parse(&bytes).unwrap_or_else(|e| panic!("parse {hex}: {e:?}"));
            let mut out = vec![0u8; d.serialized_len()];
            let n = d.serialize_into(&mut out).unwrap();
            assert_eq!(out[..n], bytes[..], "byte-exact re-serialize for {hex}");
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_sh_delivery_system() {
        let d = ExtensionDescriptor {
            tag_extension: 0x05,
            body: ExtensionBody::ShDeliverySystem(ShDeliverySystem {
                diversity_mode: ShDiversityMode::FecAtLink,
                modulations: vec![ShModulation {
                    modulation: ShModulationMode::Ofdm {
                        bandwidth: ShBandwidth::Mhz7,
                        priority: true,
                        constellation_and_hierarchy: ShConstellationAndHierarchy::Qam16Alpha1,
                        code_rate: ShCodeRate::Reserved(11),
                        guard_interval: ShGuardInterval::G1_4,
                        transmission_mode: ShTransmissionMode::Mode4k,
                        common_frequency: true,
                    },
                    interleaver: Some(ShInterleaver::Type1 {
                        common_multiplier: 21,
                    }),
                }],
            }),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"tag_extension\":5"));
        assert!(json.contains("\"shDeliverySystem\""));
    }
}
