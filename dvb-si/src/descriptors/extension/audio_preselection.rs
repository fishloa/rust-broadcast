use super::*;

impl ExtensionBodyDef for AudioPreselection<'_> {
    const TAG_EXTENSION: u8 = 0x19;
    const NAME: &'static str = "AUDIO_PRESELECTION";
}

// ===========================================================================
//  Section 0x19 — audio_preselection_descriptor (Table 110, §6.4.1)
// ---------------------------------------------------------------------------
//  num_preselections then a variable preselection loop whose entries carry
//  conditional language / message / aux-component / future-extension fields.
//  The loop is kept raw (SAT precedent); the count byte is typed.
// ===========================================================================
/// audio_preselection body (Table 110); `preselection_loop` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AudioPreselection<'a> {
    /// num_preselections(5).
    pub num_preselections: u8,
    /// Raw preselection loop.
    pub preselection_loop: &'a [u8],
}

impl<'a> Parse<'a> for AudioPreselection<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(invalid("audio_preselection: count byte missing"));
        }
        Ok(AudioPreselection {
            num_preselections: sel[0] >> 3,
            preselection_loop: &sel[1..],
        })
    }
}

impl Serialize for AudioPreselection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self.preselection_loop.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.num_preselections << 3;
        buf[1..len].copy_from_slice(self.preselection_loop);
        Ok(len)
    }
}
