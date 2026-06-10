use super::*;

impl ExtensionBodyDef for VvcSubpicturesDescriptor<'_> {
    const TAG_EXTENSION: u8 = 0x23;
    const NAME: &'static str = "VVC_SUBPICTURES";
}

// ===========================================================================
//  Section 0x23 — vvc_subpictures_descriptor (Table 162a, §6.4.17)
// ---------------------------------------------------------------------------
//  byte 0: default_service_mode(1) service_description_present(1)
//          number_of_vvc_subpictures(6)
//  then a loop of (component_tag, vvc_subpicture_id) entries,
//  then one packed byte with processing_mode(3),
//  then optional length-delimited service_description text.
// ===========================================================================

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
