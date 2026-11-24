//! AVS3 / AVS2 video sample entries in ISOBMFF — GY/T 420-2025 Annex A.3.3
//! (A.2.3 for AVS2).
//!
//! The configuration records and sample-group entries live in
//! [`isobmff::avs3`] / [`isobmff::avs2`].
//!
//! | Type | FourCC | Spec |
//! |------|--------|------|
//! | [`AvsVideoSampleEntry`] | `avs3` / `lav3` / `avst` / `lav2` | A.3.3.1 / A.3.3.2 (A.2.3.1 / A.2.3.2) |

use crate::error::{Error, Result};
use crate::sample_entries::{SampleEntryChild, VisualSampleEntryFields};
use alloc::vec::Vec;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};
use isobmff::avs2::LA2C_FOURCC;
use isobmff::avs3::{AV3C_FOURCC, Avs3ConfigurationBox, LAVC_FOURCC, LAvsConfigurationBox};
use isobmff::box_types::BOX_HEADER_MIN_SIZE;

/// FourCC of the AVS3 sample entry.
pub const AVS3_FOURCC: [u8; 4] = *b"avs3";
/// FourCC of the LAVS3 sample entry.
pub const LAV3_FOURCC: [u8; 4] = *b"lav3";

/// Which AVS video sample entry / primary configuration box a track uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AvsVideoEntryKind {
    /// `avs3` — AVS3 track, primary box `av3c`.
    Avs3,
    /// `lav3` — AVS3 temporal-layer track without the lowest level, primary box `lavc`.
    LAvs3,
    /// `avst` — AVS2 track, primary box `av2c`.
    Avs2,
    /// `lav2` — AVS2 temporal-layer track without the lowest level, primary box `la2c`.
    LAvs2,
}

impl AvsVideoEntryKind {
    /// FourCC of the sample entry.
    #[must_use]
    pub const fn entry_fourcc(self) -> [u8; 4] {
        match self {
            Self::Avs3 => AVS3_FOURCC,
            Self::LAvs3 => LAV3_FOURCC,
            Self::Avs2 => isobmff::avs2::AVST_FOURCC,
            Self::LAvs2 => isobmff::avs2::LAV2_FOURCC,
        }
    }

    /// FourCC of the primary configuration box of this entry kind.
    #[must_use]
    pub const fn config_fourcc(self) -> [u8; 4] {
        match self {
            Self::Avs3 => AV3C_FOURCC,
            Self::LAvs3 => LAVC_FOURCC,
            Self::Avs2 => isobmff::avs2::AV2C_FOURCC,
            Self::LAvs2 => isobmff::avs2::LA2C_FOURCC,
        }
    }

    /// Decode from a sample-entry FourCC.
    #[must_use]
    pub fn from_entry_fourcc(f: [u8; 4]) -> Option<Self> {
        [Self::Avs3, Self::LAvs3, Self::Avs2, Self::LAvs2]
            .into_iter()
            .find(|k| k.entry_fourcc() == f)
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Avs3 => "AVS3 sample entry (avs3)",
            Self::LAvs3 => "LAVS3 sample entry (lav3)",
            Self::Avs2 => "AVS2 sample entry (avst)",
            Self::LAvs2 => "LAVS2 sample entry (lav2)",
        }
    }
}
broadcast_common::impl_spec_display!(AvsVideoEntryKind);

/// The typed primary configuration box of an [`AvsVideoSampleEntry`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AvsVideoConfig {
    /// `av3c`.
    Avs3(Avs3ConfigurationBox),
    /// `av2c`.
    Avs2(isobmff::avs2::Avs2ConfigurationBox),
    /// `lavc` or `la2c`.
    Layers(LAvsConfigurationBox),
}

impl AvsVideoConfig {
    /// FourCC-style label of the configuration box kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Avs3(_) => "av3c",
            Self::Avs2(_) => "av2c",
            Self::Layers(_) => "lavc/la2c",
        }
    }
}
broadcast_common::impl_spec_display!(AvsVideoConfig);

/// An AVS2/AVS3 video sample entry (`avs3`, `lav3`, `avst`, `lav2`): a
/// `VisualSampleEntry` carrying the primary configuration box
/// ([`AvsVideoEntryKind::config_fourcc`]). A second configuration box (`lavc` /
/// `la2c` alongside `av3c` / `av2c`, A.3.3.1 b) and any `pasp`/`btrt`/... children
/// stay in `children` verbatim at their wire positions;
/// [`AvsVideoSampleEntry::secondary_layer_config`] decodes the layer box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AvsVideoSampleEntry {
    /// Entry kind (fixes the sample-entry and primary-config FourCCs).
    pub kind: AvsVideoEntryKind,
    /// Fixed VisualSampleEntry fields.
    pub visual: VisualSampleEntryFields,
    /// The primary configuration box.
    pub config: AvsVideoConfig,
    /// The entry's children in wire order (the primary config box at its position).
    pub children: Vec<SampleEntryChild>,
}

impl AvsVideoSampleEntry {
    /// Parse a full sample-entry box (including the 8-byte header).
    pub fn parse_entry(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "AVS sample entry")?;
        let mut cursor = isobmff::box_types::BOX_TYPE_SIZE;
        let fourcc = broadcast_common::wire::read_u32(bytes, &mut cursor, "AVS sample entry type")?
            .to_be_bytes();
        let kind = AvsVideoEntryKind::from_entry_fourcc(fourcc).ok_or(Error::UnexpectedBox {
            expected: "avs3/lav3/avst/lav2",
        })?;
        let visual = VisualSampleEntryFields::parse_body(bytes, "AVS sample entry")?;
        let region = VisualSampleEntryFields::children_region(bytes, "AVS sample entry")?;
        let cfg_fourcc = kind.config_fourcc();
        let cfg = crate::sample_entries::find_config_box(region, &cfg_fourcc).ok_or(
            Error::UnexpectedBox {
                expected: "AVS configuration box",
            },
        )?;
        let body = &cfg[isobmff::box_types::box_header_len(cfg)?..];
        let config = match kind {
            AvsVideoEntryKind::Avs3 => AvsVideoConfig::Avs3(Avs3ConfigurationBox::parse(body)?),
            AvsVideoEntryKind::Avs2 => {
                AvsVideoConfig::Avs2(isobmff::avs2::Avs2ConfigurationBox::parse(body)?)
            }
            AvsVideoEntryKind::LAvs3 | AvsVideoEntryKind::LAvs2 => {
                AvsVideoConfig::Layers(LAvsConfigurationBox::parse(body)?)
            }
        };
        let children = crate::sample_entries::entry_children(region, &cfg_fourcc)?;
        Ok(Self {
            kind,
            visual,
            config,
            children,
        })
    }

    /// Decode the second configuration box (`lavc` in an `avs3` entry, `la2c` in
    /// an `avst` entry) if the entry carries one.
    #[must_use]
    pub fn secondary_layer_config(&self) -> Option<Result<LAvsConfigurationBox>> {
        let want = match self.kind {
            AvsVideoEntryKind::Avs3 => LAVC_FOURCC,
            AvsVideoEntryKind::Avs2 => LA2C_FOURCC,
            _ => return None,
        };
        self.children.iter().find_map(|c| match c {
            SampleEntryChild::Other(b) if b.get(4..8) == Some(&want[..]) => {
                let hdr = isobmff::box_types::box_header_len(b).ok()?;
                Some(LAvsConfigurationBox::parse(&b[hdr..]).map_err(Into::into))
            }
            _ => None,
        })
    }

    fn config_body_len(&self) -> usize {
        match &self.config {
            AvsVideoConfig::Avs3(c) => c.serialized_len(),
            AvsVideoConfig::Avs2(c) => c.serialized_len(),
            AvsVideoConfig::Layers(c) => c.serialized_len(),
        }
    }

    fn write_config_body(&self, buf: &mut [u8]) -> Result<usize> {
        match &self.config {
            AvsVideoConfig::Avs3(c) => c.serialize_into(buf).map_err(Into::into),
            AvsVideoConfig::Avs2(c) => c.serialize_into(buf).map_err(Into::into),
            AvsVideoConfig::Layers(c) => c.serialize_into(buf).map_err(Into::into),
        }
    }
}

impl Serialize for AvsVideoSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let config_len = BOX_HEADER_MIN_SIZE + self.config_body_len();
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
        c += isobmff::box_types::write_box_header(buf, &self.kind.entry_fourcc(), need)?;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        let cfg_fourcc = self.kind.config_fourcc();
        let write_config = |buf: &mut [u8], c: &mut usize| -> Result<()> {
            let len = BOX_HEADER_MIN_SIZE + self.config_body_len();
            let len = broadcast_common::len::fit_u32(len, "config box size")?;
            buf[*c..*c + isobmff::box_types::BOX_TYPE_SIZE].copy_from_slice(&len.to_be_bytes());
            *c += isobmff::box_types::BOX_TYPE_SIZE;
            buf[*c..*c + isobmff::box_types::BOX_TYPE_SIZE].copy_from_slice(&cfg_fourcc);
            *c += isobmff::box_types::BOX_TYPE_SIZE;
            *c += self.write_config_body(&mut buf[*c..])?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use isobmff::avs2::{AV2C_FOURCC, AVST_FOURCC, Avs2ConfigurationBox, LAV2_FOURCC};
    use isobmff::avs3::{AvsFrameRateCode, LibraryDependencyIdc, TemporalLayerInfo};

    fn visual() -> VisualSampleEntryFields {
        VisualSampleEntryFields {
            width: 3840,
            height: 2160,
            ..VisualSampleEntryFields::default()
        }
    }

    fn child_box(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 8 + body.len()];
        b[..4].copy_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        b[4..8].copy_from_slice(&fourcc);
        b[8..].copy_from_slice(body);
        b
    }

    fn layers_body(layers: &LAvsConfigurationBox) -> Vec<u8> {
        let mut body = vec![0u8; layers.serialized_len()];
        layers.serialize_into(&mut body).unwrap();
        body
    }

    /// `avs3` entry's children, and an unrelated child (`btrt`) before it is
    /// not mistaken for one.
    #[test]
    fn secondary_layer_config_selects_the_layers_fourcc_and_ignores_others() {
        let lavc = LAvsConfigurationBox::new(vec![TemporalLayerInfo::new(
            0,
            AvsFrameRateCode::Fps25,
            100,
            0,
        )]);
        let btrt = child_box(*b"btrt", &[0u8; 12]);
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs3,
            visual: visual(),
            config: AvsVideoConfig::Avs3(Avs3ConfigurationBox::new(
                vec![0, 0, 1, 0xB0],
                LibraryDependencyIdc::MainWithoutLibrary,
            )),
            children: vec![
                SampleEntryChild::Config,
                SampleEntryChild::Other(btrt.clone()),
                SampleEntryChild::Other(child_box(LAVC_FOURCC, &layers_body(&lavc))),
            ],
        };
        assert_eq!(entry.secondary_layer_config().unwrap().unwrap(), lavc);

        let only_other = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs3,
            visual: visual(),
            config: AvsVideoConfig::Avs3(Avs3ConfigurationBox::new(
                vec![],
                LibraryDependencyIdc::Library,
            )),
            children: vec![SampleEntryChild::Config, SampleEntryChild::Other(btrt)],
        };
        assert!(only_other.secondary_layer_config().is_none());
    }

    /// A.2.3.1 b): an `avst` entry's second configuration box is `la2c`.
    #[test]
    fn avst_entry_decodes_its_la2c_secondary_layer_config() {
        let layers = LAvsConfigurationBox::new(vec![TemporalLayerInfo::new(
            1,
            AvsFrameRateCode::Fps50,
            5,
            0,
        )]);
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs2,
            visual: visual(),
            config: AvsVideoConfig::Avs2(isobmff::avs2::Avs2ConfigurationBox::new(vec![
                0, 0, 1, 0xB0,
            ])),
            children: vec![
                SampleEntryChild::Config,
                SampleEntryChild::Other(child_box(
                    isobmff::avs2::LA2C_FOURCC,
                    &layers_body(&layers),
                )),
            ],
        };
        assert_eq!(entry.secondary_layer_config().unwrap().unwrap(), layers);
    }

    #[test]
    fn entry_kind_and_config_labels_are_pinned() {
        for (v, label) in [
            (AvsVideoEntryKind::Avs3, "AVS3 sample entry (avs3)"),
            (AvsVideoEntryKind::LAvs3, "LAVS3 sample entry (lav3)"),
            (AvsVideoEntryKind::Avs2, "AVS2 sample entry (avst)"),
            (AvsVideoEntryKind::LAvs2, "LAVS2 sample entry (lav2)"),
        ] {
            assert_eq!(v.name(), label);
            assert_eq!(v.to_string(), label);
        }
        for (v, label) in [
            (
                AvsVideoConfig::Avs3(Avs3ConfigurationBox::new(
                    vec![],
                    LibraryDependencyIdc::Library,
                )),
                "av3c",
            ),
            (
                AvsVideoConfig::Avs2(Avs2ConfigurationBox::new(vec![])),
                "av2c",
            ),
            (
                AvsVideoConfig::Layers(LAvsConfigurationBox::new(vec![])),
                "lavc/la2c",
            ),
        ] {
            assert_eq!(v.name(), label);
            assert_eq!(v.to_string(), label);
        }
    }

    /// An `avs3` sample entry with `av3c`, a `lavc` second config box and a
    /// `pasp` box in wire order, round-trips byte-exact; the secondary layer
    /// configuration is decodable.
    #[test]
    fn avs3_sample_entry_with_second_config_round_trips() {
        let av3c = Avs3ConfigurationBox::new(
            vec![0, 0, 1, 0xB0],
            LibraryDependencyIdc::MainWithoutLibrary,
        );
        let lavc = LAvsConfigurationBox::new(vec![TemporalLayerInfo::new(
            0,
            AvsFrameRateCode::Fps25,
            100,
            0,
        )]);
        let mut lavc_box = vec![];
        let mut body = vec![0u8; lavc.serialized_len()];
        lavc.serialize_into(&mut body).unwrap();
        lavc_box.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        lavc_box.extend_from_slice(&LAVC_FOURCC);
        lavc_box.extend_from_slice(&body);
        let pasp = [0, 0, 0, 16, b'p', b'a', b's', b'p', 0, 0, 0, 1, 0, 0, 0, 1];
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs3,
            visual: visual(),
            config: AvsVideoConfig::Avs3(av3c.clone()),
            children: vec![
                SampleEntryChild::Config,
                SampleEntryChild::Other(lavc_box),
                SampleEntryChild::Other(pasp.to_vec()),
            ],
        };
        let mut buf = vec![0u8; entry.serialized_len()];
        let n = entry.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[4..8], b"avs3");
        let back = AvsVideoSampleEntry::parse_entry(&buf[..n]).unwrap();
        assert_eq!(back, entry);
        assert_eq!(back.visual.width, 3840);
        assert_eq!(back.secondary_layer_config().unwrap().unwrap(), lavc);
        let mut again = vec![0u8; back.serialized_len()];
        back.serialize_into(&mut again).unwrap();
        assert_eq!(again, buf, "byte-exact round trip");
    }

    /// A `lav3` entry carries only `lavc` as its primary configuration.
    #[test]
    fn lav3_sample_entry_round_trips() {
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::LAvs3,
            visual: visual(),
            config: AvsVideoConfig::Layers(LAvsConfigurationBox::new(vec![
                TemporalLayerInfo::new(1, AvsFrameRateCode::Fps50, 5, 0),
            ])),
            children: vec![SampleEntryChild::Config],
        };
        let mut buf = vec![0u8; entry.serialized_len()];
        entry.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[4..8], b"lav3");
        let back = AvsVideoSampleEntry::parse_entry(&buf).unwrap();
        assert_eq!(back, entry);
        assert!(back.secondary_layer_config().is_none());
        assert!(matches!(
            AvsVideoSampleEntry::parse_entry(&[0, 0, 0, 8, b'a', b'v', b'c', b'1']),
            Err(crate::error::Error::UnexpectedBox { .. })
        ));
    }

    /// An `avst` entry with `av2c` round-trips; the fourcc constants agree
    /// with the entry kinds.
    #[test]
    fn avst_sample_entry_round_trips() {
        assert_eq!(AvsVideoEntryKind::Avs2.entry_fourcc(), AVST_FOURCC);
        assert_eq!(AvsVideoEntryKind::Avs2.config_fourcc(), AV2C_FOURCC);
        assert_eq!(AvsVideoEntryKind::LAvs2.entry_fourcc(), LAV2_FOURCC);
        assert_eq!(AvsVideoEntryKind::LAvs2.config_fourcc(), LA2C_FOURCC);
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs2,
            visual: VisualSampleEntryFields {
                width: 1920,
                height: 1080,
                ..VisualSampleEntryFields::default()
            },
            config: AvsVideoConfig::Avs2(Avs2ConfigurationBox::new(vec![0, 0, 1, 0xB0, 0x20])),
            children: vec![SampleEntryChild::Config],
        };
        let mut buf = vec![0u8; entry.serialized_len()];
        entry.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[4..8], b"avst");
        let back = AvsVideoSampleEntry::parse_entry(&buf).unwrap();
        assert_eq!(back, entry);
    }
}
