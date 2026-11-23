//! The transport-stream-loop section shared by the NIT (ETSI EN 300 468
//! §5.2.1) and the BAT (§5.2.2): a 16-bit `table_id_extension` (`network_id`
//! / `bouquet_id`), the version / section-number header, a first descriptor
//! loop, and a `transport_stream_loop` of [`TransportStreamEntry`].

use super::{CRC_LEN, SectionBits};
use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::len::need_out;
use broadcast_common::reserved::ReservedOnes;

/// Width of the `reserved_future_use` length fields.
const TS_LOOP_RESERVED_BITS: u32 = 4;

pub(super) const MIN_HEADER_LEN: usize = 3;
pub(super) const EXTENSION_HEADER_LEN: usize = 5;
pub(super) const POST_EXTENSION_LEN: usize = 2;
pub(super) const MIN_SECTION_LEN: usize =
    MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + CRC_LEN;
pub(super) const TS_HEADER_LEN: usize = 6;
pub(super) const LOOP_LENGTH_FIELD_LEN: usize = 2;

/// Byte offsets in the section header.
const ID_OFFSET: usize = 3;
const DESC_LENGTH_OFFSET: usize = 8;

/// Byte offsets inside one transport-stream entry.
const TS_TSID_LEN: usize = 2;
const TS_DESC_LENGTH_OFFSET: usize = 4;

/// One transport-stream entry inside a NIT or BAT `transport_stream_loop`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct TransportStreamEntry<'a> {
    /// transport_stream_id of the described TS.
    pub transport_stream_id: u16,
    /// original_network_id of the described TS.
    pub original_network_id: u16,
    /// Per-TS descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub descriptors: DescriptorLoop<'a>,
    /// `reserved_future_use(4)` before `transport_descriptors_length`
    /// (EN 300 468 §5.2.1 / §5.2.2, `1111`); preserved from the wire.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_future_use: ReservedOnes<TS_LOOP_RESERVED_BITS>,
}

impl<'a> TransportStreamEntry<'a> {
    /// Construct a transport-stream entry with the spec `reserved_future_use` fill.
    #[must_use]
    pub fn new(
        transport_stream_id: u16,
        original_network_id: u16,
        descriptors: DescriptorLoop<'a>,
    ) -> Self {
        Self {
            transport_stream_id,
            original_network_id,
            descriptors,
            reserved_future_use: ReservedOnes::FILL,
        }
    }
}

pub(super) struct Labels {
    pub(super) section: &'static str,
    pub(super) loop_header: &'static str,
    pub(super) trailing: &'static str,
    pub(super) entry: &'static str,
    pub(super) extension: &'static str,
}

pub(super) struct Parsed<'a> {
    pub(super) id: u16,
    pub(super) version_number: u8,
    pub(super) current_next_indicator: bool,
    pub(super) section_number: u8,
    pub(super) last_section_number: u8,
    pub(super) descriptors: DescriptorLoop<'a>,
    pub(super) transport_streams: Vec<TransportStreamEntry<'a>>,
    pub(super) bits: SectionBits,
    pub(super) reserved_descriptors_length: u8,
    pub(super) reserved_transport_stream_loop_length: u8,
}

pub(super) struct View<'v, 'a> {
    pub(super) id: u16,
    pub(super) version_number: u8,
    pub(super) current_next_indicator: bool,
    pub(super) section_number: u8,
    pub(super) last_section_number: u8,
    pub(super) descriptors: &'v DescriptorLoop<'a>,
    pub(super) transport_streams: &'v [TransportStreamEntry<'a>],
    pub(super) bits: &'v SectionBits,
    pub(super) reserved_descriptors_length: u8,
    pub(super) reserved_transport_stream_loop_length: u8,
    pub(super) descriptors_length_what: &'static str,
}

pub(super) fn parse<'a>(bytes: &'a [u8], labels: &Labels) -> Result<Parsed<'a>> {
    let total = super::check_section_length(
        bytes.len(),
        MIN_HEADER_LEN,
        super::section_length_of(bytes),
        MIN_SECTION_LEN,
    )?;

    let id = crate::wire::be_u16(bytes, ID_OFFSET, labels.extension)?;
    let desc_length =
        super::desc_loop_len_of(bytes[DESC_LENGTH_OFFSET], bytes[DESC_LENGTH_OFFSET + 1]);
    let desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
    let desc_end = desc_start + desc_length;
    let body_end = total - CRC_LEN;
    if desc_end > body_end {
        return Err(Error::SectionLengthOverflow {
            declared: desc_length,
            available: body_end.saturating_sub(desc_start),
        });
    }

    let ts_loop_start = desc_end;
    if body_end - ts_loop_start < LOOP_LENGTH_FIELD_LEN {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: body_end - ts_loop_start + LOOP_LENGTH_FIELD_LEN,
                have: body_end - ts_loop_start,
                what: labels.loop_header,
            },
        ));
    }
    let ts_loop_length = super::desc_loop_len_of(bytes[ts_loop_start], bytes[ts_loop_start + 1]);
    let mut pos = ts_loop_start + LOOP_LENGTH_FIELD_LEN;
    let loop_end = pos + ts_loop_length;
    if loop_end > body_end {
        return Err(Error::SectionLengthOverflow {
            declared: ts_loop_length,
            available: body_end - pos,
        });
    }
    super::check_loop_reaches_crc(loop_end, body_end, labels.trailing)?;

    let mut transport_streams = Vec::new();
    while pos < loop_end {
        if pos + TS_HEADER_LEN > loop_end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: pos + TS_HEADER_LEN,
                    have: loop_end,
                    what: labels.entry,
                },
            ));
        }
        let hdr = &bytes[pos..pos + TS_HEADER_LEN];
        let transport_descriptors_length =
            super::desc_loop_len_of(hdr[TS_DESC_LENGTH_OFFSET], hdr[TS_DESC_LENGTH_OFFSET + 1]);
        let ts_desc_start = pos + TS_HEADER_LEN;
        let ts_desc_end = ts_desc_start + transport_descriptors_length;
        if ts_desc_end > loop_end {
            return Err(Error::SectionLengthOverflow {
                declared: transport_descriptors_length,
                available: loop_end - ts_desc_start,
            });
        }
        transport_streams.push(TransportStreamEntry {
            transport_stream_id: crate::wire::be_u16(hdr, 0, labels.extension)?,
            original_network_id: crate::wire::be_u16(hdr, TS_TSID_LEN, labels.extension)?,
            descriptors: DescriptorLoop::new(&bytes[ts_desc_start..ts_desc_end]),
            reserved_future_use: ReservedOnes::from_bits(u64::from(super::len_reserved_of(
                hdr[TS_DESC_LENGTH_OFFSET],
            ))),
        });
        pos = ts_desc_end;
    }

    Ok(Parsed {
        id,
        version_number: super::version_number_of(bytes[super::VERSION_FLAGS_OFFSET]),
        current_next_indicator: super::current_next_of(bytes[super::VERSION_FLAGS_OFFSET]),
        section_number: bytes[super::SECTION_NUMBER_OFFSET],
        last_section_number: bytes[super::LAST_SECTION_NUMBER_OFFSET],
        descriptors: DescriptorLoop::new(&bytes[desc_start..desc_end]),
        transport_streams,
        bits: SectionBits::parse(bytes[1], Some(bytes[super::VERSION_FLAGS_OFFSET])),
        reserved_descriptors_length: super::len_reserved_of(bytes[DESC_LENGTH_OFFSET]),
        reserved_transport_stream_loop_length: super::len_reserved_of(bytes[ts_loop_start]),
    })
}

pub(super) fn serialized_len(
    descriptors: &DescriptorLoop<'_>,
    transport_streams: &[TransportStreamEntry<'_>],
) -> usize {
    let ts_bytes: usize = transport_streams
        .iter()
        .map(|ts| TS_HEADER_LEN + ts.descriptors.len())
        .sum();
    MIN_HEADER_LEN
        + EXTENSION_HEADER_LEN
        + POST_EXTENSION_LEN
        + descriptors.len()
        + LOOP_LENGTH_FIELD_LEN
        + ts_bytes
        + CRC_LEN
}

pub(super) fn serialize_into(table_id: u8, view: &View<'_, '_>, buf: &mut [u8]) -> Result<usize> {
    let len = serialized_len(view.descriptors, view.transport_streams);
    need_out(buf, len)?;

    buf[0] = table_id;
    buf[1] = view.bits.b1()?;
    super::write_section_length(buf, len - MIN_HEADER_LEN)?;
    buf[ID_OFFSET..super::VERSION_FLAGS_OFFSET].copy_from_slice(&view.id.to_be_bytes());
    buf[super::VERSION_FLAGS_OFFSET] = view
        .bits
        .b5(view.version_number, view.current_next_indicator)?;
    buf[super::SECTION_NUMBER_OFFSET] = view.section_number;
    buf[super::LAST_SECTION_NUMBER_OFFSET] = view.last_section_number;

    let desc_length = broadcast_common::len::fit_bits(
        view.descriptors.len() as u64,
        super::DESC_LOOP_LEN_BITS,
        view.descriptors_length_what,
    )?;
    super::write_desc_loop_len_r(
        &mut buf[DESC_LENGTH_OFFSET..DESC_LENGTH_OFFSET + LOOP_LENGTH_FIELD_LEN],
        desc_length as usize,
        view.reserved_descriptors_length,
    )?;
    let desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
    buf[desc_start..desc_start + view.descriptors.len()].copy_from_slice(view.descriptors.raw());

    let ts_loop_start = desc_start + view.descriptors.len();
    let ts_loop_length = broadcast_common::len::fit_bits(
        (len - ts_loop_start - LOOP_LENGTH_FIELD_LEN - CRC_LEN) as u64,
        super::DESC_LOOP_LEN_BITS,
        "transport_stream_loop_length",
    )?;
    super::write_desc_loop_len_r(
        &mut buf[ts_loop_start..ts_loop_start + LOOP_LENGTH_FIELD_LEN],
        ts_loop_length as usize,
        view.reserved_transport_stream_loop_length,
    )?;

    let mut pos = ts_loop_start + LOOP_LENGTH_FIELD_LEN;
    for ts in view.transport_streams {
        buf[pos..pos + TS_TSID_LEN].copy_from_slice(&ts.transport_stream_id.to_be_bytes());
        buf[pos + TS_TSID_LEN..pos + TS_DESC_LENGTH_OFFSET]
            .copy_from_slice(&ts.original_network_id.to_be_bytes());
        let ts_desc_length = broadcast_common::len::fit_bits(
            ts.descriptors.len() as u64,
            super::DESC_LOOP_LEN_BITS,
            "transport_descriptors_length",
        )?;
        super::write_desc_loop_len_r(
            &mut buf[pos + TS_DESC_LENGTH_OFFSET..pos + TS_HEADER_LEN],
            ts_desc_length as usize,
            ts.reserved_future_use.get() as u8,
        )?;
        let ts_desc_start = pos + TS_HEADER_LEN;
        buf[ts_desc_start..ts_desc_start + ts.descriptors.len()]
            .copy_from_slice(ts.descriptors.raw());
        pos = ts_desc_start + ts.descriptors.len();
    }

    super::write_crc(buf, len - CRC_LEN);
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field is 4 bits wide, so a wider value is unrepresentable and cannot
    /// spill into the 12-bit transport_descriptors_length below it.
    #[test]
    fn reserved_future_use_cannot_hold_more_than_four_bits() {
        let e = TransportStreamEntry {
            reserved_future_use: ReservedOnes::from_bits(0xFF),
            ..TransportStreamEntry::new(1, 2, DescriptorLoop::new(&[]))
        };
        assert_eq!(e.reserved_future_use.get(), 0x0F);
    }
}
