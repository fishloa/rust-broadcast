//! CENC/CBCS sample encryption — `CencEncryptor` (issue #564).
//!
//! Applies AES-128 sample protection (`cenc` CTR / `cbcs` CBC-pattern,
//! ISO/IEC 23001-7 §10) to a cleartext [`Media`]'s samples in place,
//! implementing the hub [`broadcast_common::Encrypt`] trait — the inverse of
//! [`crate::cenc_decrypt::CencDecryptor`]'s [`broadcast_common::Decrypt`].
//! Dispatches the actual cipher work to the shared, crate-internal cipher core
//! (the same module the decrypt path uses) and records
//! the resulting per-track/per-sample crypto metadata onto
//! [`crate::media::Track::encryption`] — exactly the shape
//! [`crate::cenc_decrypt::CencDecryptor::from_fmp4`] recovers from an
//! already-protected file (the two are duals).
//!
//! # Subsample map
//!
//! For an AVC/HEVC/VVC (NAL-carried) track under [`SubsamplePolicy::Video`],
//! each length-prefixed NAL unit in a sample ([`crate::annexb::iter_length_prefixed_nals`])
//! contributes one [`crate::cenc::SubSampleEntry`]: the 4-byte length prefix
//! plus the codec's NAL header (1 byte AVC, 2 bytes HEVC/VVC — ITU-T H.264
//! §7.3.1 / H.265 §7.3.1.2 / H.266 §7.3.1.2) is left clear, and the rest of
//! the NAL (its payload) is protected. Any other track, or
//! [`SubsamplePolicy::WholeSample`], protects the whole sample in one range
//! (an empty subsample map — ISO/IEC 23001-7 §9.3, "no subsample structure").
//!
//! # Spec citations
//!
//! - **Sample encryption / subsamples**: ISO/IEC 23001-7 §9.
//! - **AES-CTR (`cenc`) / AES-CBC pattern (`cbcs`)**: ISO/IEC 23001-7 §10 —
//!   see the crate-internal `cenc_crypto` module for the cipher-core citations.
//! - **`tenc`**: ISO/IEC 23001-7 §12.2.
//!
//! This module is gated on the `cenc` feature.

use alloc::vec::Vec;

use broadcast_common::Encrypt;

use crate::annexb::{NAL_LENGTH_SIZE, iter_length_prefixed_nals};
use crate::cenc::{CencScheme, SampleEncryptionEntry, SubSampleEntry, TrackEncryptionBox};
use crate::cenc_crypto::{self, CbcsOp};
use crate::error::{Error, Result};
use crate::media::{Media, TrackEncryption};
use crate::nal::NalCodec;
use crate::pipeline::CodecConfig;

/// Size of a KID / content key / AES-128 key **or block**, in bytes (AES-128's
/// key length and block length coincide).
const KEY_LEN: usize = 16;

/// Per-sample IV size this encryptor always emits (8-byte per-sample IV,
/// zero-padded to 16 bytes by the cipher core — the common CMAF convention;
/// ISO/IEC 23001-7 §12.2 permits 8 or 16). A fixed choice keeps
/// [`EncryptConfig`] simple; `tenc.default_constant_IV` is therefore never
/// needed here (only relevant when `default_Per_Sample_IV_Size == 0`).
const PER_SAMPLE_IV_SIZE: u8 = 8;

/// Default `cbcs` pattern (`crypt_byte_block`:`skip_byte_block`) — 1 crypt
/// block then 9 skip blocks, the common CMAF/DASH-IF `cbcs` convention
/// (ISO/IEC 23001-7 §10.2).
const DEFAULT_CBCS_PATTERN: (u8, u8) = (1, 9);

/// How to derive each sample's initialization vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IvGen {
    /// Per-sample 8-byte IV = big-endian `base + sample_index` (the cipher
    /// core zero-pads it to 16 bytes). The default.
    Counter {
        /// The first sample's IV value; each following sample increments by 1.
        base: u64,
    },
    /// Caller-supplied per-sample IVs, one per sample, in decode order. Each
    /// IV must be at most 16 bytes.
    Explicit(Vec<Vec<u8>>),
}

impl Default for IvGen {
    fn default() -> Self {
        IvGen::Counter { base: 0 }
    }
}

/// How the protected byte ranges (subsample map) of each sample are chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubsamplePolicy {
    /// NAL-aware: for AVC/HEVC/VVC tracks, clear the length-prefix + NAL
    /// header of every NAL unit and protect the remainder (see the module
    /// docs); any other track falls back to whole-sample protection.
    Video,
    /// Protect every sample in full (no subsample structure).
    WholeSample,
}

/// Configuration for [`CencEncryptor::encrypt`].
#[derive(Debug, Clone)]
pub struct EncryptConfig {
    /// The protection scheme to apply (`cenc` AES-CTR or `cbcs` AES-CBC
    /// pattern).
    pub scheme: CencScheme,
    /// The 16-byte Key ID recorded in `tenc.default_KID`.
    pub kid: [u8; KEY_LEN],
    /// The 16-byte AES-128 content key used to protect every sample.
    pub key: [u8; KEY_LEN],
    /// How each sample's IV is derived. Defaults to [`IvGen::Counter`] with
    /// `base: 0`.
    pub iv: IvGen,
    /// `cbcs` pattern (`crypt_byte_block`, `skip_byte_block`); defaults to
    /// `1:9` when `None`. Ignored for `cenc`.
    pub pattern: Option<(u8, u8)>,
    /// How the subsample map is chosen.
    pub subsample: SubsamplePolicy,
}

/// Applies CENC/CBCS sample protection to a [`Media`], implementing
/// [`Encrypt`] — the inverse of [`crate::cenc_decrypt::CencDecryptor`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CencEncryptor;

impl Encrypt for CencEncryptor {
    type Media = Media;
    type Config = EncryptConfig;
    type Error = Error;

    /// Encrypt every track's samples in `media` in place per `cfg`, recording
    /// the resulting crypto metadata onto each [`crate::media::Track::encryption`].
    ///
    /// A single [`EncryptConfig`] (scheme/KID/key) is applied uniformly to
    /// every track in `media`.
    fn encrypt(&self, media: &mut Media, cfg: &EncryptConfig) -> Result<()> {
        let pattern = match cfg.scheme {
            CencScheme::Cbcs => cfg.pattern.unwrap_or(DEFAULT_CBCS_PATTERN),
            CencScheme::Cenc => (0, 0),
        };
        let tenc = TrackEncryptionBox {
            // `cbcs` pattern fields only carry meaning under version 1
            // (ISO/IEC 23001-7 §12.2); `cenc` has no pattern, so version 0.
            version: if cfg.scheme == CencScheme::Cbcs { 1 } else { 0 },
            default_crypt_byte_block: pattern.0,
            default_skip_byte_block: pattern.1,
            default_is_protected: 1,
            default_per_sample_iv_size: PER_SAMPLE_IV_SIZE,
            default_kid: cfg.kid,
            // Only meaningful when default_per_sample_iv_size == 0, which this
            // encryptor never emits (see PER_SAMPLE_IV_SIZE's docs).
            default_constant_iv: None,
        };

        for track in &mut media.tracks {
            let nal_codec = nal_codec_for(&track.spec.config);
            let sample_count = track.samples.len();
            let mut entries = Vec::with_capacity(sample_count);

            for (idx, sample) in track.samples.iter_mut().enumerate() {
                let iv = resolve_iv(&cfg.iv, idx, sample_count)?;
                let subsamples = match (cfg.subsample, nal_codec) {
                    (SubsamplePolicy::Video, Some(codec)) => nal_subsamples(codec, &sample.data)?,
                    _ => Vec::new(),
                };
                let entry = SampleEncryptionEntry {
                    initialization_vector: iv,
                    subsamples,
                };

                match cfg.scheme {
                    CencScheme::Cenc => cenc_crypto::apply_ctr(
                        &entry.initialization_vector,
                        &cfg.key,
                        &entry.subsamples,
                        &mut sample.data,
                    )?,
                    CencScheme::Cbcs => cenc_crypto::cbcs_sample(
                        &tenc,
                        &entry,
                        &cfg.key,
                        &mut sample.data,
                        CbcsOp::Encrypt,
                    )?,
                }

                entries.push(entry);
            }

            track.encryption = Some(TrackEncryption {
                scheme: cfg.scheme,
                tenc: tenc.clone(),
                samples: entries,
            });
        }
        Ok(())
    }
}

/// Map a track's codec config to the NAL-header layout used to build its
/// subsample map, or `None` for a track this encryptor cannot walk as NAL
/// units (audio, or any other non-NAL-carried codec) — such tracks always
/// fall back to whole-sample protection regardless of [`SubsamplePolicy`].
fn nal_codec_for(config: &CodecConfig) -> Option<NalCodec> {
    match config {
        CodecConfig::Avc { .. } => Some(NalCodec::Avc),
        CodecConfig::Hevc { .. } => Some(NalCodec::Hevc),
        CodecConfig::Vvc { .. } => Some(NalCodec::Vvc),
        _ => None,
    }
}

/// Build a NAL-aware subsample map for one sample's length-prefixed NAL data:
/// each NAL's 4-byte length prefix + `codec`'s NAL header is clear, and the
/// remainder of the NAL (its payload) is protected — one
/// [`SubSampleEntry`] per NAL unit (ISO/IEC 23001-7 §9.3).
fn nal_subsamples(codec: NalCodec, data: &[u8]) -> Result<Vec<SubSampleEntry>> {
    let header_len: usize = match codec {
        NalCodec::Avc => 1,
        NalCodec::Hevc | NalCodec::Vvc => 2,
    };
    let nals = iter_length_prefixed_nals(data)?;
    let mut out = Vec::with_capacity(nals.len());
    for nal in nals {
        // A NAL too short to carry its own header (should not occur in a
        // well-formed stream) is left entirely clear rather than under- or
        // over-running the header boundary.
        let clear_header = header_len.min(nal.len());
        out.push(SubSampleEntry {
            bytes_of_clear_data: (NAL_LENGTH_SIZE + clear_header) as u16,
            bytes_of_protected_data: (nal.len() - clear_header) as u32,
        });
    }

    let total: usize = out
        .iter()
        .map(|s| s.bytes_of_clear_data as usize + s.bytes_of_protected_data as usize)
        .sum();
    if total != data.len() {
        return Err(Error::InvalidInput(
            "NAL subsample map does not cover the whole sample",
        ));
    }
    Ok(out)
}

/// Resolve sample `idx`'s IV from the configured [`IvGen`].
fn resolve_iv(iv_gen: &IvGen, idx: usize, sample_count: usize) -> Result<Vec<u8>> {
    match iv_gen {
        IvGen::Counter { base } => {
            let v = base.checked_add(idx as u64).ok_or(Error::InvalidInput(
                "CENC IV counter overflow (base + sample_index)",
            ))?;
            Ok(v.to_be_bytes().to_vec())
        }
        IvGen::Explicit(ivs) => {
            if ivs.len() != sample_count {
                return Err(Error::InvalidInput(
                    "IvGen::Explicit IV count does not match the track's sample count",
                ));
            }
            let iv = &ivs[idx];
            if iv.len() > KEY_LEN {
                return Err(Error::InvalidInput(
                    "IvGen::Explicit IV longer than 16 bytes",
                ));
            }
            Ok(iv.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    //! Byte-exact IR-level round-trip tests: encrypt with [`CencEncryptor`]
    //! (the public surface), then reverse with the shared cipher core
    //! ([`cenc_crypto::apply_ctr`] / [`cenc_crypto::cbcs_sample`] +
    //! [`CbcsOp::Decrypt`]) directly — the same functions
    //! [`crate::cenc_decrypt::CencDecryptor`] calls — using each recorded
    //! [`crate::cenc::SampleEncryptionEntry`]'s IV/subsample map. Only
    //! reachable from an in-crate unit test (`cenc_crypto` is `pub(crate)`);
    //! `tests/cenc_encrypt.rs` covers the equivalent public-API-only surface
    //! (see that file's docs for why it does not repeat this exact reversal).

    use super::*;
    use broadcast_common::Unpackage;

    use crate::ts_demux::TsDemux;

    const KID: [u8; 16] = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x00,
    ];
    const KEY: [u8; 16] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10,
    ];

    /// The real cleartext H.264 capture used by `cenc_decrypt`'s tests too,
    /// narrowed to its single AVC video track so every test has a
    /// deterministic, single-track `Media` (avoids the multi-track ambiguity
    /// an `IvGen::Explicit` list — one list shared by every track — would hit
    /// if the fixture also carried an audio track with a different sample
    /// count).
    fn clear_media() -> Media {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264");
        path.push("main.ts");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let mut demux = TsDemux::new();
        let media = demux
            .unpackage(bytes.as_slice())
            .expect("demux fixtures/ts/h264/main.ts");
        media
            .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .expect("AVC video track present")
    }

    fn snapshot(media: &Media) -> Vec<Vec<u8>> {
        media.tracks[0]
            .samples
            .iter()
            .map(|s| s.data.clone())
            .collect()
    }

    #[test]
    fn cenc_round_trip_reverses_byte_identical() {
        let mut media = clear_media();
        let original = snapshot(&media);

        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            key: KEY,
            iv: IvGen::Counter { base: 7 },
            pattern: None,
            subsample: SubsamplePolicy::Video,
        };
        CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");

        let track = &mut media.tracks[0];
        let enc = track.encryption.clone().expect("track.encryption Some");
        assert_eq!(enc.scheme, CencScheme::Cenc);
        assert_eq!(enc.tenc.default_kid, KID);
        assert_eq!(enc.samples.len(), track.samples.len());

        // Encryption must have actually changed at least one sample's bytes
        // (real cipher, not a passthrough).
        assert!(
            track
                .samples
                .iter()
                .zip(original.iter())
                .any(|(s, o)| &s.data != o),
            "encrypt must change protected bytes"
        );

        for (sample, entry) in track.samples.iter_mut().zip(enc.samples.iter()) {
            cenc_crypto::apply_ctr(
                &entry.initialization_vector,
                &KEY,
                &entry.subsamples,
                &mut sample.data,
            )
            .expect("reverse apply_ctr");
        }
        let reversed: Vec<Vec<u8>> = track.samples.iter().map(|s| s.data.clone()).collect();
        assert_eq!(reversed, original, "cenc round trip must be byte-identical");
    }

    #[test]
    fn cbcs_round_trip_reverses_byte_identical() {
        let mut media = clear_media();
        let original = snapshot(&media);

        let cfg = EncryptConfig {
            scheme: CencScheme::Cbcs,
            kid: KID,
            key: KEY,
            iv: IvGen::Counter { base: 0 },
            pattern: Some((1, 9)),
            subsample: SubsamplePolicy::Video,
        };
        CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");

        let track = &mut media.tracks[0];
        let enc = track.encryption.clone().expect("track.encryption Some");
        assert_eq!(enc.scheme, CencScheme::Cbcs);
        assert_eq!(enc.tenc.default_crypt_byte_block, 1);
        assert_eq!(enc.tenc.default_skip_byte_block, 9);
        assert_eq!(enc.samples.len(), track.samples.len());

        assert!(
            track
                .samples
                .iter()
                .zip(original.iter())
                .any(|(s, o)| &s.data != o),
            "encrypt must change protected bytes"
        );

        for (sample, entry) in track.samples.iter_mut().zip(enc.samples.iter()) {
            cenc_crypto::cbcs_sample(&enc.tenc, entry, &KEY, &mut sample.data, CbcsOp::Decrypt)
                .expect("reverse cbcs_sample");
        }
        let reversed: Vec<Vec<u8>> = track.samples.iter().map(|s| s.data.clone()).collect();
        assert_eq!(reversed, original, "cbcs round trip must be byte-identical");
    }

    #[test]
    fn whole_sample_policy_yields_empty_subsample_map() {
        let mut media = clear_media();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            key: KEY,
            iv: IvGen::default(),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
        };
        CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");
        let enc = media.tracks[0].encryption.as_ref().expect("Some");
        assert!(
            enc.samples.iter().all(|e| e.subsamples.is_empty()),
            "WholeSample policy must record an empty subsample map"
        );
    }

    #[test]
    fn explicit_iv_count_mismatch_errors() {
        let mut media = clear_media();
        let n = media.tracks[0].samples.len();
        assert!(n > 1, "fixture must have more than one sample to bite");
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            key: KEY,
            iv: IvGen::Explicit(alloc::vec![alloc::vec![0u8; 8]; n - 1]),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
        };
        let err = CencEncryptor.encrypt(&mut media, &cfg).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn explicit_iv_too_long_errors() {
        let mut media = clear_media();
        let n = media.tracks[0].samples.len();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            key: KEY,
            iv: IvGen::Explicit(alloc::vec![alloc::vec![0u8; 17]; n]),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
        };
        let err = CencEncryptor.encrypt(&mut media, &cfg).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }
}
