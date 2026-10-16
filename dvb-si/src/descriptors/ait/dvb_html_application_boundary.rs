//! DVB-HTML Application Boundary Descriptor — ETSI TS 102 727 V1.1.1 §10.10.3,
//! Table 89 (AIT tag 0x0A). PDF p.174, transcribed in
//! `docs/descriptors/ts_102_727/89-*.md`, verified against the rendered page.
//!
//! Optional, in the "application" AIT loop; several may be present (the
//! boundary is their union). Body: `label_length`(8), `label_bytes`×N1, then
//! `regular_expression_bytes` for the remainder of the descriptor.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::DvbText;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for dvb_html_application_boundary_descriptor (AIT namespace).
pub const TAG: u8 = 0x0A;
const HEADER_LEN: usize = 2;
const LABEL_LEN_FIELD: usize = 1;

/// DVB-HTML Application Boundary Descriptor (AIT tag 0x0A).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DvbHtmlApplicationBoundaryDescriptor<'a> {
    /// `label_bytes` — label associated with the data set the regular
    /// expression selects (usable for pre-fetching, transport specific).
    pub label: DvbText<'a>,
    /// `regular_expression_bytes` — regular expression generating every URL in
    /// the application domain (TS 102 727 §9.3.1.4.1). Consumes the rest of
    /// the descriptor.
    pub regular_expression: DvbText<'a>,
}

impl<'a> Parse<'a> for DvbHtmlApplicationBoundaryDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "DvbHtmlApplicationBoundaryDescriptor",
            "unexpected tag for dvb_html_application_boundary_descriptor",
        )?;
        let label_len = *body.first().ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "dvb_html_application_boundary_descriptor missing label_length",
        })? as usize;
        let end = LABEL_LEN_FIELD + label_len;
        if body.len() < end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "dvb_html_application_boundary_descriptor label overruns body",
            });
        }
        Ok(Self {
            label: DvbText::new(&body[LABEL_LEN_FIELD..end]),
            regular_expression: DvbText::new(&body[end..]),
        })
    }
}

impl Serialize for DvbHtmlApplicationBoundaryDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + LABEL_LEN_FIELD + self.label.len() + self.regular_expression.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let label_len = broadcast_common::len::fit_u8(self.label.len(), "label_length")?;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[HEADER_LEN] = label_len;
        let label_end = HEADER_LEN + LABEL_LEN_FIELD + self.label.len();
        buf[HEADER_LEN + LABEL_LEN_FIELD..label_end].copy_from_slice(self.label.raw());
        buf[label_end..len].copy_from_slice(self.regular_expression.raw());
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for DvbHtmlApplicationBoundaryDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DVB_HTML_APPLICATION_BOUNDARY";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Table 89: label "img", regular expression "http://a/.*".
    fn sample() -> Vec<u8> {
        let mut v = vec![TAG, 0, 3];
        v.extend_from_slice(b"img");
        v.extend_from_slice(b"http://a/.*");
        v[1] = u8::try_from(v.len() - 2).unwrap();
        v
    }

    #[test]
    fn parse_fields() {
        let bytes = sample();
        let d = DvbHtmlApplicationBoundaryDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.label.raw(), b"img");
        assert_eq!(d.regular_expression.raw(), b"http://a/.*");
    }

    #[test]
    fn round_trip_byte_identical() {
        for bytes in [sample(), vec![TAG, 1, 0]] {
            let d = DvbHtmlApplicationBoundaryDescriptor::parse(&bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
            assert_eq!(
                DvbHtmlApplicationBoundaryDescriptor::parse(&out).unwrap(),
                d
            );
        }
    }

    #[test]
    fn malformed_rejected() {
        assert!(DvbHtmlApplicationBoundaryDescriptor::parse(&[TAG, 0]).is_err());
        assert!(DvbHtmlApplicationBoundaryDescriptor::parse(&[TAG, 2, 9, b'a']).is_err());
        let big = vec![b'x'; 256];
        let d = DvbHtmlApplicationBoundaryDescriptor {
            label: DvbText::new(&big),
            regular_expression: DvbText::new(b""),
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
