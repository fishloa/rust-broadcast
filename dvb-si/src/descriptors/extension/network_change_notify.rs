use super::*;

impl ExtensionBodyDef for NetworkChangeNotify<'_> {
    const TAG_EXTENSION: u8 = 0x07;
    const NAME: &'static str = "NETWORK_CHANGE_NOTIFY";
}

// ===========================================================================
//  Section 0x07 — network_change_notify_descriptor (Table 149, §6.4.9)
// ---------------------------------------------------------------------------
//  Two-level loop: per cell_id a length-delimited inner change loop whose
//  entries carry conditional invariant-TS fields. Kept raw (SAT precedent).
// ===========================================================================
/// network_change_notify body (Table 149); `cell_loop` is the raw outer loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct NetworkChangeNotify<'a> {
    /// Raw `for(cell)` loop body.
    pub cell_loop: &'a [u8],
}

impl<'a> Parse<'a> for NetworkChangeNotify<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        Ok(NetworkChangeNotify { cell_loop: sel })
    }
}

impl Serialize for NetworkChangeNotify<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        self.cell_loop.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[..len].copy_from_slice(self.cell_loop);
        Ok(len)
    }
}
