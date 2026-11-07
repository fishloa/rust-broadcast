//! The transport-stream-loop section shared by the NIT (ETSI EN 300 468
//! §5.2.1) and the BAT (§5.2.2): a 16-bit `table_id_extension` (`network_id`
//! / `bouquet_id`), the version / section-number header, a first descriptor
//! loop, and a `transport_stream_loop` of [`TransportStreamEntry`].

use super::{CRC_LEN, SectionBits};
use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::len::need_out;

pub(super) const MIN_HEADER_LEN: usize = 3;
pub(super) const EXTENSION_HEADER_LEN: usize = 5;
pub(super) const POST_EXTENSION_LEN: usize = 2;
pub(super) const MIN_SECTION_LEN: usize =
    MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + CRC_LEN;
pub(super) const TS_HEADER_LEN: usize = 6;
pub(super) const LOOP_LENGTH_FIELD_LEN: usize = 2;

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
    pub reserved_future_use: u8,
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
            reserved_future_use: super::RESERVED_4,
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

    let id = crate::wire::be_u16(bytes, 3, labels.extension)?;
    let desc_length = super::desc_loop_len_of(bytes[8], bytes[9]);
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
        let transport_descriptors_length = super::desc_loop_len_of(hdr[4], hdr[5]);
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
            original_network_id: crate::wire::be_u16(hdr, 2, labels.extension)?,
            descriptors: DescriptorLoop::new(&bytes[ts_desc_start..ts_desc_end]),
            reserved_future_use: super::len_reserved_of(hdr[4]),
        });
        pos = ts_desc_end;
    }

    Ok(Parsed {
        id,
        version_number: super::version_number_of(bytes[5]),
        current_next_indicator: super::current_next_of(bytes[5]),
        section_number: bytes[6],
        last_section_number: bytes[7],
        descriptors: DescriptorLoop::new(&bytes[desc_start..desc_end]),
        transport_streams,
        bits: SectionBits::parse(bytes[1], Some(bytes[5])),
        reserved_descriptors_length: super::len_reserved_of(bytes[8]),
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
    buf[3..5].copy_from_slice(&view.id.to_be_bytes());
    buf[5] = view
        .bits
        .b5(view.version_number, view.current_next_indicator)?;
    buf[6] = view.section_number;
    buf[7] = view.last_section_number;

    let desc_length = broadcast_common::len::fit_bits(
        view.descriptors.len() as u64,
        12,
        view.descriptors_length_what,
    )?;
    super::write_desc_loop_len_r(
        &mut buf[8..10],
        desc_length as usize,
        view.reserved_descriptors_length,
    )?;
    let desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
    buf[desc_start..desc_start + view.descriptors.len()].copy_from_slice(view.descriptors.raw());

    let ts_loop_start = desc_start + view.descriptors.len();
    let ts_loop_length = broadcast_common::len::fit_bits(
        (len - ts_loop_start - LOOP_LENGTH_FIELD_LEN - CRC_LEN) as u64,
        12,
        "transport_stream_loop_length",
    )?;
    super::write_desc_loop_len_r(
        &mut buf[ts_loop_start..ts_loop_start + LOOP_LENGTH_FIELD_LEN],
        ts_loop_length as usize,
        view.reserved_transport_stream_loop_length,
    )?;

    let mut pos = ts_loop_start + LOOP_LENGTH_FIELD_LEN;
    for ts in view.transport_streams {
        buf[pos..pos + 2].copy_from_slice(&ts.transport_stream_id.to_be_bytes());
        buf[pos + 2..pos + 4].copy_from_slice(&ts.original_network_id.to_be_bytes());
        let ts_desc_length = broadcast_common::len::fit_bits(
            ts.descriptors.len() as u64,
            12,
            "transport_descriptors_length",
        )?;
        super::write_desc_loop_len_r(
            &mut buf[pos + 4..pos + 6],
            ts_desc_length as usize,
            ts.reserved_future_use,
        )?;
        let ts_desc_start = pos + TS_HEADER_LEN;
        buf[ts_desc_start..ts_desc_start + ts.descriptors.len()]
            .copy_from_slice(ts.descriptors.raw());
        pos = ts_desc_start + ts.descriptors.len();
    }

    super::write_crc(buf, len - CRC_LEN);
    Ok(len)
}
