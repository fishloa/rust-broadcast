//! Target Region Name Descriptor — ETSI EN 300 468 §6.4.13 (tag_extension 0x0A).
use super::*;
use alloc::vec::Vec;
use broadcast_core::len::{need_in, need_out};

impl<'a> ExtensionBodyDef<'a> for TargetRegionName<'a> {
    const TAG_EXTENSION: u8 = 0x0A;
    const NAME: &'static str = "TARGET_REGION_NAME";
}

/// Bytes of the leading country_code + ISO_639_language_code header.
const HEADER_LEN: usize = 2 * ISO_639_LEN;
/// Offset of the ISO_639_language_code.
const LANGUAGE_CODE_OFFSET: usize = ISO_639_LEN;
/// Shift of `region_depth` in the entry flags byte.
const REGION_DEPTH_SHIFT: u32 = 6;
/// Mask of `region_name_length` in the entry flags byte.
const REGION_NAME_LENGTH_MASK: u8 = 0x3F;
/// Width of `region_name_length`.
const REGION_NAME_LENGTH_BITS: u32 = 6;
/// Mask of `region_depth`.
const REGION_DEPTH_MASK: u8 = 0x03;
/// Lowest `region_depth` that carries a secondary region code.
const SECONDARY_REGION_MIN_DEPTH: u8 = 2;
/// `region_depth` that carries a tertiary region code.
const TERTIARY_REGION_DEPTH: u8 = 3;
/// Width of `tertiary_region_code`.
const TERTIARY_REGION_CODE_LEN: usize = core::mem::size_of::<u16>();

fn decode_lang_code(bytes: &[u8], offset: usize) -> LangCode {
    let mut code = [0u8; ISO_639_LEN];
    code.copy_from_slice(&bytes[offset..offset + ISO_639_LEN]);
    LangCode(code)
}
/// target_region_name body (Table 157, §6.4.13). The region loop is unfolded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetRegionName<'a> {
    /// country_code(24).
    pub country_code: LangCode,
    /// ISO_639_language_code(24).
    pub iso_639_language_code: LangCode,
    /// Region name entries (the loop).
    pub regions: Vec<TargetRegionNameEntry<'a>>,
}

/// An entry in the target_region_name_descriptor region loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TargetRegionNameEntry<'a> {
    /// region_depth(2).
    pub region_depth: u8,
    /// region name (char run, name_length bytes) — DVB Annex-A text.
    pub region_name: DvbText<'a>,
    /// primary_region_code(8), always present.
    pub primary_region_code: u8,
    /// secondary_region_code(8), present iff region_depth>=2.
    pub secondary_region_code: Option<u8>,
    /// tertiary_region_code(16), present iff region_depth==3.
    pub tertiary_region_code: Option<u16>,
}

impl<'a> Parse<'a> for TargetRegionName<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        need_in(sel, HEADER_LEN, "target_region_name body")?;
        let country_code = decode_lang_code(sel, 0);
        let iso_639_language_code = decode_lang_code(sel, LANGUAGE_CODE_OFFSET);
        let mut regions = Vec::new();
        let mut pos = HEADER_LEN;
        while pos < sel.len() {
            let byte = sel[pos];
            pos += 1;
            let region_depth = byte >> REGION_DEPTH_SHIFT;
            let name_length = (byte & REGION_NAME_LENGTH_MASK) as usize;
            need_in(sel, pos + name_length, "target_region_name body")?;
            let region_name = DvbText::new(&sel[pos..pos + name_length]);
            pos += name_length;
            if pos >= sel.len() {
                return Err(Error::BufferTooShort(
                    broadcast_core::error::BufferTooShort {
                        need: pos + 1,
                        have: sel.len(),
                        what: "target_region_name body",
                    },
                ));
            }
            let primary_region_code = sel[pos];
            pos += 1;
            let secondary_region_code = if region_depth >= SECONDARY_REGION_MIN_DEPTH {
                if pos >= sel.len() {
                    return Err(Error::BufferTooShort(
                        broadcast_core::error::BufferTooShort {
                            need: pos + 1,
                            have: sel.len(),
                            what: "target_region_name body",
                        },
                    ));
                }
                let val = sel[pos];
                pos += 1;
                Some(val)
            } else {
                None
            };
            let tertiary_region_code = if region_depth == TERTIARY_REGION_DEPTH {
                if pos + 1 >= sel.len() {
                    return Err(Error::BufferTooShort(
                        broadcast_core::error::BufferTooShort {
                            need: pos + TERTIARY_REGION_CODE_LEN,
                            have: sel.len(),
                            what: "target_region_name body",
                        },
                    ));
                }
                let (b, _) = sel
                    .get(pos..)
                    .and_then(|s| s.split_first_chunk::<{ TERTIARY_REGION_CODE_LEN }>())
                    .ok_or(Error::BufferTooShort(
                        broadcast_core::error::BufferTooShort {
                            need: pos + TERTIARY_REGION_CODE_LEN,
                            have: sel.len(),
                            what: "target_region_name body",
                        },
                    ))?;
                let val = u16::from_be_bytes(*b);
                pos += TERTIARY_REGION_CODE_LEN;
                Some(val)
            } else {
                None
            };
            regions.push(TargetRegionNameEntry {
                region_depth,
                region_name,
                primary_region_code,
                secondary_region_code,
                tertiary_region_code,
            });
        }
        Ok(TargetRegionName {
            country_code,
            iso_639_language_code,
            regions,
        })
    }
}

impl Serialize for TargetRegionName<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self
                .regions
                .iter()
                // Byte counts must come from the Options the writer actually
                // writes; `region_depth` duplicating them used to leave stale
                // bytes (r03-W6).
                .map(|r| {
                    1 + r.region_name.raw().len()
                        + 1
                        + usize::from(r.secondary_region_code.is_some())
                        + usize::from(r.tertiary_region_code.is_some()) * TERTIARY_REGION_CODE_LEN
                })
                .sum::<usize>()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[..ISO_639_LEN].copy_from_slice(&self.country_code.0);
        buf[ISO_639_LEN..HEADER_LEN].copy_from_slice(&self.iso_639_language_code.0);
        let mut pos = HEADER_LEN;
        for region in &self.regions {
            let raw = region.region_name.raw();
            let region_name_length = broadcast_core::len::fit_bits(
                raw.len() as u64,
                REGION_NAME_LENGTH_BITS,
                "region_name_length",
            )? as u8;
            buf[pos] = (broadcast_core::len::fit_mask(
                region.region_depth,
                REGION_DEPTH_MASK,
                "region_depth",
            )? << REGION_DEPTH_SHIFT)
                | region_name_length;
            pos += 1;
            buf[pos..pos + raw.len()].copy_from_slice(raw);
            pos += raw.len();
            buf[pos] = region.primary_region_code;
            pos += 1;
            if let Some(sec) = region.secondary_region_code {
                buf[pos] = sec;
                pos += 1;
            }
            if let Some(tert) = region.tertiary_region_code {
                buf[pos..pos + TERTIARY_REGION_CODE_LEN].copy_from_slice(&tert.to_be_bytes());
                pos += TERTIARY_REGION_CODE_LEN;
            }
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};
    use crate::text::LangCode;
    use alloc::vec;

    #[test]
    fn parse_target_region_name_structured() {
        let sel = [b'g', b'b', b'r', b'e', b'n', b'g', 0x42, b'H', b'i', 0x12];
        let bytes = wrap(0x0A, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegionName(b) => {
                assert_eq!(b.country_code, LangCode(*b"gbr"));
                assert_eq!(b.iso_639_language_code, LangCode(*b"eng"));
                assert_eq!(b.regions.len(), 1);
                assert_eq!(b.regions[0].region_depth, 1);
                assert_eq!(b.regions[0].region_name.raw(), b"Hi");
                assert_eq!(b.regions[0].primary_region_code, 0x12);
                assert_eq!(b.regions[0].secondary_region_code, None);
                assert_eq!(b.regions[0].tertiary_region_code, None);
            }
            other => panic!("expected TargetRegionName, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn target_region_name_tsduck_empty() {
        let bytes = test_fixtures::hex_bytes("7f070a666f6f626172");
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegionName(b) => {
                assert_eq!(b.country_code, LangCode(*b"foo"));
                assert_eq!(b.iso_639_language_code, LangCode(*b"bar"));
                assert!(b.regions.is_empty());
            }
            other => panic!("expected TargetRegionName, got {other:?}"),
        }
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    #[test]
    fn target_region_name_tsduck_full() {
        let bytes = test_fixtures::hex_bytes(
            "7f260a4142434445464c726567696f6e20666f6f203112cc726567696f6e2062617220323456789a",
        );
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegionName(b) => {
                assert_eq!(b.country_code, LangCode(*b"ABC"));
                assert_eq!(b.iso_639_language_code, LangCode(*b"DEF"));
                assert_eq!(b.regions.len(), 2);
                // [0] region_depth=1, region_name="region foo 1", primary=0x12
                assert_eq!(b.regions[0].region_depth, 1);
                assert_eq!(b.regions[0].region_name.raw(), b"region foo 1");
                assert_eq!(b.regions[0].primary_region_code, 0x12);
                assert_eq!(b.regions[0].secondary_region_code, None);
                assert_eq!(b.regions[0].tertiary_region_code, None);
                // [1] region_depth=3, region_name="region bar 2", primary=0x34, sec=0x56, tert=0x789A
                assert_eq!(b.regions[1].region_depth, 3);
                assert_eq!(b.regions[1].region_name.raw(), b"region bar 2");
                assert_eq!(b.regions[1].primary_region_code, 0x34);
                assert_eq!(b.regions[1].secondary_region_code, Some(0x56));
                assert_eq!(b.regions[1].tertiary_region_code, Some(0x789A));
            }
            other => panic!("expected TargetRegionName, got {other:?}"),
        }
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    /// `region_name_length` is a 6-bit field (max 63, #1129): 63 bytes still
    /// serialize; 64 must be rejected instead of silently wrapping the length.
    #[test]
    fn region_name_length_boundary() {
        let make = |len: usize| TargetRegionName {
            country_code: LangCode(*b"ABC"),
            iso_639_language_code: LangCode(*b"DEF"),
            regions: vec![TargetRegionNameEntry {
                region_depth: 0,
                region_name: DvbText::new(&VEC_63[..len]),
                primary_region_code: 0,
                secondary_region_code: None,
                tertiary_region_code: None,
            }],
        };
        const VEC_63: [u8; 64] = [b'x'; 64];

        let ok = make(63);
        let mut buf = vec![0u8; ok.serialized_len()];
        ok.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[2 * ISO_639_LEN] & 0x3F, 63);

        let over = make(64);
        let mut buf = vec![0u8; over.serialized_len()];
        let err = over.serialize_into(&mut buf).unwrap_err();
        assert!(
            matches!(err, Error::FieldOverflow(_)),
            "expected FieldOverflow, got {err:?}"
        );
    }
}
