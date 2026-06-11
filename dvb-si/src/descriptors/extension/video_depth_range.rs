//! Video Depth Range Descriptor — ETSI EN 300 468 §6.4.16.1 (tag_extension 0x10).
use super::*;

impl<'a> ExtensionBodyDef<'a> for VideoDepthRange<'a> {
    const TAG_EXTENSION: u8 = 0x10;
    const NAME: &'static str = "VIDEO_DEPTH_RANGE";
}

/// One depth range entry (Table 160 inner loop).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DepthRange<'a> {
    /// range_type(8) — Table 161.
    pub range_type: u8,
    /// Body interpreted by `range_type`.
    pub body: DepthRangeBody<'a>,
}

/// Body of a [`DepthRange`], keyed on `range_type` (Table 161).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub enum DepthRangeBody<'a> {
    /// `0x00` — production_disparity_hint_info() (Table 162).
    /// Two 12-bit two's-complement signed values packed into 3 bytes.
    ProductionDisparityHint {
        /// video_max_disparity_hint (12 tcimsbf).
        max: i16,
        /// video_min_disparity_hint (12 tcimsbf).
        min: i16,
    },
    /// `0x01` — multi-region SEI present (empty body).
    MultiRegionSei,
    /// Any other `range_type`: raw `range_selector` bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    Other(&'a [u8]),
}

/// video_depth_range body (Table 160) — fully typed loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct VideoDepthRange<'a> {
    /// Depth range entries in wire order.
    pub ranges: Vec<DepthRange<'a>>,
}

/// Sign-extend a 12-bit two's-complement value to `i16` (bit 11 is the sign).
fn sext12(v: u16) -> i16 {
    if v & 0x800 != 0 {
        (v | 0xF000) as i16
    } else {
        v as i16
    }
}

impl<'a> Parse<'a> for VideoDepthRange<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        let mut pos = 0;
        let mut ranges = Vec::new();
        loop {
            if pos == sel.len() {
                break;
            }
            // Need at least range_type + range_length (Table 160).
            if sel.len() - pos < VD_RANGE_HDR_LEN {
                return Err(Error::BufferTooShort {
                    need: pos + VD_RANGE_HDR_LEN,
                    have: sel.len(),
                    what: "video_depth_range body",
                });
            }
            let range_type = sel[pos];
            let range_length = sel[pos + 1] as usize;
            pos += VD_RANGE_HDR_LEN;
            if sel.len() < pos + range_length {
                return Err(Error::BufferTooShort {
                    need: pos + range_length,
                    have: sel.len(),
                    what: "video_depth_range body",
                });
            }
            let body = match range_type {
                // Table 161: production_disparity_hint_info() — Table 162.
                0x00 => {
                    if range_length < VD_DISPARITY_LEN {
                        return Err(invalid(
                            "video_depth_range: production_disparity_hint requires 3+ bytes",
                        ));
                    }
                    // Two 12-bit tcimsbf values packed into 3 bytes (b0..b2):
                    let b0 = sel[pos];
                    let b1 = sel[pos + 1];
                    let b2 = sel[pos + 2];
                    let max = sext12((u16::from(b0) << 4) | (u16::from(b1) >> 4));
                    let min = sext12(((u16::from(b1) & 0x0F) << 8) | u16::from(b2));
                    DepthRangeBody::ProductionDisparityHint { max, min }
                }
                // Table 161: multi-region SEI present (no body).
                0x01 => DepthRangeBody::MultiRegionSei,
                // Any other range_type: raw range_selector bytes.
                _ => DepthRangeBody::Other(&sel[pos..pos + range_length]),
            };
            ranges.push(DepthRange { range_type, body });
            pos += range_length;
        }
        Ok(VideoDepthRange { ranges })
    }
}

impl Serialize for VideoDepthRange<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        self.ranges
            .iter()
            .map(|r| {
                VD_RANGE_HDR_LEN
                    + match &r.body {
                        DepthRangeBody::ProductionDisparityHint { .. } => VD_DISPARITY_LEN,
                        DepthRangeBody::MultiRegionSei => 0,
                        DepthRangeBody::Other(s) => s.len(),
                    }
            })
            .sum()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let mut p = 0;
        for r in &self.ranges {
            buf[p] = r.range_type;
            match &r.body {
                DepthRangeBody::ProductionDisparityHint { max, min } => {
                    // Table 162: two 12-bit tcimsbf values packed into 3 bytes.
                    buf[p + 1] = VD_DISPARITY_LEN as u8;
                    let max_bits = *max as u16 & 0x0FFF;
                    let min_bits = *min as u16 & 0x0FFF;
                    buf[p + 2] = (max_bits >> 4) as u8;
                    buf[p + 3] = (((max_bits & 0x0F) << 4) | ((min_bits >> 8) & 0x0F)) as u8;
                    buf[p + 4] = min_bits as u8;
                    p += VD_RANGE_HDR_LEN + VD_DISPARITY_LEN;
                }
                DepthRangeBody::MultiRegionSei => {
                    buf[p + 1] = 0;
                    p += VD_RANGE_HDR_LEN;
                }
                DepthRangeBody::Other(s) => {
                    buf[p + 1] = s.len() as u8;
                    buf[p + 2..p + 2 + s.len()].copy_from_slice(s);
                    p += VD_RANGE_HDR_LEN + s.len();
                }
            }
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_video_depth_range_two_entries_round_trip() {
        // Two depth ranges in one selector:
        //   entry 1: range_type 0x00, range_length 3, disparity max=100 min=-50
        //   entry 2: range_type 0x05, range_length 2, raw [0xAA, 0xBB]
        // max=100 -> 0x0074 bits -> sext12=100; min=-50 -> 0x0032 bits,
        // two's complement of 50 is 0x0FCE, sext12 -> -50.
        let max_val: i16 = 100;
        let min_val: i16 = -50;
        let max_b = max_val as u16 & 0x0FFF; // 0x0064
        let min_b = min_val as u16 & 0x0FFF; // 0x0FCE
        let sel = [
            0x00,
            0x03,
            (max_b >> 4) as u8,
            (((max_b & 0x0F) << 4) | ((min_b >> 8) & 0x0F)) as u8,
            min_b as u8,
            0x05,
            0x02,
            0xAA,
            0xBB,
        ];
        let bytes = wrap(0x10, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::VideoDepthRange));
        match &d.body {
            ExtensionBody::VideoDepthRange(b) => {
                assert_eq!(b.ranges.len(), 2);
                assert_eq!(b.ranges[0].range_type, 0x00);
                match &b.ranges[0].body {
                    DepthRangeBody::ProductionDisparityHint { max, min } => {
                        assert_eq!(*max, 100);
                        assert_eq!(*min, -50);
                    }
                    _ => panic!("expected ProductionDisparityHint"),
                }
                assert_eq!(b.ranges[1].range_type, 0x05);
                match &b.ranges[1].body {
                    DepthRangeBody::Other(s) => assert_eq!(s, &[0xAA, 0xBB]),
                    _ => panic!("expected Other"),
                }
            }
            other => panic!("expected VideoDepthRange, got {other:?}"),
        }
        // parse → serialize → parse round-trip (byte-identical)
        round_trip(&d);
    }

    #[test]
    fn parse_video_depth_range_negative_edge_round_trip() {
        // ProductionDisparityHint with max=-1 (0x0FFF), min=0
        let max_val: i16 = -1;
        let min_val: i16 = 0;
        let max_b = max_val as u16 & 0x0FFF;
        let min_b = min_val as u16 & 0x0FFF;
        let sel = [
            0x00,
            0x03,
            (max_b >> 4) as u8,
            (((max_b & 0x0F) << 4) | ((min_b >> 8) & 0x0F)) as u8,
            min_b as u8,
        ];
        let bytes = wrap(0x10, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VideoDepthRange(b) => {
                assert_eq!(b.ranges.len(), 1);
                match &b.ranges[0].body {
                    DepthRangeBody::ProductionDisparityHint { max, min } => {
                        assert_eq!(*max, -1);
                        assert_eq!(*min, 0);
                    }
                    _ => panic!("expected ProductionDisparityHint"),
                }
            }
            other => panic!("expected VideoDepthRange, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_video_depth_range_multi_region_sei_round_trip() {
        // range_type 0x01 with empty body
        let sel = [0x01, 0x00, 0x01, 0x00];
        let bytes = wrap(0x10, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VideoDepthRange(b) => {
                assert_eq!(b.ranges.len(), 2);
                assert!(matches!(b.ranges[0].body, DepthRangeBody::MultiRegionSei));
                assert!(matches!(b.ranges[1].body, DepthRangeBody::MultiRegionSei));
            }
            other => panic!("expected VideoDepthRange, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_video_depth_range_empty_selector() {
        let bytes = wrap(0x10, &[]);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VideoDepthRange(b) => {
                assert!(b.ranges.is_empty());
            }
            other => panic!("expected VideoDepthRange, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_video_depth_range_rejects_truncated() {
        // Only range_type byte, no range_length
        let sel = [0x00];
        let bytes = wrap(0x10, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_video_depth_range_rejects_overrun() {
        // range_length=5 but only 2 bytes follow
        let sel = [0x00, 0x05, 0xAA, 0xBB];
        let bytes = wrap(0x10, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::BufferTooShort { .. }
        ));
    }
}
