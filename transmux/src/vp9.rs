//! VP9 sample entry in ISOBMFF — the `vp09` VisualSampleEntry.
//!
//! WebM Project "VP Codec ISO Media File Format Binding"
//! (<https://www.webmproject.org/vp9/mp4/>, VPCodecConfigurationBox).
//!
//! The `vpcC` configuration record itself lives in [`isobmff::vp9`].

use crate::error::{Error, Result};
use crate::sample_entries::VisualSampleEntryFields;
use alloc::vec::Vec;
use broadcast_common::error::BufferTooShort;
use broadcast_common::len::need_out;
use broadcast_common::{Parse, Serialize};
use isobmff::box_types::BOX_HEADER_MIN_SIZE;
use isobmff::vp9::{VPCC_FOURCC, Vp9ConfigurationBox};

/// FourCC of the VP9 sample entry.
pub const VP09_FOURCC: [u8; 4] = *b"vp09";

/// VP9 sample entry (`vp09`) — a `VisualSampleEntry` carrying a `vpcC` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Vp9SampleEntry {
    /// Fixed VisualSampleEntry fields.
    pub visual: VisualSampleEntryFields,
    /// The `vpcC` configuration box.
    pub config: Vp9ConfigurationBox,
    /// The entry's children in wire order (the config box plus `pasp`/`btrt`/
    /// `colr`/…). A `pasp` commonly precedes `vpcC` in real files, so the
    /// config box's position is recorded rather than assumed (audit item 3,
    /// round 3).
    pub children: Vec<crate::sample_entries::SampleEntryChild>,
}

impl Vp9SampleEntry {
    /// Parse from full box bytes (including the 8-byte header).
    pub fn parse_entry(bytes: &[u8]) -> Result<Self> {
        let visual = VisualSampleEntryFields::parse_body(bytes, "vp09")?;
        let region = VisualSampleEntryFields::children_region(bytes, "vp09")?;
        let vpcc = crate::sample_entries::find_config_box(region, &VPCC_FOURCC).ok_or(
            Error::BufferTooShort(BufferTooShort {
                need: 0,
                have: 0,
                what: "vp09 missing vpcC",
            }),
        )?;
        let config =
            Vp9ConfigurationBox::parse(&vpcc[isobmff::box_types::box_header_len(vpcc)?..])?;
        let children = crate::sample_entries::entry_children(region, &VPCC_FOURCC)?;
        Ok(Self {
            visual,
            config,
            children,
        })
    }
}

impl Serialize for Vp9SampleEntry {
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
        c += isobmff::box_types::write_box_header(buf, &VP09_FOURCC, need)?;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        // The config box keeps its wire position: a real `vp09`/`av01` entry
        // often carries `pasp` *before* it (audit item 3, round 3).
        let write_config = |buf: &mut [u8], c: &mut usize| -> Result<()> {
            let len = BOX_HEADER_MIN_SIZE + self.config.serialized_len();
            let len = broadcast_common::len::fit_u32(len, "config box size")?;
            buf[*c..*c + core::mem::size_of::<u32>()].copy_from_slice(&len.to_be_bytes());
            *c += core::mem::size_of::<u32>();
            buf[*c..*c + VPCC_FOURCC.len()].copy_from_slice(&VPCC_FOURCC);
            *c += VPCC_FOURCC.len();
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
