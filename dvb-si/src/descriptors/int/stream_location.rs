//! IP/MAC_stream_location_descriptor — ETSI EN 301 192 V1.7.1 §8.4.5.14
//! Table 32 (tag 0x13).
//!
//! Locates the IP/MAC stream in a DVB network (an MPEG-2 TS) through
//! `network_id`, `original_network_id`, `transport_stream_id`, `service_id`
//! and `component_tag`. Allowed in the INT platform and operational loops
//! (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::len::need_out;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for IP/MAC_stream_location_descriptor.
pub const TAG: u8 = 0x13;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 9;
/// Offset of `network_id` in the body.
const NETWORK_ID_OFFSET: usize = 0;
/// Offset of `original_network_id` in the body.
const ORIGINAL_NETWORK_ID_OFFSET: usize = NETWORK_ID_OFFSET + size_of::<u16>();
/// Offset of `transport_stream_id` in the body.
const TRANSPORT_STREAM_ID_OFFSET: usize = ORIGINAL_NETWORK_ID_OFFSET + size_of::<u16>();
/// Offset of `service_id` in the body.
const SERVICE_ID_OFFSET: usize = TRANSPORT_STREAM_ID_OFFSET + size_of::<u16>();
/// Offset of `component_tag` in the body.
const COMPONENT_TAG_OFFSET: usize = SERVICE_ID_OFFSET + size_of::<u16>();

/// IP/MAC_stream_location_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct IpMacStreamLocationDescriptor<'a> {
    /// `network_id` — identifies the delivery system (ETSI TS 101 162).
    pub network_id: u16,
    /// `original_network_id` of the originating delivery system.
    pub original_network_id: u16,
    /// `transport_stream_id` of the TS carrying the stream.
    pub transport_stream_id: u16,
    /// `service_id` — the `program_number` of the PMT.
    pub service_id: u16,
    /// `component_tag` of the component stream.
    pub component_tag: u8,
    /// Bytes after the 9 defined body bytes inside `descriptor_length`; the
    /// spec defines none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> Parse<'a> for IpMacStreamLocationDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "IpMacStreamLocationDescriptor",
            "unexpected tag for IP/MAC_stream_location_descriptor",
        )?;
        let (f, trailing) =
            body.split_first_chunk::<BODY_LEN>()
                .ok_or(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "IP/MAC_stream_location_descriptor body shorter than 9 bytes",
                })?;
        Ok(Self {
            network_id: crate::wire::be_u16(f, NETWORK_ID_OFFSET, "network_id")?,
            original_network_id: crate::wire::be_u16(
                f,
                ORIGINAL_NETWORK_ID_OFFSET,
                "original_network_id",
            )?,
            transport_stream_id: crate::wire::be_u16(
                f,
                TRANSPORT_STREAM_ID_OFFSET,
                "transport_stream_id",
            )?,
            service_id: crate::wire::be_u16(f, SERVICE_ID_OFFSET, "service_id")?,
            component_tag: f[COMPONENT_TAG_OFFSET],
            trailing,
        })
    }
}

impl Serialize for IpMacStreamLocationDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[HEADER_LEN + NETWORK_ID_OFFSET..HEADER_LEN + ORIGINAL_NETWORK_ID_OFFSET]
            .copy_from_slice(&self.network_id.to_be_bytes());
        buf[HEADER_LEN + ORIGINAL_NETWORK_ID_OFFSET..HEADER_LEN + TRANSPORT_STREAM_ID_OFFSET]
            .copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[HEADER_LEN + TRANSPORT_STREAM_ID_OFFSET..HEADER_LEN + SERVICE_ID_OFFSET]
            .copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[HEADER_LEN + SERVICE_ID_OFFSET..HEADER_LEN + COMPONENT_TAG_OFFSET]
            .copy_from_slice(&self.service_id.to_be_bytes());
        buf[HEADER_LEN + COMPONENT_TAG_OFFSET] = self.component_tag;
        buf[HEADER_LEN + BODY_LEN..len].copy_from_slice(self.trailing);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for IpMacStreamLocationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "IP_MAC_STREAM_LOCATION";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse_and_round_trip() {
        // TSDuck test-022: network 0x1234, onid 0x5678, tsid 0x9ABC,
        // service 0x6543, component_tag 0x21.
        let bytes = [TAG, 9, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0x65, 0x43, 0x21];
        let d = IpMacStreamLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.network_id, 0x1234);
        assert_eq!(d.original_network_id, 0x5678);
        assert_eq!(d.transport_stream_id, 0x9ABC);
        assert_eq!(d.service_id, 0x6543);
        assert_eq!(d.component_tag, 0x21);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn trailing_round_trip_and_rejects_short() {
        let bytes = [TAG, 11, 0, 1, 0, 2, 0, 3, 0, 4, 5, 0xEE, 0xFF];
        let d = IpMacStreamLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.trailing, &[0xEE, 0xFF]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert!(matches!(
            IpMacStreamLocationDescriptor::parse(&[TAG, 8, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(Error::InvalidDescriptor { .. })
        ));
    }
}
