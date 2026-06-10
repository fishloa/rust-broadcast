use super::*;

impl super::sealed::Sealed for TargetRegionName<'_> {}
impl ExtensionBodyDef for TargetRegionName<'_> {
    const TAG_EXTENSION: u8 = 0x0A;
    const NAME: &'static str = "TARGET_REGION_NAME";
}

// ===========================================================================
//  Section 0x0A — target_region_name_descriptor (Table 157, §6.4.13)
// ---------------------------------------------------------------------------
//  Leading country_code(24) + ISO_639_language_code(24) then a region-name
//  loop whose entries are region_depth-conditional; the loop is unfolded
//  into typed entries.
// ===========================================================================
/// target_region_name body (Table 157, §6.4.13). The region loop is unfolded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetRegionName<'a> {
    /// country_code(24).
    pub country_code: LangCode,
    /// ISO_639_language_code(24).
    pub iso_639_language_code: LangCode,
    /// Region name entries (the loop).
    pub regions: Vec<TargetRegionNameEntry<'a>>,
}

/// An entry in the target_region_name_descriptor region loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetRegionNameEntry<'a> {
    /// region_depth(2).
    pub region_depth: u8,
    /// region name (char run, name_length bytes) — DVB Annex-A text.
    pub region_name: DvbText<'a>,
    /// primary_region_code(8), always present.
    pub primary_region_code: u8,
    /// secondary_region_code(8), present iff region_depth>=2.
    pub secondary_region_code: Option<u8>,
    /// tertiary_region_code(16), present iff region_depth==3.
    pub tertiary_region_code: Option<u16>,
}

impl<'a> Parse<'a> for TargetRegionName<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < 2 * ISO_639_LEN {
            return Err(invalid("target_region_name: header truncated"));
        }
        let country_code = LangCode([sel[0], sel[1], sel[2]]);
        let iso_639_language_code = LangCode([sel[3], sel[4], sel[5]]);
        let mut regions = Vec::new();
        let mut pos = 2 * ISO_639_LEN;
        while pos < sel.len() {
            let byte = sel[pos];
            pos += 1;
            let region_depth = byte >> 6;
            let name_length = (byte & 0x3F) as usize;
            if pos + name_length > sel.len() {
                return Err(invalid("target_region_name: region entry overruns body"));
            }
            let region_name = DvbText::new(&sel[pos..pos + name_length]);
            pos += name_length;
            if pos >= sel.len() {
                return Err(invalid("target_region_name: region entry overruns body"));
            }
            let primary_region_code = sel[pos];
            pos += 1;
            let secondary_region_code = if region_depth >= 2 {
                if pos >= sel.len() {
                    return Err(invalid("target_region_name: region entry overruns body"));
                }
                let val = sel[pos];
                pos += 1;
                Some(val)
            } else {
                None
            };
            let tertiary_region_code = if region_depth == 3 {
                if pos + 1 >= sel.len() {
                    return Err(invalid("target_region_name: region entry overruns body"));
                }
                let val = u16::from_be_bytes([sel[pos], sel[pos + 1]]);
                pos += 2;
                Some(val)
            } else {
                None
            };
            regions.push(TargetRegionNameEntry {
                region_depth,
                region_name,
                primary_region_code,
                secondary_region_code,
                tertiary_region_code,
            });
        }
        Ok(TargetRegionName {
            country_code,
            iso_639_language_code,
            regions,
        })
    }
}

impl Serialize for TargetRegionName<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        2 * ISO_639_LEN
            + self
                .regions
                .iter()
                .map(|r| {
                    1 + r.region_name.raw().len()
                        + 1
                        + if r.region_depth >= 2 { 1 } else { 0 }
                        + if r.region_depth == 3 { 2 } else { 0 }
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
        buf[ISO_639_LEN..2 * ISO_639_LEN].copy_from_slice(&self.iso_639_language_code.0);
        let mut pos = 2 * ISO_639_LEN;
        for region in &self.regions {
            let raw = region.region_name.raw();
            buf[pos] = ((region.region_depth & 0x03) << 6) | (raw.len() as u8 & 0x3F);
            pos += 1;
            buf[pos..pos + raw.len()].copy_from_slice(raw);
            pos += raw.len();
            buf[pos] = region.primary_region_code;
            pos += 1;
            if let Some(sec) = region.secondary_region_code {
                buf[pos] = sec;
                pos += 1;
            }
            if let Some(tert) = region.tertiary_region_code {
                buf[pos..pos + 2].copy_from_slice(&tert.to_be_bytes());
                pos += 2;
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
    fn parse_target_region_name_structured() {
        let sel = [b'g', b'b', b'r', b'e', b'n', b'g', 0x42, b'H', b'i', 0x12];
        let bytes = wrap(0x0A, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegionName(b) => {
                assert_eq!(b.country_code, LangCode(*b"gbr"));
                assert_eq!(b.iso_639_language_code, LangCode(*b"eng"));
                assert_eq!(b.regions.len(), 1);
                assert_eq!(b.regions[0].region_depth, 1);
                assert_eq!(b.regions[0].region_name.raw(), b"Hi");
                assert_eq!(b.regions[0].primary_region_code, 0x12);
                assert_eq!(b.regions[0].secondary_region_code, None);
                assert_eq!(b.regions[0].tertiary_region_code, None);
            }
            other => panic!("expected TargetRegionName, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn target_region_name_tsduck_empty() {
        let bytes = from_hex("7f070a666f6f626172");
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegionName(b) => {
                assert_eq!(b.country_code, LangCode(*b"foo"));
                assert_eq!(b.iso_639_language_code, LangCode(*b"bar"));
                assert!(b.regions.is_empty());
            }
            other => panic!("expected TargetRegionName, got {other:?}"),
        }
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    #[test]
    fn target_region_name_tsduck_full() {
        let bytes = from_hex(
            "7f260a4142434445464c726567696f6e20666f6f203112cc726567696f6e2062617220323456789a",
        );
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegionName(b) => {
                assert_eq!(b.country_code, LangCode(*b"ABC"));
                assert_eq!(b.iso_639_language_code, LangCode(*b"DEF"));
                assert_eq!(b.regions.len(), 2);
                // [0] region_depth=1, region_name="region foo 1", primary=0x12
                assert_eq!(b.regions[0].region_depth, 1);
                assert_eq!(b.regions[0].region_name.raw(), b"region foo 1");
                assert_eq!(b.regions[0].primary_region_code, 0x12);
                assert_eq!(b.regions[0].secondary_region_code, None);
                assert_eq!(b.regions[0].tertiary_region_code, None);
                // [1] region_depth=3, region_name="region bar 2", primary=0x34, sec=0x56, tert=0x789A
                assert_eq!(b.regions[1].region_depth, 3);
                assert_eq!(b.regions[1].region_name.raw(), b"region bar 2");
                assert_eq!(b.regions[1].primary_region_code, 0x34);
                assert_eq!(b.regions[1].secondary_region_code, Some(0x56));
                assert_eq!(b.regions[1].tertiary_region_code, Some(0x789A));
            }
            other => panic!("expected TargetRegionName, got {other:?}"),
        }
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }
}
