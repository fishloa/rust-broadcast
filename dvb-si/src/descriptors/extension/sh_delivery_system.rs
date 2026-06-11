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
pub enum ShPolarization {
    /// Linear horizontal.
    LinearHorizontal,
    /// Linear vertical.
    LinearVertical,
    /// Circular left.
    CircularLeft,
    /// Circular right.
    CircularRight,
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

/// Guard interval for OFDM — ETSI EN 300 468 Table 131 (2 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ShGuardInterval {
    /// 1/32.
    G1_32,
    /// 1/16.
    G1_16,
    /// 1/8.
    G1_8,
    /// 1/4.
    G1_4,
}

/// Transmission mode for OFDM — ETSI EN 300 468 Table 132 (2 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ShTransmissionMode {
    /// 1k mode.
    Mode1k,
    /// 2k mode.
    Mode2k,
    /// 4k mode.
    Mode4k,
    /// 8k mode.
    Mode8k,
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
            // Need flags byte + 2 modulation bytes
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
                // TDM
                let pol_raw = mb0 >> 6;
                let polarization = match pol_raw {
                    0 => ShPolarization::LinearHorizontal,
                    1 => ShPolarization::LinearVertical,
                    2 => ShPolarization::CircularLeft,
                    _ => ShPolarization::CircularRight,
                };
                let ro_raw = (mb0 >> 4) & 0x03;
                let roll_off = match ro_raw {
                    0 => ShRollOff::Alpha035,
                    1 => ShRollOff::Alpha025,
                    2 => ShRollOff::Alpha015,
                    v => ShRollOff::Reserved(v),
                };
                let mm_raw = (mb0 >> 2) & 0x03;
                let modulation_mode = match mm_raw {
                    0 => ShModulationModeType::Qpsk,
                    1 => ShModulationModeType::Psk8,
                    2 => ShModulationModeType::Apsk16,
                    v => ShModulationModeType::Reserved(v),
                };
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
                // OFDM
                let bw_raw = mb0 >> 5;
                let bandwidth = match bw_raw {
                    0 => ShBandwidth::Mhz8,
                    1 => ShBandwidth::Mhz7,
                    2 => ShBandwidth::Mhz6,
                    3 => ShBandwidth::Mhz5,
                    4 => ShBandwidth::Mhz1_7,
                    v => ShBandwidth::Reserved(v),
                };
                let priority = ((mb0 >> 4) & 0x01) != 0;
                let cah_raw = (mb0 >> 1) & 0x07;
                let constellation_and_hierarchy = match cah_raw {
                    0 => ShConstellationAndHierarchy::Qpsk,
                    1 => ShConstellationAndHierarchy::Qam16NonHierarchical,
                    2 => ShConstellationAndHierarchy::Qam16Alpha1,
                    3 => ShConstellationAndHierarchy::Qam16Alpha2,
                    4 => ShConstellationAndHierarchy::Qam16Alpha3,
                    v => ShConstellationAndHierarchy::Reserved(v),
                };
                let code_rate_raw = ((mb0 & 0x01) << 3) | (mb1 >> 5);
                let code_rate = ShCodeRate::from_u8(code_rate_raw);
                let gi_raw = (mb1 >> 3) & 0x03;
                let guard_interval = match gi_raw {
                    0 => ShGuardInterval::G1_32,
                    1 => ShGuardInterval::G1_16,
                    2 => ShGuardInterval::G1_8,
                    _ => ShGuardInterval::G1_4,
                };
                let tm_raw = (mb1 >> 1) & 0x03;
                let transmission_mode = match tm_raw {
                    0 => ShTransmissionMode::Mode1k,
                    1 => ShTransmissionMode::Mode2k,
                    2 => ShTransmissionMode::Mode4k,
                    _ => ShTransmissionMode::Mode8k,
                };
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
                    // 4-byte Type0 interleaver block
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
                    // 1-byte Type1 interleaver block
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
        // diversity_mode(4) | reserved_future_use(4)=1
        buf[0] = (self.diversity_mode.to_u8() << 4) | 0x0F;
        let mut p = 1;
        for m in &self.modulations {
            let modulation_type_bit = matches!(m.modulation, ShModulationMode::Ofdm { .. }) as u8;
            let interleaver_presence_bit = m.interleaver.is_some() as u8;
            let interleaver_type_bit =
                matches!(m.interleaver, Some(ShInterleaver::Type1 { .. })) as u8;
            // modulation_type(1) | interleaver_presence(1) | interleaver_type(1)
            //   | reserved_future_use(5)=1
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
                    let pol = match polarization {
                        ShPolarization::LinearHorizontal => 0,
                        ShPolarization::LinearVertical => 1,
                        ShPolarization::CircularLeft => 2,
                        ShPolarization::CircularRight => 3,
                    };
                    let ro = match roll_off {
                        ShRollOff::Alpha035 => 0,
                        ShRollOff::Alpha025 => 1,
                        ShRollOff::Alpha015 => 2,
                        ShRollOff::Reserved(v) => v & 0x03,
                    };
                    let mm = match modulation_mode {
                        ShModulationModeType::Qpsk => 0,
                        ShModulationModeType::Psk8 => 1,
                        ShModulationModeType::Apsk16 => 2,
                        ShModulationModeType::Reserved(v) => v & 0x03,
                    };
                    let cr = code_rate.to_u8();
                    buf[p] =
                        (pol << 6) | ((ro & 0x03) << 4) | ((mm & 0x03) << 2) | ((cr >> 2) & 0x03);
                    // code_rate low 2 | symbol_rate(5) | reserved_future_use(1)=1
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
                    let bw = match bandwidth {
                        ShBandwidth::Mhz8 => 0,
                        ShBandwidth::Mhz7 => 1,
                        ShBandwidth::Mhz6 => 2,
                        ShBandwidth::Mhz5 => 3,
                        ShBandwidth::Mhz1_7 => 4,
                        ShBandwidth::Reserved(v) => v & 0x07,
                    };
                    let cah = match constellation_and_hierarchy {
                        ShConstellationAndHierarchy::Qpsk => 0,
                        ShConstellationAndHierarchy::Qam16NonHierarchical => 1,
                        ShConstellationAndHierarchy::Qam16Alpha1 => 2,
                        ShConstellationAndHierarchy::Qam16Alpha2 => 3,
                        ShConstellationAndHierarchy::Qam16Alpha3 => 4,
                        ShConstellationAndHierarchy::Reserved(v) => v & 0x07,
                    };
                    let gi = match guard_interval {
                        ShGuardInterval::G1_32 => 0,
                        ShGuardInterval::G1_16 => 1,
                        ShGuardInterval::G1_8 => 2,
                        ShGuardInterval::G1_4 => 3,
                    };
                    let tm = match transmission_mode {
                        ShTransmissionMode::Mode1k => 0,
                        ShTransmissionMode::Mode2k => 1,
                        ShTransmissionMode::Mode4k => 2,
                        ShTransmissionMode::Mode8k => 3,
                    };
                    let cr = code_rate.to_u8();
                    buf[p] = (bw << 5)
                        | (u8::from(*priority) << 4)
                        | ((cah & 0x07) << 1)
                        | ((cr >> 3) & 0x01);
                    buf[p + 1] = ((cr & 0x07) << 5)
                        | ((gi & 0x03) << 3)
                        | ((tm & 0x03) << 1)
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
                    // common_multiplier(6) | reserved_future_use(2)=1
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
    fn sh_code_rate_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ShCodeRate::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn parse_sh_tdm_no_interleaver() {
        // diversity_mode=0x0D (1101), one TDM entry, no interleaver.
        // TDM: polarization=2 (circular-left), roll_off=1 (0.25), modulation_mode=3 (reserved),
        //      code_rate=10, symbol_rate=21.
        // flags: mod_type=0, inter_pres=0, inter_type=0 -> 0x00
        // mb0 = (2<<6)|(1<<4)|(3<<2)|((10>>2)&3) = 0x80|0x10|0x0C|0x02 = 0x9E
        // mb1 = ((10&3)<<6)|(21<<1) = (2<<6)|42 = 0x80|0x2A = 0xAA
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
        // diversity_mode=0x05 (reserved), one OFDM entry, interleaver Type1.
        // OFDM: bw=1(7MHz), pri=true, cah=2(16QAM alpha=1), cr=11, gi=3(1/4), tm=2(4k), cf=true
        // Interleaver Type1: cm=21(0x15)
        // flags: mod_type=1, inter_pres=1, inter_type=1 -> 0xE0
        // mb0 = (1<<5)|(1<<4)|(2<<1)|((11>>3)&1) = 0x20|0x10|0x04|0x01 = 0x35
        // mb1 = ((11&7)<<5)|(3<<3)|(2<<1)|1 = 0x60|0x18|0x04|0x01 = 0x7D
        // Type1 byte: (21<<2) = 0x54
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
        // diversity_mode=0x08, one TDM entry, interleaver Type0.
        // TDM: pol=0, ro=3(reserved), mm=1(8PSK), cr=5, sr=10
        // Type0: cm=10, lt=20, ns=30, sd=100, nli=40
        // flags: mod_type=0, inter_pres=1, inter_type=0 -> 0x40
        // mb0 = (0<<6)|(3<<4)|(1<<2)|((5>>2)&3) = 0x30|0x04|0x01 = 0x35
        // mb1 = ((5&3)<<6)|(10<<1) = (1<<6)|20 = 0x40|0x14 = 0x54
        // Type0 byte0: (10<<2)|(20>>4) = 40|1 = 0x29
        // Type0 byte1: ((20&15)<<4)|(30>>2) = (4<<4)|7 = 0x47
        // Type0 byte2: ((30&3)<<6)|(100>>2) = (2<<6)|25 = 0x99
        // Type0 byte3: ((100&3)<<6)|40 = 0x28
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
        // diversity_mode=0x0D
        // Entry 1: TDM (same as test 1), no interleaver.
        // Entry 2: OFDM bw=4(1.7MHz) pri=false cah=5(reserved) cr=9 gi=1(1/16) tm=1(2k) cf=false,
        //          Type0 interleaver cm=15 lt=25 ns=35 sd=50 nli=55
        // Entry1: flags=0x00, mb0=0x9E, mb1=0xAA
        // Entry2 flags: 0xC0 (mod=1, pres=1, type=0)
        // OFDM mb0: (4<<5)|(0<<4)|(5<<1)|((9>>3)&1) = 0x80|0x0A|0x01 = 0x8B
        // OFDM mb1: ((9&7)<<5)|(1<<3)|(1<<1)|0 = 0x20|0x08|0x02 = 0x2A
        // Type0 byte0: (15<<2)|(25>>4) = 60|1 = 0x3D
        // Type0 byte1: ((25&15)<<4)|(35>>2) = (9<<4)|8 = 0x98
        // Type0 byte2: ((35&3)<<6)|(50>>2) = (3<<6)|12 = 0xCC
        // Type0 byte3: ((50&3)<<6)|55 = (2<<6)|55 = 0xB7
        let sel = [
            0xD0, 0x00, 0x9E, 0xAA, 0xC0, 0x8B, 0x2A, 0x3D, 0x98, 0xCC, 0xB7,
        ];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, ShDiversityMode::FecAtLink);
                assert_eq!(b.modulations.len(), 2);
                // Entry 1
                let m0 = &b.modulations[0];
                assert!(matches!(m0.modulation, ShModulationMode::Tdm { .. }));
                assert!(m0.interleaver.is_none());
                // Entry 2
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
        // Complete entry followed by a lone flags byte with no modulation block
        let sel = [0xD0, 0x00, 0x9E, 0xAA, 0x00];
        let bytes = wrap(0x05, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_sh_single_diversity_byte() {
        // Only diversity_mode byte, no modulations.
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
        // From the tsduck reference test vectors in mod.rs
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
