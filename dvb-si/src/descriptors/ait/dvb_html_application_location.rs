//! DVB-HTML Application Location Descriptor — ETSI TS 102 727 V1.1.1 §10.10.2,
//! Table 87 (AIT tag 0x09; physical-root semantics Table 88). PDF p.173,
//! transcribed in `docs/descriptors/ts_102_727/87-*.md`, verified against the
//! rendered page.
//!
//! One instance in the "application" AIT loop per DVB-HTML application. Body:
//! `physical_root_length`(8), `physical_root_bytes`×N1, then
//! `initial_path_bytes` for the remainder of the descriptor. Both are text
//! strings (modelled as [`DvbText`], like the sibling DVB-J location and the
//! simple_application_location descriptors).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::DvbText;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for dvb_html_application_location_descriptor (AIT namespace).
pub const TAG: u8 = 0x09;
const HEADER_LEN: usize = 2;
const ROOT_LEN_FIELD: usize = 1;

/// DVB-HTML Application Location Descriptor (AIT tag 0x09).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DvbHtmlApplicationLocationDescriptor<'a> {
    /// `physical_root_bytes` — physical root of the entry point; transport
    /// specific (a directory for `protocol_id` 0x0001, one of the base URLs of
    /// the transport_protocol_descriptor for 0x0002; Table 88).
    pub physical_root: DvbText<'a>,
    /// `initial_path_bytes` — URL path of the entry document, relative to the
    /// physical root. Consumes the rest of the descriptor.
    pub initial_path: DvbText<'a>,
}

impl<'a> Parse<'a> for DvbHtmlApplicationLocationDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DvbHtmlApplicationLocationDescriptor",
            "unexpected tag for dvb_html_application_location_descriptor",
        )?;
        let root_len = *body.first().ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "dvb_html_application_location_descriptor missing physical_root_length",
        })? as usize;
        let end = ROOT_LEN_FIELD + root_len;
        if body.len() < end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "dvb_html_application_location_descriptor physical_root overruns body",
            });
        }
        Ok(Self {
            physical_root: DvbText::new(&body[ROOT_LEN_FIELD..end]),
            initial_path: DvbText::new(&body[end..]),
        })
    }
}

impl Serialize for DvbHtmlApplicationLocationDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + ROOT_LEN_FIELD + self.physical_root.len() + self.initial_path.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let root_len =
            broadcast_common::len::fit_u8(self.physical_root.len(), "physical_root_length")?;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[HEADER_LEN] = root_len;
        let root_end = HEADER_LEN + ROOT_LEN_FIELD + self.physical_root.len();
        buf[HEADER_LEN + ROOT_LEN_FIELD..root_end].copy_from_slice(self.physical_root.raw());
        buf[root_end..len].copy_from_slice(self.initial_path.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for DvbHtmlApplicationLocationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DVB_HTML_APPLICATION_LOCATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TS 102 727 §10.10.2.2 example: physical root `application/`, initial
    /// path `main/index.htm`.
    fn spec_example() -> Vec<u8> {
        let mut v = vec![TAG, 0, 12];
        v.extend_from_slice(b"application/");
        v.extend_from_slice(b"main/index.htm");
        v[1] = u8::try_from(v.len() - 2).unwrap();
        v
    }

    #[test]
    fn parse_spec_example() {
        let bytes = spec_example();
        let d = DvbHtmlApplicationLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.physical_root.raw(), b"application/");
        assert_eq!(d.initial_path.raw(), b"main/index.htm");
    }

    #[test]
    fn round_trip_spec_example_and_empty_root() {
        for bytes in [
            spec_example(),
            vec![TAG, 1, 0],
            vec![TAG, 4, 0, b'a', b'/', b'b'],
        ] {
            let d = DvbHtmlApplicationLocationDescriptor::parse(&bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
            assert_eq!(
                DvbHtmlApplicationLocationDescriptor::parse(&out).unwrap(),
                d
            );
        }
    }

    #[test]
    fn malformed_rejected() {
        assert!(DvbHtmlApplicationLocationDescriptor::parse(&[TAG, 0]).is_err());
        assert!(DvbHtmlApplicationLocationDescriptor::parse(&[TAG, 2, 5, b'a']).is_err());
        let big = vec![b'x'; 256];
        let d = DvbHtmlApplicationLocationDescriptor {
            physical_root: DvbText::new(&big),
            initial_path: DvbText::new(b""),
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
