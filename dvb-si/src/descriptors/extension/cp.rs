//! CP Descriptor — ETSI EN 300 468 §6.4.3, Table 113 (tag_extension 0x02).
//!
//! Content protection signalling: a CP_system_id, the PID carrying the CP
//! stream, and a private_data byte run.
use super::*;

impl<'a> ExtensionBodyDef<'a> for Cp<'a> {
    const TAG_EXTENSION: u8 = 0x02;
    const NAME: &'static str = "CP";
}

const CP_HEADER_LEN: usize = 4; // CP_system_id(2) + reserved(3)/CP_PID(13)
/// Maximum 13-bit PID value.
const MAX_PID: u16 = 0x1FFF;

/// CP descriptor body (Table 113, §6.4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Cp<'a> {
    /// CP_system_id(16).
    pub cp_system_id: u16,
    /// CP_PID(13) — the PID carrying the CP stream.
    pub cp_pid: u16,
    /// private_data_byte run (the rest of the descriptor).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for Cp<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < CP_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: CP_HEADER_LEN,
                have: sel.len(),
                what: "CP body",
            });
        }
        Ok(Cp {
            cp_system_id: u16::from_be_bytes([sel[0], sel[1]]),
            cp_pid: u16::from_be_bytes([sel[2], sel[3]]) & MAX_PID,
            private_data: &sel[CP_HEADER_LEN..],
        })
    }
}

impl Serialize for Cp<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        CP_HEADER_LEN + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0..2].copy_from_slice(&self.cp_system_id.to_be_bytes());
        // reserved_future_use(3) set to '1' per DVB convention, then CP_PID(13).
        buf[2..4].copy_from_slice(&(0xE000 | (self.cp_pid & MAX_PID)).to_be_bytes());
        buf[CP_HEADER_LEN..len].copy_from_slice(self.private_data);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_cp() {
        // CP_system_id=0x1234, reserved=111 + CP_PID=0x0100, private=[0xAA,0xBB]
        let sel = [0x12, 0x34, 0xE1, 0x00, 0xAA, 0xBB];
        let bytes = wrap(0x02, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::Cp));
        match &d.body {
            ExtensionBody::Cp(b) => {
                assert_eq!(b.cp_system_id, 0x1234);
                assert_eq!(b.cp_pid, 0x0100);
                assert_eq!(b.private_data, &[0xAA, 0xBB]);
            }
            other => panic!("expected Cp, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_cp_rejects_short() {
        let err = Cp::parse(&[0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }
}
