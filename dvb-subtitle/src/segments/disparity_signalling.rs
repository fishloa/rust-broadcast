//! Disparity Signalling Segment — ETSI EN 300 743 §7.2.7, Table 29 (segment_type 0x15).
//!
//! Supports plano-stereoscopic 3DTV subtitling by allowing disparity values
//! to be ascribed to regions or subregions.

use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// The disparity_signalling_segment segment_type.
pub const SEGMENT_TYPE: u8 = 0x15;
/// Header: 6 bytes.
pub const HEADER_LEN: usize = 6;
/// Fixed body: dss_version_number(4b)+page_flag(1b)+reserved(3b) + page_default_disparity_shift(1) = 2 bytes.
pub const FIXED_LEN: usize = 2;

/// A disparity shift update sequence as defined in Table 30.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DisparityShiftUpdateSequence {
    /// Interval duration in 90kHz STC units (24-bit).
    pub interval_duration: u32,
    /// Interval count + disparity shift integer pairs. `division_period_count`
    /// (Table 30) is exactly this list's length — "the number of ... time
    /// intervals within the following ... 'for' loop" — so it is derived on
    /// serialize rather than stored (#1108/DS-W3: a stored count could
    /// diverge from the data it's supposed to describe).
    pub intervals: alloc::vec::Vec<DisparityShiftInterval>,
}

/// An interval within a disparity shift update sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DisparityShiftInterval {
    /// Interval count multiplier.
    pub interval_count: u8,
    /// Disparity shift integer part (signed).
    pub disparity_shift_integer: i8,
}

impl DisparityShiftUpdateSequence {
    fn serialized_len(&self) -> usize {
        1 + 3 + 1 + self.intervals.len() * 2
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<()> {
        let body_len = 3 + 1 + self.intervals.len() * 2;
        buf[0] = broadcast_common::len::fit_u8(body_len, "disparity_shift_update_sequence_length")?;
        buf[1] = (self.interval_duration >> 16) as u8;
        buf[2] = (self.interval_duration >> 8) as u8;
        buf[3] = self.interval_duration as u8;
        buf[4] = broadcast_common::len::fit_u8(self.intervals.len(), "division_period_count")?;
        for (i, interval) in self.intervals.iter().enumerate() {
            let off = 5 + i * 2;
            buf[off] = interval.interval_count;
            buf[off + 1] = interval.disparity_shift_integer as u8;
        }
        Ok(())
    }

    fn parse(bytes: &[u8]) -> Result<(Self, usize)> {
        if bytes.len() < 5 {
            return Err(Error::BufferTooShort {
                need: 5,
                have: bytes.len(),
                what: "disparity_shift_update_sequence",
            });
        }
        let seq_len = bytes[0] as usize;
        if bytes.len() < 1 + seq_len {
            return Err(Error::BufferTooShort {
                need: 1 + seq_len,
                have: bytes.len(),
                what: "disparity_shift_update_sequence data",
            });
        }
        let dur = ((bytes[1] as u32) << 16) | ((bytes[2] as u32) << 8) | (bytes[3] as u32);
        let count = bytes[4];
        let total = 1 + seq_len;
        let mut intervals = alloc::vec::Vec::new();
        let mut ipos = 5;
        for _ in 0..count {
            if ipos + 2 > total {
                return Err(Error::BufferTooShort {
                    need: ipos + 2,
                    have: total,
                    what: "disparity_shift_update_sequence interval",
                });
            }
            intervals.push(DisparityShiftInterval {
                interval_count: bytes[ipos],
                disparity_shift_integer: bytes[ipos + 1] as i8,
            });
            ipos += 2;
        }
        Ok((
            DisparityShiftUpdateSequence {
                interval_duration: dur,
                intervals,
            },
            total,
        ))
    }
}

/// A subregion within a region for disparity signalling.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Subregion {
    /// Horizontal position relative to page (only if multiple subregions).
    pub subregion_horizontal_position: Option<u16>,
    /// Width in pixels (only if multiple subregions).
    pub subregion_width: Option<u16>,
    /// Disparity shift integer part (signed).
    pub subregion_disparity_shift_integer: i8,
    /// Disparity shift fractional part (unsigned, in 1/16 pixel).
    pub subregion_disparity_shift_fractional: u8,
    /// Reserved bits in the subregion shift/frac byte (bits `[3:0]`).
    pub reserved: u8,
    /// Update sequence if flag is set.
    pub update_sequence: Option<DisparityShiftUpdateSequence>,
}

impl Subregion {
    fn serialized_len(&self) -> usize {
        let base = if self.subregion_horizontal_position.is_some() {
            4 + 2
        } else {
            2
        };
        base + self
            .update_sequence
            .as_ref()
            .map_or(0, |u| u.serialized_len())
    }

    /// `has_pos` comes from the *region*, not this subregion (Table 29: the
    /// position fields are present for every subregion in a region, or none
    /// of them, per `number_of_subregions_minus_1`) — see DisparityRegion::
    /// serialize_into. Errors if this subregion's stored `Option`s disagree
    /// with that (#1108/DS-W3): writing this subregion's own state instead
    /// could silently misframe the following subregions/regions.
    fn serialize_into(&self, buf: &mut [u8], has_pos: bool) -> Result<()> {
        let stored_has_pos =
            self.subregion_horizontal_position.is_some() && self.subregion_width.is_some();
        if stored_has_pos != has_pos {
            return Err(Error::SubregionPositionMismatch);
        }
        let mut off = 0;
        if let (Some(h), Some(w)) = (self.subregion_horizontal_position, self.subregion_width) {
            buf[0..2].copy_from_slice(&h.to_be_bytes());
            buf[2..4].copy_from_slice(&w.to_be_bytes());
            off = 4;
        }
        buf[off] = self.subregion_disparity_shift_integer as u8;
        buf[off + 1] = (self.subregion_disparity_shift_fractional << 4) | (self.reserved & 0x0F);
        off += 2;
        if let Some(ref seq) = self.update_sequence {
            seq.serialize_into(&mut buf[off..])?;
        }
        Ok(())
    }
}

/// A region entry in the disparity signalling segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DisparityRegion {
    /// Region identifier.
    pub region_id: u8,
    /// Whether an update sequence follows for this region's subregions.
    pub update_sequence_region_flag: bool,
    /// Reserved bits in the region flags byte (bits `[6:2]`).
    pub reserved_flags: u8,
    /// Subregions within this region.
    pub subregions: alloc::vec::Vec<Subregion>,
}

impl DisparityRegion {
    fn serialized_len(&self) -> usize {
        2 + self
            .subregions
            .iter()
            .map(|s| s.serialized_len())
            .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<()> {
        let count = self.subregions.len();
        // number_of_subregions_minus_1 is 2 bits (Table 29): only 1..=4
        // subregions are representable. Pre-fix, `as u8` + `& 0x03` silently
        // wrapped an out-of-range count instead of rejecting it (#1108/DS-W3).
        if count == 0 || count > 4 {
            return Err(Error::InvalidSubregionCount(count));
        }
        let num = (count - 1) as u8;
        buf[0] = self.region_id;
        buf[1] = (u8::from(self.update_sequence_region_flag) << 7)
            | (self.reserved_flags & 0x7C)
            | (num & 0x03);
        // Position fields are present for every subregion, or none, per
        // whether the region itself has more than one (Table 29's
        // `if (number_of_subregions_minus_1 > 0)` guards the whole `for` body).
        let has_pos = count > 1;
        let mut off = 2;
        for sub in &self.subregions {
            sub.serialize_into(&mut buf[off..], has_pos)?;
            off += sub.serialized_len();
        }
        Ok(())
    }
}

/// Disparity Signalling Segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DisparitySignallingSegment {
    /// The page_id from the segment header.
    pub page_id: u16,
    /// DSS version number (modulo 16).
    pub dss_version_number: u8,
    /// Whether a page-level update sequence follows.
    pub update_sequence_page_flag: bool,
    /// Reserved bits in body byte 0 (bits `[2:0]`).
    pub reserved: u8,
    /// Default disparity shift for the whole page (signed).
    pub page_default_disparity_shift: i8,
    /// Page-level update sequence if flag is set.
    pub page_update_sequence: Option<DisparityShiftUpdateSequence>,
    /// Region entries.
    pub regions: alloc::vec::Vec<DisparityRegion>,
}

impl<'a> Parse<'a> for DisparitySignallingSegment {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + FIXED_LEN,
                have: bytes.len(),
                what: "disparity_signalling_segment",
            });
        }
        if bytes[0] != crate::pes_data_field::SYNC_BYTE {
            return Err(Error::BadSyncByte(bytes[0]));
        }
        if bytes[1] != SEGMENT_TYPE {
            return Err(Error::UnknownSegmentType(bytes[1]));
        }
        let page_id = u16::from_be_bytes([bytes[2], bytes[3]]);
        let segment_length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let total = HEADER_LEN + segment_length;
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "disparity_signalling_segment data",
            });
        }
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        if body.len() < FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN,
                have: body.len(),
                what: "disparity_signalling_segment body",
            });
        }
        let dss_version_number = body[0] >> 4;
        let update_sequence_page_flag = (body[0] & 0x08) != 0;
        let reserved = body[0] & 0x07;
        let page_default_disparity_shift = body[1] as i8;

        let mut pos = FIXED_LEN;
        let page_update_sequence = if update_sequence_page_flag {
            let (seq, seq_len) = DisparityShiftUpdateSequence::parse(&body[pos..])?;
            pos += seq_len;
            Some(seq)
        } else {
            None
        };

        let mut regions = alloc::vec::Vec::new();
        while pos < body.len() {
            // Table 29's region loop is `while (processed_length <
            // segment_length)` with no stuffing provision, so 1 leftover
            // byte means the segment is malformed, not that it should be
            // silently dropped (#1108/DS-W6: this used to just `break`).
            if pos + 2 > body.len() {
                return Err(Error::BufferTooShort {
                    need: pos + 2,
                    have: body.len(),
                    what: "disparity_signalling_segment region header",
                });
            }
            let region_id = body[pos];
            pos += 1;
            let update_flag = (body[pos] & 0x80) != 0;
            let reserved_flags = body[pos] & 0x7C;
            let num_subregions_minus_1 = body[pos] & 0x03;
            pos += 1;

            let mut subregions = alloc::vec::Vec::new();
            for _n in 0..=num_subregions_minus_1 {
                let has_pos = num_subregions_minus_1 > 0;
                let entry_len = if has_pos { 4 + 2 } else { 2 };
                if pos + entry_len > body.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + entry_len,
                        have: body.len(),
                        what: "subregion entry",
                    });
                }

                let (hpos, width) = if has_pos {
                    let h = u16::from_be_bytes([body[pos], body[pos + 1]]);
                    let w = u16::from_be_bytes([body[pos + 2], body[pos + 3]]);
                    pos += 4;
                    (Some(h), Some(w))
                } else {
                    (None, None)
                };

                let shift_int = body[pos] as i8;
                let shift_frac = body[pos + 1] >> 4;
                let reserved = body[pos + 1] & 0x0F;
                pos += 2;

                let sub_update = if update_flag {
                    let (seq, seq_len) = DisparityShiftUpdateSequence::parse(&body[pos..])?;
                    pos += seq_len;
                    Some(seq)
                } else {
                    None
                };

                subregions.push(Subregion {
                    subregion_horizontal_position: hpos,
                    subregion_width: width,
                    subregion_disparity_shift_integer: shift_int,
                    subregion_disparity_shift_fractional: shift_frac,
                    reserved,
                    update_sequence: sub_update,
                });
            }

            regions.push(DisparityRegion {
                region_id,
                update_sequence_region_flag: update_flag,
                reserved_flags,
                subregions,
            });
        }

        Ok(DisparitySignallingSegment {
            page_id,
            dss_version_number,
            update_sequence_page_flag,
            reserved,
            page_default_disparity_shift,
            page_update_sequence,
            regions,
        })
    }
}

impl Serialize for DisparitySignallingSegment {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + FIXED_LEN
            + self
                .page_update_sequence
                .as_ref()
                .map_or(0, |s| s.serialized_len())
            + self
                .regions
                .iter()
                .map(|r| r.serialized_len())
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "disparity_signalling_segment serialize",
            });
        }
        buf[0] = 0x0F;
        buf[1] = SEGMENT_TYPE;
        buf[2..4].copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = crate::segments::check_segment_length(len - HEADER_LEN)?;
        buf[4..6].copy_from_slice(&seg_len.to_be_bytes());

        buf[6] = (self.dss_version_number << 4)
            | (u8::from(self.update_sequence_page_flag) << 3)
            | (self.reserved & 0x07);
        buf[7] = self.page_default_disparity_shift as u8;

        let mut off = HEADER_LEN + FIXED_LEN;
        if let Some(ref seq) = self.page_update_sequence {
            seq.serialize_into(&mut buf[off..])?;
            off += seq.serialized_len();
        }
        for region in &self.regions {
            region.serialize_into(&mut buf[off..])?;
            off += region.serialized_len();
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    #[test]
    fn round_trip_simple() {
        let bytes = [
            0x0F, 0x15, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        ];
        let seg = DisparitySignallingSegment::parse(&bytes).unwrap();
        assert_eq!(seg.dss_version_number, 0);
        assert!(!seg.update_sequence_page_flag);
        assert_eq!(seg.regions.len(), 1);
        assert_eq!(seg.regions[0].region_id, 1);
        assert_eq!(seg.regions[0].subregions.len(), 1);
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.regions[0].region_id = 5;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = DisparitySignallingSegment::parse(&out2).unwrap();
        assert_eq!(reparse.regions[0].region_id, 5);
    }

    /// DS-W2 (#1108): sync_byte wasn't checked by any typed segment parser.
    #[test]
    fn rejects_bad_sync_byte() {
        let bytes = [
            0x00, 0x15, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        ];
        assert!(matches!(
            DisparitySignallingSegment::parse(&bytes),
            Err(Error::BadSyncByte(0x00))
        ));
    }

    /// DS-W6 (#1108): Table 29's region loop has no stuffing provision, so a
    /// single leftover byte after the last complete region is now rejected,
    /// not silently dropped via a bare `break`.
    #[test]
    fn rejects_trailing_partial_region() {
        let bytes = [
            0x0F, 0x15, 0x00, 0x01, 0x00, 0x07, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xFF,
        ];
        assert!(matches!(
            DisparitySignallingSegment::parse(&bytes),
            Err(Error::BufferTooShort { .. })
        ));
    }

    #[test]
    fn round_trip_with_page_update_seq() {
        let bytes = [
            0x0F, 0x15, 0x00, 0x01, 0x00, 0x0B, 0x88, 0x05, 0x08, 0x00, 0x00, 0x0A, 0x02, 0x01,
            0x10, 0x03, 0x20,
        ];
        let seg = DisparitySignallingSegment::parse(&bytes).unwrap();
        assert_eq!(seg.dss_version_number, 8);
        assert!(seg.update_sequence_page_flag);
        assert_eq!(seg.regions.len(), 0);
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.page_default_disparity_shift = 10;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = DisparitySignallingSegment::parse(&out2).unwrap();
        assert_eq!(reparse.page_default_disparity_shift, 10);
    }

    #[test]
    fn round_trip_with_update_sequence() {
        let bytes = [
            0x0F, 0x15, 0x00, 0x01, 0x00, 0x16, 0x88, 0x05, 0x08, 0x00, 0x00, 0x0A, 0x02, 0x01,
            0x10, 0x03, 0x20, 0x01, 0x80, 0x10, 0x00, 0x06, 0x00, 0x00, 0x14, 0x01, 0x01, 0x08,
        ];
        let seg = DisparitySignallingSegment::parse(&bytes).unwrap();
        assert_eq!(seg.dss_version_number, 8);
        assert!(seg.update_sequence_page_flag);
        assert_eq!(seg.page_default_disparity_shift, 5);
        let seq = seg.page_update_sequence.as_ref().unwrap();
        assert_eq!(seq.intervals.len(), 2);
        assert_eq!(seg.regions.len(), 1);
        assert_eq!(seg.regions[0].region_id, 1);
        assert!(seg.regions[0].update_sequence_region_flag);
        assert_eq!(seg.regions[0].subregions.len(), 1);
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.dss_version_number = 3;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = DisparitySignallingSegment::parse(&out2).unwrap();
        assert_eq!(reparse.dss_version_number, 3);
    }

    /// DS-W3 (#1108): `division_period_count` is now derived from
    /// `intervals.len()`, so appending an interval must grow the on-wire
    /// count byte and re-parse with the new interval intact.
    #[test]
    fn division_period_count_tracks_intervals_len() {
        let bytes = [
            0x0F, 0x15, 0x00, 0x01, 0x00, 0x0B, 0x88, 0x05, 0x08, 0x00, 0x00, 0x0A, 0x02, 0x01,
            0x10, 0x03, 0x20,
        ];
        let seg = DisparitySignallingSegment::parse(&bytes).unwrap();
        let mut seg2 = seg.clone();
        seg2.page_update_sequence
            .as_mut()
            .unwrap()
            .intervals
            .push(DisparityShiftInterval {
                interval_count: 5,
                disparity_shift_integer: -1,
            });
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        // division_period_count is body[4] of the update sequence, which
        // starts right after the 8-byte fixed header (offset HEADER_LEN+FIXED_LEN).
        let seq_off = HEADER_LEN + FIXED_LEN;
        assert_eq!(
            out2[seq_off + 4],
            3,
            "division_period_count must track intervals.len()"
        );
        let reparse = DisparitySignallingSegment::parse(&out2).unwrap();
        assert_eq!(reparse.page_update_sequence.unwrap().intervals.len(), 3);
    }

    /// DS-W3 (#1108): `number_of_subregions_minus_1` is 2 bits (Table 29), so
    /// 0 or more than 4 subregions must be rejected, not silently wrapped by
    /// `as u8 & 0x03`.
    #[test]
    fn rejects_out_of_range_subregion_count() {
        let region = DisparityRegion {
            region_id: 1,
            update_sequence_region_flag: false,
            reserved_flags: 0,
            subregions: alloc::vec::Vec::new(),
        };
        let mut buf = [0u8; 16];
        assert!(matches!(
            region.serialize_into(&mut buf),
            Err(Error::InvalidSubregionCount(0))
        ));

        let sub = Subregion {
            subregion_horizontal_position: None,
            subregion_width: None,
            subregion_disparity_shift_integer: 0,
            subregion_disparity_shift_fractional: 0,
            reserved: 0,
            update_sequence: None,
        };
        let region5 = DisparityRegion {
            region_id: 1,
            update_sequence_region_flag: false,
            reserved_flags: 0,
            subregions: alloc::vec![sub; 5],
        };
        let mut buf = [0u8; 64];
        assert!(matches!(
            region5.serialize_into(&mut buf),
            Err(Error::InvalidSubregionCount(5))
        ));
    }

    /// DS-W3 (#1108): a region with 2+ subregions requires the position
    /// fields present on ALL of them (Table 29); one subregion disagreeing
    /// with the others must be rejected, not silently misframed.
    #[test]
    fn rejects_inconsistent_subregion_position() {
        let with_pos = Subregion {
            subregion_horizontal_position: Some(10),
            subregion_width: Some(20),
            subregion_disparity_shift_integer: 0,
            subregion_disparity_shift_fractional: 0,
            reserved: 0,
            update_sequence: None,
        };
        let without_pos = Subregion {
            subregion_horizontal_position: None,
            subregion_width: None,
            subregion_disparity_shift_integer: 0,
            subregion_disparity_shift_fractional: 0,
            reserved: 0,
            update_sequence: None,
        };
        let region = DisparityRegion {
            region_id: 1,
            update_sequence_region_flag: false,
            reserved_flags: 0,
            subregions: alloc::vec![with_pos, without_pos],
        };
        let mut buf = [0u8; 32];
        assert!(matches!(
            region.serialize_into(&mut buf),
            Err(Error::SubregionPositionMismatch)
        ));
    }
}
