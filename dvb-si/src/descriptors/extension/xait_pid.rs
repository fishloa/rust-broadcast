//! XAIT PID Descriptor — ETSI TS 102 727 §10.17.3, Table 95
//! (extension descriptor: tag 0x7F, tag_extension 0x0C).
//!
//! Signals the PID carrying the XAIT (e.g. in the NIT network descriptor loop).
//! Table 95 defines a single 16-bit `xait_PID` field (`uimsbf`); the carried
//! value is a transport PID, so in practice it is `<= 0x1FFF` (a 13-bit PID,
//! default `0x1FFC`). We store and round-trip the full 16-bit field verbatim
//! rather than masking — the spec defines no reserved-bit split here, so this
//! is byte-exact against any real stream.
use super::*;

/// Selector length: the 2-byte `xait_PID` field.
const XAIT_PID_LEN: usize = 2;
/// Largest valid transport PID (`xait_PID` is semantically a PID).
pub const MAX_PID: u16 = 0x1FFF;

impl ExtensionBodyDef<'_> for XaitPid {
    const TAG_EXTENSION: u8 = 0x0C;
    const NAME: &'static str = "XAIT_PID";
}

/// xait_pid body (Table 95) — the 16-bit `xait_PID` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct XaitPid {
    /// `xait_PID` (16 bits, `uimsbf`); the PID carrying the XAIT. A valid PID is
    /// `<= `[`MAX_PID`]; default when absent is `0x1FFC`.
    pub xait_pid: u16,
}

impl XaitPid {
    /// Whether [`Self::xait_pid`] is a valid 13-bit transport PID (`<= 0x1FFF`).
    #[must_use]
    pub const fn is_valid_pid(&self) -> bool {
        self.xait_pid <= MAX_PID
    }
}

impl<'a> Parse<'a> for XaitPid {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < XAIT_PID_LEN {
            return Err(Error::BufferTooShort {
                need: XAIT_PID_LEN,
                have: sel.len(),
                what: "xait_pid body",
            });
        }
        Ok(Self {
            xait_pid: u16::from_be_bytes([sel[0], sel[1]]),
        })
    }
}

impl Serialize for XaitPid {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        XAIT_PID_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0..2].copy_from_slice(&self.xait_pid.to_be_bytes());
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_default_pid() {
        // xait_PID = 0x1FFC (the spec default).
        let sel = [0x1F, 0xFC];
        let bytes = wrap(0x0C, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::XaitPid));
        match &d.body {
            ExtensionBody::XaitPid(b) => {
                assert_eq!(b.xait_pid, 0x1FFC);
                assert!(b.is_valid_pid());
            }
            other => panic!("expected XaitPid, got {other:?}"),
        }
    }

    #[test]
    fn serialize_round_trip() {
        let d = ExtensionDescriptor {
            tag_extension: 0x0C,
            body: ExtensionBody::XaitPid(XaitPid { xait_pid: 0x0100 }),
        };
        round_trip(&d);
    }

    #[test]
    fn serialize_byte_identical() {
        // xait_PID = 0x0100, stored and round-tripped verbatim.
        let sel = [0x01, 0x00];
        let bytes = wrap(0x0C, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }

    #[test]
    fn high_bits_preserved_verbatim() {
        // No reserved-bit split: a value with high bits set round-trips exactly.
        let sel = [0xE1, 0x00];
        let bytes = wrap(0x0C, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::XaitPid(b) => {
                assert_eq!(b.xait_pid, 0xE100);
                assert!(!b.is_valid_pid());
            }
            other => panic!("expected XaitPid, got {other:?}"),
        }
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }
}
