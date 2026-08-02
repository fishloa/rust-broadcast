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
//! # IV uniqueness is per *key*, for all time
//!
//! AES-CTR (`cenc`) requires every sample ciphered under one content key to
//! use a distinct IV — not just within one [`Encrypt::encrypt`] call, but
//! across *every* call ever made with that key (ISO/IEC 23001-7 §9.2). A
//! [`CencEncryptor`] is therefore a **stateful** value bound to one content
//! key ([`CencEncryptor::new`]): [`IvGen::Counter`]'s running index lives on
//! the encryptor instance, not in [`EncryptConfig`], so calling `encrypt`
//! twice on the *same* instance continues the counter instead of restarting
//! it — reuse *one* instance across every call sharing a key (video-only +
//! audio-only splits of one asset, successive live segments, …). See
//! [`CencEncryptor`]'s own docs for the caller obligation this does and does
//! not enforce.
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

use alloc::collections::BTreeSet;
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

/// How to derive each sample's initialization vector.
///
/// # IV uniqueness is per *key*, not per track — and not per call
///
/// ISO/IEC 23001-7 §9.2 requires each sample's IV to be unique for the
/// **content key** it is used with. One [`EncryptConfig`] carries one key and
/// is applied to *every* track of the [`Media`], so "unique within this track"
/// is **not** sufficient: under `cenc` (AES-CTR) two samples sharing a key and
/// a counter block produce ciphertexts whose XOR is the XOR of their
/// plaintexts — a two-time pad that discloses both without the key. Every
/// variant below is therefore indexed/counted across the **whole `Media`** of
/// one [`Encrypt::encrypt`] call — and, for [`IvGen::Counter`], across every
/// call ever made on the same [`CencEncryptor`] instance (see that type's
/// docs): uniqueness does not reset just because a second call started.
/// [`CencEncryptor::encrypt`] additionally rejects any duplicate per-sample IV
/// it is about to use, *before* ciphering a single byte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum IvGen {
    /// Per-sample 8-byte IV = big-endian `next_counter + sample_index`, where
    /// `sample_index` runs **continuously across every track** of the
    /// [`Media`] in (track, sample) order — it does *not* restart per track
    /// (see this enum's docs: that would reuse one AES-CTR keystream between,
    /// say, video sample *i* and audio sample *i*) — and `next_counter` is
    /// the encrypting [`CencEncryptor`] instance's own running index, which it
    /// advances after every successful call rather than resetting to a
    /// caller-supplied `base` (see that type's docs for why the counter lives
    /// there and not here). The cipher core zero-pads the 8 bytes to a
    /// 16-byte counter block. The default.
    #[default]
    Counter,
    /// Caller-supplied per-sample IVs, one per sample of the **whole
    /// `Media`**: exactly `media.tracks.iter().map(|t| t.samples.len()).sum()`
    /// entries, consumed in (track, sample) order (track 0's samples first,
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
    /// **`cbcs`-only** — [`CencEncryptor::encrypt`] rejects this variant under
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

pub use super::cenc::ConstantIvSenc;

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
///
/// Does **not** carry the content key: the key lives on the [`CencEncryptor`]
/// instance itself ([`CencEncryptor::new`]), since one encryptor is bound to
/// one key for its whole lifetime (see that type's docs) — a per-call `key`
/// field here would let a caller silently pair one running [`IvGen::Counter`]
/// with a *different* key from call to call, which is exactly backwards.
#[derive(Debug, Clone)]
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

/// Applies CENC/CBCS sample protection to a [`Media`], implementing
/// [`Encrypt`] — the inverse of [`crate::cenc_decrypt::CencDecryptor`].
///
/// # Bound to one key, for the life of the instance
///
/// AES-CTR IV uniqueness is a property of the **key**, not of any one
/// `encrypt` call (see the module docs and [`IvGen`]'s). A `CencEncryptor` is
/// therefore constructed with its content key ([`CencEncryptor::new`]) and
/// carries the running [`IvGen::Counter`] index as its own state, advancing
/// it after every successful `encrypt` rather than restarting at a
/// caller-supplied base each time — reuse *one* instance across every call
/// that shares a key (e.g. a video-only pass then an audio-only pass over the
/// same asset, or successive segments of one live key period).
///
/// This closes the two-time-pad this type previously permitted when used as
/// a stateless, `Default`-constructed unit value (fixed in 0.20.0): calling
/// `CencEncryptor::new(key).encrypt(...)` a second time for a *different*
/// call that shares `key` used to restart the counter at the config's `base`,
/// silently reproducing the exact keystream-reuse defect this type exists to
/// prevent within one call.
///
/// **What this does not (and structurally cannot) enforce**: constructing
/// *two* separate `CencEncryptor::new(key)` instances with the *same* `key`
/// still collides — each starts its own counter at 0 — because the type has
/// no way to know another instance ever used `key` before. Reusing one
/// instance's counter, never re-deriving a "fresh" one for a key already in
/// use, is the caller's obligation; [`CencEncryptor::resume`] exists for the
/// one legitimate case that looks like a fresh instance (recovering
/// in-process state across e.g. a process restart, from a persisted
/// `next_counter()`).
#[derive(Debug)]
pub struct CencEncryptor {
    /// The AES-128 content key every `encrypt` call on this instance uses.
    key: [u8; KEY_LEN],
    /// The next [`IvGen::Counter`] value this instance will hand out — the
    /// running index described above. Unused (and unadvanced) by
    /// [`IvGen::Explicit`]/[`IvGen::Constant`] calls.
    next_counter: u64,
}

impl CencEncryptor {
    /// Construct a fresh encryptor bound to `key`, with its
    /// [`IvGen::Counter`] index starting at `0`.
    ///
    /// Reuse the returned value across every `encrypt` call that shares
    /// `key` — see this type's docs for why constructing a second
    /// `CencEncryptor::new(key)` with the same `key` reintroduces the
    /// two-time pad this type exists to prevent.
    pub fn new(key: [u8; KEY_LEN]) -> Self {
        Self {
            key,
            next_counter: 0,
        }
    }

    /// Construct an encryptor bound to `key`, resuming its [`IvGen::Counter`]
    /// index at `next_counter` instead of `0`.
    ///
    /// For recovering an in-process encryptor's state across a boundary that
    /// doesn't preserve the Rust value itself (e.g. a process restart in a
    /// live-streaming pipeline) from a previously observed
    /// [`CencEncryptor::next_counter`] — **not** a substitute for reusing one
    /// instance in the common case, and not a safe way to "pick up" someone
    /// else's counter unless `next_counter` is known to be past every IV that
    /// instance ever produced for `key`.
    pub fn resume(key: [u8; KEY_LEN], next_counter: u64) -> Self {
        Self { key, next_counter }
    }

    /// The next [`IvGen::Counter`] value this instance will hand out.
    /// Snapshot this (alongside the key, out of band) to reconstruct
    /// equivalent state later via [`CencEncryptor::resume`].
    pub fn next_counter(&self) -> u64 {
        self.next_counter
    }
}

impl Encrypt for CencEncryptor {
    type Media = Media;
    type Config = EncryptConfig;
    type Error = Error;

    /// Encrypt every track's samples in `media` in place per `cfg`, recording
    /// the resulting crypto metadata onto each [`crate::media::Track::encryption`].
    ///
    /// `cfg` (scheme/KID/IV mode/pattern/subsample policy) is applied
    /// uniformly to every track in `media`, ciphered with `self`'s bound
    /// content key. Because that one key covers every track — and, for
    /// [`IvGen::Counter`], every call ever made on `self` (see
    /// [`CencEncryptor`]'s docs) — the per-sample IV must be unique across
    /// all of it (ISO/IEC 23001-7 §9.2). This method enforces that in two
    /// phases, fully separated so the second can never run against stale or
    /// drifted data:
    ///
    /// 1. **Plan, then validate — before ciphering anything.** An internal
    ///    validation pass checks [`IvGen::Explicit`]'s count/length and
    ///    [`IvGen::Counter`]'s overflow bound; an internal planning pass then
    ///    resolves the *exact* IV every sample of `media` will use (in
    ///    (track, sample) order, continuously across tracks); a final
    ///    internal check rejects that plan outright if it contains any
    ///    duplicate. Only a plan that passes is ever handed to the cipher —
    ///    the same values, not a value recomputed afterwards that could
    ///    drift from what was checked.
    /// 2. **Cipher from the validated plan.** The main loop consumes
    ///    `plan[track][sample]` directly to build each
    ///    [`crate::cenc::SampleEncryptionEntry`] and to seed the cipher core —
    ///    it never calls IV resolution again, so there is no path by which
    ///    the recorded IV can differ from the one the uniqueness check saw.
    ///
    /// A configuration this rejects — at either phase — leaves `media`
    /// byte-identical to its input: no track's samples are touched, and no
    /// `Track::encryption` is populated, until the *whole* plan is proven
    /// duplicate-free. This is the fix for the original backstop's timing
    /// bug: a check with the same purpose used to run only after every track
    /// had already been keystreamed in place, so a reintroduced per-track
    /// index reset (the original vulnerability) rejected the config but left
    /// `media` two-time-padded with no rollback.
    ///
    /// [`IvGen::Constant`] is rejected outright under [`CencScheme::Cenc`] (a
    /// constant AES-CTR counter is a keystream repeat by construction), also
    /// before anything else is validated or touched.
    fn encrypt(&mut self, media: &mut Media, cfg: &EncryptConfig) -> Result<()> {
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
                p
            }
            CencScheme::Cenc => (0, 0),
            // `CencScheme` is `#[non_exhaustive]` (and now defined in
            // `broadcast-common`): reject a scheme this crate has no cipher
            // for rather than silently encrypting under the wrong one.
            other => return Err(Error::UnsupportedCencScheme { scheme: other }),
        };
        let (per_sample_iv_size, default_constant_iv) =
            tenc_iv_fields(&cfg.iv, cfg.constant_iv_senc)?;
        let tenc = TrackEncryptionBox {
            // `cbcs` pattern fields only carry meaning under version 1
            // (ISO/IEC 23001-7 §12.2); `cenc` has no pattern, so version 0.
            version: if cfg.scheme == CencScheme::Cbcs { 1 } else { 0 },
            default_crypt_byte_block: pattern.0,
            default_skip_byte_block: pattern.1,
            default_is_protected: 1,
            default_per_sample_iv_size: per_sample_iv_size,
            default_kid: cfg.kid,
            default_constant_iv,
        };

        // IV uniqueness spans the whole `Media` (and, for `Counter`, every
        // prior call on `self`), so every count/length/overflow check is
        // done up front, before the first sample is ciphered — a rejected
        // config leaves `media` untouched rather than half-encrypted.
        let total_samples: usize = media.tracks.iter().map(|t| t.samples.len()).sum();
        self.validate_iv_gen(&cfg.iv, total_samples)?;

        // F1 fix: resolve the *entire* planned (track, sample) -> IV mapping
        // first, and validate that plan for duplicates before a single
        // sample is touched. The main loop below then consumes this exact
        // plan (never re-resolving), so what was validated and what gets
        // recorded/ciphered can never drift apart.
        let plan = self.plan_sample_ivs(media, &cfg.iv, cfg.constant_iv_senc)?;
        assert_ivs_unique(&plan, &cfg.iv, cfg.constant_iv_senc)?;

        for (track, track_ivs) in media.tracks.iter_mut().zip(plan.iter()) {
            let nal_codec = nal_codec_for(&track.spec.config);
            let sample_count = track.samples.len();
            let mut entries = Vec::with_capacity(sample_count);

            for (sample, iv) in track.samples.iter_mut().zip(track_ivs.iter()) {
                let subsamples = match (cfg.subsample, nal_codec) {
                    (SubsamplePolicy::Video, Some(codec)) => nal_subsamples(codec, &sample.data)?,
                    _ => Vec::new(),
                };
                let entry = SampleEncryptionEntry {
                    initialization_vector: iv.clone(),
                    subsamples,
                };

                match cfg.scheme {
                    CencScheme::Cenc => cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
                        cenc_crypto::apply_ctr(
                            &entry.initialization_vector,
                            &self.key,
                            &entry.subsamples,
                            buf,
                        )
                    })?,
                    CencScheme::Cbcs => cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
                        cenc_crypto::cbcs_sample(&tenc, &entry, &self.key, buf, CbcsOp::Encrypt)
                    })?,
                    // Unreachable in practice: the same scheme was already
                    // validated at the top of `encrypt`. Kept as an error (not
                    // `unreachable!`) so a future scheme added to
                    // `broadcast-common` can never turn this into a panic.
                    other => return Err(Error::UnsupportedCencScheme { scheme: other }),
                };

                entries.push(entry);
            }

            track.encryption = Some(TrackEncryption {
                scheme: cfg.scheme,
                tenc: tenc.clone(),
                samples: entries,
                constant_iv_senc: cfg.constant_iv_senc,
            });
        }

        // Only `IvGen::Counter` consumes `self`'s running index; a
        // successful call using it must never hand out any of these IVs
        // again, from any future call on `self` (see this type's docs) — the
        // overflow this could hit was already proven impossible by
        // `validate_iv_gen` above, using the same starting point.
        if matches!(cfg.iv, IvGen::Counter) {
            self.next_counter += total_samples as u64;
        }
        Ok(())
    }
}

impl CencEncryptor {
    /// Validate an [`IvGen`] against the total sample count of the whole
    /// [`Media`], before any sample is ciphered or planned.
    ///
    /// Checks (a) that [`IvGen::Explicit`] carries exactly one IV per sample
    /// of the *whole* `Media` — not per track (ISO/IEC 23001-7 §9.2:
    /// uniqueness is per key, and one key covers every track), (b) that each
    /// IV it will hand the cipher is a valid length, and (c) that no two IVs
    /// are equal, and — for [`IvGen::Counter`] — that continuing `self`'s
    /// running index across `total_samples` more samples cannot overflow.
    /// Doing this up-front keeps `encrypt` atomic for the whole class of IV
    /// misconfiguration: a rejected config never leaves a partially-encrypted
    /// `Media` behind.
    ///
    /// [`IvGen::Counter`] needs no duplicate set here: `next_counter + idx`
    /// with a `checked_add` guard is strictly increasing, hence
    /// collision-free by construction within this call — [`assert_ivs_unique`]
    /// is the backstop that catches a future regression in that construction
    /// (e.g. a reintroduced per-track reset) rather than trusting the math
    /// alone. [`IvGen::Constant`] carries no per-sample IV at all (it lives
    /// once in `tenc.default_constant_IV`), and is `cbcs`-only — where the CBC
    /// chain, not a keystream, depends on it (see [`IvGen::Constant`]).
    fn validate_iv_gen(&self, iv_gen: &IvGen, total_samples: usize) -> Result<()> {
        match iv_gen {
            IvGen::Counter => {
                // Surface the overflow up-front rather than mid-pass.
                let last = total_samples.saturating_sub(1) as u64;
                self.next_counter
                    .checked_add(last)
                    .ok_or(Error::InvalidInput(
                        "CENC IV counter overflow (next_counter + sample_index)",
                    ))?;
                Ok(())
            }
            IvGen::Explicit(ivs) => {
                if ivs.len() != total_samples {
                    return Err(Error::InvalidInput(
                        "IvGen::Explicit must supply exactly one IV per sample of the whole Media \
                         (the sum of every track's sample count), consumed in (track, sample) order — \
                         one content key covers every track, so IV uniqueness is per key, not per track",
                    ));
                }
                let mut seen: BTreeSet<&[u8]> = BTreeSet::new();
                for iv in ivs {
                    if !VALID_EXPLICIT_IV_LENS.contains(&iv.len()) {
                        return Err(Error::InvalidInput(
                            "CENC per-sample IV must be 8 or 16 bytes",
                        ));
                    }
                    if !seen.insert(iv.as_slice()) {
                        return Err(Error::InvalidInput(
                            "duplicate IvGen::Explicit per-sample IV: an IV must be unique per \
                             content key (ISO/IEC 23001-7 §9.2) — reusing one under cenc (AES-CTR) \
                             reuses its keystream (two-time pad)",
                        ));
                    }
                }
                Ok(())
            }
            IvGen::Constant(_) => Ok(()),
        }
    }

    /// Resolve the **entire** planned per-sample IV sequence for `media`
    /// under `iv_gen`, in (track, sample) order, continuously across tracks —
    /// the exact values [`Encrypt::encrypt`] will go on to record and cipher
    /// with, computed *before* it ciphers anything.
    ///
    /// This is the data [`assert_ivs_unique`] validates and the main cipher
    /// loop consumes directly (never re-resolved), so there is no window in
    /// which what was checked for uniqueness can differ from what gets used.
    fn plan_sample_ivs(
        &self,
        media: &Media,
        iv_gen: &IvGen,
        constant_iv_senc: ConstantIvSenc,
    ) -> Result<Vec<Vec<Vec<u8>>>> {
        let mut media_sample_idx = 0usize;
        let mut plan = Vec::with_capacity(media.tracks.len());
        for track in &media.tracks {
            let mut track_ivs = Vec::with_capacity(track.samples.len());
            for _ in &track.samples {
                track_ivs.push(self.resolve_iv(iv_gen, media_sample_idx, constant_iv_senc)?);
                media_sample_idx += 1;
            }
            plan.push(track_ivs);
        }
        Ok(plan)
    }

    /// Resolve the per-sample `senc` IV for `idx` — the sample's index within
    /// the **whole [`Media`]** (see [`IvGen`]), not within its track — from
    /// the configured [`IvGen`].
    ///
    /// Assumes [`Self::validate_iv_gen`] has already checked the list length,
    /// IV lengths, and overflow bound for this `Media`; the length/overflow
    /// guards here are kept as a belt-and-braces second line, never the only
    /// one. [`IvGen::Constant`] returns the 16-byte constant IV when
    /// `constant_iv_senc` is [`ConstantIvSenc::Emit`] (the default —
    /// recorded per-sample in `senc`), or an empty IV when
    /// [`ConstantIvSenc::Omit`] (the spec-minimal shape where the IV lives
    /// only in `tenc.default_constant_IV`).
    fn resolve_iv(
        &self,
        iv_gen: &IvGen,
        idx: usize,
        constant_iv_senc: ConstantIvSenc,
    ) -> Result<Vec<u8>> {
        match iv_gen {
            IvGen::Counter => {
                let v = self
                    .next_counter
                    .checked_add(idx as u64)
                    .ok_or(Error::InvalidInput(
                        "CENC IV counter overflow (next_counter + sample_index)",
                    ))?;
                Ok(v.to_be_bytes().to_vec())
            }
            IvGen::Explicit(ivs) => {
                let iv = ivs.get(idx).ok_or(Error::InvalidInput(
                    "IvGen::Explicit must supply exactly one IV per sample of the whole Media",
                ))?;
                if !VALID_EXPLICIT_IV_LENS.contains(&iv.len()) {
                    return Err(Error::InvalidInput(
                        "CENC per-sample IV must be 8 or 16 bytes",
                    ));
                }
                Ok(iv.clone())
            }
            IvGen::Constant(iv) => match constant_iv_senc {
                ConstantIvSenc::Emit => Ok(iv.to_vec()),
                ConstantIvSenc::Omit => Ok(Vec::new()),
            },
        }
    }
}

/// Reject any duplicate per-sample IV anywhere in a **planned** IV sequence
/// (see [`CencEncryptor::plan_sample_ivs`]) — called *before* a single sample
/// is ciphered.
///
/// [`CencEncryptor::validate_iv_gen`] already rejects the reachable
/// misconfigurations, so this can only fire if the plan-generation bookkeeping
/// itself is wrong (e.g. a per-track index reset — the original defect). It is
/// deliberately a check on the planned *output* rather than trusting the
/// generator's math: that makes the whole class of keystream-reuse bug
/// impossible to reintroduce silently, instead of fixing one instance of it —
/// and, because the plan is the same data the cipher loop then consumes
/// unmodified, a rejection here happens strictly before `media` is touched.
/// Cost is one `BTreeSet` of borrowed slices (no IV is cloned).
///
/// [`IvGen::Constant`] IVs are skipped: when
/// [`ConstantIvSenc::Emit`](super::ConstantIvSenc::Emit) every entry carries
/// the same constant IV by design (the duplicate check would be a false
/// positive), and when [`ConstantIvSenc::Omit`](super::ConstantIvSenc::Omit)
/// every entry is empty (there is no per-sample IV at all). Every other
/// `IvGen` + `constant_iv_senc` combination is checked — including `cenc` +
/// `Counter` with `Emit` set, where the short-circuit is intentionally NOT
/// applied: `Emit` only makes sense with `cbcs`+`Constant`, and a future
/// regression that introduces duplicates on a `cenc`/`Counter` path must
/// still be caught by this backstop.
fn assert_ivs_unique(
    plan: &[Vec<Vec<u8>>],
    iv_gen: &IvGen,
    constant_iv_senc: ConstantIvSenc,
) -> Result<()> {
    // Only skip the check for the one combination where repetition is by
    // design: cbcs with a constant IV being replicated into every entry.
    if matches!(iv_gen, IvGen::Constant(_)) && matches!(constant_iv_senc, ConstantIvSenc::Emit) {
        return Ok(());
    }
    let mut seen: BTreeSet<&[u8]> = BTreeSet::new();
    for track_ivs in plan {
        for iv in track_ivs {
            if iv.is_empty() {
                continue;
            }
            if !seen.insert(iv.as_slice()) {
                return Err(Error::InvalidInput(
                    "duplicate CENC per-sample IV planned across the Media's tracks: an IV must \
                     be unique per content key (ISO/IEC 23001-7 §9.2), and one CencEncryptor key \
                     covers every track — reuse under cenc (AES-CTR) is a two-time pad",
                ));
            }
        }
    }
    Ok(())
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
/// - [`IvGen::Counter`]: `default_per_sample_iv_size = 8` (every counter IV is
///   an 8-byte big-endian value — see [`CencEncryptor::resolve_iv`]), no
///   constant IV.
/// - [`IvGen::Explicit`]: `default_per_sample_iv_size` is the shared length of
///   every supplied IV (checked uniform here, since the wire format has only
///   one track-wide size — a per-sample length mismatch would otherwise
///   silently desync `senc`'s IV field width from `saiz`'s per-sample aux
///   size), no constant IV. That shared length is also validated here to be
///   exactly 8 or 16 bytes — an empty (or any other length) IV would build an
///   all-zero or malformed AES-CTR/CBC counter (a two-time-pad, in the
///   all-zero case). An empty list falls back to the 8-byte default (there is
///   no sample to measure; [`CencEncryptor::validate_iv_gen`] will itself
///   reject the count mismatch against the `Media`'s real total sample
///   count).
fn tenc_iv_fields(
    iv_gen: &IvGen,
    constant_iv_senc: ConstantIvSenc,
) -> Result<(u8, Option<Vec<u8>>)> {
    match iv_gen {
        IvGen::Constant(iv) => match constant_iv_senc {
            ConstantIvSenc::Emit => Ok((16, Some(iv.to_vec()))),
            ConstantIvSenc::Omit => Ok((0, Some(iv.to_vec()))),
        },
        IvGen::Counter => Ok((PER_SAMPLE_IV_SIZE, None)),
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
                None => PER_SAMPLE_IV_SIZE as usize,
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
    use bytes::Bytes;

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
    /// narrowed to its single AVC video track so the per-scheme cipher tests
    /// below have a deterministic, single-track `Media`. Cross-track IV
    /// uniqueness (the property a single-track `Media` structurally cannot
    /// test) is covered by [`multi_track_media`]'s tests and by
    /// `tests/cenc_encrypt.rs`'s `counter_ivs_are_unique_across_every_track`.
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

    /// A real **two-track** (H.264 video + AAC audio) TS capture with both
    /// tracks kept — the shape that exposes cross-track keystream reuse.
    fn multi_track_media() -> Media {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264_aac.ts");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let mut demux = TsDemux::new();
        let media = demux
            .unpackage(bytes.as_slice())
            .expect("demux fixtures/ts/h264_aac.ts");
        assert!(
            media.tracks.len() > 1,
            "h264_aac.ts must be a multi-track fixture (got {})",
            media.tracks.len()
        );
        media
    }

    fn snapshot(media: &Media) -> Vec<Bytes> {
        media.tracks[0]
            .samples
            .iter()
            .map(|s| s.data.clone())
            .collect()
    }

    /// Every sample's bytes of every track, in (track, sample) order.
    fn snapshot_all(media: &Media) -> Vec<Bytes> {
        media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.clone()))
            .collect()
    }

    /// `n` distinct IVs of `len` bytes (an IV must be unique per content key).
    fn distinct_ivs(n: usize, len: usize) -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| {
                let mut iv = alloc::vec![0xABu8; len];
                iv[len - 1] = i as u8;
                iv[len - 2] = (i >> 8) as u8;
                iv
            })
            .collect()
    }

    #[test]
    fn cenc_round_trip_reverses_byte_identical() {
        let mut media = clear_media();
        let original = snapshot(&media);

        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            iv: IvGen::Counter,
            pattern: None,
            subsample: SubsamplePolicy::Video,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        CencEncryptor::resume(KEY, 7)
            .encrypt(&mut media, &cfg)
            .expect("encrypt");

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
                .any(|(s, o)| s.data != *o),
            "encrypt must change protected bytes"
        );

        for (sample, entry) in track.samples.iter_mut().zip(enc.samples.iter()) {
            cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
                cenc_crypto::apply_ctr(&entry.initialization_vector, &KEY, &entry.subsamples, buf)
            })
            .expect("reverse apply_ctr");
        }
        let reversed: Vec<Bytes> = track.samples.iter().map(|s| s.data.clone()).collect();
        assert_eq!(reversed, original, "cenc round trip must be byte-identical");
    }

    #[test]
    fn cbcs_round_trip_reverses_byte_identical() {
        let mut media = clear_media();
        let original = snapshot(&media);

        let cfg = EncryptConfig {
            scheme: CencScheme::Cbcs,
            kid: KID,
            iv: IvGen::Counter,
            pattern: Some((1, 9)),
            subsample: SubsamplePolicy::Video,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .expect("encrypt");

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
                .any(|(s, o)| s.data != *o),
            "encrypt must change protected bytes"
        );

        for (sample, entry) in track.samples.iter_mut().zip(enc.samples.iter()) {
            cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
                cenc_crypto::cbcs_sample(&enc.tenc, entry, &KEY, buf, CbcsOp::Decrypt)
            })
            .expect("reverse cbcs_sample");
        }
        let reversed: Vec<Bytes> = track.samples.iter().map(|s| s.data.clone()).collect();
        assert_eq!(reversed, original, "cbcs round trip must be byte-identical");
    }

    #[test]
    fn whole_sample_policy_yields_empty_subsample_map() {
        let mut media = clear_media();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            iv: IvGen::default(),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .expect("encrypt");
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
            iv: IvGen::Explicit(alloc::vec![alloc::vec![0u8; 8]; n - 1]),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        let err = CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn explicit_iv_too_long_errors() {
        let mut media = clear_media();
        let n = media.tracks[0].samples.len();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            iv: IvGen::Explicit(alloc::vec![alloc::vec![0u8; 17]; n]),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        let err = CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    /// `IvGen::Explicit` with empty (0-byte) per-sample IVs must error, not
    /// silently build an all-zero AES-CTR counter (a two-time-pad — the same
    /// keystream would be reused for every sample, making the plaintext
    /// trivially recoverable).
    #[test]
    fn explicit_iv_empty_errors() {
        let mut media = clear_media();
        let n = media.tracks[0].samples.len();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            iv: IvGen::Explicit(alloc::vec![alloc::vec![]; n]),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        let err = CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    /// `IvGen::Explicit` with a uniform, but non-8/16-byte, per-sample IV
    /// length must error (only 8 and 16 bytes are valid on the wire —
    /// ISO/IEC 23001-7 §9.2/§12.2).
    #[test]
    fn explicit_iv_wrong_uniform_length_errors() {
        let mut media = clear_media();
        let n = media.tracks[0].samples.len();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            iv: IvGen::Explicit(alloc::vec![alloc::vec![0u8; 12]; n]),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        let err = CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    /// `IvGen::Explicit` accepts both valid per-sample IV lengths — 8 and 16
    /// bytes (ISO/IEC 23001-7 §9.2/§12.2) — recording the matching
    /// `tenc.default_per_sample_iv_size` for each.
    #[test]
    fn explicit_iv_valid_lengths_are_ok() {
        for len in [8usize, 16] {
            let mut media = clear_media();
            let n = media.tracks[0].samples.len();
            let cfg = EncryptConfig {
                scheme: CencScheme::Cenc,
                kid: KID,
                iv: IvGen::Explicit(distinct_ivs(n, len)),
                pattern: None,
                subsample: SubsamplePolicy::WholeSample,
                constant_iv_senc: ConstantIvSenc::default(),
            };
            CencEncryptor::new(KEY)
                .encrypt(&mut media, &cfg)
                .unwrap_or_else(|e| panic!("{len}-byte explicit IV must be accepted: {e:?}"));
            let enc = media.tracks[0].encryption.as_ref().expect("Some");
            assert_eq!(
                enc.tenc.default_per_sample_iv_size, len as u8,
                "tenc.default_per_sample_iv_size must match the actual IV length used"
            );
        }
    }

    /// `cbcs` pattern `crypt_byte_block == 0` with a nonzero
    /// `skip_byte_block` must error — otherwise the whole range is left
    /// silently unprotected while `tenc.default_is_protected` still claims
    /// protection (see `cenc_crypto::cbcs_sample`'s guard).
    #[test]
    fn cbcs_pattern_zero_crypt_nonzero_skip_errors() {
        let mut media = clear_media();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cbcs,
            kid: KID,
            iv: IvGen::Counter,
            pattern: Some((0, 9)),
            subsample: SubsamplePolicy::Video,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        let err = CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    /// A `cbcs` pattern component above 15 must error rather than silently
    /// truncate to its low 4 bits when packed into `tenc` (ISO/IEC 23001-7
    /// §12.2: `(crypt_byte_block << 4) | skip_byte_block`) — e.g. `(17, 9)`
    /// would otherwise silently become `(1, 9)` on the wire.
    #[test]
    fn cbcs_pattern_component_too_large_errors() {
        let mut media = clear_media();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cbcs,
            kid: KID,
            iv: IvGen::Counter,
            pattern: Some((17, 9)),
            subsample: SubsamplePolicy::Video,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        let err = CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    /// **Multi-track byte-exact round trip, both schemes.** Encrypt a real
    /// two-track (video + audio) `Media` with one `EncryptConfig`, then reverse
    /// every track's samples with the shared cipher core using that track's own
    /// recorded `tenc`/`senc` metadata, and require every sample of every track
    /// back byte-identical.
    ///
    /// The single-track round trips above cannot catch a cross-track
    /// bookkeeping error (wrong IV attributed to the wrong track's samples);
    /// this one can, because the per-sample IVs now differ *between* tracks as
    /// well as within them.
    #[test]
    fn multi_track_round_trip_reverses_byte_identical_both_schemes() {
        for scheme in [CencScheme::Cenc, CencScheme::Cbcs] {
            let mut media = multi_track_media();
            let original = snapshot_all(&media);
            let cfg = EncryptConfig {
                scheme,
                kid: KID,
                iv: IvGen::Counter,
                pattern: if scheme == CencScheme::Cbcs {
                    Some((1, 9))
                } else {
                    None
                },
                subsample: SubsamplePolicy::Video,
                constant_iv_senc: ConstantIvSenc::default(),
            };
            CencEncryptor::resume(KEY, 3)
                .encrypt(&mut media, &cfg)
                .unwrap_or_else(|e| panic!("{}: encrypt: {e:?}", scheme.name()));
            assert_ne!(
                snapshot_all(&media),
                original,
                "{}: encrypt must change protected bytes",
                scheme.name()
            );

            for track in &mut media.tracks {
                let enc = track.encryption.clone().expect("track.encryption Some");
                assert_eq!(enc.samples.len(), track.samples.len());
                for (sample, entry) in track.samples.iter_mut().zip(enc.samples.iter()) {
                    cenc_crypto::rewrite_in_place(&mut sample.data, |buf| match scheme {
                        CencScheme::Cenc => cenc_crypto::apply_ctr(
                            &entry.initialization_vector,
                            &KEY,
                            &entry.subsamples,
                            buf,
                        ),
                        CencScheme::Cbcs => {
                            cenc_crypto::cbcs_sample(&enc.tenc, entry, &KEY, buf, CbcsOp::Decrypt)
                        }
                        // This test drives exactly the two schemes above; a
                        // third reaching here means the caller list changed
                        // without the reverse-cipher being taught about it.
                        other => panic!("test drives only cenc/cbcs, got {other}"),
                    })
                    .unwrap_or_else(|e| panic!("{}: reverse: {e:?}", scheme.name()));
                }
            }
            assert_eq!(
                snapshot_all(&media),
                original,
                "{}: multi-track round trip must be byte-identical",
                scheme.name()
            );
        }
    }

    /// The IV counter must not restart per track: the last IV of track 0 and
    /// the first IV of track 1 must be consecutive, and the whole `Media`'s
    /// IVs must be distinct. (`tests/cenc_encrypt.rs` asserts the same
    /// uniqueness property through the public API; this pins the *continuity*
    /// of the counter, which is what makes the uniqueness hold.)
    #[test]
    fn counter_iv_runs_continuously_across_tracks() {
        const BASE: u64 = 0x0102_0304_0506_0708;
        let mut media = multi_track_media();
        let first_track_len = media.tracks[0].samples.len();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            iv: IvGen::Counter,
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        CencEncryptor::resume(KEY, BASE)
            .encrypt(&mut media, &cfg)
            .expect("encrypt");

        let ivs: Vec<Vec<u8>> = media
            .tracks
            .iter()
            .flat_map(|t| {
                t.encryption
                    .as_ref()
                    .expect("Some")
                    .samples
                    .iter()
                    .map(|e| e.initialization_vector.clone())
            })
            .collect();
        for (i, iv) in ivs.iter().enumerate() {
            assert_eq!(
                iv.as_slice(),
                &(BASE + i as u64).to_be_bytes()[..],
                "IV {i} (Media-wide index) must be base + i"
            );
        }
        // The specific boundary the original defect got wrong.
        assert_eq!(
            ivs[first_track_len].as_slice(),
            &(BASE + first_track_len as u64).to_be_bytes()[..],
            "track 1's first IV must continue track 0's counter, not restart at base"
        );
    }

    /// `IvGen::Constant` + `cenc` is rejected, and the `Media` is left
    /// untouched (no track partially encrypted, no metadata recorded).
    #[test]
    fn constant_iv_under_cenc_errors_and_leaves_media_untouched() {
        let mut media = multi_track_media();
        let original = snapshot_all(&media);
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            iv: IvGen::Constant([0x5Au8; KEY_LEN]),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        let err = CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(snapshot_all(&media), original, "no sample may be ciphered");
        assert!(media.tracks.iter().all(|t| t.encryption.is_none()));
    }

    /// A rejected `IvGen::Explicit` list (wrong total count) must be caught
    /// *before* any cipher work, leaving the `Media` byte-identical — not with
    /// track 0 encrypted and track 1 clear.
    #[test]
    fn rejected_explicit_iv_list_leaves_media_untouched() {
        let mut media = multi_track_media();
        let original = snapshot_all(&media);
        let first_track_len = media.tracks[0].samples.len();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            // The old per-track meaning: one IV per sample of track 0 only.
            iv: IvGen::Explicit(distinct_ivs(first_track_len, 8)),
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        let err = CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(snapshot_all(&media), original, "no sample may be ciphered");
        assert!(media.tracks.iter().all(|t| t.encryption.is_none()));
    }

    /// The pre-cipher plan-uniqueness backstop bites: a **planned** IV
    /// sequence (see [`CencEncryptor::plan_sample_ivs`]) that repeats an IV
    /// across tracks is rejected by [`assert_ivs_unique`] — the exact shape a
    /// reintroduced per-track index reset (the original defect) would
    /// produce. Built by hand (the only way to reach it now that generation
    /// is correct) to prove the guard is real rather than dead code; it is
    /// what makes a future reintroduction of that defect fail loudly, and —
    /// because it runs on the plan *before* any cipher work — fail before
    /// `media` is ever touched (see
    /// [`planned_ivs_are_validated_before_any_sample_is_touched`] for the
    /// end-to-end proof of that ordering).
    #[test]
    fn planned_duplicate_ivs_are_rejected_before_ciphering() {
        // Two tracks; track 1's plan replays track 0's first IV instead of
        // continuing from where track 0 left off — exactly what a per-track
        // counter reset produces.
        let dup = alloc::vec![0u8, 0, 0, 0, 0, 0, 0, 1];
        let plan: Vec<Vec<Vec<u8>>> = alloc::vec![
            alloc::vec![dup.clone(), alloc::vec![0u8, 0, 0, 0, 0, 0, 0, 2]],
            alloc::vec![dup],
        ];
        let err = assert_ivs_unique(&plan, &IvGen::Counter, ConstantIvSenc::Omit).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));

        // A correctly-generated (strictly increasing, Media-wide) plan must
        // pass unchanged.
        let good_plan: Vec<Vec<Vec<u8>>> = alloc::vec![
            alloc::vec![
                alloc::vec![0u8, 0, 0, 0, 0, 0, 0, 0],
                alloc::vec![0u8, 0, 0, 0, 0, 0, 0, 1],
            ],
            alloc::vec![alloc::vec![0u8, 0, 0, 0, 0, 0, 0, 2]],
        ];
        assert!(
            assert_ivs_unique(&good_plan, &IvGen::Counter, ConstantIvSenc::Omit).is_ok(),
            "a correctly Media-wide-continuous plan must pass the backstop"
        );
    }

    /// **Regression #783**: `cenc` + `IvGen::Counter` + `ConstantIvSenc::Emit`
    /// must NOT short-circuit the duplicate-IV check.  The `Emit` variant
    /// only makes semantic sense with `cbcs`+`Constant`; a duplicate IV on
    /// a `cenc`/`Counter` path (even with `Emit` set) is still a two-time
    /// pad and must be caught by this backstop.
    #[test]
    fn emit_does_not_short_circuit_duplicate_check_for_cenc_counter() {
        let dup = alloc::vec![0u8; 8];
        // Two identical IVs across tracks — same shape as
        // `planned_duplicate_ivs_are_rejected_before_ciphering`.
        let plan: Vec<Vec<Vec<u8>>> = alloc::vec![alloc::vec![dup.clone()], alloc::vec![dup],];
        let err = assert_ivs_unique(&plan, &IvGen::Counter, ConstantIvSenc::Emit)
            .expect_err("duplicate IV on cenc+Counter+Emit must be rejected");
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    /// **F1 regression, end to end**: a `Media` whose `encrypt()` call is
    /// rejected (a wrong `IvGen::Explicit` count, one of the plan-validation
    /// failure modes) leaves every sample byte-identical to the input — no
    /// track partially keystreamed. This exercises the real
    /// [`Encrypt::encrypt`] entry point (not [`assert_ivs_unique`] in
    /// isolation, which [`planned_duplicate_ivs_are_rejected_before_ciphering`]
    /// covers), so it pins the property the whole plan-then-cipher restructure
    /// exists for: planning and validating happen entirely before the cipher
    /// loop, so a rejected plan can never leave a half-encrypted `Media`
    /// behind. See also [`rejected_explicit_iv_list_leaves_media_untouched`]
    /// and [`constant_iv_under_cenc_errors_and_leaves_media_untouched`] above,
    /// which pin the same property for the other two rejection paths
    /// ([`IvGen::Explicit`]'s per-track-sized list and [`IvGen::Constant`]
    /// under `cenc`).
    #[test]
    fn planned_ivs_are_validated_before_any_sample_is_touched() {
        let mut media = multi_track_media();
        let original = snapshot_all(&media);
        let n = media.tracks[0].samples.len();
        assert!(n > 1, "fixture must have more than one sample to bite");
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            iv: IvGen::Explicit(distinct_ivs(n - 1, 8)), // wrong total count
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };
        let err = CencEncryptor::new(KEY)
            .encrypt(&mut media, &cfg)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(
            snapshot_all(&media),
            original,
            "a rejected plan must leave every track's samples byte-identical"
        );
        assert!(media.tracks.iter().all(|t| t.encryption.is_none()));
    }

    /// **F2 regression test.** Two successive `encrypt()` calls on the SAME
    /// [`CencEncryptor`] instance (hence the same content key) — the shape a
    /// video-only + audio-only split of one asset, or successive live
    /// segments under one key period, actually produces — must never repeat
    /// a per-sample IV. Before this fix, [`IvGen::Counter`]'s `base` lived in
    /// `EncryptConfig` and was always whatever the caller passed (`0` via
    /// [`IvGen::default`]), so two calls sharing a key and using the default
    /// reproduced the exact two-time pad this type exists to prevent within
    /// one call. Now the running counter lives on the [`CencEncryptor`]
    /// instance and advances after each call, so reusing the instance
    /// continues instead of restarting.
    #[test]
    fn successive_encrypt_calls_on_one_instance_produce_disjoint_ivs() {
        let cfg = EncryptConfig {
            scheme: CencScheme::Cenc,
            kid: KID,
            iv: IvGen::Counter,
            pattern: None,
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc: ConstantIvSenc::default(),
        };

        let mut enc = CencEncryptor::new(KEY);

        let mut media_a = clear_media();
        enc.encrypt(&mut media_a, &cfg).expect("first encrypt");
        let ivs_a: alloc::collections::BTreeSet<Vec<u8>> = media_a.tracks[0]
            .encryption
            .as_ref()
            .expect("Some")
            .samples
            .iter()
            .map(|e| e.initialization_vector.clone())
            .collect();

        let mut media_b = clear_media();
        enc.encrypt(&mut media_b, &cfg)
            .expect("second encrypt, same instance/key");
        let ivs_b: alloc::collections::BTreeSet<Vec<u8>> = media_b.tracks[0]
            .encryption
            .as_ref()
            .expect("Some")
            .samples
            .iter()
            .map(|e| e.initialization_vector.clone())
            .collect();

        assert!(
            ivs_a.is_disjoint(&ivs_b),
            "two encrypt() calls sharing one CencEncryptor (hence one key) must never reuse an \
             IV — a shared IV under one key is a two-time pad (ISO/IEC 23001-7 §9.2)"
        );

        // The documented residual: a FRESH `CencEncryptor::new(KEY)` (the
        // pre-fix shape every call used to be, via `IvGen::default()`
        // restarting at `base: 0`) reproduces media_a's exact IV set —
        // proving the fix is "reuse one instance per key", not some property
        // of the fixture, and that constructing a second fresh instance for
        // a key already in use is a caller error this type documents but
        // cannot detect.
        let mut media_c = clear_media();
        CencEncryptor::new(KEY)
            .encrypt(&mut media_c, &cfg)
            .expect("fresh instance, third encrypt");
        let ivs_c: alloc::collections::BTreeSet<Vec<u8>> = media_c.tracks[0]
            .encryption
            .as_ref()
            .expect("Some")
            .samples
            .iter()
            .map(|e| e.initialization_vector.clone())
            .collect();
        assert_eq!(
            ivs_a, ivs_c,
            "a fresh CencEncryptor::new(KEY) must reproduce media_a's exact IV set: \
             CencEncryptor::new always starts the counter at 0, so reusing the SAME key across \
             SEPARATE fresh instances is the one collision this type cannot structurally prevent \
             — documented as the caller's obligation"
        );
    }

    /// **The provably-taken fast path (media plane step 2b, G12)**: every
    /// sample straight out of [`clear_media`]'s fresh `TsDemux` — the shape
    /// `CencEncryptor::encrypt` actually sees in real use, before any sample
    /// has been fanned out to another consumer — is uniquely owned, so
    /// [`cenc_crypto::rewrite_in_place`] must take the zero-copy
    /// `try_into_mut` branch for every one of them. Complements the
    /// mechanism-level proof in `cenc_crypto`'s own tests (which also proves
    /// the shared/fallback branch) and the whole-pipeline allocation count in
    /// `tests/alloc_measurement.rs`.
    #[test]
    fn real_fixture_samples_take_the_zero_copy_fast_path() {
        let mut media = clear_media();
        let track = &mut media.tracks[0];
        assert!(track.samples.len() > 1, "fixture must carry samples");
        for sample in &mut track.samples {
            let took_fast_path =
                cenc_crypto::rewrite_in_place(&mut sample.data, |_buf| Ok(())).expect("no-op ok");
            assert!(
                took_fast_path,
                "a freshly-demuxed, not-yet-fanned-out sample must take the zero-copy path"
            );
        }
    }

    /// The inverse: once a sample has been fanned out (cloned to a second
    /// consumer — a refcount bump, per the whole point of switching to
    /// `Bytes`), a subsequent in-place rewrite of the ORIGINAL must fall back
    /// to a copy rather than mutate bytes the other consumer still holds.
    #[test]
    fn fanned_out_sample_forces_the_copy_fallback() {
        let mut media = clear_media();
        let sample = &mut media.tracks[0].samples[0];
        let fanned_out_consumer = sample.data.clone(); // refcount 2
        let took_fast_path = cenc_crypto::rewrite_in_place(&mut sample.data, |buf| {
            buf[0] ^= 0xFF;
            Ok(())
        })
        .expect("rewrite ok");
        assert!(
            !took_fast_path,
            "a sample already fanned out to another consumer must not take the fast path"
        );
        assert_ne!(
            sample.data[0], fanned_out_consumer[0],
            "the rewritten handle's first byte must differ from the untouched consumer's"
        );
    }
}
