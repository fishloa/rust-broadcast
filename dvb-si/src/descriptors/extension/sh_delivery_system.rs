//! SH Delivery System Descriptor — ETSI EN 300 468 §6.4.6.2 (tag_extension 0x05).
use super::*;

impl super::sealed::Sealed for ShDeliverySystem {}
impl ExtensionBodyDef for ShDeliverySystem {
    const TAG_EXTENSION: u8 = 0x05;
    const NAME: &'static str = "SH_DELIVERY_SYSTEM";
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_sh_tdm_no_interleaver() {
        // diversity_mode=0x0D (1101), one TDM entry, no interleaver.
        // TDM: polarization=2, roll_off=1, modulation_mode=3,
        //      code_rate=10 (1010), symbol_rate=21 (10101).
        // flags: mod_type=0, inter_pres=0, inter_type=0 -> 0x00
        // mb0 = (2<<6)|(1<<4)|(3<<2)|((10>>2)&3) = 0x80|0x10|0x0C|0x02 = 0x9E
        // mb1 = ((10&3)<<6)|(21<<1) = (2<<6)|42 = 0x80|0x2A = 0xAA
        let sel = [0xD0, 0x00, 0x9E, 0xAA];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ShDeliverySystem));
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, 0x0D);
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
                        assert_eq!(*polarization, 2);
                        assert_eq!(*roll_off, 1);
                        assert_eq!(*modulation_mode, 3);
                        assert_eq!(*code_rate, 10);
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
        // diversity_mode=0x05, one OFDM entry, interleaver Type1.
        // OFDM: bw=1, pri=true, cah=2, cr=11(0x0B), gi=3, tm=2, cf=true
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
                assert_eq!(b.diversity_mode, 0x05);
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
                        assert_eq!(*bandwidth, 1);
                        assert!(*priority);
                        assert_eq!(*constellation_and_hierarchy, 2);
                        assert_eq!(*code_rate, 11);
                        assert_eq!(*guard_interval, 3);
                        assert_eq!(*transmission_mode, 2);
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
        // TDM: pol=0, ro=3, mm=1, cr=5, sr=10
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
                assert_eq!(b.diversity_mode, 0x08);
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
                        assert_eq!(*polarization, 0);
                        assert_eq!(*roll_off, 3);
                        assert_eq!(*modulation_mode, 1);
                        assert_eq!(*code_rate, 5);
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
        // Entry 2: OFDM bw=4 pri=false cah=5 cr=9 gi=1 tm=1 cf=false,
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
                assert_eq!(b.diversity_mode, 0x0D);
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
                        assert_eq!(*bandwidth, 4);
                        assert!(!priority);
                        assert_eq!(*constellation_and_hierarchy, 5);
                        assert_eq!(*code_rate, 9);
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
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
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
                assert_eq!(b.diversity_mode, 0x0D);
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
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_sh_delivery_system() {
        let d = ExtensionDescriptor {
            tag_extension: 0x05,
            body: ExtensionBody::ShDeliverySystem(ShDeliverySystem {
                diversity_mode: 0x0D,
                modulations: vec![ShModulation {
                    modulation: ShModulationMode::Ofdm {
                        bandwidth: 1,
                        priority: true,
                        constellation_and_hierarchy: 2,
                        code_rate: 11,
                        guard_interval: 3,
                        transmission_mode: 2,
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
