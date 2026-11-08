//! Transport Stream Description Table — ISO/IEC 13818-1 (H.222.0 08/2023) §2.4.4.13.
//!
//! TSDT is carried on PID 0x0002 with table_id 0x03. It provides a
//! means of describing the transport stream using descriptors.

use crate::descriptors::DescriptorLoop;
use crate::error::Result;
use broadcast_common::{Parse, Serialize};

/// table_id for Transport Stream Description Table.
pub const TABLE_ID: u8 = 0x03;
/// Well-known PID on which TSDT is carried.
pub const PID: mpeg_ts::pid::Pid = mpeg_ts::pid::Pid::new(0x0002);

/// Transport Stream Description Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct TsdtSection<'a> {
    /// 16-bit table_id_extension.
    pub table_id_extension: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// Descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub descriptors: DescriptorLoop<'a>,
    /// Header bit fields (`'0'` after `section_syntax_indicator`, the two
    /// `reserved(2)`; default [`SectionBits::psi`](super::SectionBits::psi)).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bits: super::SectionBits,
}

impl<'a> TsdtSection<'a> {
    /// Construct a TSDT section with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        table_id_extension: u16,
        version_number: u8,
        current_next_indicator: bool,
        section_number: u8,
        last_section_number: u8,
        descriptors: DescriptorLoop<'a>,
    ) -> Self {
        Self {
            table_id_extension,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            descriptors,
            bits: super::SectionBits::psi(),
        }
    }
}

impl<'a> Parse<'a> for TsdtSection<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let p = super::descriptor_section::parse(bytes, &[TABLE_ID], "TsdtSection", "tsdt")?;
        Ok(TsdtSection {
            table_id_extension: p.table_id_extension,
            version_number: p.version_number,
            current_next_indicator: p.current_next_indicator,
            section_number: p.section_number,
            last_section_number: p.last_section_number,
            descriptors: p.descriptors,
            bits: p.bits,
        })
    }
}

impl Serialize for TsdtSection<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        super::descriptor_section::serialized_len(&self.descriptors)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        super::descriptor_section::serialize_into(
            TABLE_ID,
            &super::descriptor_section::View {
                table_id_extension: self.table_id_extension,
                version_number: self.version_number,
                current_next_indicator: self.current_next_indicator,
                section_number: self.section_number,
                last_section_number: self.last_section_number,
                descriptors: &self.descriptors,
                bits: &self.bits,
            },
            buf,
        )
    }
}
impl<'a> crate::traits::TableDef<'a> for TsdtSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "TRANSPORT_STREAM_DESCRIPTION";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::tables::CRC_LEN;
    use alloc::{vec, vec::Vec};

    const EXTENSION_HEADER_LEN: usize = 5;

    fn build_tsdt(table_id_extension: u16, version: u8, descriptors: &[u8]) -> Vec<u8> {
        let section_length: u16 = (EXTENSION_HEADER_LEN + descriptors.len() + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID);
        v.push(super::super::SECTION_B1_FLAGS_PSI | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&table_id_extension.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0x00);
        v.push(0x00);
        v.extend_from_slice(descriptors);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_tsdt(0x1234, 5, &[]);
        bytes[0] = 0x02;
        assert!(matches!(
            TsdtSection::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x02, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = TsdtSection::parse(&[0x03, 0xB0]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort(..)));
    }

    #[test]
    fn parse_empty_descriptor_loop() {
        let bytes = build_tsdt(0x1234, 5, &[]);
        let tsdt = TsdtSection::parse(&bytes).unwrap();
        assert_eq!(tsdt.table_id_extension, 0x1234);
        assert_eq!(tsdt.version_number, 5);
        assert!(tsdt.current_next_indicator);
        assert_eq!(tsdt.section_number, 0);
        assert_eq!(tsdt.last_section_number, 0);
        assert!(tsdt.descriptors.is_empty());
    }

    #[test]
    fn parse_with_descriptors() {
        let descriptors = [0x01, 0x03, 0xAA, 0xBB, 0xCC];
        let bytes = build_tsdt(0xABCD, 7, &descriptors);
        let tsdt = TsdtSection::parse(&bytes).unwrap();
        assert_eq!(tsdt.table_id_extension, 0xABCD);
        assert_eq!(tsdt.version_number, 7);
        assert_eq!(tsdt.descriptors.raw(), &descriptors[..]);
    }

    #[test]
    fn serialize_round_trip() {
        let descriptors = [0x4D, 0x02, 0x01, 0x02];
        let bytes = build_tsdt(0xCAFE, 3, &descriptors);
        let tsdt = TsdtSection::parse(&bytes).unwrap();
        let mut buf = vec![0u8; tsdt.serialized_len()];
        tsdt.serialize_into(&mut buf).unwrap();
        let re = TsdtSection::parse(&buf).unwrap();
        assert_eq!(tsdt, re);
    }

    #[test]
    fn serialize_round_trip_empty() {
        let bytes = build_tsdt(0x0001, 0, &[]);
        let tsdt = TsdtSection::parse(&bytes).unwrap();
        let mut buf = vec![0u8; tsdt.serialized_len()];
        tsdt.serialize_into(&mut buf).unwrap();
        let re = TsdtSection::parse(&buf).unwrap();
        assert_eq!(tsdt, re);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(TABLE_ID, 0x03);
        assert_eq!(PID.value(), 0x0002);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn tsdt_serializes_typed_loop() {
        // TSDT now borrows its descriptor loop (3.0): serialize-only, the loop
        // emits the typed descriptor sequence (here a short_event, tag 0x4D).
        let descriptors = [0x4D, 0x02, b'e', b'n']; // valid short_event header bytes
        let bytes = build_tsdt(0xDEAD, 9, &descriptors);
        let tsdt = TsdtSection::parse(&bytes).unwrap();
        let v = serde_json::to_value(&tsdt).unwrap();
        assert!(
            v["descriptors"].is_array(),
            "descriptors must serialize as a typed sequence, got {v}"
        );
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            TsdtSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    /// section_length is 12 bits (max 4095 = EXTENSION_HEADER_LEN(5) +
    /// descriptors + CRC_LEN(4), so 4086 is the largest loop that fits).
    #[test]
    fn serialize_rejects_descriptor_loop_exceeding_section_length() {
        let raw = vec![0xAAu8; 4087];
        let tsdt = TsdtSection::new(1, 0, true, 0, 0, DescriptorLoop::from(raw.as_slice()));
        let mut buf = vec![0u8; tsdt.serialized_len()];
        assert!(matches!(
            tsdt.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[test]
    fn serialize_accepts_descriptor_loop_at_section_length_boundary() {
        let raw = vec![0xAAu8; 4086];
        let tsdt = TsdtSection::new(1, 0, true, 0, 0, DescriptorLoop::from(raw.as_slice()));
        let mut buf = vec![0u8; tsdt.serialized_len()];
        tsdt.serialize_into(&mut buf).expect("boundary fits");
        let reparsed = TsdtSection::parse(&buf).expect("reparse");
        assert_eq!(tsdt.descriptors.raw(), reparsed.descriptors.raw());
    }
}
