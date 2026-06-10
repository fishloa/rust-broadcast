//! C2 Delivery System Descriptor — ETSI EN 300 468 §6.4.6.1 (tag_extension 0x0D).
use super::*;

impl super::sealed::Sealed for C2DeliverySystem {}
impl ExtensionBodyDef for C2DeliverySystem {
    const TAG_EXTENSION: u8 = 0x0D;
    const NAME: &'static str = "C2_DELIVERY_SYSTEM";
}
/// C2_delivery_system body (Table 115) — fully typed, fixed 7 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct C2DeliverySystem {
    /// plp_id(8).
    pub plp_id: u8,
    /// data_slice_id(8).
    pub data_slice_id: u8,
    /// C2_System_tuning_frequency(32).
    pub c2_system_tuning_frequency: u32,
    /// C2_System_tuning_frequency_type(2).
    pub c2_system_tuning_frequency_type: u8,
    /// active_OFDM_symbol_duration(3).
    pub active_ofdm_symbol_duration: u8,
    /// guard_interval(3).
    pub guard_interval: u8,
}

impl<'a> Parse<'a> for C2DeliverySystem {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < C2_LEN {
            return Err(invalid("C2_delivery_system: truncated"));
        }
        let packed = sel[6];
        Ok(C2DeliverySystem {
            plp_id: sel[0],
            data_slice_id: sel[1],
            c2_system_tuning_frequency: u32::from_be_bytes([sel[2], sel[3], sel[4], sel[5]]),
            c2_system_tuning_frequency_type: packed >> 6,
            active_ofdm_symbol_duration: (packed >> 3) & 0x07,
            guard_interval: packed & 0x07,
        })
    }
}

impl Serialize for C2DeliverySystem {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        C2_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.plp_id;
        buf[1] = self.data_slice_id;
        buf[2..6].copy_from_slice(&self.c2_system_tuning_frequency.to_be_bytes());
        buf[6] = (self.c2_system_tuning_frequency_type << 6)
            | ((self.active_ofdm_symbol_duration & 0x07) << 3)
            | (self.guard_interval & 0x07);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn parse_c2_delivery_system() {
        let packed = (0x02 << 6) | (0x01 << 3) | 0x01;
        let sel = [0x05, 0x09, 0x12, 0x34, 0x56, 0x78, packed];
        let bytes = wrap(0x0D, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::C2DeliverySystem(b) => {
                assert_eq!(b.plp_id, 0x05);
                assert_eq!(b.data_slice_id, 0x09);
                assert_eq!(b.c2_system_tuning_frequency, 0x1234_5678);
                assert_eq!(b.c2_system_tuning_frequency_type, 0x02);
                assert_eq!(b.active_ofdm_symbol_duration, 0x01);
                assert_eq!(b.guard_interval, 0x01);
            }
            other => panic!("expected C2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }
}
