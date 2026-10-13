//! C2 Bundle Delivery System Descriptor — ETSI EN 300 468 §6.4.6.4 (tag_extension 0x16).
use super::*;
use alloc::vec::Vec;

use super::c2_delivery_system::{ActiveOfdmSymbolDuration, C2GuardInterval, C2TuningFrequencyType};

impl ExtensionBodyDef<'_> for C2BundleDeliverySystem {
    const TAG_EXTENSION: u8 = 0x16;
    const NAME: &'static str = "C2_BUNDLE_DELIVERY_SYSTEM";
}
/// Width mask of the 7-bit `reserved_zero_future_use` after `primary_channel`.
const RESERVED_ZERO_FUTURE_USE_MASK: u8 = 0x7F;

/// Spec default of `reserved_zero_future_use(7)` after `primary_channel`: all
/// zeros (EN 300 468 V1.16.1-V1.20.1 Table 139, audit §6.1 class B; V1.15.1
/// printed it as `reserved_0_future_use`(7) with the same value 0).
pub const C2_BUNDLE_RESERVED_ZERO_FUTURE_USE_DEFAULT: u8 = 0;

/// One C2 bundle entry (Table 139 inner loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct C2BundleEntry {
    /// plp_id(8).
    pub plp_id: u8,
    /// data_slice_id(8).
    pub data_slice_id: u8,
    /// C2_System_tuning_frequency(32).
    pub c2_system_tuning_frequency: u32,
    /// C2_System_tuning_frequency_type(2) — Table 116.
    pub c2_system_tuning_frequency_type: C2TuningFrequencyType,
    /// active_OFDM_symbol_duration(3).
    pub active_ofdm_symbol_duration: ActiveOfdmSymbolDuration,
    /// guard_interval(3).
    pub guard_interval: C2GuardInterval,
    /// primary_channel(1).
    pub primary_channel: bool,
    /// `reserved_zero_future_use(7)` after `primary_channel` (7 bits,
    /// right-aligned). Captured on parse and re-emitted verbatim;
    /// [`C2BundleEntry::new`] defaults to
    /// [`C2_BUNDLE_RESERVED_ZERO_FUTURE_USE_DEFAULT`] (`0`).
    pub reserved_zero_future_use: u8,
}

impl C2BundleEntry {
    /// Build an entry with the spec-default reserved bits.
    #[must_use]
    pub const fn new(
        plp_id: u8,
        data_slice_id: u8,
        c2_system_tuning_frequency: u32,
        c2_system_tuning_frequency_type: C2TuningFrequencyType,
        active_ofdm_symbol_duration: ActiveOfdmSymbolDuration,
        guard_interval: C2GuardInterval,
        primary_channel: bool,
    ) -> Self {
        Self {
            plp_id,
            data_slice_id,
            c2_system_tuning_frequency,
            c2_system_tuning_frequency_type,
            active_ofdm_symbol_duration,
            guard_interval,
            primary_channel,
            reserved_zero_future_use: C2_BUNDLE_RESERVED_ZERO_FUTURE_USE_DEFAULT,
        }
    }
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
        if !sel.len().is_multiple_of(C2_BUNDLE_ENTRY_LEN) {
            return Err(invalid(
                "C2_bundle_delivery_system: not a whole number of entries",
            ));
        }
        let mut entries = Vec::with_capacity(sel.len() / C2_BUNDLE_ENTRY_LEN);
        for chunk in sel.chunks_exact(C2_BUNDLE_ENTRY_LEN) {
            let packed = chunk[6];
            let freq = crate::wire::be_u32(chunk, 2, "c2_bundle_delivery_system")?;
            entries.push(C2BundleEntry {
                plp_id: chunk[0],
                data_slice_id: chunk[1],
                c2_system_tuning_frequency: freq,
                c2_system_tuning_frequency_type: C2TuningFrequencyType::from_u8(packed >> 6),
                active_ofdm_symbol_duration: ActiveOfdmSymbolDuration::from_u8(
                    (packed >> 3) & 0x07,
                ),
                guard_interval: C2GuardInterval::from_u8(packed & 0x07),
                primary_channel: (chunk[7] & 0x80) != 0,
                reserved_zero_future_use: chunk[7] & RESERVED_ZERO_FUTURE_USE_MASK,
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
            buf[p + 6] = (e.c2_system_tuning_frequency_type.to_u8() << 6)
                | ((e.active_ofdm_symbol_duration.to_u8() & 0x07) << 3)
                | (e.guard_interval.to_u8() & 0x07);
            buf[p + 7] = (u8::from(e.primary_channel) << 7)
                | (e.reserved_zero_future_use & RESERVED_ZERO_FUTURE_USE_MASK);
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
            let packed = (0x01u8 << 6) | 0x01;
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
                assert_eq!(
                    b.entries[0].c2_system_tuning_frequency_type,
                    C2TuningFrequencyType::C2SystemCentreFrequency
                );
                assert_eq!(b.entries[1].plp_id, 0x05);
                assert_eq!(b.entries[1].guard_interval, C2GuardInterval::G1_64);
            }
            other => panic!("expected C2BundleDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_c2_bundle_rejects_partial_entry() {
        let sel = [0x01, 0x02, 0x03];
        let bytes = wrap(0x16, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
        ));
    }

    /// Every `reserved_zero_future_use(7)` value (all 128) re-serializes
    /// byte-identically; the spec value is zero.
    #[test]
    fn reserved_preserved() {
        for r in 0u8..=0x7F {
            let sel = [0x01, 0x02, 0x00, 0x00, 0x10, 0x00, 0x41, 0x80 | r];
            let bytes = wrap(0x16, &sel);
            let d = ExtensionDescriptor::parse(&bytes).unwrap();
            match &d.body {
                ExtensionBody::C2BundleDeliverySystem(b) => {
                    assert_eq!(b.entries[0].reserved_zero_future_use, r);
                }
                other => panic!("expected C2BundleDeliverySystem, got {other:?}"),
            }
            round_trip(&d);
        }
    }

    /// `new()` serializes the spec value (zeros).
    #[test]
    fn default_reserved_is_spec_value() {
        let e = C2BundleEntry::new(
            1,
            2,
            0x1000,
            C2TuningFrequencyType::C2SystemCentreFrequency,
            ActiveOfdmSymbolDuration::from_u8(0),
            C2GuardInterval::G1_64,
            true,
        );
        let b = C2BundleDeliverySystem { entries: vec![e] };
        let mut buf = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[7], 0x80);
    }
}
