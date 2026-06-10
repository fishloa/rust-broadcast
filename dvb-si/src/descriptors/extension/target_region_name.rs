use super::*;

impl ExtensionBodyDef for TargetRegionName<'_> {
    const TAG_EXTENSION: u8 = 0x0A;
    const NAME: &'static str = "TARGET_REGION_NAME";
}

// ===========================================================================
//  Section 0x0A — target_region_name_descriptor (Table 157, §6.4.13)
// ===========================================================================
/// target_region_name body (Table 157); `region_loop` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetRegionName<'a> {
    /// country_code(24).
    pub country_code: LangCode,
    /// ISO_639_language_code(24).
    pub iso_639_language_code: LangCode,
    /// Raw region loop (length-delimited name entries).
    pub region_loop: &'a [u8],
}

impl<'a> Parse<'a> for TargetRegionName<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < 2 * ISO_639_LEN {
            return Err(invalid("target_region_name: header truncated"));
        }
        Ok(TargetRegionName {
            country_code: LangCode([sel[0], sel[1], sel[2]]),
            iso_639_language_code: LangCode([sel[3], sel[4], sel[5]]),
            region_loop: &sel[2 * ISO_639_LEN..],
        })
    }
}

impl Serialize for TargetRegionName<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        2 * ISO_639_LEN + self.region_loop.len()
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
        buf[2 * ISO_639_LEN..len].copy_from_slice(self.region_loop);
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
    fn parse_target_region_name_loop_raw() {
        let sel = [b'g', b'b', b'r', b'e', b'n', b'g', 0x44, 0x55];
        let bytes = wrap(0x0A, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegionName(b) => {
                assert_eq!(b.country_code, LangCode(*b"gbr"));
                assert_eq!(b.iso_639_language_code, LangCode(*b"eng"));
                assert_eq!(b.region_loop, &[0x44, 0x55]);
            }
            other => panic!("expected TargetRegionName, got {other:?}"),
        }
        round_trip(&d);
    }
}
