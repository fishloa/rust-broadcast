//! Provider Usage Descriptor — ETSI TS 102 727 V1.1.1 §10.15, Table 94 (AIT tag
//! 0x13). PDF p.181, transcribed in `docs/descriptors/ts_102_727/94-*.md`,
//! verified against the rendered page.
//!
//! Used in either the application (inner) or common (outer) AIT loop of an
//! application that uses an `XletBoundProvider` without carrying the provider
//! classes itself. Body: repeated `provider_name_length`(8) +
//! `provider_name_byte`×N entries filling `descriptor_length`. The name is
//! "a string" with no character set specified by the clause, hence opaque
//! bytes (the table prints the inner loop bounds as `i<N; i++`, a typo for
//! `j<provider_name_length`; see the transcription's Spec note).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for provider_usage_descriptor (AIT namespace).
pub const TAG: u8 = 0x13;
const HEADER_LEN: usize = 2;
const NAME_LEN_FIELD: usize = 1;

/// Provider Usage Descriptor (AIT tag 0x13).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ProviderUsageDescriptor<'a> {
    /// `provider_name_byte` arrays, one per used provider, in wire order.
    pub provider_names: Vec<&'a [u8]>,
}

impl<'a> ProviderUsageDescriptor<'a> {
    /// Build from the used provider names.
    #[must_use]
    pub fn new(provider_names: Vec<&'a [u8]>) -> Self {
        Self { provider_names }
    }
}

impl<'a> Parse<'a> for ProviderUsageDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut rest = descriptor_body(
            bytes,
            TAG,
            "ProviderUsageDescriptor",
            "unexpected tag for provider_usage_descriptor",
        )?;
        let mut provider_names = Vec::new();
        while let Some((&n, tail)) = rest.split_first() {
            let n = usize::from(n);
            if tail.len() < n {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "provider_usage_descriptor name overruns body",
                });
            }
            provider_names.push(&tail[..n]);
            rest = &tail[n..];
        }
        Ok(Self { provider_names })
    }
}

impl Serialize for ProviderUsageDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self
                .provider_names
                .iter()
                .map(|n| NAME_LEN_FIELD + n.len())
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
        for n in &self.provider_names {
            buf[pos] = broadcast_common::len::fit_u8(n.len(), "provider_name_length")?;
            pos += NAME_LEN_FIELD;
            buf[pos..pos + n.len()].copy_from_slice(n);
            pos += n.len();
        }
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ProviderUsageDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "PROVIDER_USAGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Table 94 hand-built vector: tag 0x13, descriptor_length 8;
    //   03 'a' 'b' 'c'   provider_name_length=3 + "abc"
    //   00               provider_name_length=0 (empty name)
    //   02 'x' 'y'       provider_name_length=2 + "xy"
    const BYTES: [u8; 10] = [TAG, 8, 3, b'a', b'b', b'c', 0, 2, b'x', b'y'];

    #[test]
    fn parse_and_round_trip() {
        let d = ProviderUsageDescriptor::parse(&BYTES).unwrap();
        assert_eq!(d.provider_names, vec![&b"abc"[..], &b""[..], &b"xy"[..]]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, BYTES);
    }

    #[test]
    fn empty_loop_and_errors() {
        let d = ProviderUsageDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.provider_names.is_empty());
        assert!(ProviderUsageDescriptor::parse(&[TAG, 2, 5, b'a']).is_err());
        assert!(ProviderUsageDescriptor::parse(&[0x12, 0]).is_err());
        let long = vec![0u8; 256];
        let d = ProviderUsageDescriptor::new(vec![&long]);
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
