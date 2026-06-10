//! Service Relocated Descriptor — ETSI EN 300 468 §6.4.10 (tag_extension 0x0B).
use super::*;

impl super::sealed::Sealed for ServiceRelocated {}
impl ExtensionBodyDef for ServiceRelocated {
    const TAG_EXTENSION: u8 = 0x0B;
    const NAME: &'static str = "SERVICE_RELOCATED";
}
/// service_relocated body (Table 152) — fully typed, fixed 6 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceRelocated {
    /// old_original_network_id(16).
    pub old_original_network_id: u16,
    /// old_transport_stream_id(16).
    pub old_transport_stream_id: u16,
    /// old_service_id(16).
    pub old_service_id: u16,
}

impl<'a> Parse<'a> for ServiceRelocated {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < SERVICE_RELOCATED_LEN {
            return Err(invalid("service_relocated: truncated"));
        }
        Ok(ServiceRelocated {
            old_original_network_id: u16::from_be_bytes([sel[0], sel[1]]),
            old_transport_stream_id: u16::from_be_bytes([sel[2], sel[3]]),
            old_service_id: u16::from_be_bytes([sel[4], sel[5]]),
        })
    }
}

impl Serialize for ServiceRelocated {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        SERVICE_RELOCATED_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0..2].copy_from_slice(&self.old_original_network_id.to_be_bytes());
        buf[2..4].copy_from_slice(&self.old_transport_stream_id.to_be_bytes());
        buf[4..6].copy_from_slice(&self.old_service_id.to_be_bytes());
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_service_relocated() {
        let sel = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        let bytes = wrap(0x0B, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ServiceRelocated));
        match &d.body {
            ExtensionBody::ServiceRelocated(b) => {
                assert_eq!(b.old_original_network_id, 0x1234);
                assert_eq!(b.old_transport_stream_id, 0x5678);
                assert_eq!(b.old_service_id, 0x9ABC);
            }
            other => panic!("expected ServiceRelocated, got {other:?}"),
        }
        round_trip(&d);
    }
}
