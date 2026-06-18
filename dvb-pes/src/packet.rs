//! PES packet header parsing (ISO/IEC 13818-1 §2.4.3.6, Table 2-21).

use crate::error::{Error, Result};
use crate::stream_id::StreamId;
use crate::timestamp::{self, Dts, Pts};
use crate::PACKET_START_CODE_PREFIX;

const MIN_LEN: usize = 6; // start_code(3) + stream_id(1) + PES_packet_length(2)
const HEADER_FIXED: usize = 3; // 2 flag bytes + PES_header_data_length

/// The optional PES header present for non-special `stream_id`s
/// (§2.4.3.6). The variable optional fields (PTS/DTS, ESCR, ES_rate, trick mode,
/// …) are retained verbatim in [`optional_fields`](Self::optional_fields);
/// `pts`/`dts` are decoded from their front for convenience.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PesHeader<'a> {
    /// PES_scrambling_control (2 bits).
    pub scrambling_control: u8,
    /// PES_priority.
    pub pes_priority: bool,
    /// data_alignment_indicator.
    pub data_alignment_indicator: bool,
    /// copyright.
    pub copyright: bool,
    /// original_or_copy.
    pub original_or_copy: bool,
    /// ESCR_flag.
    pub escr_flag: bool,
    /// ES_rate_flag.
    pub es_rate_flag: bool,
    /// DSM_trick_mode_flag.
    pub dsm_trick_mode_flag: bool,
    /// additional_copy_info_flag.
    pub additional_copy_info_flag: bool,
    /// PES_CRC_flag.
    pub pes_crc_flag: bool,
    /// PES_extension_flag.
    pub pes_extension_flag: bool,
    /// Presentation time stamp, if `PTS_DTS_flags` indicated one.
    pub pts: Option<Pts>,
    /// Decoding time stamp, if `PTS_DTS_flags` was `11`.
    pub dts: Option<Dts>,
    /// The raw optional-header data block (`PES_header_data_length` bytes): the
    /// PTS/DTS plus any ESCR/ES_rate/trick-mode/CRC/extension sub-fields.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub optional_fields: &'a [u8],
}

/// A parsed PES packet.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PesPacket<'a> {
    /// stream_id (Table 2-22).
    pub stream_id: StreamId,
    /// PES_packet_length as carried; `0` means unbounded (video).
    pub pes_packet_length: u16,
    /// Optional PES header (absent for the special `stream_id`s).
    pub header: Option<PesHeader<'a>>,
    /// The elementary-stream bytes (`PES_packet_data_byte`s).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub payload: &'a [u8],
}

impl<'a> PesPacket<'a> {
    /// Parse a PES packet from the bytes starting at its `packet_start_code_prefix`.
    pub fn parse(b: &'a [u8]) -> Result<Self> {
        if b.len() < MIN_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_LEN,
                have: b.len(),
                what: "PES packet header",
            });
        }
        if b[0..3] != PACKET_START_CODE_PREFIX {
            return Err(Error::BadStartCode(
                (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]),
            ));
        }
        let stream_id = StreamId(b[3]);
        let pes_packet_length = u16::from_be_bytes([b[4], b[5]]);
        // Where the payload ends: bounded by PES_packet_length unless 0 (unbounded).
        let payload_end = if pes_packet_length == 0 {
            b.len()
        } else {
            (MIN_LEN + pes_packet_length as usize).min(b.len())
        };

        if !stream_id.has_optional_header() {
            return Ok(PesPacket {
                stream_id,
                pes_packet_length,
                header: None,
                payload: &b[MIN_LEN..payload_end],
            });
        }

        if b.len() < MIN_LEN + HEADER_FIXED {
            return Err(Error::BufferTooShort {
                need: MIN_LEN + HEADER_FIXED,
                have: b.len(),
                what: "PES optional header",
            });
        }
        let f1 = b[6];
        let f2 = b[7];
        let hdl = usize::from(b[8]);
        let hdr_start = MIN_LEN + HEADER_FIXED;
        let hdr_end = hdr_start + hdl;
        if b.len() < hdr_end {
            return Err(Error::BufferTooShort {
                need: hdr_end,
                have: b.len(),
                what: "PES_header_data_length",
            });
        }
        let optional_fields = &b[hdr_start..hdr_end];

        let pts_dts_flags = (f2 >> 6) & 0x03;
        let (pts, dts) = match pts_dts_flags {
            0b10 => (
                Some(Pts(timestamp::read(optional_fields, 0b0010, "PTS")?)),
                None,
            ),
            0b11 => {
                let pts = Pts(timestamp::read(optional_fields, 0b0011, "PTS")?);
                let dts_bytes = optional_fields.get(5..).unwrap_or(&[]);
                let dts = Dts(timestamp::read(dts_bytes, 0b0001, "DTS")?);
                (Some(pts), Some(dts))
            }
            _ => (None, None),
        };

        let header = PesHeader {
            scrambling_control: (f1 >> 4) & 0x03,
            pes_priority: f1 & 0x08 != 0,
            data_alignment_indicator: f1 & 0x04 != 0,
            copyright: f1 & 0x02 != 0,
            original_or_copy: f1 & 0x01 != 0,
            escr_flag: f2 & 0x20 != 0,
            es_rate_flag: f2 & 0x10 != 0,
            dsm_trick_mode_flag: f2 & 0x08 != 0,
            additional_copy_info_flag: f2 & 0x04 != 0,
            pes_crc_flag: f2 & 0x02 != 0,
            pes_extension_flag: f2 & 0x01 != 0,
            pts,
            dts,
            optional_fields,
        };

        Ok(PesPacket {
            stream_id,
            pes_packet_length,
            header: Some(header),
            payload: &b[hdr_end.min(payload_end)..payload_end],
        })
    }

    /// Serialized length in bytes.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        let hdr = self
            .header
            .as_ref()
            .map_or(0, |h| HEADER_FIXED + h.optional_fields.len());
        MIN_LEN + hdr + self.payload.len()
    }

    /// Serialize back to bytes (byte-identical to a spec-compliant input).
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "PES serialize output",
            });
        }
        buf[0..3].copy_from_slice(&PACKET_START_CODE_PREFIX);
        buf[3] = self.stream_id.0;
        buf[4..6].copy_from_slice(&self.pes_packet_length.to_be_bytes());
        let payload_at = match &self.header {
            None => MIN_LEN,
            Some(h) => {
                if h.optional_fields.len() > 255 {
                    return Err(Error::OptionalFieldsTooLarge(h.optional_fields.len()));
                }
                let f1 = 0x80
                    | ((h.scrambling_control & 0x03) << 4)
                    | (u8::from(h.pes_priority) << 3)
                    | (u8::from(h.data_alignment_indicator) << 2)
                    | (u8::from(h.copyright) << 1)
                    | u8::from(h.original_or_copy);
                let pts_dts_flags = match (h.pts.is_some(), h.dts.is_some()) {
                    (true, true) => 0b11,
                    (true, false) => 0b10,
                    _ => 0b00,
                };
                let f2 = (pts_dts_flags << 6)
                    | (u8::from(h.escr_flag) << 5)
                    | (u8::from(h.es_rate_flag) << 4)
                    | (u8::from(h.dsm_trick_mode_flag) << 3)
                    | (u8::from(h.additional_copy_info_flag) << 2)
                    | (u8::from(h.pes_crc_flag) << 1)
                    | u8::from(h.pes_extension_flag);
                buf[6] = f1;
                buf[7] = f2;
                buf[8] = h.optional_fields.len() as u8;
                let hdr_end = MIN_LEN + HEADER_FIXED + h.optional_fields.len();
                buf[MIN_LEN + HEADER_FIXED..hdr_end].copy_from_slice(h.optional_fields);
                hdr_end
            }
        };
        buf[payload_at..len].copy_from_slice(self.payload);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::vec;

    fn round_trip(b: &[u8]) {
        let pkt = PesPacket::parse(b).unwrap();
        let mut out = vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], b, "round-trip mismatch");
        assert_eq!(PesPacket::parse(&out).unwrap(), pkt);
    }

    #[test]
    fn video_pts_only() {
        // stream_id 0xE0, len=0x0A, flags 0x80/0x80, hdl=5, PTS=0, payload AA BB.
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01,
            0xAA, 0xBB,
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        assert_eq!(pkt.stream_id, StreamId(0xE0));
        let h = pkt.header.as_ref().unwrap();
        assert_eq!(h.pts, Some(Pts(0)));
        assert!(h.dts.is_none());
        assert_eq!(pkt.payload, &[0xAA, 0xBB]);
        round_trip(&b);
    }

    #[test]
    fn pts_and_dts() {
        // PTS_DTS_flags=11, hdl=10. PTS prefix 0011, DTS prefix 0001.
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0F, 0x80, 0xC0, 0x0A, 0x31, 0x00, 0x03, 0x00, 0x01,
            0x11, 0x00, 0x05, 0x00, 0x01, 0xCC,
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        let h = pkt.header.as_ref().unwrap();
        assert!(h.pts.is_some());
        assert!(h.dts.is_some());
        round_trip(&b);
    }

    #[test]
    fn special_stream_no_header() {
        // padding_stream 0xBE: bytes after length are payload directly.
        let b = [0x00, 0x00, 0x01, 0xBE, 0x00, 0x03, 0xFF, 0xFF, 0xFF];
        let pkt = PesPacket::parse(&b).unwrap();
        assert!(pkt.header.is_none());
        assert_eq!(pkt.payload, &[0xFF, 0xFF, 0xFF]);
        round_trip(&b);
    }

    #[test]
    fn unbounded_length_zero() {
        // PES_packet_length=0 (video): payload runs to end of buffer.
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x00, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01,
            0x01, 0x02, 0x03,
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        assert_eq!(pkt.pes_packet_length, 0);
        assert_eq!(pkt.payload, &[0x01, 0x02, 0x03]);
        round_trip(&b);
    }

    #[test]
    fn rejects_bad_start_code() {
        let err = PesPacket::parse(&[0x00, 0x00, 0x02, 0xE0, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BadStartCode(0x000002)));
    }

    #[test]
    fn rejects_short() {
        let err = PesPacket::parse(&[0x00, 0x00, 0x01]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_rejects_oversized_optional_fields() {
        // A manually-built header with > 255 optional bytes cannot fit
        // PES_header_data_length (u8) — must error, not silently truncate.
        let big = vec![0u8; 256];
        let pkt = PesPacket {
            stream_id: StreamId(0xE0),
            pes_packet_length: 0,
            header: Some(PesHeader {
                scrambling_control: 0,
                pes_priority: false,
                data_alignment_indicator: false,
                copyright: false,
                original_or_copy: false,
                escr_flag: false,
                es_rate_flag: false,
                dsm_trick_mode_flag: false,
                additional_copy_info_flag: false,
                pes_crc_flag: false,
                pes_extension_flag: false,
                pts: None,
                dts: None,
                optional_fields: &big,
            }),
            payload: &[],
        };
        let mut out = vec![0u8; pkt.serialized_len()];
        assert!(matches!(
            pkt.serialize_into(&mut out),
            Err(Error::OptionalFieldsTooLarge(256))
        ));
    }
}
