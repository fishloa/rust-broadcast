//! The iTunes gapless playback item (`iTunSMPB`).
//!
//! This is a de-facto convention rather than an ISO/IEC or Apple box
//! definition: a freeform `----` item in an `ilst` whose `mean` is
//! `com.apple.iTunes` and whose `name` is `iTunSMPB` carries one `data` value
//! of ASCII hex text. The value is a run of space-separated hexadecimal
//! integers whose first four fields are the gapless facts: a reserved zero,
//! the priming samples before the programme, the padding samples after it, and
//! the programme's original sample count. Apple's encoders write twelve
//! fields; a reader must accept any run long enough to hold the first four.
//!
//! The layout is taken from Apple's own writer (`afconvert`, AudioToolbox),
//! cross-checked against FFmpeg's reader `ff_itunes_parse_smpb`
//! (<https://github.com/FFmpeg/FFmpeg>, LGPL-2.1-or-later) and the field widths
//! observed in `private/fixtures/mp4/tags/afconvert-aac.m4a`. No published
//! standard defines it, so this module names it a de-facto format.
//!
//! The typed view borrows the item's text, so [`Metadata`](super::Metadata)'s
//! own serialization is untouched and a file that carries the item round-trips
//! byte for byte.

use crate::error::{Error, Result};

use super::{Metadata, MetadataItem};

/// The freeform item's `mean` namespace.
pub const ITUNSMPB_MEAN: &str = "com.apple.iTunes";
/// The freeform item's `name`.
pub const ITUNSMPB_NAME: &str = "iTunSMPB";
/// The fewest hexadecimal fields a value can carry and still state a trim.
pub const ITUNSMPB_MIN_FIELDS: usize = 4;
/// The number of hexadecimal digits in an 8-bit..32-bit field.
const FIELD_HEX_DIGITS: usize = 8;
/// The number of hexadecimal digits in the original sample count.
const COUNT_HEX_DIGITS: usize = 16;

/// The iTunes gapless playback facts of one `iTunSMPB` value.
///
/// Counts are samples per channel at the track's sample rate. `priming` is the
/// encoder's lead-in before the first programme sample, `padding` the samples
/// after the last one and `original_samples` the programme's length.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Itunsmpb {
    text: alloc::string::String,
    priming: u32,
    padding: u32,
    original_samples: u64,
}

impl Itunsmpb {
    /// Parse the hexadecimal value text.
    pub fn parse_text(text: &str) -> Result<Self> {
        let mut fields = text.split_ascii_whitespace();
        let reserved = fields
            .next()
            .ok_or(Error::InvalidInput("iTunSMPB value has no reserved field"))?;
        let priming = fields
            .next()
            .ok_or(Error::InvalidInput("iTunSMPB value has no priming field"))?;
        let padding = fields
            .next()
            .ok_or(Error::InvalidInput("iTunSMPB value has no padding field"))?;
        let count = fields
            .next()
            .ok_or(Error::InvalidInput("iTunSMPB value has no original sample count"))?;
        let _reserved = parse_u32_hex(reserved, FIELD_HEX_DIGITS, "iTunSMPB reserved field")?;
        Ok(Self {
            text: text.into(),
            priming: parse_u32_hex(priming, FIELD_HEX_DIGITS, "iTunSMPB priming field")?,
            padding: parse_u32_hex(padding, FIELD_HEX_DIGITS, "iTunSMPB padding field")?,
            original_samples: parse_u64_hex(
                count,
                COUNT_HEX_DIGITS,
                "iTunSMPB original sample count",
            )?,
        })
    }

    /// The value text exactly as the item carried it.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Samples per channel to discard from the start of the programme.
    #[must_use]
    pub const fn priming(&self) -> u32 {
        self.priming
    }

    /// Samples per channel to discard from the end of the programme.
    #[must_use]
    pub const fn padding(&self) -> u32 {
        self.padding
    }

    /// The programme's length in samples per channel, before the trim.
    #[must_use]
    pub const fn original_samples(&self) -> u64 {
        self.original_samples
    }
}

fn parse_u32_hex(field: &str, digits: usize, what: &'static str) -> Result<u32> {
    if field.len() != digits {
        return Err(Error::InvalidInput(what));
    }
    u32::from_str_radix(field, 16).map_err(|_| Error::InvalidInput(what))
}

fn parse_u64_hex(field: &str, digits: usize, what: &'static str) -> Result<u64> {
    if field.len() != digits {
        return Err(Error::InvalidInput(what));
    }
    u64::from_str_radix(field, 16).map_err(|_| Error::InvalidInput(what))
}

impl Metadata {
    /// The file's `iTunSMPB` gapless facts, when the item list carries them.
    ///
    /// The first freeform item whose `mean`/`name` name the convention wins;
    /// a value that is not valid hexadecimal text is ignored rather than
    /// failing the whole metadata read.
    #[must_use]
    pub fn itunsmpb(&self) -> Option<Itunsmpb> {
        self.items.iter().find_map(|item| match item {
            MetadataItem::Freeform { mean, name, values }
                if mean == ITUNSMPB_MEAN && name == ITUNSMPB_NAME =>
            {
                values
                    .first()
                    .and_then(|value| value.text())
                    .and_then(|text| Itunsmpb::parse_text(text).ok())
            }
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{FreeformValue, MetadataItem};
    use alloc::vec;

    const APPLE_TEXT: &str = " 00000000 00000840 000002F4 00000000000204CC 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000";

    fn metadata_with(text: &str) -> Metadata {
        let values = vec![FreeformValue {
            data_type: 1,
            locale: 0,
            bytes: text.as_bytes().to_vec(),
        }];
        Metadata {
            items: vec![MetadataItem::Freeform {
                mean: ITUNSMPB_MEAN.into(),
                name: ITUNSMPB_NAME.into(),
                values,
            }],
        }
    }

    #[test]
    fn an_apple_value_parses_its_four_leading_fields() {
        let metadata = metadata_with(APPLE_TEXT);
        let itunsmpb = metadata.itunsmpb().expect("parses");
        assert_eq!(itunsmpb.priming(), 2112);
        assert_eq!(itunsmpb.padding(), 756);
        assert_eq!(itunsmpb.original_samples(), 132_300);
        assert_eq!(itunsmpb.text(), APPLE_TEXT);
    }

    #[test]
    fn a_short_value_needs_only_the_four_leading_fields() {
        let itunsmpb =
            Itunsmpb::parse_text(" 00000000 00000100 00000020 0000000000001000").expect("parses");
        assert_eq!(itunsmpb.priming(), 256);
        assert_eq!(itunsmpb.padding(), 32);
        assert_eq!(itunsmpb.original_samples(), 4096);
    }

    #[test]
    fn a_value_with_a_bad_hex_field_is_refused() {
        assert!(Itunsmpb::parse_text(" 00000000 0000zzzz 00000020 0000000000001000").is_err());
        assert!(Itunsmpb::parse_text(" 00000000 00000100 00000020").is_err());
        assert!(Itunsmpb::parse_text("").is_err());
    }

    #[test]
    fn the_item_round_trips_byte_for_byte() {
        use crate::metadata::{Ilst, IlstItem};
        use broadcast_core::{Parse, Serialize};

        let values = [FreeformValue {
            data_type: 1,
            locale: 0,
            bytes: APPLE_TEXT.as_bytes().to_vec(),
        }];
        let item = IlstItem::freeform(ITUNSMPB_MEAN, ITUNSMPB_NAME, &values);
        let ilst = Ilst { items: vec![item] };
        let mut buf = vec![0xAAu8; ilst.serialized_len()];
        let len = ilst.serialize_into(&mut buf).expect("serializes");
        buf.truncate(len);
        let parsed = Ilst::parse(
            &buf,
        )
        .expect("parses");
        let mut out = vec![0xBBu8; parsed.serialized_len()];
        let n = parsed.serialize_into(&mut out).expect("serializes");
        assert_eq!(&out[..n], &buf[..]);
        let meta = Metadata::from_meta(&crate::metadata::Meta {
            flavor: crate::metadata::MetaFlavor::Iso,
            version: 0,
            flags: 0,
            children: vec![crate::metadata::MetaChild::Ilst(parsed)],
        });
        assert_eq!(meta.itunsmpb().expect("parses").text(), APPLE_TEXT);
    }
}
