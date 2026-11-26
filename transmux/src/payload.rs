//! Payload plug-ins: one sample of an opaque data track, rendered as values.
//!
//! A [`CodecConfig::Data`](crate::ir::CodecConfig::Data) track carries coded
//! bytes this crate deliberately never interprets (camera telemetry, KLV
//! metadata, a broadcaster's private stream). A [`PayloadDecoder`] is the
//! plug-in that does interpret one: it claims the tracks it understands and
//! turns one sample into named [`PayloadValue`]s, so a diagnostic tool can
//! print readable values without the muxing hub taking a dependency on every
//! metadata standard it might carry.
//!
//! The registry is caller-owned ([`PayloadRegistry`]): there is no global or
//! static plug-in table, so which decoders exist is decided by the binary's
//! cargo features and nothing is linked that the caller did not ask for.
//!
//! # Built-in plug-ins
//!
//! - [`St0601Decoder`] (`motion-imagery-metadata` feature) decodes a MISB
//!   ST 0601 UAS Datalink Local Set, the KLV packet
//!   [`crate::klv::UasLocalSet`] frames. Its item values come from the
//!   `motion-imagery-metadata` crate's typed decode; its keys are the ST 0601
//!   tag numbers as `tag_<n>`, because that crate's public `St0601Value` names
//!   what a value *is*, not what the tag means, and exposes no tag-name table.
//!
//! # Spec citations
//!
//! - **MISB ST 0601** UAS Datalink Local Set: tag 1 Checksum (a running 16-bit
//!   sum, §5.5/§7.1) and tag 2 Precision Time Stamp (µs since the POSIX
//!   epoch). See `private/specs-verified/mpeg/transmux/klv/klv-misb0601.md`.
//! - **SMPTE ST 336** KLV framing, via [`crate::klv`].
//!
//! `no_std` + `alloc`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use bytes::Bytes;

use crate::error::Result;
use crate::ir::{Sample, TrackSpec};

/// One decoded value of a data-track sample.
///
/// `#[non_exhaustive]`: a future plug-in's value shape is additive.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum PayloadScalar {
    /// UTF-8 text.
    Text(String),
    /// A signed integer.
    Integer(i64),
    /// An unsigned integer.
    Unsigned(u64),
    /// A real number.
    Float(f64),
    /// Raw bytes a plug-in does not interpret further.
    Bytes(Bytes),
    /// An instant, in microseconds since the POSIX epoch.
    Timestamp(u64),
    /// A boolean flag.
    Boolean(bool),
}

/// One named value a [`PayloadDecoder`] read out of a sample.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PayloadValue {
    /// The value's name within the payload, e.g. an ST 0601 `tag_5`.
    pub key: String,
    /// The decoded value.
    pub value: PayloadScalar,
}

impl PayloadValue {
    /// A named value.
    pub fn new(key: impl Into<String>, value: PayloadScalar) -> Self {
        Self {
            key: key.into(),
            value,
        }
    }
}

/// A plug-in that renders one sample of a data track as structured values.
pub trait PayloadDecoder: Send + Sync {
    /// The plug-in's stable name, as a caller selects it (e.g. on a CLI).
    fn name(&self) -> &'static str;

    /// Whether this plug-in understands `sample` on the track `spec` describes.
    ///
    /// A claim is about the *content* as well as the track: two data tracks in
    /// one file may carry different payloads under the same container identity.
    fn claims(&self, spec: &TrackSpec, sample: &Sample) -> bool;

    /// Decode one sample this plug-in claims.
    ///
    /// # Errors
    ///
    /// A malformed payload is reported here, never as a panic and never by
    /// silently dropping the sample — the caller decides what a bad sample
    /// means.
    fn decode(&self, sample: &Sample) -> Result<Vec<PayloadValue>>;
}

/// A caller-owned set of payload plug-ins.
///
/// [`decode`](Self::decode) runs the first decoder whose
/// [`claims`](PayloadDecoder::claims) accepts the sample, so registration
/// order is claim precedence.
#[derive(Default)]
pub struct PayloadRegistry {
    decoders: Vec<Box<dyn PayloadDecoder>>,
}

impl PayloadRegistry {
    /// A registry with no plug-ins.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a plug-in after every one already registered.
    pub fn register(&mut self, decoder: Box<dyn PayloadDecoder>) {
        self.decoders.push(decoder);
    }

    /// Whether no plug-in is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decoders.is_empty()
    }

    /// Decode `sample` with the first registered plug-in that claims it.
    ///
    /// Returns `None` when no plug-in claims the sample, else the claiming
    /// plug-in's name and its result.
    pub fn decode(
        &self,
        spec: &TrackSpec,
        sample: &Sample,
    ) -> Option<Result<(&'static str, Vec<PayloadValue>)>> {
        let decoder = self.decoders.iter().find(|d| d.claims(spec, sample))?;
        Some(
            decoder
                .decode(sample)
                .map(|values| (decoder.name(), values)),
        )
    }
}

#[cfg(feature = "motion-imagery-metadata")]
pub use st0601::St0601Decoder;

#[cfg(feature = "motion-imagery-metadata")]
mod st0601 {
    use alloc::format;
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;
    use broadcast_core::Parse;
    use bytes::Bytes;

    use crate::error::Result;
    use crate::ir::{CodecConfig, Sample, TrackSpec};
    use crate::klv::{LocalSetItem, TAG_CHECKSUM, UAS_LS_KEY, UasLocalSet};

    use super::{PayloadDecoder, PayloadScalar, PayloadValue};

    /// The MISB ST 0601 UAS Datalink Local Set payload plug-in.
    ///
    /// Claims a sample of any `CodecConfig::Data` track whose bytes start with
    /// the [`UAS_LS_KEY`] Universal Label; the container identity is
    /// irrelevant, so a KLV packet carried over a TS private PES, an MP4
    /// `gpmd` sample entry or a Matroska track decodes the same way.
    pub struct St0601Decoder;

    impl St0601Decoder {
        /// The plug-in's name.
        pub const NAME: &'static str = "motion-imagery-metadata";
    }

    impl PayloadDecoder for St0601Decoder {
        fn name(&self) -> &'static str {
            Self::NAME
        }

        fn claims(&self, spec: &TrackSpec, sample: &Sample) -> bool {
            matches!(spec.config, CodecConfig::Data { .. }) && sample.data.starts_with(&UAS_LS_KEY)
        }

        fn decode(&self, sample: &Sample) -> Result<Vec<PayloadValue>> {
            let set = UasLocalSet::parse(&sample.data)?;
            let checksum_valid = UasLocalSet::verify_checksum(&sample.data)?;
            let mut values = Vec::with_capacity(set.items.len() + 1);
            for item in set.items.iter().filter(|i| i.tag != TAG_CHECKSUM) {
                values.push(PayloadValue::new(st0601_key(item), st0601_scalar(item)));
            }
            values.push(PayloadValue::new(
                "checksum_valid",
                PayloadScalar::Boolean(checksum_valid),
            ));
            Ok(values)
        }
    }

    /// The ST 0601 tag number as this crate's key (`tag_<n>`).
    fn st0601_key(item: &LocalSetItem) -> String {
        format!("tag_{}", item.tag)
    }

    /// Map one typed ST 0601 item value onto an IR scalar.
    fn st0601_scalar(item: &LocalSetItem) -> PayloadScalar {
        use motion_imagery_metadata::St0601Value;
        let raw = || PayloadScalar::Bytes(Bytes::copy_from_slice(&item.value));
        match item.value_typed() {
            St0601Value::Number(value) => PayloadScalar::Float(value),
            St0601Value::Unsigned(value) => PayloadScalar::Unsigned(value),
            St0601Value::Signed(value) => PayloadScalar::Integer(value),
            St0601Value::Text(text) => PayloadScalar::Text(text.to_string()),
            St0601Value::Time(instant) => match u64::try_from(instant.as_microsecond()) {
                Ok(micros) => PayloadScalar::Timestamp(micros),
                Err(_) => PayloadScalar::Integer(instant.as_microsecond()),
            },
            St0601Value::Nested(set) => PayloadScalar::Bytes(Bytes::copy_from_slice(set.raw())),
            St0601Value::Pack(pack) => PayloadScalar::Bytes(Bytes::copy_from_slice(pack.raw())),
            St0601Value::Bytes(bytes) => PayloadScalar::Bytes(Bytes::copy_from_slice(bytes)),
            St0601Value::OutOfRange => PayloadScalar::Text("out_of_range".into()),
            St0601Value::Invalid => PayloadScalar::Text("invalid".into()),
            _ => raw(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CodecConfig, DataIdentity};
    #[cfg(feature = "motion-imagery-metadata")]
    use crate::payload::st0601::St0601Decoder;
    use bytes::Bytes;

    struct NeverClaims;

    impl PayloadDecoder for NeverClaims {
        fn name(&self) -> &'static str {
            "never"
        }

        fn claims(&self, _spec: &TrackSpec, _sample: &Sample) -> bool {
            false
        }

        fn decode(&self, _sample: &Sample) -> Result<Vec<PayloadValue>> {
            Ok(Vec::new())
        }
    }

    struct AlwaysClaims;

    impl PayloadDecoder for AlwaysClaims {
        fn name(&self) -> &'static str {
            "always"
        }

        fn claims(&self, _spec: &TrackSpec, _sample: &Sample) -> bool {
            true
        }

        fn decode(&self, _sample: &Sample) -> Result<Vec<PayloadValue>> {
            Ok(vec![PayloadValue::new("k", PayloadScalar::Unsigned(1))])
        }
    }

    fn timescale() -> broadcast_core::timescale::Timescale {
        broadcast_core::timescale::Timescale::try_new(90_000).expect("90 kHz")
    }

    fn data_track() -> TrackSpec {
        TrackSpec::new(
            crate::TrackId::new(1),
            timescale(),
            CodecConfig::Data {
                identity: DataIdentity::Mp4 {
                    handler: *b"meta",
                    sample_entry: Bytes::new(),
                },
            },
        )
    }

    /// A registry with no claiming decoder returns `None`, and registration
    /// order is claim precedence.
    #[test]
    fn no_claiming_decoder_is_none() {
        let sample = Sample::timed(vec![0u8], 0, 0, None);
        let spec = data_track();
        let mut registry = PayloadRegistry::new();
        assert!(registry.is_empty());
        registry.register(Box::new(NeverClaims));
        assert_eq!(registry.decode(&spec, &sample), None);
        registry.register(Box::new(AlwaysClaims));
        let (name, values) = registry
            .decode(&spec, &sample)
            .expect("the second decoder claims")
            .expect("decode succeeds");
        assert_eq!(name, "always");
        assert_eq!(values.len(), 1);
        assert!(!registry.is_empty());
    }

    /// The registry stays `Send + Sync`, so a caller can hold it in a worker
    /// thread or a shared service.
    #[test]
    fn registry_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PayloadRegistry>();
    }

    /// A claiming decoder returns the ST 0601 keys; a non-data track, or a
    /// data track whose sample is not a ST 0601 packet, is not claimed.
    #[cfg(feature = "motion-imagery-metadata")]
    #[test]
    fn st0601_claims_only_data_track_packets() {
        use broadcast_core::Parse;

        use crate::klv::{LocalSetItem, TAG_PRECISION_TIMESTAMP, UasLocalSet};

        let packet = UasLocalSet::from_items(vec![LocalSetItem::new(
            TAG_PRECISION_TIMESTAMP,
            0u64.to_be_bytes().to_vec(),
        )])
        .serialize_with_checksum()
        .expect("serialize a UAS Local Set");
        let sample = Sample::timed(packet.clone(), 0, 0, None);
        let decoder = St0601Decoder;

        let data = TrackSpec::new(
            crate::TrackId::new(1),
            timescale(),
            CodecConfig::Data {
                identity: DataIdentity::Mp4 {
                    handler: *b"meta",
                    sample_entry: Bytes::new(),
                },
            },
        );
        assert!(decoder.claims(&data, &sample));
        assert!(!decoder.claims(&data, &Sample::timed(vec![0u8; 8], 0, 0, None)));
        let video = TrackSpec::new(
            crate::TrackId::new(2),
            timescale(),
            CodecConfig::Vp8 {
                width: 16,
                height: 16,
            },
        );
        assert!(!decoder.claims(&video, &sample));

        let values = decoder
            .decode(&sample)
            .expect("decode a well-formed packet");
        assert!(values.iter().any(|v| v.key == "checksum_valid"));
        assert!(values.iter().any(|v| v.key == "tag_2"));
        let reparsed = UasLocalSet::parse(&packet).expect("parse");
        assert_eq!(reparsed.precision_timestamp(), Some(0));
    }
}
