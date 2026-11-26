//! CENC/CBCS sample encryption — [`CencEncryptStage`] (issue #564).
//!
//! Applies AES-128 sample protection (`cenc` CTR / `cbcs` CBC-pattern,
//! ISO/IEC 23001-7 §10) to a stream of [`MuxInput`] one sample at a time — the
//! inverse of [`crate::cenc_decrypt::CencDecryptStage`]. A `Track` announcement
//! passes through with [`crate::TrackProtection`] set; every `Sample` is
//! ciphered and carries its own [`crate::ir::SampleProtection`]. The cipher
//! core is shared with the decrypt stage.
//!
//! # IV uniqueness is per *key*, for all time
//!
//! AES-CTR (`cenc`) requires every sample ciphered under one content key to
//! use a distinct IV — not just within one [`CencEncryptStage::feed`] call, but
//! across *every* call ever made with that key (ISO/IEC 23001-7 §9.2). A
//! [`CencEncryptStage`] is therefore a **stateful** value bound to one content
//! key ([`CencEncryptStage::new`]): [`IvGen::Counter`]'s running index lives on
//! the encryptor instance, not in [`EncryptConfig`], so calling `encrypt`
//! twice on the *same* instance continues the counter instead of restarting
//! it — reuse *one* instance across every call sharing a key (video-only +
//! audio-only splits of one asset, successive live segments, …). See
//! [`CencEncryptStage`]'s own docs for the caller obligation this does and does
//! not enforce.
//!
//! # Subsample map
//!
//! For an AVC/HEVC/VVC (NAL-carried) track under [`SubsamplePolicy::Video`],
//! each length-prefixed NAL unit in a sample ([`broadcast_bitstream::annexb::iter_length_prefixed_nals`])
//! contributes one [`isobmff::cenc::SubSampleEntry`]: the 4-byte length prefix
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
//!
//! # Example
//!
//! ```rust
//! use transmux::{CencEncryptStage, CencScheme, EncryptConfig, SubsamplePolicy};
//!
//! let cfg = EncryptConfig::new(CencScheme::Cenc, [0x11; 16], SubsamplePolicy::Video);
//! // One stage per content key: its IV counter continues across every feed.
//! let stage = CencEncryptStage::new([0x22; 16], cfg)?;
//! # Ok::<(), transmux::Error>(())
//! ```

use core::fmt;

use alloc::collections::VecDeque;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec::Vec;

use broadcast_common::stage::{Stage, Timestamp};

use crate::cenc_crypto::{self, CbcsOp};
use crate::error::{Error, Result};
use crate::ir::{SampleProtection, TrackId, TrackProtection};
use crate::mux_stage::MuxInput;
use crate::pipeline::CodecConfig;
use broadcast_bitstream::annexb::{NAL_LENGTH_SIZE, iter_length_prefixed_nals};
use broadcast_bitstream::nal::NalCodec;
use bytes::Bytes;
use isobmff::cenc::{
    CencScheme, KEY_LEN, SampleEncryptionEntry, SubSampleEntry, TrackEncryptionBox,
};

/// Length of a constant `cbcs` IV: one AES block.
const CONSTANT_IV_LEN: usize = 16;

/// Per-sample IV size (bytes) for [`IvGen::Counter`] (every counter IV is an
/// 8-byte big-endian value) and the fallback for an empty [`IvGen::Explicit`]
/// list — the common CMAF `cenc` convention (ISO/IEC 23001-7 §12.2 permits 8
/// or 16). [`IvGen::Constant`] derives `16` per sample when
/// [`ConstantIvSenc::Emit`] (the default), or `0` when
/// [`ConstantIvSenc::Omit`] — see [`tenc_iv_fields`].
const PER_SAMPLE_IV_SIZE: u8 = 8;

/// Default `cbcs` pattern (`crypt_byte_block`:`skip_byte_block`) — 1 crypt
/// block then 9 skip blocks, the common CMAF/DASH-IF `cbcs` convention
/// (ISO/IEC 23001-7 §10.2).
const DEFAULT_CBCS_PATTERN: (u8, u8) = (1, 9);

/// Maximum value of a `cbcs` pattern component (`crypt_byte_block` /
/// `skip_byte_block`). `tenc` packs both into a single byte, one nibble each
/// (ISO/IEC 23001-7 §12.2: `(default_crypt_byte_block << 4) |
/// default_skip_byte_block`), so any component above 15 would silently
/// truncate to its low 4 bits on the wire rather than error.
const CBCS_PATTERN_MAX: u8 = 0x0F;

/// Valid per-sample IV lengths for a `senc` entry — ISO/IEC 23001-7 §9.2/§12.2
/// permit exactly 8 or 16 bytes; any other length (including empty) desyncs
/// the AES-CTR/CBC IV derivation from `tenc.default_per_sample_iv_size` and
/// `saiz`'s per-sample aux info size.
const VALID_EXPLICIT_IV_LENS: [usize; 2] = [8, 16];

/// AES block size (16 bytes) as a `u128`, for the AES-CTR counter-range
/// arithmetic ([`assert_ctr_ranges_disjoint`]).
const BLOCK_LEN_U128: u128 = 16;

/// How to derive each sample's initialization vector.
///
/// # IV uniqueness is per *key*, not per track — and not per call
///
/// ISO/IEC 23001-7 §9.2 requires each sample's IV to be unique for the
/// **content key** it is used with. One [`EncryptConfig`] carries one key and
/// is applied to *every* track of the the input stream, so "unique within this track"
/// is **not** sufficient: under `cenc` (AES-CTR) two samples sharing a key and
/// a counter block produce ciphertexts whose XOR is the XOR of their
/// plaintexts — a two-time pad that discloses both without the key. Every
/// variant below is therefore indexed/counted across the **whole input** of
/// one [`CencEncryptStage::feed`] call — and, for [`IvGen::Counter`], across every
/// call ever made on the same [`CencEncryptStage`] instance (see that type's
/// docs): uniqueness does not reset just because a second call started.
/// [`CencEncryptStage::feed`] additionally rejects any duplicate per-sample IV
/// it is about to use, *before* ciphering a single byte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum IvGen {
    /// Per-sample IV = big-endian `next_counter + sample_index`, where
    /// `sample_index` runs **continuously across every track** of the
    /// the input stream in (track, sample) order — it does *not* restart per track
    /// (see this enum's docs: that would reuse one AES-CTR keystream between,
    /// say, video sample *i* and audio sample *i*) — and `next_counter` is
    /// the encrypting [`CencEncryptStage`] instance's own running index, which it
    /// advances after every successful call rather than resetting to a
    /// caller-supplied `base` (see that type's docs for why the counter lives
    /// there and not here). The default.
    ///
    /// The IV width is **scheme-dependent** (audit r05-W4): 8 bytes under
    /// [`CencScheme::Cenc`], the common CMAF convention the cipher core
    /// zero-pads to a 16-byte counter block (ISO/IEC 23001-7 §12.2 permits 8
    /// or 16); 16 bytes under [`CencScheme::Cbcs`] — the 64-bit counter in the
    /// low half of a 16-byte big-endian block — because an 8-byte `cbcs`
    /// per-sample IV is the shape Bento4's `mp4decrypt` silently no-ops (see
    /// [`IvGen::Constant`]).
    #[default]
    Counter,
    /// Caller-supplied per-sample IVs, one per sample of the **whole
    /// input**: exactly one entry per fed sample, consumed in (track, sample) order (track 0's samples first,
    /// then track 1's, …). A list sized to a single track's sample count is
    /// rejected — see this enum's docs for why per-track IV lists are unsafe.
    ///
    /// Each IV must be exactly 8 or 16 bytes (ISO/IEC 23001-7 §9.2/§12.2 — no
    /// other length is valid on the wire, and an empty or otherwise-sized IV
    /// would desync the AES-CTR/CBC derivation), every IV in the list must
    /// have the same length (`tenc.default_per_sample_iv_size` is one value
    /// for the whole track), and no two entries may be equal.
    Explicit(Vec<Vec<u8>>),
    /// A single 16-byte IV shared by every sample of the track, recorded as
    /// `tenc.default_constant_IV` with `default_per_sample_iv_size == 0`
    /// (ISO/IEC 23001-7 §12.2) rather than a per-sample `senc` entry.
    ///
    /// **`cbcs`-only** — [`CencEncryptStage::feed`] rejects this variant under
    /// [`CencScheme::Cenc`]. A constant IV is fundamentally incompatible with
    /// AES-CTR: the counter block is derived from the IV alone, so every
    /// sample of the track would be encrypted with the *same* keystream, and
    /// the XOR of any two ciphertexts would disclose the XOR of their
    /// plaintexts (a two-time pad) without the key. `cbcs` is not affected the
    /// same way — its AES-CBC chain is seeded from the IV but every block's
    /// input then depends on the preceding ciphertext, so identical plaintext
    /// blocks in different samples do not yield a recoverable keystream.
    ///
    /// The standard `cbcs` convention — real `cbcs` deployments overwhelmingly
    /// use a constant IV (confirmed against Bento4's `mp4encrypt`, which always
    /// emits one for `cbcs` regardless of the `--key` IV given it), and
    /// Bento4's `mp4decrypt` requires it (or a genuine 16-byte per-sample IV)
    /// to actually decrypt `cbcs` — an 8-byte per-sample IV silently no-ops.
    Constant([u8; KEY_LEN]),
}

pub use super::ir::ConstantIvSenc;

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

/// Configuration for [`CencEncryptStage::feed`].
///
/// Does **not** carry the content key: the key lives on the [`CencEncryptStage`]
/// instance itself ([`CencEncryptStage::new`]), since one encryptor is bound to
/// one key for its whole lifetime (see that type's docs) — a per-call `key`
/// field here would let a caller silently pair one running [`IvGen::Counter`]
/// with a *different* key from call to call, which is exactly backwards.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EncryptConfig {
    /// The protection scheme to apply (`cenc` AES-CTR or `cbcs` AES-CBC
    /// pattern).
    pub scheme: CencScheme,
    /// The 16-byte Key ID recorded in `tenc.default_KID`.
    pub kid: [u8; KEY_LEN],
    /// How each sample's IV is derived. Defaults to [`IvGen::Counter`].
    pub iv: IvGen,
    /// `cbcs` pattern (`crypt_byte_block`, `skip_byte_block`); defaults to
    /// `1:9` when `None`. Ignored for `cenc`.
    pub pattern: Option<(u8, u8)>,
    /// How the subsample map is chosen.
    pub subsample: SubsamplePolicy,
    /// Whether to emit a `senc` box for `cbcs` + [`IvGen::Constant`] tracks
    /// (default: [`ConstantIvSenc::Emit`] — emits a `senc` with the constant
    /// IV replicated in each sample entry for maximum decryptor interop).
    /// See [`ConstantIvSenc`] for the rationale and the opt-out shape.
    pub constant_iv_senc: ConstantIvSenc,
}

impl EncryptConfig {
    /// Build a protection configuration from its required scheme, Key ID, and
    /// subsample policy. `iv` defaults to [`IvGen::Counter`], `pattern` to
    /// `None` (the standard 1:9 `cbcs` pattern), and `constant_iv_senc` to
    /// [`ConstantIvSenc::Emit`].
    #[must_use]
    pub fn new(scheme: CencScheme, kid: [u8; KEY_LEN], subsample: SubsamplePolicy) -> Self {
        Self {
            scheme,
            kid,
            iv: IvGen::Counter,
            pattern: None,
            subsample,
            constant_iv_senc: ConstantIvSenc::default(),
        }
    }

    /// Set the protection scheme.
    #[must_use]
    pub fn with_scheme(mut self, scheme: CencScheme) -> Self {
        self.scheme = scheme;
        self
    }

    /// Set the 16-byte Key ID.
    #[must_use]
    pub fn with_kid(mut self, kid: [u8; KEY_LEN]) -> Self {
        self.kid = kid;
        self
    }

    /// Set how each sample's IV is derived.
    #[must_use]
    pub fn with_iv(mut self, iv: IvGen) -> Self {
        self.iv = iv;
        self
    }

    /// Set the `cbcs` pattern.
    #[must_use]
    pub fn with_pattern(mut self, pattern: (u8, u8)) -> Self {
        self.pattern = Some(pattern);
        self
    }

    /// Set how the subsample map is chosen.
    #[must_use]
    pub fn with_subsample(mut self, subsample: SubsamplePolicy) -> Self {
        self.subsample = subsample;
        self
    }

    /// Set whether to emit a `senc` box for constant-IV `cbcs` tracks.
    #[must_use]
    pub fn with_constant_iv_senc(mut self, constant_iv_senc: ConstantIvSenc) -> Self {
        self.constant_iv_senc = constant_iv_senc;
        self
    }
}

/// Applies CENC/CBCS protection to a stream of [`MuxInput`], one sample at a
/// time.
///
/// A `Track` announcement passes through with its [`TrackProtection`] set;
/// each `Sample` is ciphered and gets its own [`SampleProtection`]. The
/// stage is bound to one content key for its life: AES-CTR IV uniqueness is a
/// property of the key, not of any one call (see [`IvGen`]), so reuse one
/// stage across every stream segment that shares the key. [`CencEncryptStage::resume`]
/// reconstructs the counter state from a persisted [`CencEncryptStage::next_counter`].
pub struct CencEncryptStage {
    key: [u8; KEY_LEN],
    cfg: EncryptConfig,
    tenc: TrackEncryptionBox,
    next_counter: u64,
    explicit_pos: usize,
    ranges: BTreeMap<u128, u128>,
    tracks: BTreeMap<TrackId, Option<NalCodec>>,
    ready: VecDeque<MuxInput>,
}

/// Manual `Debug`: redacts the bound content key (a derived `Debug` would
/// print it verbatim).
impl fmt::Debug for CencEncryptStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CencEncryptStage")
            .field("key", &"<redacted>")
            .field("scheme", &self.cfg.scheme)
            .field("next_counter", &self.next_counter)
            .finish()
    }
}

/// Wipe the bound content key on drop (issue #1142).
impl Drop for CencEncryptStage {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.key.zeroize();
    }
}

impl CencEncryptStage {
    /// A stage bound to `key`, applying `cfg` to every track, its
    /// [`IvGen::Counter`] index starting at `0`.
    ///
    /// # Errors
    ///
    /// A configuration this crate cannot apply: an unknown scheme, a `cbcs`
    /// pattern outside `0..=15` (or `crypt_byte_block == 0` with a nonzero
    /// skip), a constant IV under `cenc`, or an [`IvGen::Explicit`] list with
    /// a non-8/16-byte IV, mixed IV lengths, or a duplicate IV.
    pub fn new(key: [u8; KEY_LEN], cfg: EncryptConfig) -> Result<Self> {
        Self::build(key, cfg, 0)
    }

    /// [`CencEncryptStage::new`] with the [`IvGen::Counter`] index resuming at
    /// `next_counter` (a value previously read from
    /// [`CencEncryptStage::next_counter`]).
    ///
    /// # Errors
    ///
    /// Same as [`CencEncryptStage::new`].
    pub fn resume(key: [u8; KEY_LEN], cfg: EncryptConfig, next_counter: u64) -> Result<Self> {
        Self::build(key, cfg, next_counter)
    }

    fn build(key: [u8; KEY_LEN], cfg: EncryptConfig, next_counter: u64) -> Result<Self> {
        let (pattern, scheme) = validate_config(&cfg)?;
        let (per_sample_iv_size, default_constant_iv) =
            tenc_iv_fields(cfg.scheme, &cfg.iv, cfg.constant_iv_senc)?;
        let tenc = TrackEncryptionBox {
            version: if scheme == CencScheme::Cbcs { 1 } else { 0 },
            default_crypt_byte_block: pattern.0,
            default_skip_byte_block: pattern.1,
            default_is_protected: 1,
            default_per_sample_iv_size: per_sample_iv_size,
            default_kid: cfg.kid,
            default_constant_iv,
        };
        Ok(Self {
            key,
            cfg,
            tenc,
            next_counter,
            explicit_pos: 0,
            ranges: BTreeMap::new(),
            tracks: BTreeMap::new(),
            ready: VecDeque::new(),
        })
    }

    /// The next [`IvGen::Counter`] value this stage will hand out.
    pub fn next_counter(&self) -> u64 {
        self.next_counter
    }

    /// The per-sample IV width (bytes) [`IvGen::Counter`] emits under
    /// `scheme` — 8 under [`CencScheme::Cenc`], 16 under
    /// [`CencScheme::Cbcs`] (see [`IvGen::Counter`]).
    const fn counter_iv_size(scheme: CencScheme) -> usize {
        match scheme {
            CencScheme::Cenc => PER_SAMPLE_IV_SIZE as usize,
            _ => KEY_LEN,
        }
    }

    fn resolve_iv(&mut self, track: TrackId) -> Result<Vec<u8>> {
        match &self.cfg.iv {
            IvGen::Counter => {
                let v = self.next_counter;
                if Self::counter_iv_size(self.cfg.scheme) == PER_SAMPLE_IV_SIZE as usize {
                    Ok(v.to_be_bytes().to_vec())
                } else {
                    Ok(u128::from(v).to_be_bytes().to_vec())
                }
            }
            IvGen::Explicit(ivs) => {
                let iv = ivs.get(self.explicit_pos).ok_or(Error::CencIvList {
                    track,
                    message: "IvGen::Explicit has no IV left for this sample",
                })?;
                Ok(iv.clone())
            }
            IvGen::Constant(iv) => match self.cfg.constant_iv_senc {
                ConstantIvSenc::Emit => Ok(iv.to_vec()),
                ConstantIvSenc::Omit => Ok(Vec::new()),
            },
        }
    }

    fn encrypt_sample(&mut self, track: TrackId, mut sample: crate::ir::Sample) -> Result<()> {
        let nal = *self
            .tracks
            .get(&track)
            .ok_or(Error::CencUnannouncedTrack { track })?;
        let iv = self.resolve_iv(track)?;
        if iv.len() != usize::from(self.tenc.default_per_sample_iv_size) {
            return Err(Error::CencIvSizeMismatch {
                track,
                tenc: self.tenc.default_per_sample_iv_size,
                sample: iv.len(),
            });
        }
        let subsamples = match (self.cfg.subsample, nal) {
            (SubsamplePolicy::Video, Some(codec)) => nal_subsamples(codec, &sample.data)?,
            _ => Vec::new(),
        };
        self.check_ctr_range(&iv, sample.data.len(), &subsamples)?;
        let entry = SampleEncryptionEntry {
            initialization_vector: iv,
            subsamples,
        };
        match self.cfg.scheme {
            CencScheme::Cenc => cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
                cenc_crypto::apply_ctr(
                    &entry.initialization_vector,
                    &self.key,
                    &entry.subsamples,
                    buf,
                )
            })?,
            CencScheme::Cbcs => cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
                cenc_crypto::cbcs_sample(
                    &self.tenc,
                    &entry.initialization_vector,
                    &entry.subsamples,
                    &self.key,
                    buf,
                    CbcsOp::Encrypt,
                )
            })?,
            other => return Err(Error::UnsupportedCencScheme { scheme: other }),
        };
        sample.protection = Some(Arc::new(SampleProtection {
            iv: Bytes::from(entry.initialization_vector),
            subsamples: entry.subsamples,
        }));
        if matches!(self.cfg.iv, IvGen::Counter) {
            self.next_counter = self
                .next_counter
                .checked_add(1)
                .ok_or(Error::InvalidInput("CENC IV counter overflow"))?;
        } else if matches!(self.cfg.iv, IvGen::Explicit(_)) {
            self.explicit_pos = self.explicit_pos.saturating_add(1);
        }
        self.ready.push_back(MuxInput::Sample { track, sample });
        Ok(())
    }

    /// Reject a planned `cenc` AES-CTR counter-block range overlapping an
    /// earlier one (ISO/IEC 23001-7 §10.1). `cbcs` has no counter.
    fn check_ctr_range(
        &mut self,
        iv: &[u8],
        len: usize,
        subsamples: &[SubSampleEntry],
    ) -> Result<()> {
        if self.cfg.scheme != CencScheme::Cenc || iv.is_empty() {
            return Ok(());
        }
        let mut counter = [0u8; KEY_LEN];
        counter[..iv.len()].copy_from_slice(iv);
        let start = u128::from_be_bytes(counter);
        let blocks = protected_block_count(len, subsamples).ok_or(Error::InvalidInput(
            "CENC protected byte count overflows the u128 counter space",
        ))?;
        if blocks == 0 {
            return Ok(());
        }
        let end = start.checked_add(blocks).ok_or(Error::InvalidInput(
            "CENC AES-CTR counter block range overflows 128 bits",
        ))?;
        if let Some((_, previous_end)) = self.ranges.range(..=start).next_back()
            && *previous_end > start
        {
            return Err(Error::InvalidInput(CTR_OVERLAP));
        }
        if let Some((next_start, _)) = self.ranges.range(start..).next()
            && *next_start < end
        {
            return Err(Error::InvalidInput(CTR_OVERLAP));
        }
        self.ranges.insert(start, end);
        Ok(())
    }
}

const CTR_OVERLAP: &str = concat!(
    "overlapping CENC AES-CTR counter-block ranges: under cenc the per-sample IV is the 128-bit ",
    "counter block and the counter advances once per 16-byte protected block (ISO/IEC 23001-7 ",
    "§10.1), so a sequential IV list such as base+i reuses sample i's later keystream blocks as ",
    "sample i+1's first (a two-time pad)",
);

impl Stage for CencEncryptStage {
    type In<'a> = MuxInput;
    type Out = MuxInput;
    type Error = Error;

    fn feed(&mut self, input: Self::In<'_>, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(mut spec) => {
                let nal = nal_codec_for(&spec.config);
                spec.protection = Some(TrackProtection::new(
                    self.cfg.scheme,
                    self.tenc.clone(),
                    self.cfg.constant_iv_senc,
                ));
                self.tracks.insert(spec.track_id, nal);
                self.ready.push_back(MuxInput::Track(spec));
            }
            MuxInput::Sample { track, sample } => self.encrypt_sample(track, sample)?,
            other => self.ready.push_back(other),
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<MuxInput> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if let IvGen::Explicit(ivs) = &self.cfg.iv
            && self.explicit_pos != ivs.len()
        {
            return Err(Error::InvalidInput(
                "IvGen::Explicit must supply exactly one IV per sample",
            ));
        }
        Ok(())
    }
}

/// Validate an [`EncryptConfig`] once, returning the effective `cbcs`
/// pattern (zero for `cenc`) and the scheme.
fn validate_config(cfg: &EncryptConfig) -> Result<((u8, u8), CencScheme)> {
    if cfg.scheme == CencScheme::Cenc && matches!(cfg.iv, IvGen::Constant(_)) {
        return Err(Error::InvalidInput(
            "IvGen::Constant is cbcs-only: a constant IV under cenc (AES-CTR) derives one \
             counter block for every sample, reusing a single keystream (two-time pad)",
        ));
    }
    let pattern = match cfg.scheme {
        CencScheme::Cbcs => {
            let p = cfg.pattern.unwrap_or(DEFAULT_CBCS_PATTERN);
            if p.0 > CBCS_PATTERN_MAX || p.1 > CBCS_PATTERN_MAX {
                return Err(Error::InvalidInput(
                    "cbcs pattern block counts must each be 0..=15",
                ));
            }
            if p.0 == 0 && p.1 != 0 {
                return Err(Error::InvalidInput(
                    "cbcs pattern crypt_byte_block=0 with nonzero skip leaves data unprotected",
                ));
            }
            p
        }
        CencScheme::Cenc => (0, 0),
        other => return Err(Error::UnsupportedCencScheme { scheme: other }),
    };
    if let IvGen::Explicit(ivs) = &cfg.iv {
        let mut seen: BTreeSet<&[u8]> = BTreeSet::new();
        let mut width = None;
        for iv in ivs {
            if !VALID_EXPLICIT_IV_LENS.contains(&iv.len()) {
                return Err(Error::InvalidInput(
                    "CENC per-sample IV must be 8 or 16 bytes",
                ));
            }
            if let Some(width) = width
                && iv.len() != width
            {
                return Err(Error::InvalidInput(
                    "IvGen::Explicit IVs must all share one length",
                ));
            }
            width = Some(iv.len());
            if !seen.insert(iv.as_slice()) {
                return Err(Error::InvalidInput(concat!(
                    "duplicate IvGen::Explicit per-sample IV: an IV must be unique per content key ",
                    "(ISO/IEC 23001-7 §9.2) — reuse under cenc (AES-CTR) reuses its keystream",
                )));
            }
        }
    }
    Ok((pattern, cfg.scheme))
}

/// The number of AES-CTR counter blocks a sample consumes: one per 16 protected
/// bytes, across the concatenated *protected* ranges (clear ranges are skipped
/// and never advance the counter — see [`cenc_crypto::apply_ctr`]).
///
/// When the subsample map is empty the whole sample is protected, so the count
/// is `ceil(sample_len / 16)`. Returns `None` when the count does not fit a
/// `u128`, which cannot happen for a real sample but keeps the arithmetic
/// total.
fn protected_block_count(sample_len: usize, subsamples: &[SubSampleEntry]) -> Option<u128> {
    let protected: u128 = if subsamples.is_empty() {
        sample_len as u128
    } else {
        subsamples.iter().try_fold(0u128, |acc, s| {
            acc.checked_add(s.bytes_of_protected_data as u128)
        })?
    };
    // ceil(protected / 16)
    protected
        .checked_add(BLOCK_LEN_U128 - 1)
        .map(|v| v / BLOCK_LEN_U128)
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
pub(crate) fn nal_subsamples(codec: NalCodec, data: &[u8]) -> Result<Vec<SubSampleEntry>> {
    let header_len: usize = match codec {
        NalCodec::Avc => 1,
        NalCodec::Hevc | NalCodec::Vvc => 2,
        _ => 2,
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

/// Derive `tenc`'s `(default_per_sample_iv_size, default_constant_IV)` pair
/// from the chosen [`IvGen`] and [`ConstantIvSenc`] choice (ISO/IEC 23001-7 §12.2):
///
/// - [`IvGen::Constant`] + [`ConstantIvSenc::Emit`]: `default_per_sample_iv_size = 16`,
///   `default_constant_IV = Some(iv)` — the constant IV is carried in both
///   `tenc` and replicated into every `senc` entry (the default for maximum
///   interop).
/// - [`IvGen::Constant`] + [`ConstantIvSenc::Omit`]: `default_per_sample_iv_size = 0`,
///   `default_constant_IV = Some(iv)` — the spec-minimal, `tenc`-only shape
///   (no `senc`/`saiz`/`saio`).
/// - [`IvGen::Counter`]: `default_per_sample_iv_size` is
///   [`CencEncryptStage::counter_iv_size`] of the scheme — 8 under `cenc`
///   (every counter IV is an 8-byte big-endian value), 16 under `cbcs`
///   (a genuine 16-byte per-sample IV is what a `cbcs` decryptor requires;
///   see that method) — no constant IV.
/// - [`IvGen::Explicit`]: `default_per_sample_iv_size` is the shared length of
///   every supplied IV (checked uniform here, since the wire format has only
///   one track-wide size — a per-sample length mismatch would otherwise
///   silently desync `senc`'s IV field width from `saiz`'s per-sample aux
///   size), no constant IV. That shared length is also validated here to be
///   exactly 8 or 16 bytes — an empty (or any other length) IV would build an
///   all-zero or malformed AES-CTR/CBC counter (a two-time-pad, in the
///   all-zero case). An empty list falls back to the scheme's counter IV
///   width (there is no sample to measure; [`CencEncryptStage::validate_iv_gen`]
///   will itself reject the count mismatch against the input's real total
///   sample count).
fn tenc_iv_fields(
    scheme: CencScheme,
    iv_gen: &IvGen,
    constant_iv_senc: ConstantIvSenc,
) -> Result<(u8, Option<Vec<u8>>)> {
    match iv_gen {
        IvGen::Constant(iv) => match constant_iv_senc {
            ConstantIvSenc::Emit => Ok((CONSTANT_IV_LEN as u8, Some(iv.to_vec()))),
            ConstantIvSenc::Omit => Ok((0, Some(iv.to_vec()))),
        },
        IvGen::Counter => Ok((CencEncryptStage::counter_iv_size(scheme) as u8, None)),
        IvGen::Explicit(ivs) => {
            let len = match ivs.first() {
                Some(first) => {
                    if ivs.iter().any(|iv| iv.len() != first.len()) {
                        return Err(Error::InvalidInput(
                            "IvGen::Explicit IVs must all share one length (tenc.default_per_sample_iv_size is one value for the whole track)",
                        ));
                    }
                    first.len()
                }
                None => CencEncryptStage::counter_iv_size(scheme),
            };
            if !VALID_EXPLICIT_IV_LENS.contains(&len) {
                return Err(Error::InvalidInput(
                    "CENC per-sample IV must be 8 or 16 bytes",
                ));
            }
            Ok((len as u8, None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::collect::Collected as Media;

    const KID: [u8; 16] = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x00,
    ];
    const KEY: [u8; 16] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10,
    ];

    fn clear_media() -> Media {
        let bytes = test_fixtures::read("ts/h264/main.ts");

        let media = crate::collect::media_from_ts(bytes.as_slice()).expect("demux main.ts");
        crate::repackage::select_tracks_by(&media, |t| {
            matches!(t.spec.config, CodecConfig::Avc { .. })
        })
        .expect("AVC video track present")
    }

    fn multi_track_media() -> Media {
        let bytes = test_fixtures::read("ts/h264-aac.ts");

        let media = crate::collect::media_from_ts(bytes.as_slice()).expect("demux h264-aac.ts");
        assert!(media.tracks.len() > 1, "a multi-track fixture");
        media
    }

    fn snapshot_all(media: &Media) -> Vec<Bytes> {
        media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.clone()))
            .collect()
    }

    fn cenc_cfg() -> EncryptConfig {
        EncryptConfig::new(CencScheme::Cenc, KID, SubsamplePolicy::Video)
    }

    /// Drive `stage` over `media`, rebuilding the `Media` from its output.
    fn drive(stage: &mut CencEncryptStage, media: &Media) -> Result<Media> {
        for track in &media.tracks {
            stage.feed(
                MuxInput::Track(Box::new(track.spec.clone())),
                Timestamp::ZERO,
            )?;
        }
        let mut order: Vec<(usize, TrackId)> = media
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (i, t.spec.track_id))
            .collect();
        order.sort_by_key(|&(_, id)| id);
        for (index, _) in &order {
            let track = &media.tracks[*index];
            for sample in &track.samples {
                stage.feed(
                    MuxInput::Sample {
                        track: track.spec.track_id,
                        sample: sample.clone(),
                    },
                    Timestamp::ZERO,
                )?;
            }
        }
        let mut tracks: Vec<crate::collect::CollectedTrack> = Vec::new();
        let mut index: BTreeMap<TrackId, usize> = BTreeMap::new();
        while let Some(item) = stage.poll() {
            match item {
                MuxInput::Track(spec) => {
                    index.insert(spec.track_id, tracks.len());
                    tracks.push(crate::collect::CollectedTrack::new(*spec, Vec::new()));
                }
                MuxInput::Sample { track, sample } => {
                    if let Some(&at) = index.get(&track) {
                        tracks[at].samples.push(sample);
                    }
                }
                _ => {}
            }
        }
        Ok(Media::new(tracks, media.movie_timescale))
    }

    #[test]
    fn debug_redacts_content_key() {
        let stage = CencEncryptStage::new(KEY, cenc_cfg()).expect("config");
        let out = alloc::format!("{stage:?}");
        assert!(!out.contains(&alloc::format!("{KEY:?}")));
        assert!(out.contains("<redacted>"));
    }

    #[test]
    fn cenc_round_trip_reverses_byte_identical() {
        let media = clear_media();
        let original = snapshot_all(&media);
        let mut stage = CencEncryptStage::resume(KEY, cenc_cfg(), 7).expect("config");
        let mut out = drive(&mut stage, &media).expect("encrypt");
        let track = &mut out.tracks[0];
        let enc = track.spec.protection.clone().expect("spec.protection Some");
        assert_eq!(enc.scheme, CencScheme::Cenc);
        assert_eq!(enc.tenc.default_kid, KID);
        assert!(track.samples.iter().all(|s| s.protection.is_some()));
        assert!(
            track
                .samples
                .iter()
                .zip(original.iter())
                .any(|(s, o)| s.data != *o),
            "encrypt must change protected bytes"
        );
        for sample in track.samples.iter_mut() {
            let protection = sample
                .protection
                .clone()
                .expect("every sample carries its protection");
            cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
                cenc_crypto::apply_ctr(&protection.iv, &KEY, &protection.subsamples, buf)
            })
            .expect("reverse apply_ctr");
        }
        let reversed: Vec<Bytes> = track.samples.iter().map(|s| s.data.clone()).collect();
        assert_eq!(reversed, original, "cenc round trip must be byte-identical");
    }

    #[test]
    fn cbcs_round_trip_reverses_byte_identical() {
        let media = clear_media();
        let original = snapshot_all(&media);
        let mut cfg = EncryptConfig::new(CencScheme::Cbcs, KID, SubsamplePolicy::Video);
        cfg.pattern = Some((1, 9));
        let mut stage = CencEncryptStage::new(KEY, cfg).expect("config");
        let mut out = drive(&mut stage, &media).expect("encrypt");
        let track = &mut out.tracks[0];
        let enc = track.spec.protection.clone().expect("spec.protection Some");
        assert_eq!(enc.scheme, CencScheme::Cbcs);
        assert_eq!(enc.tenc.default_crypt_byte_block, 1);
        assert_eq!(enc.tenc.default_skip_byte_block, 9);
        for sample in track.samples.iter_mut() {
            let protection = sample
                .protection
                .clone()
                .expect("every sample carries its protection");
            cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
                cenc_crypto::cbcs_sample(
                    &enc.tenc,
                    &protection.iv,
                    &protection.subsamples,
                    &KEY,
                    buf,
                    CbcsOp::Decrypt,
                )
            })
            .expect("reverse cbcs_sample");
        }
        let reversed: Vec<Bytes> = track.samples.iter().map(|s| s.data.clone()).collect();
        assert_eq!(reversed, original, "cbcs round trip must be byte-identical");
    }

    #[test]
    fn whole_sample_policy_yields_empty_subsample_map() {
        let media = clear_media();
        let cfg = EncryptConfig::new(CencScheme::Cenc, KID, SubsamplePolicy::WholeSample);
        let mut stage = CencEncryptStage::new(KEY, cfg).expect("config");
        let out = drive(&mut stage, &media).expect("encrypt");
        assert!(
            out.tracks[0].samples.iter().all(|s| s
                .protection
                .as_ref()
                .expect("Some")
                .subsamples
                .is_empty())
        );
    }

    #[test]
    fn explicit_iv_invalid_length_errors_at_construction() {
        for ivs in [
            alloc::vec![alloc::vec![0u8; 17]; 2],
            alloc::vec![alloc::vec![]; 2],
            alloc::vec![alloc::vec![0u8; 12]; 2],
        ] {
            let cfg = cenc_cfg().with_iv(IvGen::Explicit(ivs));
            assert!(matches!(
                CencEncryptStage::new(KEY, cfg),
                Err(Error::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn explicit_duplicate_iv_errors_at_construction() {
        let cfg = cenc_cfg().with_iv(IvGen::Explicit(alloc::vec![
            alloc::vec![0u8; 8],
            alloc::vec![0u8; 8]
        ]));
        assert!(matches!(
            CencEncryptStage::new(KEY, cfg),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn explicit_iv_count_mismatch_errors() {
        let media = clear_media();
        let n = media.tracks[0].samples.len();
        let mut cfg = cenc_cfg();
        cfg.iv = IvGen::Explicit(distinct_ivs(n - 1, 8));
        let mut stage = CencEncryptStage::new(KEY, cfg).expect("config");
        let err = drive(&mut stage, &media).expect_err("one IV short");
        assert!(matches!(err, Error::CencIvList { .. }));
    }

    #[test]
    fn explicit_iv_valid_lengths_are_ok() {
        for len in [8usize, 16] {
            let media = clear_media();
            let n = media.tracks[0].samples.len();
            let mut cfg = cenc_cfg();
            cfg.iv = IvGen::Explicit(distinct_ivs(n, len));
            let mut stage = CencEncryptStage::new(KEY, cfg).expect("config");
            let out = drive(&mut stage, &media).expect("encrypt");
            let enc = out.tracks[0].spec.protection.as_ref().expect("Some");
            assert_eq!(enc.tenc.default_per_sample_iv_size, len as u8);
            assert!(
                out.tracks[0]
                    .samples
                    .iter()
                    .all(|s| s.protection.as_ref().expect("Some").iv.len() == len)
            );
        }
    }

    #[test]
    fn cbcs_default_counter_emits_16_byte_ivs() {
        let media = clear_media();
        let mut cfg = EncryptConfig::new(CencScheme::Cbcs, KID, SubsamplePolicy::WholeSample);
        cfg.pattern = None;
        let mut stage = CencEncryptStage::new(KEY, cfg).expect("config");
        let out = drive(&mut stage, &media).expect("encrypt");
        let enc = out.tracks[0].spec.protection.as_ref().expect("Some");
        assert_eq!(enc.tenc.default_per_sample_iv_size, 16);
        assert!(
            out.tracks[0]
                .samples
                .iter()
                .all(|s| s.protection.as_ref().expect("Some").iv.len() == 16)
        );
    }

    #[test]
    fn cenc_default_counter_emits_8_byte_ivs() {
        let media = clear_media();
        let cfg = EncryptConfig::new(CencScheme::Cenc, KID, SubsamplePolicy::WholeSample);
        let mut stage = CencEncryptStage::new(KEY, cfg).expect("config");
        let out = drive(&mut stage, &media).expect("encrypt");
        let enc = out.tracks[0].spec.protection.as_ref().expect("Some");
        assert_eq!(enc.tenc.default_per_sample_iv_size, 8);
        assert!(
            out.tracks[0]
                .samples
                .iter()
                .all(|s| s.protection.as_ref().expect("Some").iv.len() == 8)
        );
    }

    #[test]
    fn cenc_sequential_16_byte_ivs_overlap_is_rejected() {
        let media = clear_media();
        let n = media.tracks[0].samples.len();
        let ivs: Vec<Vec<u8>> = (0..n)
            .map(|i| {
                let mut iv = alloc::vec![0u8; 16];
                iv[14] = (i >> 8) as u8;
                iv[15] = i as u8;
                iv
            })
            .collect();
        let unique: BTreeSet<&Vec<u8>> = ivs.iter().collect();
        assert_eq!(unique.len(), n, "the test IVs must all differ");
        let mut cfg = cenc_cfg();
        cfg.iv = IvGen::Explicit(ivs);
        let mut stage = CencEncryptStage::new(KEY, cfg).expect("config");
        let err = drive(&mut stage, &media).expect_err("overlapping ranges");
        assert!(matches!(err, Error::InvalidInput(_)), "got {err:?}");
    }

    #[test]
    fn cbcs_sequential_16_byte_ivs_accepted() {
        let media = clear_media();
        let n = media.tracks[0].samples.len();
        let ivs: Vec<Vec<u8>> = (0..n)
            .map(|i| {
                let mut iv = alloc::vec![0u8; 16];
                iv[14] = (i >> 8) as u8;
                iv[15] = i as u8;
                iv
            })
            .collect();
        let mut cfg = EncryptConfig::new(CencScheme::Cbcs, KID, SubsamplePolicy::WholeSample);
        cfg.iv = IvGen::Explicit(ivs);
        let mut stage = CencEncryptStage::new(KEY, cfg).expect("config");
        drive(&mut stage, &media).expect("cbcs accepts sequential IVs");
    }

    #[test]
    fn cbcs_zero_crypt_block_pattern_errors() {
        let cfg = EncryptConfig::new(CencScheme::Cbcs, KID, SubsamplePolicy::WholeSample)
            .with_pattern((0, 9));
        assert!(matches!(
            CencEncryptStage::new(KEY, cfg),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn constant_iv_under_cenc_errors() {
        let cfg = cenc_cfg().with_iv(IvGen::Constant([0x5Au8; KEY_LEN]));
        assert!(matches!(
            CencEncryptStage::new(KEY, cfg),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn counter_ivs_are_unique_across_tracks_and_continuous_in_feed_order() {
        const BASE: u64 = 0x0102_0304_0506_0708;
        let media = multi_track_media();
        let cfg = EncryptConfig::new(CencScheme::Cenc, KID, SubsamplePolicy::WholeSample);
        let mut stage = CencEncryptStage::resume(KEY, cfg, BASE).expect("config");
        let out = drive(&mut stage, &media).expect("encrypt");
        let ivs: Vec<Bytes> = out
            .tracks
            .iter()
            .flat_map(|t| {
                t.samples
                    .iter()
                    .map(|s| s.protection.as_ref().expect("Some").iv.clone())
            })
            .collect();
        let unique: BTreeSet<&Bytes> = ivs.iter().collect();
        assert_eq!(unique.len(), ivs.len(), "every IV is unique");
        for (i, iv) in ivs.iter().enumerate() {
            assert_eq!(&iv[..], &(BASE + i as u64).to_be_bytes()[..]);
        }
        assert_eq!(
            stage.next_counter(),
            BASE + u64::try_from(ivs.len()).expect("sample count")
        );
    }

    #[test]
    fn successive_runs_on_one_stage_produce_disjoint_ivs() {
        let cfg = cenc_cfg();
        let mut stage = CencEncryptStage::new(KEY, cfg).expect("config");
        let a = drive(&mut stage, &clear_media()).expect("first");
        let b = drive(&mut stage, &clear_media()).expect("second");
        let ivs_a: BTreeSet<Vec<u8>> = a.tracks[0]
            .samples
            .iter()
            .map(|s| s.protection.as_ref().expect("Some").iv.to_vec())
            .collect();
        let ivs_b: BTreeSet<Vec<u8>> = b.tracks[0]
            .samples
            .iter()
            .map(|s| s.protection.as_ref().expect("Some").iv.to_vec())
            .collect();
        assert!(ivs_a.is_disjoint(&ivs_b), "one key, disjoint IVs");
    }

    #[test]
    fn counter_overflow_is_rejected() {
        let media = clear_media();
        let cfg = cenc_cfg();
        let mut stage = CencEncryptStage::resume(KEY, cfg, u64::MAX).expect("config");
        let err = drive(&mut stage, &media).expect_err("counter overflow");
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(stage.next_counter(), u64::MAX);
    }

    #[test]
    fn unannounced_track_errors() {
        let media = clear_media();
        let sample = media.tracks[0].samples[0].clone();
        let mut stage = CencEncryptStage::new(KEY, cenc_cfg()).expect("config");
        let err = stage
            .feed(
                MuxInput::Sample {
                    track: media.tracks[0].spec.track_id,
                    sample,
                },
                Timestamp::ZERO,
            )
            .expect_err("track not announced");
        assert!(matches!(err, Error::CencUnannouncedTrack { .. }));
    }

    #[test]
    fn real_fixture_samples_take_the_zero_copy_fast_path() {
        let mut media = clear_media();
        let track = &mut media.tracks[0];
        assert!(track.samples.len() > 1, "fixture must carry samples");
        for sample in &mut track.samples {
            let took_fast_path =
                cenc_crypto::rewrite_in_place(&mut sample.data, |_buf| Ok(())).expect("no-op ok");
            assert!(took_fast_path, "freshly-demuxed samples are uniquely owned");
        }
    }

    fn distinct_ivs(n: usize, len: usize) -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| {
                let mut iv = alloc::vec![0xABu8; len];
                iv[len - 5] = (i & 0xFF) as u8;
                iv[len - 6] = ((i >> 8) & 0xFF) as u8;
                iv[len - 7] = ((i >> 16) & 0xFF) as u8;
                iv[len - 8] = ((i >> 24) & 0xFF) as u8;
                iv
            })
            .collect()
    }
}
