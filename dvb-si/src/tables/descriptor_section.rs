//! Shared wire core of the descriptor-only long sections: the CAT
//! (ISO/IEC 13818-1 §2.4.4.7) and TSDT (§2.4.4.13) both carry a 16-bit
//! `table_id_extension`, the version / section-number header, and one
//! descriptor loop that runs from byte 8 to the CRC with no length field.

use super::{CRC_LEN, SectionBits};
use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use broadcast_common::len::need_out;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const MIN_SECTION_LEN: usize = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + CRC_LEN;

pub(super) struct Parsed<'a> {
    pub(super) table_id_extension: u16,
    pub(super) version_number: u8,
    pub(super) current_next_indicator: bool,
    pub(super) section_number: u8,
    pub(super) last_section_number: u8,
    pub(super) descriptors: DescriptorLoop<'a>,
    pub(super) bits: SectionBits,
}

pub(super) struct View<'v, 'a> {
    pub(super) table_id_extension: u16,
    pub(super) version_number: u8,
    pub(super) current_next_indicator: bool,
    pub(super) section_number: u8,
    pub(super) last_section_number: u8,
    pub(super) descriptors: &'v DescriptorLoop<'a>,
    pub(super) bits: &'v SectionBits,
}

pub(super) fn parse<'a>(
    bytes: &'a [u8],
    expected: &'static [u8],
    what: &'static str,
    ext_what: &'static str,
) -> Result<Parsed<'a>> {
    if bytes.len() < MIN_SECTION_LEN {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what,
            },
        ));
    }
    if !expected.contains(&bytes[0]) {
        return Err(Error::UnexpectedTableId {
            table_id: bytes[0],
            what,
            expected,
        });
    }
    let total = super::check_section_length(
        bytes.len(),
        MIN_HEADER_LEN,
        super::section_length_of(bytes),
        MIN_SECTION_LEN,
    )?;
    let desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
    Ok(Parsed {
        table_id_extension: crate::wire::be_u16(bytes, 3, ext_what)?,
        version_number: super::version_number_of(bytes[5]),
        current_next_indicator: super::current_next_of(bytes[5]),
        section_number: bytes[6],
        last_section_number: bytes[7],
        descriptors: DescriptorLoop::new(&bytes[desc_start..total - CRC_LEN]),
        bits: SectionBits::parse(bytes[1], Some(bytes[5])),
    })
}

pub(super) fn serialized_len(descriptors: &DescriptorLoop<'_>) -> usize {
    MIN_HEADER_LEN + EXTENSION_HEADER_LEN + descriptors.len() + CRC_LEN
}

pub(super) fn serialize_into(table_id: u8, view: &View<'_, '_>, buf: &mut [u8]) -> Result<usize> {
    let len = serialized_len(view.descriptors);
    need_out(buf, len)?;
    buf[0] = table_id;
    buf[1] = view.bits.b1()?;
    super::write_section_length(buf, len - MIN_HEADER_LEN)?;
    buf[3..5].copy_from_slice(&view.table_id_extension.to_be_bytes());
    buf[5] = view
        .bits
        .b5(view.version_number, view.current_next_indicator)?;
    buf[6] = view.section_number;
    buf[7] = view.last_section_number;
    let desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN;
    buf[desc_start..desc_start + view.descriptors.len()].copy_from_slice(view.descriptors.raw());
    super::write_crc(buf, len - CRC_LEN);
    Ok(len)
}
