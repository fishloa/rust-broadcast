use super::*;

impl ExtensionBodyDef for C2BundleDeliverySystem {
    const TAG_EXTENSION: u8 = 0x16;
    const NAME: &'static str = "C2_BUNDLE_DELIVERY_SYSTEM";
}

// ===========================================================================
//  Section 0x16 — C2_bundle_delivery_system_descriptor (Table 139, §6.4.6.4)
// ---------------------------------------------------------------------------
//  A flat array of fixed 8-byte entries; fully typed.
// ===========================================================================
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
