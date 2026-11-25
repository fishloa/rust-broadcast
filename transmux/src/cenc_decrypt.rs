//! CENC decrypt — [`CencDecryptStage`], one sample at a time (ISO/IEC 23001-7).
//!
//! The demuxer already surfaces each protected track's `tenc` defaults
//! ([`TrackSpec::protection`](crate::ir::TrackSpec::protection)) and each
//! sample's IV and subsample map
//! ([`Sample::protection`](crate::ir::Sample::protection)), so decryption needs
//! no second file walk: a `Track` announcement passes through with its
//! protection cleared, and every `Sample` is deciphered in place.
//!
//! # Scheme support
//!
//! | Scheme | Cipher      | Status                                              |
//! |--------|-------------|------------------------------------------------------|
//! | `cenc` | AES-128-CTR | Supported — subsample + full-sample encryption.     |
//! | `cbcs` | AES-128-CBC | Supported — pattern cipher (`crypt`:`skip` blocks).  |
//!
//! # Spec citations
//!
//! - **Sample encryption / subsamples**: ISO/IEC 23001-7 §9.
//! - **AES-CTR (`cenc`) mode**: ISO/IEC 23001-7 §10.1 — the per-sample IV is
//!   the counter block (8- or 16-byte, left-justified and zero-padded to 16)
//!   with the low 64 bits acting as the AES block counter.
//! - **AES-CBC pattern (`cbcs`) mode**: ISO/IEC 23001-7 §10.2 — each protected
//!   range CBC-decrypts `default_crypt_byte_block` blocks then skips
//!   `default_skip_byte_block`, with the chain reset at each subsample.
//! - **`sinf`/`frma` unwrap**: ISO/IEC 14496-12:2026 §13.4.
//!
//! No AES is rolled by hand: `cenc_crypto` wraps the RustCrypto [`aes`],
//! [`ctr`], and [`cbc`] crates. This module is gated on the `cenc` feature.
//!
//! # Example
//!
//! ```rust
//! use transmux::{CencDecryptStage, KeyMap};
//!
//! let mut keys = KeyMap::new();
//! // One 16-byte KID -> 16-byte content key entry per protected track.
//! keys.insert([0x11; 16], [0x22; 16]);
//! let mut stage = CencDecryptStage::new(keys);
//! // Feed MuxInput::Track / MuxInput::Sample: a protected track is announced
//! // with its protection cleared and every sample comes out cleartext.
//! let _ = &mut stage;
//! ```

use core::fmt;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

use broadcast_common::stage::{Stage, Timestamp};
use isobmff::cenc::{KEY_LEN, TrackEncryptionBox};

use crate::cenc_crypto::{self, CbcsOp};
use crate::error::{Error, Result};
use crate::ir::{CodecConfig, DataIdentity, TrackId, TrackProtection, TrackSpec};
use crate::mux_stage::MuxInput;
pub use broadcast_common::cenc::CencScheme;

/// A map of content keys, keyed by 16-byte Key ID (KID).
///
/// Each protected sample's KID (from `tenc.default_kid`) selects a 16-byte
/// AES-128 content key.
#[derive(Clone, Default)]
pub struct KeyMap {
    keys: BTreeMap<[u8; KEY_LEN], [u8; KEY_LEN]>,
}

/// Manual `Debug`: lists the (non-secret) KIDs this map holds, never the
/// content key bytes paired with them.
impl fmt::Debug for KeyMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyMap")
            .field("kids", &self.keys.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Constant-time in the key bytes (`subtle`).
impl PartialEq for KeyMap {
    fn eq(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq;
        if self.keys.len() != other.keys.len() {
            return false;
        }
        let mut all = subtle::Choice::from(1u8);
        for ((kid_a, key_a), (kid_b, key_b)) in self.keys.iter().zip(other.keys.iter()) {
            all &= kid_a.ct_eq(kid_b) & key_a.ct_eq(key_b);
        }
        bool::from(all)
    }
}

impl Eq for KeyMap {}

/// Wipe the content keys on drop (issue #1142).
impl Drop for KeyMap {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for key in self.keys.values_mut() {
            key.zeroize();
        }
    }
}

impl KeyMap {
    /// Create an empty key map.
    pub fn new() -> Self {
        Self {
            keys: BTreeMap::new(),
        }
    }

    /// Insert a `kid -> key` mapping, returning `self` for chaining.
    pub fn with_key(mut self, kid: [u8; KEY_LEN], key: [u8; KEY_LEN]) -> Self {
        self.keys.insert(kid, key);
        self
    }

    /// Insert a `kid -> key` mapping in place.
    pub fn insert(&mut self, kid: [u8; KEY_LEN], key: [u8; KEY_LEN]) {
        self.keys.insert(kid, key);
    }

    /// Look up the content key for a KID.
    pub fn get(&self, kid: &[u8; KEY_LEN]) -> Option<&[u8; KEY_LEN]> {
        self.keys.get(kid)
    }
}

/// One announced track's cipher state.
enum TrackCrypto {
    /// Announced without `TrackSpec::protection`: samples are cleartext unless
    /// they carry their own auxiliary information, which is held until a
    /// `Track` update announces the `tenc`.
    Cleartext,
    /// A protected track's scheme, `tenc` defaults, and content key.
    Protected {
        scheme: CencScheme,
        tenc: TrackEncryptionBox,
        key: [u8; KEY_LEN],
    },
}

/// Samples held until their track's `tenc` arrives.
#[derive(Default)]
struct Pending {
    samples: Vec<crate::ir::Sample>,
    bytes: u64,
}

/// Decrypts CENC-protected samples one at a time.
///
/// A sample whose IV size disagrees with its track's `tenc` is a typed error
/// naming the track, never decrypted with a guessed IV. Samples that arrive
/// before a `Track` update names the `tenc` are held (up to the hostile-input
/// cap [`isobmff::Limits::default`] sets) and decrypted only once the real
/// `tenc` is known.
pub struct CencDecryptStage {
    keys: KeyMap,
    tracks: BTreeMap<TrackId, TrackCrypto>,
    pending: BTreeMap<TrackId, Pending>,
    pending_bytes: u64,
    max_pending_bytes: u64,
    ready: VecDeque<MuxInput>,
}

impl fmt::Debug for CencDecryptStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CencDecryptStage")
            .field("kids", &self.keys)
            .field("tracks", &self.tracks.len())
            .field("pending_bytes", &self.pending_bytes)
            .finish()
    }
}

impl CencDecryptStage {
    /// A decrypt stage over `keys`.
    #[must_use]
    pub fn new(keys: KeyMap) -> Self {
        Self {
            keys,
            tracks: BTreeMap::new(),
            pending: BTreeMap::new(),
            pending_bytes: 0,
            max_pending_bytes: isobmff::Limits::default().max_need_bytes(),
            ready: VecDeque::new(),
        }
    }

    /// Set the byte cap for samples held while waiting for a late `tenc`.
    ///
    /// The default is the hostile-input cap [`isobmff::Limits::default`]
    /// states; a caller may set its own policy.
    #[must_use]
    pub fn with_max_pending_bytes(mut self, max_pending_bytes: u64) -> Self {
        self.max_pending_bytes = max_pending_bytes;
        self
    }

    fn on_track(&mut self, mut spec: TrackSpec) -> Result<()> {
        let id = spec.track_id;
        let protection = spec.protection.take();
        match protection {
            Some(TrackProtection {
                scheme: _,
                ref tenc,
                ..
            }) if tenc.default_is_protected == 0 => {
                self.tracks.insert(id, TrackCrypto::Cleartext);
            }
            Some(protection) => {
                let scheme = protection.scheme;
                if !matches!(scheme, CencScheme::Cenc | CencScheme::Cbcs) {
                    return Err(Error::UnsupportedCencScheme { scheme });
                }
                let key =
                    *self
                        .keys
                        .get(&protection.tenc.default_kid)
                        .ok_or(Error::InvalidInput(
                            "no content key for the track's default_KID",
                        ))?;
                restore_sample_entry(&mut spec.config);
                self.tracks.insert(
                    id,
                    TrackCrypto::Protected {
                        scheme,
                        tenc: protection.tenc,
                        key,
                    },
                );
            }
            None => {
                self.tracks.insert(id, TrackCrypto::Cleartext);
            }
        }
        self.ready.push_back(MuxInput::Track(Box::new(spec)));
        self.drain_pending(id)
    }

    fn on_sample(&mut self, track: TrackId, sample: crate::ir::Sample) -> Result<()> {
        match self.tracks.get(&track) {
            Some(TrackCrypto::Protected { .. }) => self.decrypt_and_emit(track, sample),
            Some(TrackCrypto::Cleartext) if sample.protection.is_some() => {
                let bytes = u64::try_from(sample.data.len()).unwrap_or(u64::MAX);
                if self.pending_bytes.saturating_add(bytes) > self.max_pending_bytes {
                    return Err(Error::LimitExceeded {
                        what: "cenc samples held for a late tenc",
                        limit: self.max_pending_bytes,
                    });
                }
                self.pending_bytes = self.pending_bytes.saturating_add(bytes);
                let pending = self.pending.entry(track).or_default();
                pending.bytes = pending.bytes.saturating_add(bytes);
                pending.samples.push(sample);
                Ok(())
            }
            Some(TrackCrypto::Cleartext) => {
                self.ready.push_back(MuxInput::Sample { track, sample });
                Ok(())
            }
            None => Err(Error::CencUnannouncedTrack { track }),
        }
    }

    fn drain_pending(&mut self, track: TrackId) -> Result<()> {
        let Some(pending) = self.pending.remove(&track) else {
            return Ok(());
        };
        self.pending_bytes = self.pending_bytes.saturating_sub(pending.bytes);
        for sample in pending.samples {
            match self.tracks.get(&track) {
                Some(TrackCrypto::Protected { .. }) => self.decrypt_and_emit(track, sample)?,
                Some(TrackCrypto::Cleartext) => {
                    if sample.protection.is_some() {
                        return Err(Error::CencMissingSampleProtection { track });
                    }
                    self.ready.push_back(MuxInput::Sample { track, sample });
                }
                None => return Err(Error::CencUnannouncedTrack { track }),
            }
        }
        Ok(())
    }

    fn decrypt_and_emit(&mut self, track: TrackId, mut sample: crate::ir::Sample) -> Result<()> {
        let Some(TrackCrypto::Protected { scheme, tenc, key }) = self.tracks.get(&track) else {
            return Err(Error::CencUnannouncedTrack { track });
        };
        let scheme = *scheme;
        let (iv, subsamples) = if tenc.default_per_sample_iv_size == 0 {
            let iv = tenc
                .default_constant_iv
                .as_ref()
                .ok_or(Error::CencMissingSampleProtection { track })?
                .clone();
            let subsamples = sample
                .protection
                .as_ref()
                .map(|p| p.subsamples.clone())
                .unwrap_or_default();
            (iv, subsamples)
        } else {
            let protection = sample
                .protection
                .as_ref()
                .ok_or(Error::CencMissingSampleProtection { track })?;
            if protection.iv.len() != usize::from(tenc.default_per_sample_iv_size) {
                return Err(Error::CencIvSizeMismatch {
                    track,
                    tenc: tenc.default_per_sample_iv_size,
                    sample: protection.iv.len(),
                });
            }
            (protection.iv.to_vec(), protection.subsamples.clone())
        };
        cenc_crypto::validate_sample_decrypt(scheme, tenc, &iv, &subsamples, sample.data.len())?;
        cenc_crypto::rewrite_in_place(&mut sample.data, |buf| match scheme {
            CencScheme::Cenc => cenc_crypto::apply_ctr(&iv, key, &subsamples, buf),
            CencScheme::Cbcs => {
                cenc_crypto::cbcs_sample(tenc, &iv, &subsamples, key, buf, CbcsOp::Decrypt)
            }
            other => Err(Error::UnsupportedCencScheme { scheme: other }),
        })?;
        sample.protection = None;
        self.ready.push_back(MuxInput::Sample { track, sample });
        Ok(())
    }
}

/// Restore a protected `encv`/`enca` data identity to its cleartext sample
/// entry (ISO/IEC 14496-12:2026 §13.4.1).
fn restore_sample_entry(config: &mut CodecConfig) {
    if let CodecConfig::Data {
        identity: DataIdentity::Mp4 { sample_entry, .. },
    } = config
        && let Some(clear) = isobmff::cenc::clear_sample_entry(sample_entry)
    {
        *sample_entry = clear.into();
    }
}

impl Stage for CencDecryptStage {
    type In<'a> = MuxInput;
    type Out = MuxInput;
    type Error = Error;

    fn feed(&mut self, input: Self::In<'_>, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => self.on_track(*spec),
            MuxInput::Sample { track, sample } => self.on_sample(track, sample),
            other => {
                self.ready.push_back(other);
                Ok(())
            }
        }
    }

    fn poll(&mut self) -> Option<MuxInput> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if let Some((track, _)) = self.pending.iter().next() {
            return Err(Error::CencMissingSampleProtection { track: *track });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Unpackage;
    use bytes::Bytes;

    use crate::ir::{Track, TrackSpec};
    use crate::ts_demux::TsDemux;

    const KID: [u8; 16] = [
        0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b, 0xf3,
        0xb8,
    ];
    const KEY: [u8; 16] = [
        0x76, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc,
        0xbb,
    ];

    fn keys() -> KeyMap {
        KeyMap::new().with_key(KID, KEY)
    }

    fn tenc(iv_size: u8) -> TrackEncryptionBox {
        TrackEncryptionBox {
            version: 0,
            default_crypt_byte_block: 0,
            default_skip_byte_block: 0,
            default_is_protected: 1,
            default_per_sample_iv_size: iv_size,
            default_kid: KID,
            default_constant_iv: None,
        }
    }

    fn protected_track(id: u32, tenc: TrackEncryptionBox) -> Track {
        let mut spec = TrackSpec::new(
            TrackId::new(id),
            broadcast_common::timescale::Timescale::try_new(90_000).expect("timescale"),
            CodecConfig::Data {
                identity: DataIdentity::Mp4 {
                    handler: *b"vide",
                    sample_entry: Bytes::new(),
                },
            },
        );
        spec.protection = Some(TrackProtection::new(
            CencScheme::Cenc,
            tenc,
            crate::ir::ConstantIvSenc::default(),
        ));
        Track::new(spec, Vec::new())
    }

    fn encrypted_cenc_sample() -> crate::ir::Sample {
        let bytes = test_fixtures::read("ts/h264/main.ts");
        let mut demux = TsDemux::new();
        let media = demux.unpackage(bytes.as_slice()).expect("demux");
        let clear = crate::repackage::select_tracks_by(&media, |t| {
            matches!(t.spec.config, CodecConfig::Avc { .. })
        })
        .expect("AVC");
        let mut sample = clear.tracks[0].samples[0].clone();
        let iv = Bytes::from(alloc::vec![0u8; 8]);
        let subs = crate::cenc_encrypt::nal_subsamples(crate::nal::NalCodec::Avc, &sample.data)
            .expect("subsamples");
        cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
            cenc_crypto::apply_ctr(&iv, &KEY, &subs, buf)
        })
        .expect("cipher");
        sample.protection = Some(alloc::sync::Arc::new(crate::ir::SampleProtection {
            iv,
            subsamples: subs,
        }));
        sample
    }

    fn sample_of(track: TrackId, sample: crate::ir::Sample) -> MuxInput {
        MuxInput::Sample { track, sample }
    }

    fn poll_all(stage: &mut CencDecryptStage) -> Vec<MuxInput> {
        let mut out = Vec::new();
        while let Some(item) = stage.poll() {
            out.push(item);
        }
        out
    }

    #[test]
    fn a_clear_sample_passes_through_unchanged() {
        let mut stage = CencDecryptStage::new(keys());
        stage
            .feed(
                MuxInput::Track(Box::new(protected_track(1, tenc(8)).spec)),
                Timestamp::ZERO,
            )
            .expect("track");
        let sample = encrypted_cenc_sample();
        let data = sample.data.clone();
        stage
            .feed(sample_of(TrackId::new(1), sample), Timestamp::ZERO)
            .expect("sample");
        let _ = data;
        let out = poll_all(&mut stage);
        assert!(matches!(out[0], MuxInput::Track(_)));
        assert!(matches!(out[1], MuxInput::Sample { .. }));
    }

    /// Review Focus: a `Track` update after the first samples announces the
    /// `tenc`; the held samples must be decrypted with it, never guessed.
    #[test]
    fn samples_before_a_late_tenc_are_held_then_decrypted() {
        let mut stage = CencDecryptStage::new(keys());
        let mut spec = protected_track(1, tenc(8)).spec;
        spec.protection = None;
        stage
            .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
            .expect("initial track");
        let sample = encrypted_cenc_sample();
        let want = sample.data.clone();
        stage
            .feed(sample_of(TrackId::new(1), sample), Timestamp::ZERO)
            .expect("held sample");
        let early = poll_all(&mut stage);
        assert!(
            early
                .iter()
                .all(|item| !matches!(item, MuxInput::Sample { .. })),
            "nothing is emitted before the tenc is known"
        );

        spec.protection = Some(TrackProtection::new(
            CencScheme::Cenc,
            tenc(8),
            crate::ir::ConstantIvSenc::default(),
        ));
        stage
            .feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)
            .expect("late track update");
        let out = poll_all(&mut stage);
        let decrypted = out.iter().find_map(|item| match item {
            MuxInput::Sample { sample, .. } => Some(sample),
            _ => None,
        });
        let decrypted = decrypted.expect("held sample emitted after the update");
        assert_ne!(decrypted.data, want);
        assert!(decrypted.protection.is_none());
    }

    #[test]
    fn an_iv_size_disagreement_is_a_typed_error_naming_the_track() {
        let mut stage = CencDecryptStage::new(keys());
        stage
            .feed(
                MuxInput::Track(Box::new(protected_track(7, tenc(8)).spec)),
                Timestamp::ZERO,
            )
            .expect("track");
        let mut sample = encrypted_cenc_sample();
        sample.protection = Some(alloc::sync::Arc::new(crate::ir::SampleProtection {
            iv: Bytes::from(alloc::vec![0u8; 16]),
            subsamples: Vec::new(),
        }));
        let err = stage
            .feed(sample_of(TrackId::new(7), sample), Timestamp::ZERO)
            .expect_err("16-byte IV against an 8-byte tenc");
        assert!(
            matches!(
                err,
                Error::CencIvSizeMismatch {
                    track,
                    tenc: 8,
                    sample: 16
                } if track == TrackId::new(7)
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn a_missing_key_is_an_error_at_announcement() {
        let mut stage = CencDecryptStage::new(KeyMap::new());
        let err = stage
            .feed(
                MuxInput::Track(Box::new(protected_track(1, tenc(8)).spec)),
                Timestamp::ZERO,
            )
            .expect_err("no key");
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn unfinished_pending_samples_are_an_error() {
        let mut stage = CencDecryptStage::new(keys());
        let mut spec = protected_track(1, tenc(8)).spec;
        spec.protection = None;
        stage
            .feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)
            .expect("track");
        stage
            .feed(
                sample_of(TrackId::new(1), encrypted_cenc_sample()),
                Timestamp::ZERO,
            )
            .expect("held");
        let err = stage.finish().expect_err("still pending");
        assert!(matches!(err, Error::CencMissingSampleProtection { .. }));
    }

    #[test]
    fn pending_cap_is_enforced() {
        let mut stage = CencDecryptStage::new(keys()).with_max_pending_bytes(1);
        let mut spec = protected_track(1, tenc(8)).spec;
        spec.protection = None;
        stage
            .feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)
            .expect("track");
        let err = stage
            .feed(
                sample_of(TrackId::new(1), encrypted_cenc_sample()),
                Timestamp::ZERO,
            )
            .expect_err("cap");
        assert!(matches!(err, Error::LimitExceeded { .. }));
    }

    #[test]
    fn a_cleartext_track_with_protected_samples_is_an_error_at_finish() {
        let mut stage = CencDecryptStage::new(keys());
        let spec = TrackSpec::new(
            TrackId::new(1),
            broadcast_common::timescale::Timescale::try_new(90_000).expect("timescale"),
            CodecConfig::Data {
                identity: DataIdentity::Mp4 {
                    handler: *b"vide",
                    sample_entry: Bytes::new(),
                },
            },
        );
        stage
            .feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)
            .expect("track");
        let sample = encrypted_cenc_sample();
        stage
            .feed(sample_of(TrackId::new(1), sample), Timestamp::ZERO)
            .expect("held");
        assert!(stage.finish().is_err());
    }
}
