use super::*;

impl ExtensionBodyDef for ShDeliverySystem {
    const TAG_EXTENSION: u8 = 0x05;
    const NAME: &'static str = "SH_DELIVERY_SYSTEM";
}

// ===========================================================================
//  Section 0x05 — SH_delivery_system_descriptor (Table 119, §6.4.6.2)
// ---------------------------------------------------------------------------
//  diversity_mode(4) then a variable modulation loop whose entries carry
//  conditional TDM/OFDM modulation parameters (Tables 120-131)
//  and optional interleaver blocks (Table 122). Fully typed.
// ===========================================================================

/// SH_delivery_system body (Table 119, §6.4.6.2). The modulation loop is
/// unfolded; `modulation_type` (Table 121) selects Tdm/Ofdm,
/// `interleaver_presence` (Table 122) gates the interleaver, and
/// `interleaver_type` selects its layout. Diversity mode: Table 120.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ShDeliverySystem {
    /// `diversity_mode` (4 bits) — Table 120.
    pub diversity_mode: u8,
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
        polarization: u8,
        /// roll_off (2 bits) — Table 124.
        roll_off: u8,
        /// modulation_mode (2 bits) — Table 125.
        modulation_mode: u8,
        /// code_rate (4 bits) — Table 126.
        code_rate: u8,
        /// symbol_rate (5 bits) — Table 127.
        symbol_rate: u8,
    },
    /// `modulation_type == 1` — OFDM.
    Ofdm {
        /// bandwidth (3 bits) — Table 128.
        bandwidth: u8,
        /// priority (1 bit) — Table 129.
        priority: bool,
        /// constellation_and_hierarchy (3 bits) — Table 130.
        constellation_and_hierarchy: u8,
        /// code_rate (4 bits) — Table 126.
        code_rate: u8,
        /// guard_interval (2 bits) — Table 131.
        guard_interval: u8,
        /// transmission_mode (2 bits) — Table 132.
        transmission_mode: u8,
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

impl<'a> Parse<'a> for ShDeliverySystem {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(invalid("SH_delivery_system: diversity_mode byte missing"));
        }
        let diversity_mode = sel[0] >> 4;
        let mut pos = 1;
        let mut modulations = Vec::new();
        while pos < sel.len() {
            // Need flags byte + 2 modulation bytes
            if sel.len() - pos < 3 {
                return Err(invalid(
                    "SH_delivery_system: modulation entry overruns body",
                ));
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
                let polarization = mb0 >> 6;
                let roll_off = (mb0 >> 4) & 0x03;
                let modulation_mode = (mb0 >> 2) & 0x03;
                let code_rate = ((mb0 & 0x03) << 2) | (mb1 >> 6);
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
                let bandwidth = mb0 >> 5;
                let priority = ((mb0 >> 4) & 0x01) != 0;
                let constellation_and_hierarchy = (mb0 >> 1) & 0x07;
                let code_rate = ((mb0 & 0x01) << 3) | (mb1 >> 5);
                let guard_interval = (mb1 >> 3) & 0x03;
                let transmission_mode = (mb1 >> 1) & 0x03;
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
                        return Err(invalid(
                            "SH_delivery_system: interleaver block overruns body",
                        ));
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
                        return Err(invalid(
                            "SH_delivery_system: interleaver block overruns body",
                        ));
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
        buf[0] = (self.diversity_mode << 4) | 0x0F;
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
                    buf[p] = (polarization << 6)
                        | ((roll_off & 0x03) << 4)
                        | ((modulation_mode & 0x03) << 2)
                        | ((code_rate >> 2) & 0x03);
                    // code_rate low 2 | symbol_rate(5) | reserved_future_use(1)=1
                    buf[p + 1] = ((code_rate & 0x03) << 6) | ((symbol_rate & 0x1F) << 1) | 0x01;
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
                    buf[p] = (bandwidth << 5)
                        | (u8::from(*priority) << 4)
                        | ((constellation_and_hierarchy & 0x07) << 1)
                        | ((code_rate >> 3) & 0x01);
                    buf[p + 1] = ((code_rate & 0x07) << 5)
                        | ((guard_interval & 0x03) << 3)
                        | ((transmission_mode & 0x03) << 1)
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
