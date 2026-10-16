//! Pre-fetch Descriptor — ETSI TS 102 727 V1.1.1 §10.8.3.2, Table 82 (AIT tag
//! 0x0C). PDF pp.169-170, transcribed in `docs/descriptors/ts_102_727/82-*.md`,
//! verified against the rendered pages.
//!
//! Zero or one in the "application" AIT loop, for object-carousel transport
//! (`protocol_id` 0x0001). Body: `transport_protocol_label`(8) then entries of
//! `label_length`(8) + `label_byte`×N + `prefetch_priority`(8) up to
//! `descriptor_length`. The label is matched byte-for-byte against the
//! `label_descriptor`s of DII `moduleInfo` userInfo, hence opaque bytes.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for prefetch_descriptor (AIT namespace; the clause is titled "pre-fetch").
pub const TAG: u8 = 0x0C;
const HEADER_LEN: usize = 2;
const LABEL_LEN_FIELD: usize = 1;
const PRIORITY_LEN: usize = 1;

/// Lowest legal `prefetch_priority` (inclusive).
pub const PRIORITY_MIN: u8 = 1;
/// Highest legal `prefetch_priority` (inclusive).
pub const PRIORITY_MAX: u8 = 100;

/// One `(label, prefetch_priority)` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PrefetchEntry<'a> {
    /// `label_byte` array — module label (opaque, compared byte-for-byte).
    pub label: &'a [u8],
    /// `prefetch_priority` — 1 (lowest) to 100 (highest) per spec; parsing is
    /// tolerant of out-of-range values, [`PrefetchEntry::in_range`] reports them.
    pub prefetch_priority: u8,
}

impl PrefetchEntry<'_> {
    /// Is `prefetch_priority` within the specified 1..=100?
    #[must_use]
    pub const fn in_range(&self) -> bool {
        self.prefetch_priority >= PRIORITY_MIN && self.prefetch_priority <= PRIORITY_MAX
    }
}

/// Pre-fetch Descriptor (AIT tag 0x0C).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PrefetchDescriptor<'a> {
    /// `transport_protocol_label` of the transport_protocol_descriptor that
    /// specifies the carousel delivering the labelled modules.
    pub transport_protocol_label: u8,
    /// Label entries in wire order.
    pub entries: Vec<PrefetchEntry<'a>>,
}

impl<'a> Parse<'a> for PrefetchDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "PrefetchDescriptor",
            "unexpected tag for prefetch_descriptor",
        )?;
        let (&transport_protocol_label, mut rest) =
            body.split_first().ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "prefetch_descriptor missing transport_protocol_label",
            })?;
        let mut entries = Vec::new();
        while !rest.is_empty() {
            let n = rest[0] as usize;
            if rest.len() < LABEL_LEN_FIELD + n + PRIORITY_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "prefetch_descriptor entry overruns body",
                });
            }
            entries.push(PrefetchEntry {
                label: &rest[LABEL_LEN_FIELD..LABEL_LEN_FIELD + n],
                prefetch_priority: rest[LABEL_LEN_FIELD + n],
            });
            rest = &rest[LABEL_LEN_FIELD + n + PRIORITY_LEN..];
        }
        Ok(Self {
            transport_protocol_label,
            entries,
        })
    }
}

impl Serialize for PrefetchDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + 1
            + self
                .entries
                .iter()
                .map(|e| LABEL_LEN_FIELD + e.label.len() + PRIORITY_LEN)
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2] = self.transport_protocol_label;
        let mut pos = 3;
        for e in &self.entries {
            buf[pos] = broadcast_common::len::fit_u8(e.label.len(), "label_length")?;
            pos += LABEL_LEN_FIELD;
            buf[pos..pos + e.label.len()].copy_from_slice(e.label);
            pos += e.label.len();
            buf[pos] = e.prefetch_priority;
            pos += PRIORITY_LEN;
        }
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for PrefetchDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "PREFETCH";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Table 82: transport_protocol_label 1, entries ("img", 100) and ("", 1).
    const BYTES: [u8; 10] = [TAG, 8, 0x01, 3, b'i', b'm', b'g', 100, 0, 1];

    #[test]
    fn parse_and_round_trip() {
        let d = PrefetchDescriptor::parse(&BYTES).unwrap();
        assert_eq!(d.transport_protocol_label, 1);
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].label, b"img");
        assert!(d.entries[0].in_range());
        assert_eq!(d.entries[1].label, b"");
        assert_eq!(d.entries[1].prefetch_priority, 1);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, BYTES);
    }

    #[test]
    fn out_of_range_priority_is_preserved() {
        let bytes = [TAG, 3, 0x00, 0, 0xC8];
        let d = PrefetchDescriptor::parse(&bytes).unwrap();
        assert!(!d.entries[0].in_range());
        let mut out = [0u8; 5];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn empty_entry_list_and_truncation() {
        let d = PrefetchDescriptor::parse(&[TAG, 1, 7]).unwrap();
        assert!(d.entries.is_empty());
        assert!(PrefetchDescriptor::parse(&[TAG, 0]).is_err());
        assert!(PrefetchDescriptor::parse(&[TAG, 3, 7, 5, b'a']).is_err());
        let long = vec![0u8; 256];
        let d = PrefetchDescriptor {
            transport_protocol_label: 0,
            entries: vec![PrefetchEntry {
                label: &long,
                prefetch_priority: 1,
            }],
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
