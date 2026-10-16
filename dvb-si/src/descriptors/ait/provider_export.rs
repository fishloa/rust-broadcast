//! Provider Export Descriptor — ETSI TS 102 727 V1.1.1 §10.15, Table 93 (AIT tag
//! 0x12). PDF p.181, transcribed in `docs/descriptors/ts_102_727/93-*.md`,
//! verified against the rendered page.
//!
//! Used in the application (inner) AIT loop of an application that exports
//! an `XletBoundProvider`; "meaningless" in the common (outer) loop. Body:
//! repeated `provider_name_length`(8) + `provider_name_byte`×N +
//! `provider_class_name_length`(8) + `provider_class_name_byte`×M entries
//! filling `descriptor_length`. Neither string has a character set specified
//! by the clause, hence opaque bytes (the table prints the inner loop bounds
//! as `i<N; i++`, a typo for the preceding length field; see the
//! transcription's Spec note).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for provider_export_descriptor (AIT namespace).
pub const TAG: u8 = 0x12;
const HEADER_LEN: usize = 2;
const LEN_FIELD: usize = 1;

/// One exported provider: its name and the implementing class.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ProviderExport<'a> {
    /// `provider_name_byte` array — the provider's name.
    pub provider_name: &'a [u8],
    /// `provider_class_name_byte` array — fully qualified name of the class in
    /// the application that implements the named provider.
    pub provider_class_name: &'a [u8],
}

impl<'a> ProviderExport<'a> {
    /// Build an entry.
    #[must_use]
    pub const fn new(provider_name: &'a [u8], provider_class_name: &'a [u8]) -> Self {
        Self {
            provider_name,
            provider_class_name,
        }
    }
}

/// Provider Export Descriptor (AIT tag 0x12).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ProviderExportDescriptor<'a> {
    /// Exported providers in wire order.
    pub providers: Vec<ProviderExport<'a>>,
}

impl<'a> ProviderExportDescriptor<'a> {
    /// Build from the exported providers.
    #[must_use]
    pub fn new(providers: Vec<ProviderExport<'a>>) -> Self {
        Self { providers }
    }
}

/// Split a length-prefixed field off the front of `rest`.
fn take_field<'a>(rest: &'a [u8], reason: &'static str) -> Result<(&'a [u8], &'a [u8])> {
    let (&n, tail) = rest
        .split_first()
        .ok_or(Error::InvalidDescriptor { tag: TAG, reason })?;
    let n = usize::from(n);
    if tail.len() < n {
        return Err(Error::InvalidDescriptor { tag: TAG, reason });
    }
    Ok(tail.split_at(n))
}

impl<'a> Parse<'a> for ProviderExportDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut rest = descriptor_body(
            bytes,
            TAG,
            "ProviderExportDescriptor",
            "unexpected tag for provider_export_descriptor",
        )?;
        let mut providers = Vec::new();
        while !rest.is_empty() {
            let (provider_name, tail) = take_field(
                rest,
                "provider_export_descriptor provider_name overruns body",
            )?;
            let (provider_class_name, tail) = take_field(
                tail,
                "provider_export_descriptor provider_class_name missing or overruns body",
            )?;
            providers.push(ProviderExport {
                provider_name,
                provider_class_name,
            });
            rest = tail;
        }
        Ok(Self { providers })
    }
}

impl Serialize for ProviderExportDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self
                .providers
                .iter()
                .map(|p| {
                    LEN_FIELD + p.provider_name.len() + LEN_FIELD + p.provider_class_name.len()
                })
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
        let mut pos = HEADER_LEN;
        for p in &self.providers {
            for (field, what) in [
                (p.provider_name, "provider_name_length"),
                (p.provider_class_name, "provider_class_name_length"),
            ] {
                buf[pos] = broadcast_common::len::fit_u8(field.len(), what)?;
                pos += LEN_FIELD;
                buf[pos..pos + field.len()].copy_from_slice(field);
                pos += field.len();
            }
        }
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ProviderExportDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "PROVIDER_EXPORT";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Table 93 hand-built vector: tag 0x12, descriptor_length 14;
    //   03 'a' 'b' 'c'          provider_name_length=3 + "abc"
    //   05 'C' 'l' 'a' 's' 's'  provider_class_name_length=5 + "Class"
    //   00                      provider_name_length=0 (empty name)
    //   02 'p' 'Q'              provider_class_name_length=2 + "pQ"
    const BYTES: [u8; 16] = [
        TAG, 14, 3, b'a', b'b', b'c', 5, b'C', b'l', b'a', b's', b's', 0, 2, b'p', b'Q',
    ];

    #[test]
    fn parse_and_round_trip() {
        let d = ProviderExportDescriptor::parse(&BYTES).unwrap();
        assert_eq!(d.providers.len(), 2);
        assert_eq!(d.providers[0].provider_name, b"abc");
        assert_eq!(d.providers[0].provider_class_name, b"Class");
        assert_eq!(d.providers[1].provider_name, b"");
        assert_eq!(d.providers[1].provider_class_name, b"pQ");
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, BYTES);
    }

    #[test]
    fn empty_loop_and_errors() {
        assert!(ProviderExportDescriptor::parse(&[TAG, 0])
            .unwrap()
            .providers
            .is_empty());
        // name present, class length byte missing
        assert!(ProviderExportDescriptor::parse(&[TAG, 2, 1, b'a']).is_err());
        // class overruns
        assert!(ProviderExportDescriptor::parse(&[TAG, 4, 1, b'a', 3, b'x']).is_err());
        assert!(ProviderExportDescriptor::parse(&[0x13, 0]).is_err());
        let long = vec![0u8; 256];
        let d = ProviderExportDescriptor::new(vec![ProviderExport::new(b"a", &long)]);
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
