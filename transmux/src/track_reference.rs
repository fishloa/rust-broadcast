//! `TrackReferenceBox` (`tref`) — ISO/IEC 14496-12:2026 §8.3.3.
//!
//! One `TrackReferenceTypeBox` per reference type, each an array of 32-bit
//! `track_IDs`. The `avcp` type that ties a video track to its AVC parameter
//! set stream is ISO/IEC 14496-15:2017 §5.4.3.3.
//!
//! Spec input: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`
//! (§8.3.3).

use crate::error::{Error, Result};
use crate::init_segment::{box_body, walk_children};
use crate::ir::TrackId;
use alloc::vec::Vec;
use broadcast_common::error::{BufferTooShort, InvalidValue};
use broadcast_common::len::need_out;
use broadcast_common::{Parse, Serialize};

/// `reference_type` of the track reference from a video track to its AVC
/// parameter set stream (ISO/IEC 14496-15:2017 §5.4.3.3).
pub const AVCP_REFERENCE_TYPE: [u8; 4] = *b"avcp";

const TREF: [u8; 4] = *b"tref";
const TRACK_ID_LEN: usize = 4;
const BOX_HEADER_LEN: usize = 8;

/// `TrackReferenceTypeBox`: the box type is the `reference_type`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackReferenceTypeBox {
    /// `reference_type`, e.g. [`AVCP_REFERENCE_TYPE`], `hint`, `cdsc`.
    pub reference_type: [u8; 4],
    /// `track_IDs`: referenced track ids or track group ids.
    pub track_ids: Vec<TrackId>,
}

/// `TrackReferenceBox` (`tref`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackReferenceBox {
    /// The reference type boxes, in wire order.
    pub references: Vec<TrackReferenceTypeBox>,
}

impl TrackReferenceBox {
    /// Parse a `tref` from its body (the children, after the box header), as
    /// kept in [`crate::init_segment::TrackBox::opaque`].
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut references = Vec::new();
        walk_children(body, |child| {
            let payload = child.payload;
            if payload.len() % TRACK_ID_LEN != 0 {
                return Err(Error::InvalidValue(InvalidValue {
                    field: "track_IDs",
                    value: payload.len() as u64,
                    reason: "a TrackReferenceTypeBox body is a whole number of 32-bit track_IDs (ISO/IEC 14496-12:2026 §8.3.3.2)",
                }));
            }
            references.push(TrackReferenceTypeBox {
                reference_type: child.four_cc,
                track_ids: payload
                    .chunks_exact(TRACK_ID_LEN)
                    .map(|c| TrackId::new(u32::from_be_bytes([c[0], c[1], c[2], c[3]])))
                    .collect(),
            });
            Ok(())
        })?;
        Ok(Self { references })
    }

    /// The `track_IDs` of the reference of `reference_type`, if present.
    pub fn track_ids(&self, reference_type: &[u8; 4]) -> Option<&[TrackId]> {
        self.references
            .iter()
            .find(|r| &r.reference_type == reference_type)
            .map(|r| r.track_ids.as_slice())
    }

    fn body_len(&self) -> usize {
        self.references
            .iter()
            .map(|r| BOX_HEADER_LEN + r.track_ids.len() * TRACK_ID_LEN)
            .sum()
    }
}

impl<'a> Parse<'a> for TrackReferenceBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, _) = crate::box_types::parse_box(bytes)?;
        if !header.header.box_type.is(&TREF) {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: BOX_HEADER_LEN,
                have: bytes.len(),
                what: "tref box type",
            }));
        }
        Self::parse_body(box_body(bytes)?)
    }
}

impl Serialize for TrackReferenceBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(BOX_HEADER_LEN + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = crate::box_types::write_box_header(buf, &TREF, need)?;
        for reference in &self.references {
            let len = BOX_HEADER_LEN + reference.track_ids.len() * TRACK_ID_LEN;
            cursor += crate::box_types::write_box_header(
                &mut buf[cursor..],
                &reference.reference_type,
                len,
            )?;
            for id in &reference.track_ids {
                buf[cursor..cursor + TRACK_ID_LEN].copy_from_slice(&id.value().to_be_bytes());
                cursor += TRACK_ID_LEN;
            }
        }
        Ok(cursor)
    }
}
