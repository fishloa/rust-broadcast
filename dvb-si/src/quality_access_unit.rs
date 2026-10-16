//! `Quality_Access_Unit()` — Rec. ITU-T H.222.0 (08/2023) | ISO/IEC
//! 13818-1:2023 §2.20.1, Table 2-164 (PDF pp.250-251, printed pp.228-229),
//! syntax in `docs/tables/iso_13818_1/2-164-quality-access-unit.md`, verified
//! on the rendered pages.
//!
//! The access unit is carried "in MPEG sections" (stream_type 0x2F,
//! table_id 0x0A, Table 2-31), but the vendored H.222.0 text defines **only
//! the access-unit body**: it gives no section-header syntax (is the section
//! short or long form, is there a CRC_32?) for table_id 0x0A, unlike the
//! sibling sections 0x04, 0x05, 0x08 and 0x09. A section wrapper would
//! therefore be a guess and is deliberately absent; only the body Table 2-164
//! defines is typed here, as a standalone [`QualityAccessUnit`] that a caller
//! can parse out of the section payload it has already framed.
//!
//! ```text
//! Quality_Access_Unit() {
//!   field_size_bytes                8
//!   metric_count                    8
//!   for (i = 0; i < metric_count; i++) {
//!     metric_code                  32
//!     sample_count                  8
//!     for (j = 0; j < sample_count; j++) {
//!       '0010'(4) media_DTS[32..30](3) marker_bit(1)
//!       media_DTS[29..15](15) marker_bit(1) media_DTS[14..0](15) marker_bit(1)
//!       quality_metric_sample       N = 8 * field_size_bytes
//!     }
//!   }
//! }
//! ```
//!
//! The 40-bit `'0010'`-prefixed DTS is the shared marker-bit timestamp codec
//! ([`broadcast_common::clock33::unpack_marked`] / [`pack_marked`]); a field
//! without the `'0010'` prefix or with a clear marker bit is rejected, as the
//! table prints them as literals. `quality_metric_sample` is an unsigned
//! integer of `field_size_bytes` bytes ("padded by preceding zero bytes");
//! it is kept as that big-endian byte run (the width is data-dependent and
//! can exceed 64 bits) with [`QualitySample::value_u64`] for the common case.
//! `metric_code` semantics (ISO/IEC 23001-10 §4.2) are not vendored, so the
//! code stays numeric. The structure must consume the buffer exactly: the
//! text defines no trailing bytes.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::clock33::{self, pack_marked, unpack_marked};
use broadcast_common::{Parse, Serialize};

/// `field_size_bytes` + `metric_count`.
const HEADER_LEN: usize = 2;
/// `metric_code` (32) + `sample_count` (8).
const METRIC_HEADER_LEN: usize = 5;
/// One 5-byte marker-bit `media_DTS`.
const DTS_FIELD_LEN: usize = 5;
/// The literal `'0010'` ahead of `media_DTS[32..30]`.
const DTS_PREFIX: u8 = 0b0010;
/// A 33-bit DTS.
const DTS_BITS: u32 = 33;
const WHAT: &str = "QualityAccessUnit";

/// One `quality_metric_sample` with its `media_DTS`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct QualitySample<'a> {
    /// `media_DTS`: the 33-bit DTS of the media access unit the sample
    /// describes.
    pub media_dts: u64,
    /// `quality_metric_sample`: `field_size_bytes` bytes, a big-endian unsigned
    /// integer.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub quality_metric_sample: &'a [u8],
}

impl<'a> QualitySample<'a> {
    /// Build a sample.
    #[must_use]
    pub const fn new(media_dts: u64, quality_metric_sample: &'a [u8]) -> Self {
        Self {
            media_dts,
            quality_metric_sample,
        }
    }

    /// The sample as an integer, or `None` when it is wider than 8 bytes and
    /// its high bytes are not all zero.
    #[must_use]
    pub fn value_u64(&self) -> Option<u64> {
        const U64_BYTES: usize = 8;
        let (high, low) = self
            .quality_metric_sample
            .split_at(self.quality_metric_sample.len().saturating_sub(U64_BYTES));
        if high.iter().any(|b| *b != 0) {
            return None;
        }
        Some(low.iter().fold(0u64, |v, b| (v << 8) | u64::from(*b)))
    }
}

/// One metric of the access unit: its code and its samples.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct QualityMetric<'a> {
    /// `metric_code` (32 bits; ISO/IEC 23001-10 §4.2, not vendored).
    pub metric_code: u32,
    /// The `sample_count` samples, in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub samples: Vec<QualitySample<'a>>,
}

impl<'a> QualityMetric<'a> {
    /// Build a metric.
    #[must_use]
    pub fn new(metric_code: u32, samples: Vec<QualitySample<'a>>) -> Self {
        Self {
            metric_code,
            samples,
        }
    }
}

/// `Quality_Access_Unit()` — Table 2-164.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct QualityAccessUnit<'a> {
    /// `field_size_bytes`: the size of every `quality_metric_sample`.
    pub field_size_bytes: u8,
    /// The `metric_count` metrics, in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub metrics: Vec<QualityMetric<'a>>,
}

impl<'a> QualityAccessUnit<'a> {
    /// Build an access unit.
    #[must_use]
    pub fn new(field_size_bytes: u8, metrics: Vec<QualityMetric<'a>>) -> Self {
        Self {
            field_size_bytes,
            metrics,
        }
    }

    fn sample_len(&self) -> usize {
        DTS_FIELD_LEN + usize::from(self.field_size_bytes)
    }
}

fn short(need: usize, have: usize) -> Error {
    Error::BufferTooShort {
        need,
        have,
        what: WHAT,
    }
}

impl<'a> Parse<'a> for QualityAccessUnit<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, mut rest) = bytes
            .split_first_chunk::<HEADER_LEN>()
            .ok_or_else(|| short(HEADER_LEN, bytes.len()))?;
        let field_size_bytes = header[0];
        let metric_count = header[1];
        let sample_len = DTS_FIELD_LEN + usize::from(field_size_bytes);
        let mut metrics = Vec::new();
        for _ in 0..metric_count {
            let (mh, tail) = rest
                .split_first_chunk::<METRIC_HEADER_LEN>()
                .ok_or_else(|| short(bytes.len() + METRIC_HEADER_LEN, bytes.len()))?;
            let metric_code = u32::from_be_bytes([mh[0], mh[1], mh[2], mh[3]]);
            let sample_count = usize::from(mh[4]);
            rest = tail;
            let mut samples = Vec::new();
            for _ in 0..sample_count {
                if rest.len() < sample_len {
                    return Err(short(bytes.len() + sample_len - rest.len(), bytes.len()));
                }
                let (sample, tail) = rest.split_at(sample_len);
                let (dts, value) = sample.split_at(DTS_FIELD_LEN);
                let mut field = [0u8; DTS_FIELD_LEN];
                field.copy_from_slice(dts);
                let marked = unpack_marked(&field);
                if marked.prefix != DTS_PREFIX || marked.markers != clock33::MARKERS_ALL_SET {
                    return Err(Error::ReservedBitsViolation {
                        field: "media_DTS",
                        reason: "'0010' prefix or marker_bit missing",
                    });
                }
                samples.push(QualitySample {
                    media_dts: marked.timestamp,
                    quality_metric_sample: value,
                });
                rest = tail;
            }
            metrics.push(QualityMetric {
                metric_code,
                samples,
            });
        }
        if !rest.is_empty() {
            return Err(Error::ValueOutOfRange {
                field: "Quality_Access_Unit",
                reason: "bytes remain after the last metric (Table 2-164 defines none)",
            });
        }
        Ok(Self {
            field_size_bytes,
            metrics,
        })
    }
}

impl Serialize for QualityAccessUnit<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + self
                .metrics
                .iter()
                .map(|m| METRIC_HEADER_LEN + m.samples.len() * self.sample_len())
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
        buf[0] = self.field_size_bytes;
        buf[1] = broadcast_common::len::fit_u8(self.metrics.len(), "metric_count")?;
        let mut pos = HEADER_LEN;
        for m in &self.metrics {
            buf[pos..pos + 4].copy_from_slice(&m.metric_code.to_be_bytes());
            buf[pos + 4] = broadcast_common::len::fit_u8(m.samples.len(), "sample_count")?;
            pos += METRIC_HEADER_LEN;
            for s in &m.samples {
                if s.quality_metric_sample.len() != usize::from(self.field_size_bytes) {
                    return Err(Error::ValueOutOfRange {
                        field: "quality_metric_sample",
                        reason: "length differs from field_size_bytes",
                    });
                }
                let dts =
                    broadcast_common::len::fit_bits(s.media_dts, DTS_BITS, "media_DTS")?;
                buf[pos..pos + DTS_FIELD_LEN].copy_from_slice(&pack_marked(dts, DTS_PREFIX));
                pos += DTS_FIELD_LEN;
                buf[pos..pos + s.quality_metric_sample.len()]
                    .copy_from_slice(s.quality_metric_sample);
                pos += s.quality_metric_sample.len();
            }
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hand-computed from Table 2-164 (spec vector only: no real instance in the
    // committed corpus).
    #[test]
    fn dts_bytes_are_marker_coded() {
        // 33-bit DTS = 0x1_2345_6789:
        //   [32..30] = 0b100
        //   [29..15] = (0x1_2345_6789 >> 15) & 0x7FFF = 0x468A
        //   [14..0]  = 0x1_2345_6789 & 0x7FFF         = 0x6789
        // byte0   = '0010' | 100 | marker = 0010_1001           = 0x29
        // byte1-2 = (0x468A << 1) | marker = 0x8D15             = 0x8D 0x15
        // byte3-4 = (0x6789 << 1) | marker = 0xCF13             = 0xCF 0x13
        assert_eq!(
            pack_marked(0x1_2345_6789, DTS_PREFIX),
            [0x29, 0x8D, 0x15, 0xCF, 0x13]
        );
    }

    #[test]
    fn parse_and_round_trip() {
        let b = [
            0x02, 0x02, // field_size_bytes=2, metric_count=2
            0x00, 0x00, 0x00, 0x01, 0x02, // metric 0: code 1, 2 samples
            0x29, 0x8D, 0x15, 0xCF, 0x13, 0x01, 0x02, // DTS 0x1_2345_6789, sample 0x0102
            0x21, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, // DTS 0 (0010 000 1, 0x0001, 0x0001), sample 0
            0x00, 0x00, 0x00, 0x02, 0x00, // metric 1: code 2, 0 samples
        ];
        let d = QualityAccessUnit::parse(&b).unwrap();
        assert_eq!(d.field_size_bytes, 2);
        assert_eq!(d.metrics.len(), 2);
        assert_eq!(d.metrics[0].metric_code, 1);
        assert_eq!(d.metrics[0].samples[0].media_dts, 0x1_2345_6789);
        assert_eq!(d.metrics[0].samples[0].value_u64(), Some(0x0102));
        assert_eq!(d.metrics[0].samples[1].media_dts, 0);
        assert_eq!(d.metrics[0].samples[1].value_u64(), Some(0));
        assert!(d.metrics[1].samples.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, b);
    }

    #[test]
    fn wide_samples_and_value_u64() {
        let wide = [0u8, 0, 0, 0, 0, 0, 0, 0, 0xAB];
        assert_eq!(QualitySample::new(0, &wide).value_u64(), Some(0xAB));
        let too_wide = [1u8, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(QualitySample::new(0, &too_wide).value_u64(), None);
        let unit = QualityAccessUnit::new(
            9,
            vec![QualityMetric::new(7, vec![QualitySample::new(5, &wide)])],
        );
        let mut out = vec![0u8; unit.serialized_len()];
        unit.serialize_into(&mut out).unwrap();
        assert_eq!(QualityAccessUnit::parse(&out).unwrap(), unit);
    }

    #[test]
    fn malformed_is_rejected() {
        assert!(QualityAccessUnit::parse(&[]).is_err());
        // metric_count=1 but no metric header
        assert!(QualityAccessUnit::parse(&[0x01, 0x01]).is_err());
        // truncated sample
        assert!(QualityAccessUnit::parse(&[0x01, 0x01, 0, 0, 0, 1, 1, 0x21, 0, 1]).is_err());
        // bad DTS prefix ('0011' instead of '0010')
        let bad = [0x01, 0x01, 0, 0, 0, 1, 1, 0x31, 0x00, 0x01, 0x00, 0x01, 0x07];
        assert!(matches!(
            QualityAccessUnit::parse(&bad),
            Err(Error::ReservedBitsViolation { .. })
        ));
        // clear marker bit
        let bad = [0x01, 0x01, 0, 0, 0, 1, 1, 0x20, 0x00, 0x01, 0x00, 0x01, 0x07];
        assert!(QualityAccessUnit::parse(&bad).is_err());
        // trailing byte
        let ok = [0x01, 0x00, 0xFF];
        assert!(QualityAccessUnit::parse(&ok).is_err());
        // sample width differs from field_size_bytes / DTS over 33 bits
        let unit = QualityAccessUnit::new(
            2,
            vec![QualityMetric::new(1, vec![QualitySample::new(0, &[1])])],
        );
        assert!(unit.serialize_into(&mut [0u8; 32]).is_err());
        let unit = QualityAccessUnit::new(
            1,
            vec![QualityMetric::new(
                1,
                vec![QualitySample::new(1 << 33, &[1])],
            )],
        );
        assert!(unit.serialize_into(&mut [0u8; 32]).is_err());
    }
}
