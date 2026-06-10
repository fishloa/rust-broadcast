use super::*;

impl ExtensionBodyDef for TargetRegion {
    const TAG_EXTENSION: u8 = 0x09;
    const NAME: &'static str = "TARGET_REGION";
}

// ===========================================================================
//  Section 0x09 — target_region_descriptor (Table 156, §6.4.12)
// ---------------------------------------------------------------------------
//  Leading country_code(24) then a region loop whose entries are
//  region_depth-conditional; the loop is unfolded into typed entries.
// ===========================================================================
/// target_region body (Table 156, §6.4.12). The region loop is unfolded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TargetRegion {
    /// Leading country_code(24).
    pub country_code: LangCode,
    /// Region entries (the loop).
    pub regions: Vec<TargetRegionEntry>,
}

/// An entry in the target_region_descriptor region loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TargetRegionEntry {
    /// Per-entry country_code(24), present iff country_code_flag==1.
    pub country_code: Option<LangCode>,
    /// region_depth and its region codes.
    pub region_codes: RegionCodes,
}

/// The `region_depth` field and its associated region codes (Table 156, §6.4.12).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum RegionCodes {
    /// region_depth == 0.
    None,
    /// region_depth == 1.
    Primary {
        /// primary_region_code(8).
        primary_region_code: u8,
    },
    /// region_depth == 2.
    PrimarySecondary {
        /// primary_region_code(8).
        primary_region_code: u8,
        /// secondary_region_code(8).
        secondary_region_code: u8,
    },
    /// region_depth == 3.
    Full {
        /// primary_region_code(8).
        primary_region_code: u8,
        /// secondary_region_code(8).
        secondary_region_code: u8,
        /// tertiary_region_code(16).
        tertiary_region_code: u16,
    },
}

impl<'a> Parse<'a> for TargetRegion {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < ISO_639_LEN {
            return Err(invalid("target_region: country_code truncated"));
        }
        let country_code = LangCode([sel[0], sel[1], sel[2]]);
        let mut regions = Vec::new();
        let mut pos = ISO_639_LEN;
        while pos < sel.len() {
            let flags = sel[pos];
            pos += 1;
            let country_code_flag = (flags >> 2) & 1;
            let region_depth = flags & 0x03;
            let country_code = if country_code_flag == 1 {
                if pos + ISO_639_LEN > sel.len() {
                    return Err(invalid("target_region: region entry overruns body"));
                }
                let cc = LangCode([sel[pos], sel[pos + 1], sel[pos + 2]]);
                pos += ISO_639_LEN;
                Some(cc)
            } else {
                None
            };
            let region_codes = match region_depth {
                0 => RegionCodes::None,
                1 => {
                    if pos >= sel.len() {
                        return Err(invalid("target_region: region entry overruns body"));
                    }
                    let primary = sel[pos];
                    pos += 1;
                    RegionCodes::Primary {
                        primary_region_code: primary,
                    }
                }
                2 => {
                    if pos + 1 >= sel.len() {
                        return Err(invalid("target_region: region entry overruns body"));
                    }
                    let primary = sel[pos];
                    let secondary = sel[pos + 1];
                    pos += 2;
                    RegionCodes::PrimarySecondary {
                        primary_region_code: primary,
                        secondary_region_code: secondary,
                    }
                }
                3 => {
                    if pos + 3 >= sel.len() {
                        return Err(invalid("target_region: region entry overruns body"));
                    }
                    let primary = sel[pos];
                    let secondary = sel[pos + 1];
                    let tertiary = u16::from_be_bytes([sel[pos + 2], sel[pos + 3]]);
                    pos += 4;
                    RegionCodes::Full {
                        primary_region_code: primary,
                        secondary_region_code: secondary,
                        tertiary_region_code: tertiary,
                    }
                }
                _ => return Err(invalid("target_region: invalid region_depth")),
            };
            regions.push(TargetRegionEntry {
                country_code,
                region_codes,
            });
        }
        Ok(TargetRegion {
            country_code,
            regions,
        })
    }
}

impl Serialize for TargetRegion {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        ISO_639_LEN
            + self
                .regions
                .iter()
                .map(|r| {
                    1 + if r.country_code.is_some() {
                        ISO_639_LEN
                    } else {
                        0
                    } + match &r.region_codes {
                        RegionCodes::None => 0,
                        RegionCodes::Primary { .. } => 1,
                        RegionCodes::PrimarySecondary { .. } => 2,
                        RegionCodes::Full { .. } => 4,
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
        buf[..ISO_639_LEN].copy_from_slice(&self.country_code.0);
        let mut pos = ISO_639_LEN;
        for region in &self.regions {
            let depth = match &region.region_codes {
                RegionCodes::None => 0u8,
                RegionCodes::Primary { .. } => 1,
                RegionCodes::PrimarySecondary { .. } => 2,
                RegionCodes::Full { .. } => 3,
            };
            buf[pos] = 0xF8 | ((region.country_code.is_some() as u8) << 2) | depth;
            pos += 1;
            if let Some(cc) = &region.country_code {
                buf[pos..pos + ISO_639_LEN].copy_from_slice(&cc.0);
                pos += ISO_639_LEN;
            }
            match &region.region_codes {
                RegionCodes::None => {}
                RegionCodes::Primary {
                    primary_region_code,
                } => {
                    buf[pos] = *primary_region_code;
                    pos += 1;
                }
                RegionCodes::PrimarySecondary {
                    primary_region_code,
                    secondary_region_code,
                } => {
                    buf[pos] = *primary_region_code;
                    buf[pos + 1] = *secondary_region_code;
                    pos += 2;
                }
                RegionCodes::Full {
                    primary_region_code,
                    secondary_region_code,
                    tertiary_region_code,
                } => {
                    buf[pos] = *primary_region_code;
                    buf[pos + 1] = *secondary_region_code;
                    buf[pos + 2..pos + 4].copy_from_slice(&tertiary_region_code.to_be_bytes());
                    pos += 4;
                }
            }
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};
    use crate::text::LangCode;

    #[test]
    fn parse_target_region_structured() {
        let sel = [b'g', b'b', b'r', 0xF9, 0x12];
        let bytes = wrap(0x09, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegion(b) => {
                assert_eq!(b.country_code, LangCode(*b"gbr"));
                assert_eq!(b.regions.len(), 1);
                assert_eq!(b.regions[0].country_code, None);
                assert_eq!(
                    b.regions[0].region_codes,
                    RegionCodes::Primary {
                        primary_region_code: 0x12
                    }
                );
            }
            other => panic!("expected TargetRegion, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn target_region_tsduck_empty() {
        let bytes = from_hex("7f0409666f6f");
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegion(b) => {
                assert_eq!(b.country_code, LangCode(*b"foo"));
                assert!(b.regions.is_empty());
            }
            other => panic!("expected TargetRegion, got {other:?}"),
        }
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    #[test]
    fn target_region_tsduck_full() {
        let bytes = from_hex("7f1509626172f8fd666f6f12fa3456ff616263789abcde");
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegion(b) => {
                assert_eq!(b.country_code, LangCode(*b"bar"));
                assert_eq!(b.regions.len(), 4);
                // [0] cc=None, RegionCodes::None
                assert_eq!(b.regions[0].country_code, None);
                assert_eq!(b.regions[0].region_codes, RegionCodes::None);
                // [1] cc=Some("foo"), Primary{primary:0x12}
                assert_eq!(b.regions[1].country_code, Some(LangCode(*b"foo")));
                assert_eq!(
                    b.regions[1].region_codes,
                    RegionCodes::Primary {
                        primary_region_code: 0x12
                    }
                );
                // [2] cc=None, PrimarySecondary{primary:0x34,secondary:0x56}
                assert_eq!(b.regions[2].country_code, None);
                assert_eq!(
                    b.regions[2].region_codes,
                    RegionCodes::PrimarySecondary {
                        primary_region_code: 0x34,
                        secondary_region_code: 0x56,
                    }
                );
                // [3] cc=Some("abc"), Full{primary:0x78,secondary:0x9A,tertiary:0xBCDE}
                assert_eq!(b.regions[3].country_code, Some(LangCode(*b"abc")));
                assert_eq!(
                    b.regions[3].region_codes,
                    RegionCodes::Full {
                        primary_region_code: 0x78,
                        secondary_region_code: 0x9A,
                        tertiary_region_code: 0xBCDE,
                    }
                );
            }
            other => panic!("expected TargetRegion, got {other:?}"),
        }
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }
}
