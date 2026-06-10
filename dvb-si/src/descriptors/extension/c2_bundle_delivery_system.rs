//! C2 Bundle Delivery System Descriptor — ETSI EN 300 468 §6.4.6.4 (tag_extension 0x16).
use super::*;

impl super::sealed::Sealed for C2BundleDeliverySystem {}
impl ExtensionBodyDef for C2BundleDeliverySystem {
    const TAG_EXTENSION: u8 = 0x16;
    const NAME: &'static str = "C2_BUNDLE_DELIVERY_SYSTEM";
}
/// One C2 bundle entry (Table 139 inner loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct C2BundleEntry {
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
    /// primary_channel(1).
    pub primary_channel: bool,
}

/// C2_bundle_delivery_system body (Table 139) — fully typed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct C2BundleDeliverySystem {
    /// Bundle entries in wire order.
    pub entries: Vec<C2BundleEntry>,
}

impl<'a> Parse<'a> for C2BundleDeliverySystem {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() % C2_BUNDLE_ENTRY_LEN != 0 {
            return Err(invalid(
                "C2_bundle_delivery_system: not a whole number of entries",
            ));
        }
        let mut entries = Vec::with_capacity(sel.len() / C2_BUNDLE_ENTRY_LEN);
        for chunk in sel.chunks_exact(C2_BUNDLE_ENTRY_LEN) {
            let packed = chunk[6];
            entries.push(C2BundleEntry {
                plp_id: chunk[0],
                data_slice_id: chunk[1],
                c2_system_tuning_frequency: u32::from_be_bytes([
                    chunk[2], chunk[3], chunk[4], chunk[5],
                ]),
                c2_system_tuning_frequency_type: packed >> 6,
                active_ofdm_symbol_duration: (packed >> 3) & 0x07,
                guard_interval: packed & 0x07,
                primary_channel: (chunk[7] & 0x80) != 0,
            });
        }
        Ok(C2BundleDeliverySystem { entries })
    }
}

impl Serialize for C2BundleDeliverySystem {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        self.entries.len() * C2_BUNDLE_ENTRY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let mut p = 0;
        for e in &self.entries {
            buf[p] = e.plp_id;
            buf[p + 1] = e.data_slice_id;
            buf[p + 2..p + 6].copy_from_slice(&e.c2_system_tuning_frequency.to_be_bytes());
            buf[p + 6] = (e.c2_system_tuning_frequency_type << 6)
                | ((e.active_ofdm_symbol_duration & 0x07) << 3)
                | (e.guard_interval & 0x07);
            buf[p + 7] = u8::from(e.primary_channel) << 7;
            p += C2_BUNDLE_ENTRY_LEN;
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn parse_c2_bundle_two_entries() {
        let entry = |off: u8| {
            let packed = (0x01u8 << 6) | 0x01; // freq_type=1, ofdm=0, guard=1
                                               // 8 bytes per Table 139: ... + primary_channel(1)+reserved_zero(7)
            [off, off + 1, 0x00, 0x00, 0x10, 0x00, packed, 0x80]
        };
        let mut sel = Vec::new();
        sel.extend_from_slice(&entry(0x01));
        sel.extend_from_slice(&entry(0x05));
        let bytes = wrap(0x16, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::C2BundleDeliverySystem(b) => {
                assert_eq!(b.entries.len(), 2);
                assert_eq!(b.entries[0].plp_id, 0x01);
                assert!(b.entries[0].primary_channel);
                assert_eq!(b.entries[1].plp_id, 0x05);
                assert_eq!(b.entries[1].guard_interval, 0x01);
            }
            other => panic!("expected C2BundleDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_c2_bundle_rejects_partial_entry() {
        let sel = [0x01, 0x02, 0x03]; // 3 bytes, not a multiple of 8
        let bytes = wrap(0x16, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
        ));
    }
}
