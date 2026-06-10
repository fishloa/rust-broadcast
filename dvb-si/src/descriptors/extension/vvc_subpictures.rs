//! VVC Subpictures Descriptor — ETSI EN 300 468 §6.4.17 (tag_extension 0x23).
use super::*;

impl super::sealed::Sealed for VvcSubpicturesDescriptor<'_> {}
impl ExtensionBodyDef for VvcSubpicturesDescriptor<'_> {
    const TAG_EXTENSION: u8 = 0x23;
    const NAME: &'static str = "VVC_SUBPICTURES";
}

/// One VVC subpicture entry (Table 162a inner loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VvcSubpicture {
    /// component_tag(8).
    pub component_tag: u8,
    /// vvc_subpicture_id(8).
    pub vvc_subpicture_id: u8,
}

/// vvc_subpictures body (Table 162a) — fully typed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct VvcSubpicturesDescriptor<'a> {
    /// default_service_mode(1) — byte 0 bit 7.
    pub default_service_mode: bool,
    /// Subpicture entries in wire order.
    pub subpictures: Vec<VvcSubpicture>,
    /// processing_mode(3) — byte after the subpicture loop, bits `[2:0]`.
    pub processing_mode: u8,
    /// Length-delimited service_description text, present iff
    /// `service_description_present` (byte 0 bit 6) is set.
    pub service_description: Option<DvbText<'a>>,
}

impl<'a> Parse<'a> for VvcSubpicturesDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(invalid("vvc_subpictures: header byte missing"));
        }
        let byte0 = sel[0];
        let default_service_mode = (byte0 & 0x80) != 0;
        let service_description_present = (byte0 & 0x40) != 0;
        let n = (byte0 & 0x3F) as usize;

        // Table 162a: 1 fixed byte + n*2 subpicture bytes + 1 processing_mode byte.
        let subpicture_bytes = n * 2;
        let min_len = 1 + subpicture_bytes + 1;
        if sel.len() < min_len {
            return Err(invalid("vvc_subpictures: truncated"));
        }

        let mut pos = 1;
        let mut subpictures = Vec::with_capacity(n);
        for _ in 0..n {
            let component_tag = sel[pos];
            let vvc_subpicture_id = sel[pos + 1];
            subpictures.push(VvcSubpicture {
                component_tag,
                vvc_subpicture_id,
            });
            pos += 2;
        }

        let processing_mode = sel[pos] & 0x07;
        pos += 1;

        let service_description = if service_description_present {
            if sel.len() < pos + 1 {
                return Err(invalid(
                    "vvc_subpictures: service_description_length truncated",
                ));
            }
            let len = sel[pos] as usize;
            pos += 1;
            if sel.len() < pos + len {
                return Err(invalid(
                    "vvc_subpictures: service_description overruns body",
                ));
            }
            let text = DvbText::new(&sel[pos..pos + len]);
            pos += len;
            Some(text)
        } else {
            None
        };

        // Table 162a is exact; reject trailing bytes.
        if pos != sel.len() {
            return Err(invalid("vvc_subpictures: trailing data"));
        }

        Ok(VvcSubpicturesDescriptor {
            default_service_mode,
            subpictures,
            processing_mode,
            service_description,
        })
    }
}

impl Serialize for VvcSubpicturesDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self.subpictures.len() * 2
            + 1 // processing_mode
            + self
                .service_description
                .as_ref()
                .map_or(0, |t| 1 + t.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        // byte 0: default_service_mode(1) | service_description_present(1) | number_of_vvc_subpictures(6)
        let service_description_present = self.service_description.is_some();
        buf[0] = (u8::from(self.default_service_mode) << 7)
            | (u8::from(service_description_present) << 6)
            | (self.subpictures.len() as u8 & 0x3F);
        let mut p = 1;
        for sp in &self.subpictures {
            buf[p] = sp.component_tag;
            buf[p + 1] = sp.vvc_subpicture_id;
            p += 2;
        }
        buf[p] = self.processing_mode & 0x07;
        p += 1;
        if let Some(text) = &self.service_description {
            buf[p] = text.len() as u8;
            p += 1;
            buf[p..p + text.len()].copy_from_slice(text.raw());
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};
    use crate::text::DvbText;

    #[test]
    fn parse_vvc_subpictures_with_description_round_trip() {
        // Table 162a: default_service_mode=true, 2 subpictures,
        // processing_mode=5, service_description = "Hi" (2 bytes).
        // byte0: ds=1(0x80) | sdp=1(0x40) | n=2(0x02) = 0xC2
        // subpicture 0: component_tag=0x10, vvc_subpicture_id=0x01
        // subpicture 1: component_tag=0x11, vvc_subpicture_id=0x02
        // processing_mode byte: 0x05
        // service_description_length: 2, then b'H', b'i'
        let sel = [0xC2, 0x10, 0x01, 0x11, 0x02, 0x05, 0x02, b'H', b'i'];
        let bytes = wrap(0x23, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::VvcSubpictures));
        match &d.body {
            ExtensionBody::VvcSubpictures(b) => {
                assert!(b.default_service_mode);
                assert_eq!(b.subpictures.len(), 2);
                assert_eq!(b.subpictures[0].component_tag, 0x10);
                assert_eq!(b.subpictures[0].vvc_subpicture_id, 0x01);
                assert_eq!(b.subpictures[1].component_tag, 0x11);
                assert_eq!(b.subpictures[1].vvc_subpicture_id, 0x02);
                assert_eq!(b.processing_mode, 5);
                assert!(b.service_description.is_some());
                let desc = b.service_description.as_ref().unwrap();
                assert_eq!(desc.raw(), b"Hi");
                assert_eq!(desc.decode(), "Hi");
            }
            other => panic!("expected VvcSubpictures, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_vvc_subpictures_no_description_round_trip() {
        // default_service_mode=false, 1 subpicture, processing_mode=1,
        // no service_description.
        // byte0: ds=0 | sdp=0 | n=1 = 0x01
        let sel = [0x01, 0x20, 0x03, 0x01];
        let bytes = wrap(0x23, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VvcSubpictures(b) => {
                assert!(!b.default_service_mode);
                assert_eq!(b.subpictures.len(), 1);
                assert_eq!(b.subpictures[0].component_tag, 0x20);
                assert_eq!(b.subpictures[0].vvc_subpicture_id, 0x03);
                assert_eq!(b.processing_mode, 1);
                assert!(b.service_description.is_none());
            }
            other => panic!("expected VvcSubpictures, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_vvc_subpictures_rejects_truncated() {
        // Only 2 selector bytes, need at least 1 + 0*2 + 1 = 2 bytes.
        let sel = [0x00]; // n=0, but no processing_mode byte
        let bytes = wrap(0x23, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
        ));
    }

    #[test]
    fn parse_vvc_subpictures_rejects_overrun() {
        // service_description_present but no length byte
        let sel = [0xC0]; // ds=1, sdp=1, n=0 — missing processing_mode + desc
        let bytes = wrap(0x23, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
        ));
    }

    #[test]
    fn parse_vvc_subpictures_serialize_round_trip_no_subpictures() {
        // Zero subpictures, no description — minimal valid payload.
        // byte0: ds=1(0x80) | sdp=0 | n=0 = 0x80
        let sel = [0x80, 0x03];
        let bytes = wrap(0x23, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VvcSubpictures(b) => {
                assert!(b.default_service_mode);
                assert!(b.subpictures.is_empty());
                assert_eq!(b.processing_mode, 3);
                assert!(b.service_description.is_none());
            }
            other => panic!("expected VvcSubpictures, got {other:?}"),
        }
        round_trip(&d);
    }

    /// Serialization is deterministic for an all-owned typed body.
    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_vvc_subpictures() {
        let d = ExtensionDescriptor {
            tag_extension: 0x23,
            body: ExtensionBody::VvcSubpictures(VvcSubpicturesDescriptor {
                default_service_mode: true,
                subpictures: vec![VvcSubpicture {
                    component_tag: 0x10,
                    vvc_subpicture_id: 0x01,
                }],
                processing_mode: 5,
                service_description: Some(DvbText::new(b"Hi")),
            }),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"tag_extension\":35"));
        assert!(json.contains("\"vvcSubpictures\""));
        assert!(json.contains("\"service_description\":\"Hi\""));
    }
}
