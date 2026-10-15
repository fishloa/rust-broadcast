//! China `caption_service_descriptor` — GY/T 270-2013 clause 6.4 (Table 8,
//! character sets Table 9), tag `0x86`.
//!
//! The Chinese DTV closed-caption system (CEA-708 style, GY/T 270-2013) signals
//! its caption services with a descriptor that reuses the ATSC tag `0x86` but has
//! a different layout: a per-service `char_set` (GB 2312 / GB 13000.1 / GB 18030)
//! and a single trailing `caption_service_pid`. It is carried in the first
//! descriptor loop of the PMT (at most 16 per PMT) and in the EIT. Tag 0x86 is
//! also used by other (private) descriptors and is context dependent, so it is
//! not auto-dispatched: enable it with
//! [`crate::descriptors::DescriptorRegistry::with_china_caption_service`].
//!
//! ```text
//! descriptor_tag(8)=0x86 descriptor_length(8)
//! reserved(3)='111' number_of_services(5)
//! for each service (6 bytes):
//!   language(24) reserved(2)='11' caption_service_number(6)
//!   reserved(1)='1' wide_aspect_ratio(1) char_set(6) reserved(8)
//! reserved(3)='111' caption_service_pid(13)
//! ```
//!
//! The three reserved runs are captured on parse and re-emitted verbatim;
//! hand-built values default to the spec's all-ones. The 8-bit reserved byte
//! per service is printed as `"1111111"` (seven ones) in the standard; all
//! ones (`0xFF`) is used as the default.

use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::LangCode;
use alloc::vec::Vec;
use broadcast_common::len::{fit_bits, fit_u8};
use broadcast_common::{Parse, Serialize};
use mpeg_ts::pid::Pid;

/// Descriptor tag for the China caption_service_descriptor.
pub const TAG: u8 = 0x86;
const HEADER_LEN: usize = 2;
const SERVICE_LEN: usize = 6;
const COUNT_BYTE_LEN: usize = 1;
const PID_LEN: usize = 2;
const MAX_SERVICES: usize = 31;
const RESERVED3_ONES: u8 = 0b111;
const RESERVED2_ONES: u8 = 0b11;
const RESERVED1_ONES: u8 = 1;
const RESERVED8_ONES: u8 = 0xFF;

/// `char_set` — GY/T 270-2013 Table 9 (6 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ChinaCaptionCharSet {
    /// 0 - GB 2312-1980.
    Gb2312,
    /// 1 - GB 13000.1-2010.
    Gb13000_1,
    /// 2 - GB 18030-2005.
    Gb18030,
    /// 3..=63 - reserved; carries the value.
    Reserved(u8),
}

impl ChinaCaptionCharSet {
    /// Decode from the 6-bit wire value (lossless).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v & 0x3F {
            0 => Self::Gb2312,
            1 => Self::Gb13000_1,
            2 => Self::Gb18030,
            other => Self::Reserved(other),
        }
    }

    /// The 6-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Gb2312 => 0,
            Self::Gb13000_1 => 1,
            Self::Gb18030 => 2,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (Table 9).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Gb2312 => "GB 2312-1980",
            Self::Gb13000_1 => "GB 13000.1-2010",
            Self::Gb18030 => "GB 18030-2005",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(ChinaCaptionCharSet, Reserved);

/// One caption service entry of the descriptor loop (Table 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ChinaCaptionService {
    /// `language` (24 bits): 3-byte code defined by GB/T 4880.2-2000.
    pub language: LangCode,
    /// `reserved` (2 bits) before `caption_service_number`.
    pub reserved_a: u8,
    /// `caption_service_number` (6 bits).
    pub caption_service_number: u8,
    /// `reserved` (1 bit) before `wide_aspect_ratio`.
    pub reserved_b: u8,
    /// `wide_aspect_ratio`: `false` = 4:3, `true` = 16:9.
    pub wide_aspect_ratio: bool,
    /// `char_set` (6 bits, Table 9).
    pub char_set: ChinaCaptionCharSet,
    /// Final `reserved` (8 bits) of the entry.
    pub reserved_c: u8,
}

impl ChinaCaptionService {
    /// Build an entry with the spec's all-ones defaults for the reserved runs.
    #[must_use]
    pub const fn new(
        language: LangCode,
        caption_service_number: u8,
        wide_aspect_ratio: bool,
        char_set: ChinaCaptionCharSet,
    ) -> Self {
        Self {
            language,
            reserved_a: RESERVED2_ONES,
            caption_service_number,
            reserved_b: RESERVED1_ONES,
            wide_aspect_ratio,
            char_set,
            reserved_c: RESERVED8_ONES,
        }
    }
}

/// China `caption_service_descriptor` (GY/T 270-2013 Table 8).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ChinaCaptionServiceDescriptor {
    /// `reserved` (3 bits) before `number_of_services`.
    pub reserved_head: u8,
    /// The caption services; `number_of_services` is derived on serialize.
    pub services: Vec<ChinaCaptionService>,
    /// `reserved` (3 bits) before `caption_service_pid`.
    pub reserved_pid: u8,
    /// `caption_service_pid` (13 bits): PID of the PES carrying the captions.
    pub caption_service_pid: Pid,
}

impl ChinaCaptionServiceDescriptor {
    /// Build a descriptor with all-ones reserved defaults.
    #[must_use]
    pub fn new(services: Vec<ChinaCaptionService>, caption_service_pid: Pid) -> Self {
        Self {
            reserved_head: RESERVED3_ONES,
            services,
            reserved_pid: RESERVED3_ONES,
            caption_service_pid,
        }
    }
}

impl<'a> Parse<'a> for ChinaCaptionServiceDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ChinaCaptionServiceDescriptor",
            "unexpected tag for caption_service_descriptor",
        )?;
        if body.len() < COUNT_BYTE_LEN + PID_LEN {
            return Err(Error::BufferTooShort {
                need: COUNT_BYTE_LEN + PID_LEN,
                have: body.len(),
                what: "ChinaCaptionServiceDescriptor",
            });
        }
        let n = usize::from(body[0] & 0x1F);
        let need = COUNT_BYTE_LEN + n * SERVICE_LEN + PID_LEN;
        if body.len() != need {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length does not match number_of_services",
            });
        }
        let mut services = Vec::with_capacity(n);
        for chunk in
            body[COUNT_BYTE_LEN..COUNT_BYTE_LEN + n * SERVICE_LEN].chunks_exact(SERVICE_LEN)
        {
            services.push(ChinaCaptionService {
                language: LangCode([chunk[0], chunk[1], chunk[2]]),
                reserved_a: chunk[3] >> 6,
                caption_service_number: chunk[3] & 0x3F,
                reserved_b: chunk[4] >> 7,
                wide_aspect_ratio: chunk[4] & 0x40 != 0,
                char_set: ChinaCaptionCharSet::from_u8(chunk[4] & 0x3F),
                reserved_c: chunk[5],
            });
        }
        let tail = &body[need - PID_LEN..];
        Ok(Self {
            reserved_head: body[0] >> 5,
            services,
            reserved_pid: tail[0] >> 5,
            caption_service_pid: Pid::new(u16::from_be_bytes([tail[0] & 0x1F, tail[1]])),
        })
    }
}

impl Serialize for ChinaCaptionServiceDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + COUNT_BYTE_LEN + self.services.len() * SERVICE_LEN + PID_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.services.len() > MAX_SERVICES {
            return Err(Error::ValueOutOfRange {
                field: "number_of_services",
                reason: "exceeds 5-bit field",
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let count = fit_u8(self.services.len(), "number_of_services")?;
        let head = fit_bits(u64::from(self.reserved_head), 3, "reserved")? as u8;
        buf[HEADER_LEN] = (head << 5) | count;
        let mut i = HEADER_LEN + COUNT_BYTE_LEN;
        for s in &self.services {
            let ra = fit_bits(u64::from(s.reserved_a), 2, "reserved")? as u8;
            let num = fit_bits(
                u64::from(s.caption_service_number),
                6,
                "caption_service_number",
            )? as u8;
            let rb = fit_bits(u64::from(s.reserved_b), 1, "reserved")? as u8;
            let cs = fit_bits(u64::from(s.char_set.to_u8()), 6, "char_set")? as u8;
            buf[i..i + 3].copy_from_slice(&s.language.0);
            buf[i + 3] = (ra << 6) | num;
            buf[i + 4] = (rb << 7) | (u8::from(s.wide_aspect_ratio) << 6) | cs;
            buf[i + 5] = s.reserved_c;
            i += SERVICE_LEN;
        }
        let rp = fit_bits(u64::from(self.reserved_pid), 3, "reserved")? as u8;
        let [pid_hi, pid_lo] = self.caption_service_pid.value().to_be_bytes();
        buf[i] = (rp << 5) | pid_hi;
        buf[i + 1] = pid_lo;
        Ok(i + PID_LEN)
    }
}

impl crate::traits::DescriptorDef<'_> for ChinaCaptionServiceDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CHINA_CAPTION_SERVICE";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn round_trip(bytes: &[u8]) -> ChinaCaptionServiceDescriptor {
        let d = ChinaCaptionServiceDescriptor::parse(bytes).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        d
    }

    /// GY/T 270-2013 Table 8, one service: 'chi', caption_service_number 1,
    /// 16:9, char_set 2 (GB 18030-2005), caption_service_pid 0x0100, every
    /// reserved bit set. descriptor_length = 1 + 6 + 2 = 9.
    #[test]
    fn table_8_single_service() {
        let bytes = [TAG, 9, 0xE1, b'c', b'h', b'i', 0xC1, 0xC2, 0xFF, 0xE1, 0x00];
        let d = round_trip(&bytes);
        assert_eq!(d.services.len(), 1);
        let s = d.services[0];
        assert_eq!(s.language, LangCode(*b"chi"));
        assert_eq!(s.caption_service_number, 1);
        assert!(s.wide_aspect_ratio);
        assert_eq!(s.char_set, ChinaCaptionCharSet::Gb18030);
        assert_eq!((s.reserved_a, s.reserved_b, s.reserved_c), (3, 1, 0xFF));
        assert_eq!(d.caption_service_pid, Pid::new(0x0100));
        assert_eq!((d.reserved_head, d.reserved_pid), (7, 7));
    }

    /// Two services (primary GB 2312 4:3 service 1, secondary GB 13000.1 16:9
    /// service 2) sharing one caption PID 0x1FFE; length = 1 + 12 + 2 = 15.
    #[test]
    fn table_8_two_services() {
        let bytes = [
            TAG, 15, 0xE2, b'c', b'h', b'i', 0xC1, 0x80, 0xFF, b'e', b'n', b'g', 0xC2, 0xC1, 0xFF,
            0xFF, 0xFE,
        ];
        let d = round_trip(&bytes);
        assert_eq!(d.services[0].char_set, ChinaCaptionCharSet::Gb2312);
        assert!(!d.services[0].wide_aspect_ratio);
        assert_eq!(d.services[1].char_set, ChinaCaptionCharSet::Gb13000_1);
        assert_eq!(d.services[1].caption_service_number, 2);
        assert_eq!(d.caption_service_pid, Pid::new(0x1FFE));
    }

    #[test]
    fn rejects_bad_tag_length_and_truncation() {
        assert!(ChinaCaptionServiceDescriptor::parse(&[0x87, 3, 0xE0, 0xE0, 0]).is_err());
        // number_of_services=1 but only the empty-body length
        assert!(ChinaCaptionServiceDescriptor::parse(&[TAG, 3, 0xE1, 0xE0, 0]).is_err());
        assert!(ChinaCaptionServiceDescriptor::parse(&[TAG, 2, 0xE0, 0xE0]).is_err());
        assert!(ChinaCaptionServiceDescriptor::parse(&[TAG, 9, 0xE1]).is_err());
    }

    #[test]
    fn empty_service_list_round_trips() {
        let bytes = [TAG, 3, 0xE0, 0xE1, 0x23];
        let d = round_trip(&bytes);
        assert!(d.services.is_empty());
        assert_eq!(d.caption_service_pid, Pid::new(0x0123));
    }

    #[test]
    fn serialize_rejects_overwide_fields() {
        let svc =
            ChinaCaptionService::new(LangCode(*b"chi"), 64, false, ChinaCaptionCharSet::Gb2312);
        let d = ChinaCaptionServiceDescriptor::new(vec![svc], Pid::new(0x100));
        assert!(d.serialize_into(&mut [0u8; 16]).is_err());
        let svc = ChinaCaptionService::new(
            LangCode(*b"chi"),
            1,
            false,
            ChinaCaptionCharSet::Reserved(0x7F),
        );
        let d = ChinaCaptionServiceDescriptor::new(vec![svc; 1], Pid::new(0x100));
        let mut bad = d.clone();
        bad.reserved_pid = 8;
        assert!(bad.serialize_into(&mut [0u8; 16]).is_err());
        let many = ChinaCaptionServiceDescriptor::new(vec![svc; 32], Pid::new(0x100));
        assert!(many.serialize_into(&mut [0u8; 256]).is_err());
    }

    #[test]
    fn char_set_is_lossless_and_labelled() {
        for v in 0u8..64 {
            assert_eq!(ChinaCaptionCharSet::from_u8(v).to_u8(), v);
        }
        assert_eq!(ChinaCaptionCharSet::Gb18030.name(), "GB 18030-2005");
        assert_eq!(
            ChinaCaptionCharSet::Reserved(9).to_string(),
            "reserved(0x09)"
        );
    }

    #[test]
    fn built_defaults_are_all_ones() {
        let d = ChinaCaptionServiceDescriptor::new(
            vec![ChinaCaptionService::new(
                LangCode(*b"chi"),
                1,
                true,
                ChinaCaptionCharSet::Gb18030,
            )],
            Pid::new(0x100),
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out,
            [TAG, 9, 0xE1, b'c', b'h', b'i', 0xC1, 0xC2, 0xFF, 0xE1, 0x00]
        );
    }
}
