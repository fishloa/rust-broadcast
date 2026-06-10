use super::*;

impl ExtensionBodyDef for TargetRegion<'_> {
    const TAG_EXTENSION: u8 = 0x09;
    const NAME: &'static str = "TARGET_REGION";
}

// ===========================================================================
//  Section 0x09 — target_region_descriptor (Table 156, §6.4.12)
// ---------------------------------------------------------------------------
//  Leading country_code(24) then a region loop whose entries are
//  region_depth-conditional; the loop is kept raw (SAT precedent).
// ===========================================================================
/// target_region body (Table 156); `region_loop` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetRegion<'a> {
    /// Leading country_code(24).
    pub country_code: LangCode,
    /// Raw region loop.
    pub region_loop: &'a [u8],
}

impl<'a> Parse<'a> for TargetRegion<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < ISO_639_LEN {
            return Err(invalid("target_region: country_code truncated"));
        }
        Ok(TargetRegion {
            country_code: LangCode([sel[0], sel[1], sel[2]]),
            region_loop: &sel[ISO_639_LEN..],
        })
    }
}

impl Serialize for TargetRegion<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        ISO_639_LEN + self.region_loop.len()
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
        buf[ISO_639_LEN..len].copy_from_slice(self.region_loop);
        Ok(len)
    }
}
