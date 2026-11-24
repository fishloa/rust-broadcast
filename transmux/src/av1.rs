//! AV1 sample entry in ISOBMFF — the `av01` VisualSampleEntry.
//!
//! AOMedia "AV1 Codec ISO Media File Format Binding"
//! (<https://aomediacodec.github.io/av1-isobmff/>, §2.3 AV1CodecConfigurationBox).
//!
//! The `av1C` configuration record itself lives in [`isobmff::av1`].

use crate::error::{Error, Result};
use crate::sample_entries::VisualSampleEntryFields;
use alloc::vec::Vec;
use broadcast_common::error::BufferTooShort;
use broadcast_common::len::need_out;
use broadcast_common::{Parse, Serialize};
use isobmff::av1::{AV1C_FOURCC, Av1ConfigurationBox};
use isobmff::box_types::BOX_HEADER_MIN_SIZE;

/// FourCC of the AV1 sample entry.
pub const AV01_FOURCC: [u8; 4] = *b"av01";

/// AV1 sample entry (`av01`) — a `VisualSampleEntry` carrying an `av1C` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Av1SampleEntry {
    /// Fixed VisualSampleEntry fields.
    pub visual: VisualSampleEntryFields,
    /// The `av1C` configuration box.
    pub config: Av1ConfigurationBox,
    /// The entry's children in wire order (the config box plus `pasp`/`btrt`/
    /// `colr`/…). Only the config box was modelled, so a parse -> serialize
    /// dropped 46 bytes of a real ffmpeg file (audit item 9), and emitting the
    /// config box first unconditionally reordered an entry whose `pasp`
    /// preceded it (audit item 3, round 3).
    pub children: Vec<crate::sample_entries::SampleEntryChild>,
}

impl Av1SampleEntry {
    /// Parse from full box bytes (including the 8-byte header).
    pub fn parse_entry(bytes: &[u8]) -> Result<Self> {
        let visual = VisualSampleEntryFields::parse_body(bytes, "av01")?;
        let region = VisualSampleEntryFields::children_region(bytes, "av01")?;
        let av1c = crate::sample_entries::find_config_box(region, &AV1C_FOURCC).ok_or(
            Error::BufferTooShort(BufferTooShort {
                need: 0,
                have: 0,
                what: "av01 missing av1C",
            }),
        )?;
        let config =
            Av1ConfigurationBox::parse(&av1c[isobmff::box_types::box_header_len(av1c)?..])?;
        let children = crate::sample_entries::entry_children(region, &AV1C_FOURCC)?;
        Ok(Self {
            visual,
            config,
            children,
        })
    }
}

impl Serialize for Av1SampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let config_len = BOX_HEADER_MIN_SIZE + self.config.serialized_len();
        isobmff::box_types::box_len(
            BOX_HEADER_MIN_SIZE
                + VisualSampleEntryFields::serialized_len()
                + crate::sample_entries::entry_children_len(&self.children, config_len),
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += isobmff::box_types::write_box_header(buf, &AV01_FOURCC, need)?;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        // The config box keeps its wire position: a real `vp09`/`av01` entry
        // often carries `pasp` *before* it (audit item 3, round 3).
        let write_config = |buf: &mut [u8], c: &mut usize| -> Result<()> {
            let len = BOX_HEADER_MIN_SIZE + self.config.serialized_len();
            let len = broadcast_common::len::fit_u32(len, "config box size")?;
            buf[*c..*c + isobmff::box_types::BOX_TYPE_SIZE].copy_from_slice(&len.to_be_bytes());
            *c += isobmff::box_types::BOX_TYPE_SIZE;
            buf[*c..*c + isobmff::box_types::BOX_TYPE_SIZE].copy_from_slice(&AV1C_FOURCC);
            *c += isobmff::box_types::BOX_TYPE_SIZE;
            *c += self.config.serialize_into(&mut buf[*c..])?;
            Ok(())
        };
        crate::sample_entries::serialize_entry_children(&self.children, write_config, buf, &mut c)?;
        if c != need {
            return Err(Error::InvalidInput(
                "sample entry child layout does not account for the whole box",
            ));
        }
        Ok(c)
    }
}
