//! CP Descriptor — ETSI EN 300 468 §6.4.3, Table 113 (tag_extension 0x02).
//!
//! Content protection signalling: a CP_system_id, the PID carrying the CP
//! stream, and a private_data byte run.
use super::*;
use mpeg_ts::pid::Pid;

impl<'a> ExtensionBodyDef<'a> for Cp<'a> {
    const TAG_EXTENSION: u8 = 0x02;
    const NAME: &'static str = "CP";
}

const CP_HEADER_LEN: usize = 4; // CP_system_id(2) + reserved(3)/CP_PID(13)
/// Maximum 13-bit PID value.
const MAX_PID: u16 = 0x1FFF;
/// Shift of `reserved_future_use(3)` in the CP_PID word.
const RESERVED_SHIFT: u16 = 13;

/// Spec default of `reserved_future_use(3)` ahead of `CP_PID`: `111`
/// (EN 300 468 Table 113; `reserved` in V1.9.1-V1.15.1, `reserved_future_use`
/// in V1.17.1+, value 1 throughout; audit §6.1 class B).
pub const CP_RESERVED_FUTURE_USE_DEFAULT: u8 = 0b111;

/// CP descriptor body (Table 113, §6.4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct Cp<'a> {
    /// CP_system_id(16).
    pub cp_system_id: u16,
    /// `reserved_future_use(3)` ahead of `CP_PID` (3 bits, right-aligned).
    /// Captured on parse, re-emitted verbatim; [`Cp::new`] defaults to
    /// [`CP_RESERVED_FUTURE_USE_DEFAULT`] (`0b111`).
    pub reserved_future_use: u8,
    /// CP_PID(13) — the PID carrying the CP stream.
    pub cp_pid: Pid,
    /// private_data_byte run (the rest of the descriptor).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

impl<'a> Cp<'a> {
    /// Build a body with the spec-default reserved bits.
    #[must_use]
    pub const fn new(cp_system_id: u16, cp_pid: Pid, private_data: &'a [u8]) -> Self {
        Self {
            cp_system_id,
            reserved_future_use: CP_RESERVED_FUTURE_USE_DEFAULT,
            cp_pid,
            private_data,
        }
    }
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
            reserved_future_use: sel[2] >> 5,
            cp_pid: Pid::new(u16::from_be_bytes([sel[2], sel[3]])),
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
        // reserved_future_use(3), then CP_PID(13).
        let cp_pid = self.cp_pid.value();
        let reserved = u16::from(self.reserved_future_use & 0b111) << RESERVED_SHIFT;
        buf[2..4].copy_from_slice(&(reserved | cp_pid).to_be_bytes());
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
                assert_eq!(b.cp_pid, Pid::new(0x0100));
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

    #[test]
    fn over_13bit_pid_is_unrepresentable() {
        // Pre-fix `cp_pid & MAX_PID` silently clipped bits 13..16 (r03-W3);
        // `Cp::new` now takes a `Pid`, which `try_new` refuses to build.
        assert!(Pid::try_new(0x2000).is_err());
        assert_eq!(Pid::try_new(0x1FFF).unwrap().value(), 0x1FFF);
    }

    /// Every `reserved_future_use(3)` value re-serializes byte-identically.
    #[test]
    fn reserved_preserved() {
        for r in 0u8..=7 {
            let sel = [0x12, 0x34, (r << 5) | 0x01, 0x00, 0xAA, 0xBB];
            let bytes = wrap(0x02, &sel);
            let d = ExtensionDescriptor::parse(&bytes).unwrap();
            match &d.body {
                ExtensionBody::Cp(b) => assert_eq!(b.reserved_future_use, r),
                other => panic!("expected Cp, got {other:?}"),
            }
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes, "r {r}");
        }
    }

    /// `new()` serializes the spec value `111`.
    #[test]
    fn default_reserved_is_spec_value() {
        let c = Cp::new(0x1234, Pid::new(0x0100), &[]);
        let mut buf = vec![0u8; c.serialized_len()];
        c.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [0x12, 0x34, 0xE1, 0x00]);
    }
}
