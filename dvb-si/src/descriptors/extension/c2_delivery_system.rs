//! C2 Delivery System Descriptor — ETSI EN 300 468 §6.4.6.1 (tag_extension 0x0D).
use super::*;

impl<'a> ExtensionBodyDef<'a> for C2DeliverySystem {
    const TAG_EXTENSION: u8 = 0x0D;
    const NAME: &'static str = "C2_DELIVERY_SYSTEM";
}

/// C2 system tuning frequency type — ETSI EN 300 468 Table 116.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum C2TuningFrequencyType {
    /// 0b00 — Data Slice tuning frequency.
    DataSliceTuningFrequency,
    /// 0b01 — C2 system centre frequency.
    C2SystemCentreFrequency,
    /// 0b10 — Initial tuning position for a (dependent) Static Data Slice.
    InitialTuningPositionStaticDataSlice,
    /// Reserved / future use.
    Reserved(u8),
}

impl C2TuningFrequencyType {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => C2TuningFrequencyType::DataSliceTuningFrequency,
            1 => C2TuningFrequencyType::C2SystemCentreFrequency,
            2 => C2TuningFrequencyType::InitialTuningPositionStaticDataSlice,
            other => C2TuningFrequencyType::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            C2TuningFrequencyType::DataSliceTuningFrequency => 0,
            C2TuningFrequencyType::C2SystemCentreFrequency => 1,
            C2TuningFrequencyType::InitialTuningPositionStaticDataSlice => 2,
            C2TuningFrequencyType::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per Table 116.
    pub fn name(self) -> &'static str {
        match self {
            C2TuningFrequencyType::DataSliceTuningFrequency => "Data Slice tuning frequency",
            C2TuningFrequencyType::C2SystemCentreFrequency => "C2 system centre frequency",
            C2TuningFrequencyType::InitialTuningPositionStaticDataSlice => {
                "initial tuning position for a Static Data Slice"
            }
            C2TuningFrequencyType::Reserved(_) => "reserved",
        }
    }
}

/// Active OFDM symbol duration — ETSI EN 300 468 Table 117.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ActiveOfdmSymbolDuration {
    /// 448 µs (4k FFT mode for 8 MHz CATV systems).
    Us448,
    /// 597.33 µs (4k FFT mode for 6 MHz CATV systems).
    Us597_33,
    /// Reserved / future use.
    Reserved(u8),
}

impl ActiveOfdmSymbolDuration {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => ActiveOfdmSymbolDuration::Us448,
            1 => ActiveOfdmSymbolDuration::Us597_33,
            other => ActiveOfdmSymbolDuration::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            ActiveOfdmSymbolDuration::Us448 => 0,
            ActiveOfdmSymbolDuration::Us597_33 => 1,
            ActiveOfdmSymbolDuration::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            ActiveOfdmSymbolDuration::Us448 => "448 µs (4k FFT, 8 MHz)",
            ActiveOfdmSymbolDuration::Us597_33 => "597.33 µs (4k FFT, 6 MHz)",
            ActiveOfdmSymbolDuration::Reserved(_) => "reserved",
        }
    }
}

/// C2 guard interval — ETSI EN 300 468 Table 118.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum C2GuardInterval {
    /// 1/128.
    G1_128,
    /// 1/64.
    G1_64,
    /// Reserved / future use.
    Reserved(u8),
}

impl C2GuardInterval {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => C2GuardInterval::G1_128,
            1 => C2GuardInterval::G1_64,
            other => C2GuardInterval::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            C2GuardInterval::G1_128 => 0,
            C2GuardInterval::G1_64 => 1,
            C2GuardInterval::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            C2GuardInterval::G1_128 => "1/128",
            C2GuardInterval::G1_64 => "1/64",
            C2GuardInterval::Reserved(_) => "reserved",
        }
    }
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
    /// C2_System_tuning_frequency_type(2) — Table 116.
    pub c2_system_tuning_frequency_type: C2TuningFrequencyType,
    /// active_OFDM_symbol_duration(3) — Table 117.
    pub active_ofdm_symbol_duration: ActiveOfdmSymbolDuration,
    /// guard_interval(3) — Table 118.
    pub guard_interval: C2GuardInterval,
}

impl<'a> Parse<'a> for C2DeliverySystem {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < C2_LEN {
            return Err(Error::BufferTooShort {
                need: C2_LEN,
                have: sel.len(),
                what: "C2_delivery_system body",
            });
        }
        let packed = sel[6];
        Ok(C2DeliverySystem {
            plp_id: sel[0],
            data_slice_id: sel[1],
            c2_system_tuning_frequency: u32::from_be_bytes([sel[2], sel[3], sel[4], sel[5]]),
            c2_system_tuning_frequency_type: C2TuningFrequencyType::from_u8(packed >> 6),
            active_ofdm_symbol_duration: ActiveOfdmSymbolDuration::from_u8((packed >> 3) & 0x07),
            guard_interval: C2GuardInterval::from_u8(packed & 0x07),
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
        buf[6] = (self.c2_system_tuning_frequency_type.to_u8() << 6)
            | ((self.active_ofdm_symbol_duration.to_u8() & 0x07) << 3)
            | (self.guard_interval.to_u8() & 0x07);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn c2_tuning_frequency_type_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(C2TuningFrequencyType::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn c2_tuning_frequency_type_name() {
        assert_eq!(
            C2TuningFrequencyType::DataSliceTuningFrequency.name(),
            "Data Slice tuning frequency"
        );
        assert_eq!(
            C2TuningFrequencyType::C2SystemCentreFrequency.name(),
            "C2 system centre frequency"
        );
        assert_eq!(
            C2TuningFrequencyType::InitialTuningPositionStaticDataSlice.name(),
            "initial tuning position for a Static Data Slice"
        );
        assert_eq!(C2TuningFrequencyType::Reserved(3).name(), "reserved");
    }

    #[test]
    fn active_ofdm_symbol_duration_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(ActiveOfdmSymbolDuration::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn c2_guard_interval_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(C2GuardInterval::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn parse_c2_delivery_system() {
        let packed = (0x01u8 << 3) | 0x01;
        let sel = [0x05, 0x09, 0x12, 0x34, 0x56, 0x78, packed];
        let bytes = wrap(0x0D, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::C2DeliverySystem(b) => {
                assert_eq!(b.plp_id, 0x05);
                assert_eq!(b.data_slice_id, 0x09);
                assert_eq!(b.c2_system_tuning_frequency, 0x1234_5678);
                assert_eq!(
                    b.c2_system_tuning_frequency_type,
                    C2TuningFrequencyType::DataSliceTuningFrequency
                );
                assert_eq!(
                    b.active_ofdm_symbol_duration,
                    ActiveOfdmSymbolDuration::Us597_33
                );
                assert_eq!(b.guard_interval, C2GuardInterval::G1_64);
            }
            other => panic!("expected C2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_c2_delivery_system_centre_frequency() {
        let packed = 0x01u8 << 6;
        let sel = [0x05, 0x09, 0x12, 0x34, 0x56, 0x78, packed];
        let bytes = wrap(0x0D, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::C2DeliverySystem(b) => {
                assert_eq!(
                    b.c2_system_tuning_frequency_type,
                    C2TuningFrequencyType::C2SystemCentreFrequency
                );
            }
            other => panic!("expected C2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_c2_delivery_system_initial_tuning() {
        let packed = 0x02u8 << 6;
        let sel = [0x05, 0x09, 0x12, 0x34, 0x56, 0x78, packed];
        let bytes = wrap(0x0D, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::C2DeliverySystem(b) => {
                assert_eq!(
                    b.c2_system_tuning_frequency_type,
                    C2TuningFrequencyType::InitialTuningPositionStaticDataSlice
                );
            }
            other => panic!("expected C2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_c2_delivery_system_reserved_values() {
        let packed = (0x03u8 << 6) | (0x07u8 << 3) | 0x07;
        let sel = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, packed];
        let bytes = wrap(0x0D, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::C2DeliverySystem(b) => {
                assert_eq!(
                    b.c2_system_tuning_frequency_type,
                    C2TuningFrequencyType::Reserved(3)
                );
                assert_eq!(
                    b.active_ofdm_symbol_duration,
                    ActiveOfdmSymbolDuration::Reserved(7)
                );
                assert_eq!(b.guard_interval, C2GuardInterval::Reserved(7));
            }
            other => panic!("expected C2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }
}
