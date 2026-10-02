//! CENC decrypt — unprotect a Common-Encryption fMP4 (ISO/IEC 23001-7).
//!
//! Turns a CENC-encrypted ISOBMFF/CMAF file back into cleartext coded samples,
//! implementing the hub [`broadcast_common::Decrypt`] trait. Only the box
//! *parsers* (in [`crate::cenc`]) are reused here; this module adds the
//! `sinf`/`frma` unwrap and dispatches AES sample-decryption (both ciphers) to
//! the shared cipher core in `cenc_crypto` (factored out so an
//! encrypt path can reuse it — see that module's docs for the `cbcs` CBC
//! chain-reset rule).
//!
//! # Container support
//!
//! Both ISOBMFF layouts are supported:
//!
//! - **Progressive** (single `moov`/`mdat`, e.g. ffmpeg's `-cenc_aes_ctr`
//!   output): sample layout comes from `stsz`/`stsc`/`stco` inside `stbl`, and
//!   the per-sample IV/subsample map comes from a single `senc` also inside
//!   `stbl`.
//! - **Fragmented CMAF** (`moov` + one or more `moof`/`mdat` pairs, the
//!   real-world case): the `moov` still carries the track's crypto *defaults*
//!   (`sinf`/`tenc`), but each `traf` inside a `moof` carries its OWN `senc`
//!   (per-fragment per-sample IV/subsample map) and `trun` (per-sample sizes,
//!   resolved against the `mdat` via the `trun`/`tfhd` `default-base-is-moof`
//!   convention). Every fragment's samples are concatenated in file order into
//!   one [`crate::media::Track`], exactly like the progressive case — see
//!   this module's private `harvest_fragment_senc` and
//!   `collect_fragment_samples` helpers. The already-typed
//!   fragment parsers in [`crate::movie_fragment`] (`MovieFragmentBox`,
//!   `TrackFragmentHeaderBox`, `TrackFragmentRunBox`) are reused rather than a
//!   second hand-rolled `moof`/`traf`/`trun` walker; only the `senc` lookup
//!   (which those types do not carry) is done with this module's own
//!   box-navigation helpers.
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
//! - **AES-CTR (`cenc`) mode**: ISO/IEC 23001-7 §10.1 — the 16-byte counter is
//!   the per-sample IV (8- or 16-byte, left-justified and zero-padded to 16)
//!   with the low 64 bits acting as the AES block counter, incrementing once per
//!   16-byte cipher block across the concatenated *protected* bytes of a sample
//!   (the clear subsample ranges are skipped, not counted).
//! - **AES-CBC pattern (`cbcs`) mode**: ISO/IEC 23001-7 §10.2 — *within* one
//!   subsample's protected range (or the whole sample, when there is no
//!   subsample map), `default_crypt_byte_block` 16-byte blocks are
//!   CBC-decrypted, then `default_skip_byte_block` 16-byte blocks are passed
//!   through clear, repeating across that range; a final partial block
//!   (`< 16` bytes remaining in a crypt run) is left clear. The IV — the
//!   `tenc` version-1 `default_constant_IV` when
//!   `default_Per_Sample_IV_Size == 0`, otherwise the per-sample IV from
//!   `senc` — seeds the *first* encrypted block of *every* subsample's
//!   protected range (the chain resets at each subsample boundary, it does
//!   not carry over); within one subsample's range the chain then continues
//!   seamlessly from each encrypted block's ciphertext to the next, skip
//!   bytes never entering the chain. `cenc`'s CTR counter, by contrast, does
//!   advance continuously across the whole sample regardless of subsample
//!   boundaries — the two ciphers differ here. This `cbcs` chain-reset rule
//!   was triangulated against Bento4's `mp4decrypt` and Shaka Packager (ISO/IEC
//!   23001-7 itself is not owned by this project, so the reference
//!   implementations are the source of truth) — see `cenc_crypto`'s module
//!   docs for the full derivation, including the earlier
//!   cross-subsample-continuous version's divergence from Bento4.
//! - **`sinf`/`frma` unwrap**: ISO/IEC 14496-12:2015 §8.12 — after decryption the
//!   track's coded data is in the original (`frma`) format.
//! - **Movie fragments** (`moof`/`traf`/`tfhd`/`trun`): ISO/IEC 14496-12:2015 §8.8.
//!
//! No AES is rolled by hand: `cenc_crypto` wraps the RustCrypto
//! [`aes`], [`ctr`], and [`cbc`] crates for the block cipher and mode work.
//! This module is gated on the `cenc` feature.

use core::fmt;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use broadcast_common::{Decrypt, Parse};

use crate::box_types::{BOX_HEADER_MIN_SIZE, parse_box};
// Re-exported (not just `use`d) so `transmux::cenc_decrypt::CencScheme` keeps
// resolving for existing callers even though the type now lives in
// `crate::cenc` (issue #564 — one shared definition for decrypt/encrypt/IR).
pub use crate::cenc::CencScheme;
use crate::cenc::{SampleEncryptionEntry, TrackEncryptionBox};
use crate::cenc_crypto::{self, CbcsOp};
use crate::error::{Error, Result};
use crate::media::Media;
use crate::movie_fragment::{
    MovieFragmentBox, SAMPLE_FLAG_IS_NON_SYNC, TrackFragmentHeaderBox, TrackFragmentRunBox,
};
use crate::sample_groups::{GROUPING_TYPE_SEIG, SampleGroupDescriptionBox};

/// Size of a KID / content key / AES-128 key **or block**, in bytes (AES-128's
/// key length and block length coincide).
const KEY_LEN: usize = 16;

/// A map of content keys, keyed by 16-byte Key ID (KID).
///
/// The [`Decrypt::Keys`] material for [`CencDecryptor`]: each protected sample's
/// KID (from `tenc.default_kid`) selects a 16-byte AES-128 content key.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct KeyMap {
    keys: BTreeMap<[u8; KEY_LEN], [u8; KEY_LEN]>,
}

/// Manual `Debug`: lists the (non-secret) KIDs this map holds, never the
/// content key bytes paired with them (a derived `Debug` over the
/// `BTreeMap<kid, key>` would print both — a `tracing::debug!`/`dbg!`/panic
/// message of a value holding a `KeyMap` would then write live content keys
/// to logs).
impl fmt::Debug for KeyMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyMap")
            .field("kids", &self.keys.keys().collect::<Vec<_>>())
            .finish()
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

/// Per-track CENC crypto metadata recovered from a protected fMP4.
#[derive(Debug, Clone)]
struct TrackCrypto {
    /// The track's real `tkhd.track_id` (used to match this track's samples in
    /// [`crate::media::Media`], and to match `moof`/`traf` fragments by
    /// `tfhd.track_id` in [`harvest_fragment_senc`]).
    track_id: u32,
    /// The `tenc` defaults (KID, IV size, protection flag, and — for `cbcs` —
    /// the pattern's `crypt`:`skip` block counts and optional constant IV).
    tenc: TrackEncryptionBox,
    /// The original (unprotected) codec four-CC from `frma`.
    original_format: [u8; 4],
    /// The protection scheme from `schm`.
    scheme: CencScheme,
    /// Per-sample encryption info (IV + subsample map), in decode order.
    ///
    /// For a progressive file this is the single `stbl`-level `senc`'s
    /// entries; for a fragmented file this is every `moof`'s `traf`-level
    /// `senc` entries, concatenated in file (fragment) order — see
    /// [`harvest_fragment_senc`].
    samples: Vec<SampleEncryptionEntry>,
}

/// Decrypts CENC-protected samples of a [`Media`] using a [`KeyMap`].
///
/// Construct one from the protected file's bytes with [`CencDecryptor::from_fmp4`]
/// (which harvests the `tenc`/`senc`/`sinf` crypto metadata), then either
/// [`demux`](CencDecryptor::demux) the encrypted samples into a [`Media`] or,
/// if you already have a [`Media`] of the encrypted samples, call
/// [`Decrypt::decrypt`] directly. The decryptor matches each track's samples to
/// the recovered per-sample IV + subsample map by decode-order index.
#[derive(Clone)]
pub struct CencDecryptor {
    /// The whole protected fMP4 file (borrowing is avoided so the decryptor is
    /// `'static`-friendly for the trait impl; a `Vec` copy is acceptable here).
    file: bytes::Bytes,
    /// Per-track crypto metadata, in `moov` track order.
    tracks: Vec<TrackCrypto>,
}

/// Manual `Debug`: prints the file's length rather than its bytes (this type
/// never holds a content key — those are supplied out of band to
/// [`Decrypt::decrypt`] as a [`KeyMap`] — but a derived `Debug` would still
/// dump the whole protected file, megabytes of ciphertext, into any
/// `tracing`/`dbg!`/panic message of a value holding one).
impl fmt::Debug for CencDecryptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CencDecryptor")
            .field("file_len", &self.file.len())
            .field("tracks", &self.tracks)
            .finish()
    }
}

impl CencDecryptor {
    /// Build a decryptor by harvesting CENC metadata from a protected fMP4.
    ///
    /// Parses each track's `sinf` (`frma` + `schm` + `schi/tenc`) and its
    /// per-sample IV/subsample map (`senc` — a single `stbl`-level box for a
    /// progressive file, or every `moof`'s `traf`-level box, concatenated, for
    /// a fragmented one). Fails with [`Error::UnexpectedBox`] if no protected
    /// track is found.
    ///
    /// Copies `file` once to own it; a caller that already holds a
    /// [`bytes::Bytes`] should use [`Self::from_fmp4_bytes`], which shares the
    /// buffer instead (and lets [`Self::demux`] hand out samples as zero-copy
    /// slices of it).
    pub fn from_fmp4(file: &[u8]) -> Result<Self> {
        Self::from_fmp4_bytes(bytes::Bytes::copy_from_slice(file))
    }

    /// [`Self::from_fmp4`] over an already-shared buffer: no copy of the file,
    /// and every sample [`Self::demux`] returns is a slice of it (audit r05-O2).
    pub fn from_fmp4_bytes(file: bytes::Bytes) -> Result<Self> {
        let mut tracks = Vec::new();
        harvest_tracks(&file, &mut tracks)?;
        if tracks.is_empty() {
            return Err(Error::UnexpectedBox {
                expected: "a protected track (sinf/tenc + senc)",
            });
        }
        Ok(Self { file, tracks })
    }

    /// The original (unprotected) codec four-CC of the first protected track,
    /// from its `frma` box (e.g. `*b"avc1"`).
    pub fn original_format(&self) -> [u8; 4] {
        self.tracks
            .first()
            .map(|t| t.original_format)
            .unwrap_or(*b"\0\0\0\0")
    }

    /// The protection scheme of the first protected track (`cenc`/`cbcs`).
    pub fn scheme(&self) -> Option<CencScheme> {
        self.tracks.first().map(|t| t.scheme)
    }

    /// The `tenc` (default KID / IV size) of the first protected track.
    pub fn track_encryption(&self) -> Option<&TrackEncryptionBox> {
        self.tracks.first().map(|t| &t.tenc)
    }

    /// The per-sample encryption entries (IV + subsamples) of the first
    /// protected track, in decode order.
    pub fn sample_entries(&self) -> &[SampleEncryptionEntry] {
        self.tracks
            .first()
            .map(|t| t.samples.as_slice())
            .unwrap_or(&[])
    }

    /// Demux the protected fMP4 into a [`Media`] carrying the *encrypted* coded
    /// samples in decode order, one [`crate::media::Track`] per protected track.
    ///
    /// The returned samples are still encrypted; pass the [`Media`] to
    /// [`Decrypt::decrypt`] with the content keys to obtain cleartext. Works for
    /// both progressive and fragmented sources — see the module docs.
    ///
    /// Only protected **AVC** tracks are reconstructed: a protected track whose
    /// original format is not `avc1` (a CENC asset's `enca` audio leg, or a
    /// protected HEVC video) is **skipped** so the rest still demuxes (audit
    /// r05-W6), as are unprotected tracks. Each skipped **protected** track is
    /// recorded in [`Media::skipped`](crate::ir::Media::skipped) (its original
    /// format's FourCC and the reason), as is an **unprotected** track — so no
    /// drop is silent; a `Media`
    /// narrowed to a skipped track fails in [`Decrypt::decrypt`] with "no
    /// protected-source track matches this media track's track_id" rather than
    /// silently passing ciphertext through.
    pub fn demux(&self) -> Result<Media> {
        demux_protected(&self.file)
    }

    /// Decrypt one sample's bytes in place, dispatching on the track's scheme.
    ///
    /// Delegates to the shared cipher core in `cenc_crypto`: `cenc`
    /// (AES-CTR, ISO/IEC 23001-7 §10.1) via `cenc_crypto::apply_ctr` — the
    /// counter runs continuously across subsample boundaries; `cbcs`
    /// (AES-CBC pattern, ISO/IEC 23001-7 §10.2) via
    /// `cenc_crypto::cbcs_sample` with `CbcsOp::Decrypt` — the CBC chain
    /// instead *resets* to the sample's seed IV at the start of every
    /// subsample's protected range (see `cenc_crypto`'s module docs).
    ///
    /// Returns whether [`cenc_crypto::rewrite_in_place`]'s zero-copy fast path
    /// was taken (media plane step 2b, G12) — see that function's docs.
    fn decrypt_sample(
        scheme: CencScheme,
        tenc: &TrackEncryptionBox,
        entry: &SampleEncryptionEntry,
        key: &[u8; KEY_LEN],
        data: &mut bytes::Bytes,
    ) -> Result<bool> {
        cenc_crypto::rewrite_in_place(data, |buf| match scheme {
            CencScheme::Cenc => {
                cenc_crypto::apply_ctr(&entry.initialization_vector, key, &entry.subsamples, buf)
            }
            CencScheme::Cbcs => cenc_crypto::cbcs_sample(tenc, entry, key, buf, CbcsOp::Decrypt),
            // `CencScheme` is `#[non_exhaustive]` and now lives in
            // `broadcast-common`, so this arm is reachable if a future scheme
            // (`cens`/`cbc1`) is added there before the cipher for it lands
            // here. Reject rather than guess: picking the wrong cipher would
            // silently emit garbage plaintext.
            other => Err(Error::UnsupportedCencScheme { scheme: other }),
        })
    }
}

impl Decrypt for CencDecryptor {
    type Media = Media;
    type Keys = KeyMap;
    type Error = Error;

    /// Decrypt every protected track of `media` in place.
    ///
    /// Each media track is paired with its recovered crypto record **by
    /// `track_id`** ([`crate::pipeline::TrackSpec::track_id`] against the
    /// `tkhd.track_id` harvested from the protected source) — never by
    /// position. Positional pairing silently
    /// mis-decrypts whenever the `Media`'s track order or membership differs
    /// from the source's `moov` order: a [`Media::select_tracks_by`]-narrowed
    /// `Media` holding only the audio track would be decrypted with the
    /// *video* track's IVs, and if the two tracks' sample counts happened to
    /// coincide the count check below would not catch it either — it would
    /// just return `Ok` over garbage.
    ///
    /// A media track with no matching record is an error (the caller asked for
    /// a track this decryptor has no crypto metadata for); unmatched *records*
    /// are fine — that is exactly the narrowed-`Media` case.
    ///
    /// # Atomicity
    ///
    /// Every track is validated in full **before any sample is decrypted**
    /// (audit r05-W5): the record match, `tenc.default_is_protected`, the
    /// content key's presence, the sample count, and every sample's
    /// content-dependent precondition (IV length, subsample-map coverage,
    /// `cbcs` pattern) are checked in a first pass over the whole `Media`.
    /// Without it, a missing key on track 2 or a malformed subsample map on
    /// sample `k` returned `Err` after track 1 (or samples `0..k`) had already
    /// been decrypted in place — the caller could not tell which samples were
    /// now plaintext, and retrying under CTR XORs those samples *back* to
    /// ciphertext. A rejected call now leaves `media` byte-identical.
    fn decrypt(&self, media: &mut Media, keys: &KeyMap) -> Result<()> {
        // ── Pass 1: prove the whole `Media` decryptable, mutate nothing ──
        for track in media.tracks.iter() {
            let crypto = self
                .tracks
                .iter()
                .find(|c| c.track_id == track.spec.track_id)
                .ok_or(Error::InvalidInput(
                    "no protected-source track matches this media track's track_id",
                ))?;
            if crypto.tenc.default_is_protected == 0 {
                // Track is not protected — nothing to do.
                continue;
            }
            if keys.get(&crypto.tenc.default_kid).is_none() {
                return Err(Error::InvalidInput(
                    "no content key for the track's default_KID",
                ));
            }
            if track.samples.len() != crypto.samples.len() {
                return Err(Error::InvalidInput(
                    "sample count mismatch between media and senc",
                ));
            }
            for (sample, entry) in track.samples.iter().zip(crypto.samples.iter()) {
                crate::cenc_crypto::validate_sample_decrypt(
                    crypto.scheme,
                    &crypto.tenc,
                    entry,
                    sample.data.len(),
                )?;
            }
        }

        // ── Pass 2: decrypt. Pass 1 proved every precondition the cipher
        // checks, and the same plans (IVs, subsample maps) are consumed
        // unmodified — but nothing here is `expect`ed: a panic on a path
        // driven by untrusted input is never acceptable, so the same lookups
        // are repeated as `Err` (they cannot fire, but if the two passes ever
        // drifted the result is an error, not an abort). ──
        for track in media.tracks.iter_mut() {
            let crypto = self
                .tracks
                .iter()
                .find(|c| c.track_id == track.spec.track_id)
                .ok_or(Error::InvalidInput(
                    "no protected-source track matches this media track's track_id",
                ))?;
            if crypto.tenc.default_is_protected == 0 {
                continue;
            }
            let key = keys
                .get(&crypto.tenc.default_kid)
                .ok_or(Error::InvalidInput(
                    "no content key for the track's default_KID",
                ))?;
            if track.samples.len() != crypto.samples.len() {
                return Err(Error::InvalidInput(
                    "sample count mismatch between media and senc",
                ));
            }
            for (sample, entry) in track.samples.iter_mut().zip(crypto.samples.iter()) {
                CencDecryptor::decrypt_sample(
                    crypto.scheme,
                    &crypto.tenc,
                    entry,
                    key,
                    &mut sample.data,
                )?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// fMP4 harvesting: recover per-track crypto metadata + encrypted samples.
// ---------------------------------------------------------------------------

/// Full-box header size (`version` + `flags`).
const FULL_HDR: usize = 4;
/// `stsd` fixed header after the FullBox: `entry_count`.
const STSD_ENTRY_COUNT: usize = 4;
/// A `VisualSampleEntry` fixed body length before its child boxes
/// (ISO/IEC 14496-12 §12.1.3): 78 bytes — 6 reserved, 2 data_ref, 16
/// predefined/reserved, 2 width, 2 height, 4 hres, 4 vres, 4 reserved, 2
/// frame_count, 32 compressorname, 2 depth, 2 predefined.
const VISUAL_SAMPLE_ENTRY_HDR: usize = 78;

/// Recover crypto metadata for every protected track in `file`.
///
/// Works for both progressive (single `moov`/`mdat`) and fragmented
/// (`moov` + `moof`/`mdat`*) sources: [`harvest_track`] recovers each track's
/// `sinf`/`tenc` defaults (and, for a progressive file, its single `senc`);
/// for a fragmented file [`harvest_fragment_senc`] walks every `moof` and
/// appends each `traf`'s `senc` entries, in file order.
fn harvest_tracks(file: &[u8], out: &mut Vec<TrackCrypto>) -> Result<()> {
    let moov = find_top_box(file, b"moov").ok_or(Error::UnexpectedBox { expected: "moov" })?;
    let fragmented = find_top_box(file, b"moof").is_some();
    for trak in iter_child_boxes(moov, b"trak") {
        if let Some(crypto) = harvest_track(trak, fragmented)? {
            out.push(crypto);
        }
    }
    if fragmented {
        harvest_fragment_senc(file, out)?;
    }
    Ok(())
}

/// Recover one track's crypto metadata, if it is protected.
///
/// `fragmented` selects where the per-sample `senc` lives: `false` reads the
/// single `stbl`-level `senc` (progressive fMP4, unchanged from before);
/// `true` leaves `samples` empty — the caller ([`harvest_tracks`]) fills it in
/// afterwards from every `moof`'s `traf`-level `senc` via
/// [`harvest_fragment_senc`], since a fragmented file carries no `senc` in
/// `stbl` at all.
fn harvest_track(trak: &[u8], fragmented: bool) -> Result<Option<TrackCrypto>> {
    // Navigate trak → mdia → minf → stbl.
    let Some(stbl) = descend(trak, &[b"mdia", b"minf", b"stbl"]) else {
        return Ok(None);
    };

    // sinf lives inside the protected sample entry (encv/enca) under stsd.
    let Some(stsd) = find_box(stbl, b"stsd") else {
        return Ok(None);
    };
    let Some(sinf) = find_sinf_in_stsd(stsd) else {
        return Ok(None);
    };
    let sinf_parsed = crate::cenc::ProtectionSchemeInfoBox::parse(sinf)?;
    let scheme = sinf_parsed
        .scheme_type
        .as_ref()
        .and_then(|s| CencScheme::from_four_cc(&s.scheme_type))
        .ok_or(Error::InvalidInput(
            "sinf missing or unknown schm scheme_type",
        ))?;
    let tenc = sinf_parsed
        .scheme_info
        .as_ref()
        .and_then(|si| si.tenc.clone())
        .ok_or(Error::UnexpectedBox {
            expected: "tenc inside schi",
        })?;
    let original_format = sinf_parsed.original_format.data_format;

    // A `seig` ('CencSampleEncryptionInformationGroupEntry') sample group in
    // `stbl` means some run of this track's samples decrypts under a KID/IV
    // override, not the `tenc` default read above — key rotation within a
    // single track. This decryptor only ever decrypts with the track-wide
    // `tenc.default_KID`, so silently proceeding here would decrypt every
    // group-overridden sample with the wrong key and produce plausible but
    // wrong plaintext, with no structural signal anything went wrong. Reject
    // explicitly instead (issue #990) until seig is actually implemented.
    if container_has_seig_sgpd(stbl)? {
        return Err(Error::UnsupportedFeature(
            "seig sample-group key rotation not implemented",
        ));
    }

    let tkhd = find_box(trak, b"tkhd").ok_or(Error::UnexpectedBox { expected: "tkhd" })?;
    let track_id = crate::init_segment::TrackHeaderBox::parse(tkhd)?.track_id;

    let samples = if fragmented {
        // Filled in later by `harvest_fragment_senc`, once every `moof` has
        // been walked (each `traf`'s `senc` covers only that fragment).
        Vec::new()
    } else {
        // Progressive fMP4: a single senc lives inside stbl, covering every
        // sample of the (fragment-less) track.
        let senc = find_box(stbl, b"senc").ok_or(Error::UnexpectedBox { expected: "senc" })?;
        parse_senc_box(senc, tenc.default_per_sample_iv_size)?.entries
    };

    Ok(Some(TrackCrypto {
        track_id,
        tenc,
        original_format,
        scheme,
        samples,
    }))
}

/// Parse a full `senc` box (header + FullBox + body) into its typed form,
/// given the track's `tenc.default_per_sample_iv_size`.
fn parse_senc_box(senc: &[u8], per_sample_iv_size: u8) -> Result<crate::cenc::SampleEncryptionBox> {
    if senc.len() < BOX_HEADER_MIN_SIZE + FULL_HDR {
        return Err(Error::BufferTooShort {
            need: BOX_HEADER_MIN_SIZE + FULL_HDR,
            have: senc.len(),
            what: "senc header",
        });
    }
    let version = senc[BOX_HEADER_MIN_SIZE];
    let flags = u32::from_be_bytes([
        0,
        senc[BOX_HEADER_MIN_SIZE + 1],
        senc[BOX_HEADER_MIN_SIZE + 2],
        senc[BOX_HEADER_MIN_SIZE + 3],
    ]);
    crate::cenc::SampleEncryptionBox::parse_body(
        &senc[BOX_HEADER_MIN_SIZE + FULL_HDR..],
        version,
        flags,
        per_sample_iv_size,
    )
}

/// Walk every top-level `moof` in a fragmented file and append each `traf`'s
/// `senc` entries to the matching (by `tfhd.track_id`) [`TrackCrypto`], in
/// file order.
///
/// Reuses the already-typed [`TrackFragmentHeaderBox`] parser (from
/// [`crate::movie_fragment`]) to recover `tfhd.track_id` — `senc` itself is
/// not part of that crate's typed `moof`/`traf` structures (only
/// `tfhd`/`tfdt`/`trun` are), so it is located directly among the `traf`'s
/// sibling boxes with this module's own box-navigation helpers, the same way
/// [`find_sinf_in_stsd`] locates `sinf` among an `stsd` entry's children.
///
/// A `traf` with no matching protected track (e.g. an unencrypted audio
/// track) is skipped. A `traf` with no `senc` at all is only ever legitimate
/// for a **constant-IV, whole-sample-protected** track
/// (`tenc.default_per_sample_iv_size == 0`) — the one shape
/// [`crate::movie_fragment::protect_media_segment`] (via its private
/// `build_cenc_fragment_boxes`) deliberately omits `senc`/`saiz`/`saio` for,
/// since every sample of such a track decrypts from `tenc.default_constant_IV`
/// alone and there is nothing per-sample for `senc` to carry (ISO/IEC
/// 23001-7 §12.2/§12.3). That shape needs placeholder entries synthesized
/// here (one empty IV/subsample-map pair per `trun` sample) so
/// [`Decrypt::decrypt`]'s per-track sample-count pairing still lines up —
/// otherwise a legitimately senc-less fragment would fail with the generic
/// "sample count mismatch" error despite being fully decryptable. For any
/// other track (`default_per_sample_iv_size != 0`), a missing `senc` should
/// not happen for a genuinely protected track; it is tolerated here rather
/// than treated as fatal, and the same sample-count check downstream will
/// still catch it.
fn harvest_fragment_senc(file: &[u8], tracks: &mut [TrackCrypto]) -> Result<()> {
    for moof in iter_top_boxes(file, b"moof") {
        for traf in iter_child_boxes(moof, b"traf") {
            let Some(tfhd) = find_box(traf, b"tfhd") else {
                continue;
            };
            if tfhd.len() < BOX_HEADER_MIN_SIZE + FULL_HDR {
                return Err(Error::BufferTooShort {
                    need: BOX_HEADER_MIN_SIZE + FULL_HDR,
                    have: tfhd.len(),
                    what: "tfhd header",
                });
            }
            let tfhd_parsed = TrackFragmentHeaderBox::parse_body(&tfhd[BOX_HEADER_MIN_SIZE..])?;

            let Some(crypto) = tracks
                .iter_mut()
                .find(|t| t.track_id == tfhd_parsed.track_id)
            else {
                // This traf's track isn't one we're decrypting (e.g. the
                // unencrypted audio track alongside a protected video track).
                continue;
            };
            // A per-fragment `seig` override (a `traf`-level `sgpd`/`sbgp`,
            // the common CMAF shape for key rotation across fragments) is
            // just as unsafe to ignore as the `stbl`-level one checked in
            // `harvest_track` — see that call site's comment (issue #990).
            if container_has_seig_sgpd(traf)? {
                return Err(Error::UnsupportedFeature(
                    "seig sample-group key rotation not implemented",
                ));
            }
            match find_box(traf, b"senc") {
                Some(senc) => {
                    let senc_parsed = parse_senc_box(senc, crypto.tenc.default_per_sample_iv_size)?;
                    crypto.samples.extend(senc_parsed.entries);
                }
                None if crypto.tenc.default_per_sample_iv_size == 0 => {
                    let sample_count = traf_trun_sample_count(traf)?;
                    crypto.samples.extend(
                        core::iter::repeat_with(|| SampleEncryptionEntry {
                            initialization_vector: Vec::new(),
                            subsamples: Vec::new(),
                        })
                        .take(sample_count),
                    );
                }
                None => {
                    // Should not happen for a genuinely protected,
                    // per-sample-IV track; the sample-count mismatch check in
                    // `Decrypt::decrypt` will catch it.
                }
            }
        }
    }
    Ok(())
}

/// Sum every `trun`'s sample count inside one `traf` (ISO/IEC 14496-12:2015
/// §8.8.8) — used only to synthesize placeholder `senc` entries for a
/// legitimately senc-less constant-IV/whole-sample fragment (see
/// [`harvest_fragment_senc`]).
fn traf_trun_sample_count(traf: &[u8]) -> Result<usize> {
    let mut total = 0usize;
    for trun in
        iter_boxes(&traf[BOX_HEADER_MIN_SIZE.min(traf.len())..]).filter(|b| &b[4..8] == b"trun")
    {
        if trun.len() < BOX_HEADER_MIN_SIZE {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_MIN_SIZE,
                have: trun.len(),
                what: "trun header",
            });
        }
        let parsed = TrackFragmentRunBox::parse_body(&trun[BOX_HEADER_MIN_SIZE..])?;
        total += parsed.samples.len();
    }
    Ok(total)
}

/// Find the `sinf` box nested inside the (first) `encv`/`enca` sample entry of
/// an `stsd` box.
fn find_sinf_in_stsd(stsd: &[u8]) -> Option<&[u8]> {
    // stsd body: FullBox(4) + entry_count(4), then sample entries.
    let body_start = BOX_HEADER_MIN_SIZE + FULL_HDR + STSD_ENTRY_COUNT;
    if body_start > stsd.len() {
        return None;
    }
    for entry in iter_boxes(&stsd[body_start..]) {
        let ty = &entry[4..8];
        if ty == b"encv" || ty == b"enca" {
            // Sample-entry child boxes start after the fixed VisualSampleEntry /
            // AudioSampleEntry header. We only support protected video (encv)
            // here; the sinf is a child box, located by scanning.
            let child_start = if ty == b"encv" {
                BOX_HEADER_MIN_SIZE + VISUAL_SAMPLE_ENTRY_HDR
            } else {
                // enca: 8 reserved + 2 channelcount + 2 samplesize + 4 predefined
                // + 2 reserved + 2 timescale-hi... just scan from a safe minimum
                // (AudioSampleEntry fixed part is 28 bytes past the box header).
                BOX_HEADER_MIN_SIZE + 28
            };
            if child_start <= entry.len() {
                // The child boxes start directly at `child_start` (no container
                // header to skip), so scan them with `iter_boxes`.
                if let Some(sinf) = iter_boxes(&entry[child_start..]).find(|b| &b[4..8] == b"sinf")
                {
                    return Some(sinf);
                }
            }
        }
    }
    None
}

/// The FourCC of the first sample entry in an `stsd` box, as text
/// (`"unknown"` when the box is too short to hold one).
fn stsd_entry_fourcc(stsd: &[u8]) -> &'static str {
    let body_start = BOX_HEADER_MIN_SIZE + FULL_HDR + STSD_ENTRY_COUNT;
    let Some(first) = stsd.get(body_start..body_start + BOX_HEADER_MIN_SIZE) else {
        return "unknown";
    };
    // The entry's box type is its second four bytes.
    match &first[4..8] {
        b"avc1" | b"encv" => "avc1",
        b"mp4a" | b"enca" => "mp4a",
        b"hvc1" | b"hev1" => "hvc1",
        b"vp09" => "vp09",
        b"av01" => "av01",
        b"ac-3" => "ac-3",
        b"ec-3" => "ec-3",
        b"stpp" => "stpp",
        b"wvtt" => "wvtt",
        _ => "unknown",
    }
}

/// Demux a protected fMP4 into a [`Media`] of encrypted samples.
///
/// Supports both the progressive layout (single `moov`/`mdat`, sample layout
/// from `stsz`/`stsc`/`stco`, e.g. ffmpeg's `-cenc_aes_ctr`) and fragmented
/// CMAF (`moov` + one or more `moof`/`mdat` pairs, sample layout from each
/// fragment's `trun`) — see [`collect_fragment_samples`].
fn demux_protected(file: &bytes::Bytes) -> Result<Media> {
    use crate::AVCConfigurationBox;
    use crate::media::{Media, Track};
    use crate::pipeline::{CodecConfig, Sample, TrackSpec};

    let moov = find_top_box(file, b"moov").ok_or(Error::UnexpectedBox { expected: "moov" })?;
    let movie_timescale = mvhd_timescale(moov).unwrap_or(1000);
    let fragmented = find_top_box(file, b"moof").is_some();

    let mut tracks = Vec::new();
    let mut skipped: Vec<crate::ir::SkippedTrack> = Vec::new();
    for trak in iter_child_boxes(moov, b"trak") {
        let Some(stbl) = descend(trak, &[b"mdia", b"minf", b"stbl"]) else {
            continue;
        };
        let timescale = descend(trak, &[b"mdia"])
            .and_then(|mdia| find_box(mdia, b"mdhd"))
            .and_then(mdhd_timescale)
            .unwrap_or(movie_timescale);

        // Only protected video (encv → original avc1) is reconstructed here.
        let Some(stsd) = find_box(stbl, b"stsd") else {
            continue;
        };
        let Some(sinf) = find_sinf_in_stsd(stsd) else {
            // An **unprotected** track (no `sinf`) alongside protected ones:
            // skipped, but recorded so the drop is never silent (audit
            // r05-W6 follow-up). Its sample-entry FourCC is the `stsd` child.
            skipped.push(crate::ir::SkippedTrack::new(
                stsd_entry_fourcc(stsd).to_owned(),
                "unprotected track: CencDecryptor::demux reconstructs protected tracks only"
                    .to_owned(),
            ));
            continue;
        };
        let sinf_parsed = crate::cenc::ProtectionSchemeInfoBox::parse(sinf)?;
        if &sinf_parsed.original_format.data_format != b"avc1" {
            // Skip, rather than fail the whole file, a protected track whose
            // original format is not AVC — a typical `encv`+`enca` CENC
            // asset's audio leg, or a protected HEVC video (audit r05-W6).
            // Failing here made an ordinary CENC asset undemuxable entirely.
            // Recorded in `Media::skipped` so the skip is loud, never silent
            // (audit r05-W6 follow-up); a caller that needs the track still
            // gets a clear error from `decrypt`'s "no protected-source track
            // matches this media track's track_id".
            let fourcc = core::str::from_utf8(&sinf_parsed.original_format.data_format)
                .unwrap_or("unknown")
                .to_owned();
            skipped.push(crate::ir::SkippedTrack::new(
                fourcc,
                "protected track whose original format is not avc1: only protected AVC demux is supported"
                    .to_owned(),
            ));
            continue;
        }
        // Recover the avcC config record from inside the encv entry.
        let avc_config = find_avcc_config(stsd)?;

        let tkhd = find_box(trak, b"tkhd").ok_or(Error::UnexpectedBox { expected: "tkhd" })?;
        let track_id = crate::init_segment::TrackHeaderBox::parse(tkhd)?.track_id;

        let samples = if fragmented {
            collect_fragment_samples(file, track_id)?
        } else {
            // Sample byte layout from stsz + stsc + stco (contiguous chunks).
            let sizes = stsz_sizes(stbl, file.len())?;
            let sample_offsets = sample_file_offsets(stbl, &sizes)?;

            let mut samples = Vec::with_capacity(sizes.len());
            for (&size, &offset) in sizes.iter().zip(sample_offsets.iter()) {
                let end = offset
                    .checked_add(size)
                    .ok_or(Error::InvalidInput("sample offset + size overflow"))?;
                if end > file.len() {
                    return Err(Error::BufferTooShort {
                        need: end,
                        have: file.len(),
                        what: "protected sample data",
                    });
                }
                samples.push(Sample {
                    data: file.slice(offset..end),
                    // Progressive (non-fragmented) protected-sample recovery
                    // never parsed `stts`/`ctts` timing here (pre-existing
                    // behaviour); `None` is the honest representation now
                    // that a fabricated placeholder timestamp is no longer
                    // required to populate the struct.
                    dts: None,
                    pts: None,
                    duration: None,
                    flags: crate::ir::SampleFlags::SYNC,
                    provenance: None,
                });
            }
            samples
        };

        tracks.push(Track::new(
            TrackSpec::new(
                track_id,
                timescale,
                CodecConfig::Avc {
                    config: AVCConfigurationBox::new(avc_config),
                    width: 0,
                    height: 0,
                },
            ),
            samples,
        ));
    }

    if tracks.is_empty() {
        return Err(Error::UnexpectedBox {
            expected: "a protected AVC track",
        });
    }
    let mut media = Media::new(tracks, movie_timescale);
    media.skipped = skipped;
    Ok(media)
}

/// Collect one track's coded sample bytes from every `moof`/`mdat` fragment
/// pair in `file`, in file order.
///
/// Reuses the already-typed [`MovieFragmentBox`] parser (which in turn parses
/// `tfhd`/`tfdt`/`trun`) for the fragment structure — this mirrors
/// [`crate::media::Fmp4Demux`]'s own `moof`/`mdat` walk, scoped to a single
/// `target_track_id` and without decrypting or resolving codec config (the
/// caller, [`demux_protected`], already has that from `moov`).
fn collect_fragment_samples(
    file: &bytes::Bytes,
    target_track_id: u32,
) -> Result<Vec<crate::pipeline::Sample>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    // One `mdat` scan for the whole file, shared by every fragment.
    let mdats = crate::frag_offsets::mdat_ranges(file);
    let mut pending_moof: Option<(usize, MovieFragmentBox)> = None;
    // Running absolute decode-time cursor (media plane step 2c), seeded from
    // the first fragment's `tfdt` for this track (mirrors
    // `crate::media::Fmp4Demux`'s `TrackBuilder`); `Track::new` below anchors
    // at 0 regardless (this path never fed a caller-visible anchor), so this
    // is purely to give each recovered sample a real, internally-consistent
    // `dts`/`pts` rather than `None` — the trun/tfdt here genuinely carries
    // per-sample timing, so `None` would be a fabricated *absence*, not an
    // honest one.
    let mut next_dts: i64 = 0;
    let mut seeded = false;
    while offset + BOX_HEADER_MIN_SIZE <= file.len() {
        let (bx, consumed) = parse_box(&file[offset..])?;
        if bx.header.box_type.is(b"moof") {
            let moof = MovieFragmentBox::parse_body(bx.body)?;
            pending_moof = Some((offset, moof));
        } else if bx.header.box_type.is(b"mdat")
            && let Some((moof_off, moof)) = pending_moof.take()
        {
            if !seeded {
                if let Some(tfdt) = moof
                    .traf
                    .iter()
                    .find(|t| t.tfhd.track_id == target_track_id)
                    .and_then(|t| t.tfdt.as_ref())
                {
                    next_dts = tfdt.base_media_decode_time() as i64;
                }
                seeded = true;
            }
            absorb_protected_fragment(
                file,
                &mdats,
                moof_off,
                &moof,
                target_track_id,
                &mut next_dts,
                &mut out,
            )?;
        }
        if consumed == 0 {
            break;
        }
        offset += consumed;
    }
    Ok(out)
}

/// Resolve one `moof`'s samples for `target_track_id` into `out`, slicing
/// coded bytes from `file`.
///
/// The sample data base follows ISO/IEC 14496-12 §8.8.7/§8.8.8:
///
/// - `tfhd.base_data_offset`, when present (`base-data-offset-present`), is an
///   **absolute** file offset and anchors the track fragment's runs ("an
///   explicit anchor for the data offsets in each track run");
/// - otherwise, with `default-base-is-moof` set, the base is the first byte of
///   the enclosing `moof`;
/// - otherwise, with neither set, the base for the **first** track fragment in
///   the movie fragment is the `moof` start, and for each subsequent track
///   fragment it is "the end of the data defined by the preceding fragment".
///
/// A `trun`'s `data_offset`, when present, is relative to that base ("it is
/// relative to the base-data-offset established in the track fragment
/// header"); when absent, "the data for this run starts immediately after the
/// data of the previous run, or at the base-data-offset defined by the track
/// fragment header if this is the first run in a track fragment".
///
/// The previous code read every run from `moof_off + trun.data_offset`, so a
/// `tfhd.base_data_offset` (explicit-base files, e.g. some PIFF/Smooth-derived
/// CMAF) was ignored and a `trun` without `data_offset` sliced the wrong bytes
/// — which CTR then "decrypted" to garbage with `Ok` (audit r05-W7).
///
/// Note on sourcing: ISO/IEC 14496-12 is not vendored in `private/specs` (it is
/// a paywalled ISO document), so the clause wording above is quoted from the
/// spec text as faithfully reproduced by multiple independent implementations
/// that cite the clause/page (SRS, DumpTS, l-smash, OvenMediaEngine, JAAD,
/// Shaka Packager); the `base-data-offset-present` and `trun.data_offset`
/// wording is stable across the 2012 → 2022 editions.
fn absorb_protected_fragment(
    file: &bytes::Bytes,
    mdats: &[crate::frag_offsets::MdatRange],
    moof_off: usize,
    moof: &MovieFragmentBox,
    target_track_id: u32,
    next_dts: &mut i64,
    out: &mut Vec<crate::pipeline::Sample>,
) -> Result<()> {
    use crate::pipeline::Sample;

    // All §8.8.7/§8.8.8 addressing and bounds live in `frag_offsets`, shared
    // with `Fmp4Demux` so the two cannot drift (audit r05-W7).
    for r in crate::frag_offsets::sample_ranges_in(file, mdats, moof_off, moof, target_track_id)? {
        let dts = *next_dts;
        let pts = crate::frag_offsets::add_offset(dts, r.composition_offset)?;
        let is_sync = r.flags & SAMPLE_FLAG_IS_NON_SYNC == 0;
        out.push(Sample {
            data: file.slice(r.start..r.end),
            dts: Some(dts),
            pts: Some(pts),
            duration: Some(r.duration),
            flags: crate::ir::SampleFlags::new(is_sync),
            provenance: None,
        });
        *next_dts = crate::frag_offsets::add_duration(dts, r.duration)?;
    }
    Ok(())
}

/// Parse the avcC record from the (first) encv entry of an stsd.
fn find_avcc_config(stsd: &[u8]) -> Result<crate::avc_config::AVCDecoderConfigurationRecord> {
    let body_start = BOX_HEADER_MIN_SIZE + FULL_HDR + STSD_ENTRY_COUNT;
    for entry in iter_boxes(&stsd[body_start.min(stsd.len())..]) {
        if &entry[4..8] == b"encv" {
            let child_start = BOX_HEADER_MIN_SIZE + VISUAL_SAMPLE_ENTRY_HDR;
            if child_start <= entry.len()
                && let Some(avcc) = iter_boxes(&entry[child_start..]).find(|b| &b[4..8] == b"avcC")
            {
                // avcC full bytes → body after the 8-byte box header.
                let cfg = crate::AVCConfigurationBox::parse_body(&avcc[BOX_HEADER_MIN_SIZE..])?;
                return Ok(cfg.config);
            }
        }
    }
    Err(Error::UnexpectedBox {
        expected: "avcC inside encv",
    })
}

// ---------------------------------------------------------------------------
// Small box-navigation helpers (borrow-only, no allocation).
// ---------------------------------------------------------------------------

/// Iterate the top-level boxes of `data`, yielding each box's full bytes.
fn iter_boxes(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut offset = 0usize;
    core::iter::from_fn(move || {
        if offset + BOX_HEADER_MIN_SIZE > data.len() {
            return None;
        }
        let (bx, consumed) = parse_box(&data[offset..]).ok()?;
        if consumed == 0 {
            return None;
        }
        let size = if bx.header.size == 0 {
            data.len() - offset
        } else {
            (bx.header.size as usize).min(data.len() - offset)
        };
        let start = offset;
        offset += consumed;
        Some(&data[start..start + size])
    })
}

/// Iterate a container box's children matching a four-CC (skips the 8-byte
/// container header first).
fn iter_child_boxes<'a>(
    container: &'a [u8],
    fourcc: &'a [u8; 4],
) -> impl Iterator<Item = &'a [u8]> {
    let body = &container[BOX_HEADER_MIN_SIZE.min(container.len())..];
    iter_boxes(body).filter(move |b| &b[4..8] == fourcc)
}

/// Iterate every *top-level* box in `file` matching a four-CC (there can be
/// several `moof`s in a fragmented CMAF file, unlike the single-match
/// [`find_top_box`]).
fn iter_top_boxes<'a>(file: &'a [u8], fourcc: &[u8; 4]) -> impl Iterator<Item = &'a [u8]> {
    iter_boxes(file).filter(move |b| b[4..8] == *fourcc)
}

/// Find the first child box of `container` with the given four-CC (returns its
/// full bytes). `container` is treated as a full box (its 8-byte header is
/// skipped before scanning children).
fn find_box<'a>(container: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let body = &container[BOX_HEADER_MIN_SIZE.min(container.len())..];
    iter_boxes(body).find(|b| &b[4..8] == fourcc)
}

/// Find a *top-level* box by four-CC in a raw file (boxes start at offset 0, so
/// no container header is skipped).
fn find_top_box<'a>(file: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    iter_boxes(file).find(|b| &b[4..8] == fourcc)
}

/// Whether `container` (a `stbl` or `traf`, both direct `sgpd` parents per
/// ISO/IEC 14496-12 §8.9.3.1) has a `sgpd` box whose `grouping_type` is
/// `seig` — the CENC key-rotation sample group this decryptor does not
/// implement (see the two call sites' comments, issue #990).
///
/// A container can carry more than one `sgpd` (one per grouping type in use),
/// so every child is checked rather than just the first.
fn container_has_seig_sgpd(container: &[u8]) -> Result<bool> {
    let body = &container[BOX_HEADER_MIN_SIZE.min(container.len())..];
    for entry in iter_boxes(body).filter(|b| &b[4..8] == b"sgpd") {
        let sgpd = SampleGroupDescriptionBox::parse(entry)?;
        if sgpd.grouping_type == GROUPING_TYPE_SEIG {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Descend a chain of container four-CCs from `start`, returning the innermost
/// box's full bytes (or `None` if any link is missing).
fn descend<'a>(start: &'a [u8], path: &[&[u8; 4]]) -> Option<&'a [u8]> {
    let mut cur = start;
    for fourcc in path {
        cur = find_box(cur, fourcc)?;
    }
    Some(cur)
}

/// Read `mvhd.timescale` (handles version 0 and 1 layouts).
fn mvhd_timescale(moov: &[u8]) -> Option<u32> {
    let mvhd = find_box(moov, b"mvhd")?;
    let version = mvhd.get(BOX_HEADER_MIN_SIZE)?;
    // version 0: after FullBox(4): creation(4) modification(4) timescale(4)
    // version 1: after FullBox(4): creation(8) modification(8) timescale(4)
    let ts_off = if *version == 1 {
        BOX_HEADER_MIN_SIZE + FULL_HDR + 16
    } else {
        BOX_HEADER_MIN_SIZE + FULL_HDR + 8
    };
    Some(u32::from_be_bytes([
        *mvhd.get(ts_off)?,
        *mvhd.get(ts_off + 1)?,
        *mvhd.get(ts_off + 2)?,
        *mvhd.get(ts_off + 3)?,
    ]))
}

/// Read `mdhd.timescale` (handles version 0 and 1 layouts).
fn mdhd_timescale(mdhd: &[u8]) -> Option<u32> {
    let version = mdhd.get(BOX_HEADER_MIN_SIZE)?;
    let ts_off = if *version == 1 {
        BOX_HEADER_MIN_SIZE + FULL_HDR + 16
    } else {
        BOX_HEADER_MIN_SIZE + FULL_HDR + 8
    };
    Some(u32::from_be_bytes([
        *mdhd.get(ts_off)?,
        *mdhd.get(ts_off + 1)?,
        *mdhd.get(ts_off + 2)?,
        *mdhd.get(ts_off + 3)?,
    ]))
}

/// Read per-sample sizes from `stsz` (`sample_size == 0` → per-sample table).
///
/// `file_len` bounds the declared `sample_count`: no sample occupies fewer
/// than one byte of the file (ISO/IEC 14496-12 §8.7.3), so a count above it
/// is wire-hostile and must be rejected before anything allocates — this
/// used to run `Vec::with_capacity(count)` ahead of every length check, and
/// with `sample_size != 0` the push loop was entirely unbounded (r05-C4:
/// a ~200-byte file aborted the process on a multi-GB request).
fn stsz_sizes(stbl: &[u8], file_len: usize) -> Result<Vec<usize>> {
    let stsz = find_box(stbl, b"stsz").ok_or(Error::UnexpectedBox { expected: "stsz" })?;
    let base = BOX_HEADER_MIN_SIZE + FULL_HDR;
    let need = base + 8;
    if stsz.len() < need {
        return Err(Error::BufferTooShort {
            need,
            have: stsz.len(),
            what: "stsz header",
        });
    }
    let sample_size =
        u32::from_be_bytes([stsz[base], stsz[base + 1], stsz[base + 2], stsz[base + 3]]);
    let count = u32::from_be_bytes([
        stsz[base + 4],
        stsz[base + 5],
        stsz[base + 6],
        stsz[base + 7],
    ]) as usize;
    if count > file_len {
        return Err(Error::BufferTooShort {
            need: count,
            have: file_len,
            what: "stsz sample_count vs file length",
        });
    }
    let mut sizes;
    if sample_size != 0 {
        sizes = Vec::with_capacity(count);
        for _ in 0..count {
            sizes.push(sample_size as usize);
        }
    } else {
        let table = base + 8;
        // Checked arithmetic (`count * 4` wrapped on 32-bit targets), and
        // the table length is verified before any allocation.
        let table_bytes = count.checked_mul(4).ok_or(Error::BufferTooShort {
            need: usize::MAX,
            have: stsz.len(),
            what: "stsz sample_size table",
        })?;
        if table_bytes > stsz.len() - table {
            return Err(Error::BufferTooShort {
                need: table.saturating_add(table_bytes),
                have: stsz.len(),
                what: "stsz sample_size table",
            });
        }
        sizes = Vec::with_capacity(count);
        for i in 0..count {
            let o = table + i * 4;
            sizes.push(
                u32::from_be_bytes([stsz[o], stsz[o + 1], stsz[o + 2], stsz[o + 3]]) as usize,
            );
        }
    }
    Ok(sizes)
}

#[cfg(test)]
thread_local! {
    /// Work counter at the `stsc` expansion loop in [`sample_file_offsets`]
    /// (r05-O1): how many `stsc` entries were examined in total.
    static STSC_ENTRIES_EXAMINED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Compute each sample's absolute file offset from `stsc` + `stco`.
///
/// Maps samples to chunks (`stsc` run-length table) and each chunk to a file
/// offset (`stco`, 32-bit); within a chunk samples are contiguous in decode
/// order (ISO/IEC 14496-12 §8.7.4 / §8.7.5).
fn sample_file_offsets(stbl: &[u8], sizes: &[usize]) -> Result<Vec<usize>> {
    let stsc = find_box(stbl, b"stsc").ok_or(Error::UnexpectedBox { expected: "stsc" })?;
    let stco = find_box(stbl, b"stco").ok_or(Error::UnexpectedBox { expected: "stco" })?;
    let sc_base = BOX_HEADER_MIN_SIZE + FULL_HDR;

    // stco chunk offsets.
    if stco.len() < sc_base + 4 {
        return Err(Error::BufferTooShort {
            need: sc_base + 4,
            have: stco.len(),
            what: "stco header",
        });
    }
    let chunk_count = u32::from_be_bytes([
        stco[sc_base],
        stco[sc_base + 1],
        stco[sc_base + 2],
        stco[sc_base + 3],
    ]) as usize;
    let co_table = sc_base + 4;
    // Bound and verify before allocating (r05-C4 — capacity came from the
    // wire count ahead of this check pre-fix): checked arithmetic, compared
    // against the bytes actually present.
    let co_bytes = chunk_count.checked_mul(4).ok_or(Error::BufferTooShort {
        need: usize::MAX,
        have: stco.len(),
        what: "stco chunk offsets",
    })?;
    if co_bytes > stco.len() - co_table {
        return Err(Error::BufferTooShort {
            need: co_table.saturating_add(co_bytes),
            have: stco.len(),
            what: "stco chunk offsets",
        });
    }
    let mut chunk_offsets = Vec::with_capacity(chunk_count);
    for i in 0..chunk_count {
        let o = co_table + i * 4;
        chunk_offsets
            .push(u32::from_be_bytes([stco[o], stco[o + 1], stco[o + 2], stco[o + 3]]) as usize);
    }

    // stsc run-length: (first_chunk, samples_per_chunk, sample_desc_index).
    if stsc.len() < sc_base + 4 {
        return Err(Error::BufferTooShort {
            need: sc_base + 4,
            have: stsc.len(),
            what: "stsc header",
        });
    }
    let entry_count = u32::from_be_bytes([
        stsc[sc_base],
        stsc[sc_base + 1],
        stsc[sc_base + 2],
        stsc[sc_base + 3],
    ]) as usize;
    let sc_table = sc_base + 4;
    // Same checked-before-allocate discipline as `stco` above (r05-C4).
    let sc_bytes = entry_count.checked_mul(12).ok_or(Error::BufferTooShort {
        need: usize::MAX,
        have: stsc.len(),
        what: "stsc entries",
    })?;
    if sc_bytes > stsc.len() - sc_table {
        return Err(Error::BufferTooShort {
            need: sc_table.saturating_add(sc_bytes),
            have: stsc.len(),
            what: "stsc entries",
        });
    }
    // Expand: samples_per_chunk for each chunk index (1-based).
    //
    // One forward walk over the run table: chunk numbers only increase, so the
    // prefix of entries with `first_chunk <= chunk_no` only grows and the last
    // entry of that prefix is the applicable run. (Re-scanning the table from
    // the start for every chunk made this O(chunks x entries), audit r05-O1;
    // for any table, sorted or not, the entry chosen is the same.)
    let mut samples_per_chunk = Vec::with_capacity(chunk_count);
    let mut next_entry = 0usize;
    let mut spc = 0u32;
    for c in 0..chunk_count {
        let chunk_no = (c + 1) as u32;
        while next_entry < entry_count {
            #[cfg(test)]
            STSC_ENTRIES_EXAMINED.with(|n| n.set(n.get() + 1));
            let o = sc_table + next_entry * 12;
            let first_chunk = u32::from_be_bytes([stsc[o], stsc[o + 1], stsc[o + 2], stsc[o + 3]]);
            if first_chunk > chunk_no {
                break;
            }
            spc = u32::from_be_bytes([stsc[o + 4], stsc[o + 5], stsc[o + 6], stsc[o + 7]]);
            next_entry += 1;
        }
        samples_per_chunk.push(spc);
    }

    // Walk chunks → samples, accumulating offsets from each chunk base.
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut sample_idx = 0usize;
    for (c, &chunk_base) in chunk_offsets.iter().enumerate() {
        let per = samples_per_chunk.get(c).copied().unwrap_or(0) as usize;
        let mut cursor = chunk_base;
        for _ in 0..per {
            if sample_idx >= sizes.len() {
                break;
            }
            offsets.push(cursor);
            cursor += sizes[sample_idx];
            sample_idx += 1;
        }
    }
    if offsets.len() != sizes.len() {
        return Err(Error::InvalidInput(
            "stsc/stco sample-to-chunk mapping did not cover all samples",
        ));
    }
    Ok(offsets)
}

#[cfg(test)]
mod tests {
    //! Track-pairing tests for [`Decrypt::decrypt`].
    //!
    //! [`CencDecryptor`]'s fields are private, so only an in-crate test can
    //! build one with hand-made [`TrackCrypto`] records — which is what it takes
    //! to construct the mis-pairing case deterministically (two protected
    //! tracks with *equal* sample counts and different IVs, so a positional
    //! zip both mis-decrypts and slips past the sample-count check).

    use super::*;
    use crate::cenc_crypto;

    fn full_box(fourcc: &[u8; 4], entries: &[u8], count: u32) -> Vec<u8> {
        let mut b = Vec::new();
        let len = (BOX_HEADER_MIN_SIZE + FULL_HDR + 4 + entries.len()) as u32;
        b.extend_from_slice(&len.to_be_bytes());
        b.extend_from_slice(fourcc);
        b.extend_from_slice(&[0; FULL_HDR]);
        b.extend_from_slice(&count.to_be_bytes());
        b.extend_from_slice(entries);
        b
    }

    /// r05-O1: the per-chunk `stsc` lookup rescanned the run table from the start
    /// for every chunk (O(chunks x entries)). One entry per chunk here, so the old
    /// loop examined ~chunks^2/2 entries; the forward walk examines each once.
    #[test]
    fn stsc_expansion_examines_each_entry_once_not_once_per_chunk() {
        const CHUNKS: u32 = 400;
        let mut stsc = Vec::new();
        let mut stco = Vec::new();
        for c in 1..=CHUNKS {
            stsc.extend_from_slice(&c.to_be_bytes()); // first_chunk
            stsc.extend_from_slice(&1u32.to_be_bytes()); // samples_per_chunk
            stsc.extend_from_slice(&1u32.to_be_bytes()); // sample_description_index
            stco.extend_from_slice(&(c * 100).to_be_bytes());
        }
        let mut stbl = vec![0u8; BOX_HEADER_MIN_SIZE];
        stbl.extend(full_box(b"stsc", &stsc, CHUNKS));
        stbl.extend(full_box(b"stco", &stco, CHUNKS));
        let sizes = vec![10usize; CHUNKS as usize];
        STSC_ENTRIES_EXAMINED.with(|c| c.set(0));
        let offsets = sample_file_offsets(&stbl, &sizes).unwrap();
        let examined = STSC_ENTRIES_EXAMINED.with(core::cell::Cell::get);
        assert_eq!(offsets[0], 100);
        assert_eq!(offsets[CHUNKS as usize - 1], CHUNKS as usize * 100);
        // Linear: each entry is accepted once, plus one rejecting peek per chunk
        // (799 here; the per-chunk rescan examined 80 599).
        assert!(examined <= 2 * CHUNKS as usize, "examined {examined}");
    }
    use crate::media::Track;
    use crate::pipeline::{CodecConfig, Sample, TrackSpec};
    use broadcast_common::Unpackage;

    const VIDEO_TRACK_ID: u32 = 1;
    const AUDIO_TRACK_ID: u32 = 2;
    const KID: [u8; KEY_LEN] = [0xAA; KEY_LEN];
    const KEY: [u8; KEY_LEN] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10,
    ];
    /// The two tracks' per-sample IVs, deliberately different — the whole
    /// point of the `Media` being narrowed is that the surviving track must
    /// still get *its own* IV.
    const VIDEO_IV: [u8; 8] = [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11];
    const AUDIO_IV: [u8; 8] = [0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22];
    /// Equal on both tracks, so a positional mis-pairing is NOT caught by the
    /// "sample count mismatch between media and senc" check.
    const SAMPLES_PER_TRACK: usize = 2;

    fn tenc() -> TrackEncryptionBox {
        TrackEncryptionBox {
            version: 0,
            default_crypt_byte_block: 0,
            default_skip_byte_block: 0,
            default_is_protected: 1,
            default_per_sample_iv_size: 8,
            default_kid: KID,
            default_constant_iv: None,
        }
    }

    fn crypto(track_id: u32, iv: &[u8; 8]) -> TrackCrypto {
        TrackCrypto {
            track_id,
            tenc: tenc(),
            original_format: *b"avc1",
            scheme: CencScheme::Cenc,
            samples: (0..SAMPLES_PER_TRACK)
                .map(|_| SampleEncryptionEntry {
                    initialization_vector: iv.to_vec(),
                    subsamples: Vec::new(),
                })
                .collect(),
        }
    }

    /// A decryptor over a two-track protected source: video (`track_id` 1) then
    /// audio (`track_id` 2), in `moov` order.
    fn decryptor() -> CencDecryptor {
        CencDecryptor {
            file: bytes::Bytes::new(),
            tracks: alloc::vec![
                crypto(VIDEO_TRACK_ID, &VIDEO_IV),
                crypto(AUDIO_TRACK_ID, &AUDIO_IV),
            ],
        }
    }

    /// A codec config for the synthetic tracks. Irrelevant to track pairing
    /// (the property under test) — Opus is simply the cheapest `CodecConfig` to
    /// build by hand, needing no parsed configuration record.
    fn test_codec_config() -> CodecConfig {
        CodecConfig::Opus {
            config: crate::opus::OpusSpecificBox {
                version: 0,
                output_channel_count: 2,
                pre_skip: 0,
                input_sample_rate: 48_000,
                output_gain: 0,
                channel_mapping_family: 0,
                channel_mapping: None,
            },
            channel_count: 2,
            sample_rate: 48_000,
            sample_size: 16,
        }
    }

    /// Distinct plaintext per sample, so a wrong-IV "decryption" cannot
    /// coincidentally match.
    fn plaintext(i: usize) -> Vec<u8> {
        (0u8..64).map(|b| b.wrapping_add(i as u8 * 7)).collect()
    }

    /// One track's `Media`, its samples already encrypted with `iv`.
    fn encrypted_media(track_id: u32, iv: &[u8; 8]) -> Media {
        let samples = (0..SAMPLES_PER_TRACK)
            .map(|i| {
                let mut buf = plaintext(i);
                cenc_crypto::apply_ctr(iv, &KEY, &[], &mut buf).expect("encrypt");
                Sample {
                    data: buf.into(),
                    dts: None,
                    pts: None,
                    duration: None,
                    flags: crate::ir::SampleFlags::SYNC,
                    provenance: None,
                }
            })
            .collect();
        Media::new(
            alloc::vec![Track::new(
                TrackSpec::new(track_id, 90_000, test_codec_config()),
                samples,
            )],
            90_000,
        )
    }

    /// **The mis-pairing regression test.** A `Media` narrowed to the *second*
    /// protected track (`select_tracks_by`, e.g. audio-only) must be decrypted
    /// with that track's own IVs. A positional zip pairs it with the *first*
    /// crypto record instead — and because both tracks carry the same number of
    /// samples, the sample-count check does not notice, so the old code
    /// returned `Ok` over garbage.
    #[test]
    fn narrowed_media_decrypts_with_its_own_tracks_ivs() {
        let dec = decryptor();
        let keys = KeyMap::new().with_key(KID, KEY);
        let mut media = encrypted_media(AUDIO_TRACK_ID, &AUDIO_IV);

        dec.decrypt(&mut media, &keys).expect("decrypt");

        for (i, sample) in media.tracks[0].samples.iter().enumerate() {
            assert_eq!(
                &sample.data[..],
                &plaintext(i)[..],
                "sample {i} must be decrypted with track {AUDIO_TRACK_ID}'s IV, not \
                 whichever record happens to sit at the same position"
            );
        }
    }

    /// The first track still decrypts correctly (the pairing change must not
    /// merely swap which track is wrong).
    #[test]
    fn first_track_still_decrypts_with_its_own_ivs() {
        let dec = decryptor();
        let keys = KeyMap::new().with_key(KID, KEY);
        let mut media = encrypted_media(VIDEO_TRACK_ID, &VIDEO_IV);
        dec.decrypt(&mut media, &keys).expect("decrypt");
        for (i, sample) in media.tracks[0].samples.iter().enumerate() {
            assert_eq!(&sample.data[..], &plaintext(i)[..]);
        }
    }

    /// A `Media` holding **both** protected tracks, where the second track's
    /// `senc` has the wrong sample count — so the failure is only discovered
    /// once the loop reaches track 2.
    fn two_track_media_second_bad() -> Media {
        let video = encrypted_media(VIDEO_TRACK_ID, &VIDEO_IV);
        let audio = encrypted_media(AUDIO_TRACK_ID, &AUDIO_IV);
        Media::new(
            alloc::vec![video.tracks[0].clone(), audio.tracks[0].clone()],
            90_000,
        )
    }

    /// A decryptor whose second track's `senc` has one fewer entry than the
    /// media track's sample count.
    fn decryptor_second_track_count_mismatch() -> CencDecryptor {
        let mut second = crypto(AUDIO_TRACK_ID, &AUDIO_IV);
        second.samples.pop();
        CencDecryptor {
            file: bytes::Bytes::new(),
            tracks: alloc::vec![crypto(VIDEO_TRACK_ID, &VIDEO_IV), second],
        }
    }

    /// Audit r05-W5: `decrypt` must be atomic. A sample-count mismatch on
    /// **track 2** must be reported without track 1 having been decrypted in
    /// place — otherwise the caller cannot tell which samples are now
    /// plaintext, and under CTR a retry would XOR the already-plaintext
    /// samples *back* to ciphertext.
    #[test]
    fn decrypt_is_atomic_across_tracks() {
        let dec = decryptor_second_track_count_mismatch();
        let keys = KeyMap::new().with_key(KID, KEY);
        let mut media = two_track_media_second_bad();
        // Ciphertext snapshot of every sample, before the call.
        let before: Vec<Vec<u8>> = media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.to_vec()))
            .collect();
        // Sanity: track 1's samples really are ciphertext, not plaintext.
        assert_ne!(&media.tracks[0].samples[0].data[..], &plaintext(0)[..]);

        let err = dec.decrypt(&mut media, &keys).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "got {err:?}");

        let after: Vec<Vec<u8>> = media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.to_vec()))
            .collect();
        assert_eq!(
            after, before,
            "a rejected decrypt must leave every sample byte-identical"
        );
    }

    /// Audit r05-W5 follow-up: a malformed subsample map on **track 2** must
    /// also leave track 1 untouched. The map declares more protected bytes
    /// than the sample has, so `validate_subsample_map` rejects it.
    #[test]
    fn decrypt_is_atomic_on_bad_subsample_map_track2() {
        let mut second = crypto(AUDIO_TRACK_ID, &AUDIO_IV);
        // 64-byte samples (see `plaintext`): claim 100 protected bytes.
        second.samples[0].subsamples = alloc::vec![crate::cenc::SubSampleEntry {
            bytes_of_clear_data: 0,
            bytes_of_protected_data: 100,
        }];
        let dec = CencDecryptor {
            file: bytes::Bytes::new(),
            tracks: alloc::vec![crypto(VIDEO_TRACK_ID, &VIDEO_IV), second],
        };
        let mut media = two_track_media_second_bad();
        let before: Vec<Vec<u8>> = media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.to_vec()))
            .collect();

        let err = dec
            .decrypt(&mut media, &KeyMap::new().with_key(KID, KEY))
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_) | Error::BufferTooShort { .. }),
            "got {err:?}"
        );
        let after: Vec<Vec<u8>> = media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.to_vec()))
            .collect();
        assert_eq!(
            after, before,
            "media must be byte-identical after a rejection"
        );
    }

    /// A two-track `cbcs` decryptor whose second track's `senc` count
    /// mismatches — the `cbcs` path must be atomic too, not only `cenc`.
    #[test]
    fn decrypt_is_atomic_for_cbcs() {
        let mut t = tenc();
        t.default_per_sample_iv_size = 16;
        t.default_crypt_byte_block = 1;
        t.default_skip_byte_block = 9;
        let mk = |id: u32, iv: u8, count: usize| TrackCrypto {
            track_id: id,
            tenc: t.clone(),
            original_format: *b"avc1",
            scheme: CencScheme::Cbcs,
            samples: (0..count)
                .map(|_| SampleEncryptionEntry {
                    initialization_vector: alloc::vec![iv; 16],
                    subsamples: Vec::new(),
                })
                .collect(),
        };
        let dec = CencDecryptor {
            file: bytes::Bytes::new(),
            tracks: alloc::vec![
                mk(VIDEO_TRACK_ID, 0x11, SAMPLES_PER_TRACK),
                mk(AUDIO_TRACK_ID, 0x22, SAMPLES_PER_TRACK - 1),
            ],
        };
        let mut media = two_track_media_second_bad();
        let before: Vec<Vec<u8>> = media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.to_vec()))
            .collect();

        let err = dec
            .decrypt(&mut media, &KeyMap::new().with_key(KID, KEY))
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "got {err:?}");
        let after: Vec<Vec<u8>> = media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.to_vec()))
            .collect();
        assert_eq!(
            after, before,
            "a rejected cbcs decrypt must leave media byte-identical"
        );
    }

    /// Audit r05-W5: a **missing content key** on track 2 must likewise be
    /// caught before track 1 is touched. The key map carries only the video
    /// track's KID.
    #[test]
    fn decrypt_is_atomic_when_a_key_is_missing() {
        // Second track uses a different KID, which the key map will not hold.
        let mut second = crypto(AUDIO_TRACK_ID, &AUDIO_IV);
        second.tenc.default_kid = [0xBB; KEY_LEN];
        let dec = CencDecryptor {
            file: bytes::Bytes::new(),
            tracks: alloc::vec![crypto(VIDEO_TRACK_ID, &VIDEO_IV), second],
        };
        let mut media = two_track_media_second_bad();
        let before: Vec<Vec<u8>> = media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.to_vec()))
            .collect();

        let err = dec
            .decrypt(&mut media, &KeyMap::new().with_key(KID, KEY))
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "got {err:?}");
        let after: Vec<Vec<u8>> = media
            .tracks
            .iter()
            .flat_map(|t| t.samples.iter().map(|s| s.data.to_vec()))
            .collect();
        assert_eq!(
            after, before,
            "media must be byte-identical after a rejection"
        );
    }

    /// A media track the decryptor has no crypto record for is an error, not a
    /// silent pass-through of still-encrypted samples.
    #[test]
    fn unknown_track_id_errors() {
        let dec = decryptor();
        let keys = KeyMap::new().with_key(KID, KEY);
        let mut media = encrypted_media(99, &AUDIO_IV);
        let err = dec.decrypt(&mut media, &keys).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "got {err:?}");
    }

    /// W8: `KeyMap`'s `Debug` must list KIDs (not secret) but never the
    /// content key bytes paired with them.
    #[test]
    fn keymap_debug_redacts_key_bytes_but_shows_kids() {
        let km = KeyMap::new().with_key(KID, KEY);
        let out = alloc::format!("{km:?}");
        assert!(
            !out.contains(&alloc::format!("{KEY:?}")),
            "Debug output must not contain the key's array representation: {out}"
        );
        assert!(
            out.contains(&alloc::format!("{KID:?}")),
            "KIDs are not secret and should still be visible: {out}"
        );
    }

    /// W8: `CencDecryptor` never stores a content key (keys are supplied out
    /// of band to [`Decrypt::decrypt`]), but its `Debug` must also never dump
    /// the raw protected file bytes — pin that the output stays bounded
    /// regardless of file size, rather than growing with it.
    #[test]
    fn decryptor_debug_does_not_dump_raw_file_bytes() {
        let mut small = decryptor();
        small.file = bytes::Bytes::from_static(&[0x42u8; 10]);
        let mut large = decryptor();
        large.file = bytes::Bytes::from(alloc::vec![0x42u8; 10_000]);

        let small_out = alloc::format!("{small:?}");
        let large_out = alloc::format!("{large:?}");

        assert!(!small_out.contains(&alloc::format!("{KEY:?}")));
        assert!(!large_out.contains(&alloc::format!("{KEY:?}")));
        // A derived `Debug` over `file: Vec<u8>` would make the 10_000-byte
        // file's output roughly 1000x longer than the 10-byte file's; the
        // manual impl instead prints only `file_len` (a handful of digits),
        // so the two stay within a few characters of each other (both
        // decryptors are otherwise identical).
        let diff = large_out.len().abs_diff(small_out.len());
        assert!(
            diff < 20,
            "Debug output must not scale with file size (it should print file_len, not the \
             bytes): small={small_out} ({} chars), large={large_out} ({} chars)",
            small_out.len(),
            large_out.len()
        );
        assert!(large_out.contains("file_len"));
    }

    /// Audit r05-W7 (audio half): for **every** layout fixture, the audio
    /// track (track 2) must decrypt sample-for-sample to `clear.mp4`'s audio
    /// plaintext, across every `moof`. This is the traf the base rules exist
    /// for — the second traf in each moof is where `base_data_offset` /
    /// default-base-is-moof / the omit carry and the implicit-trun rule all
    /// bite. The clear reference is `clear.mp4` demuxed with the crate's own
    /// `Fmp4Demux` (whose addressing is the shared `frag_offsets` resolver);
    /// `clear_samples.txt` is its per-sample hash manifest.
    #[test]
    fn all_layouts_decrypt_audio_to_clear() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/cenc_frag_layouts"
        );
        let read = |name: &str| {
            std::fs::read(alloc::format!("{dir}/{name}"))
                .unwrap_or_else(|e| panic!("read {dir}/{name}: {e}"))
        };
        // Independent-of-this-test oracle: the clear file's own audio samples,
        // read with the plain (unencrypted) `Fmp4Demux`.
        let clear_bytes = read("clear.mp4");
        let mut fd = crate::media::Fmp4Demux::new();
        let clear_media = fd.unpackage(clear_bytes.as_slice()).expect("demux clear");
        let clear_audio: Vec<Vec<u8>> = clear_media
            .tracks
            .iter()
            .find(|t| t.spec.track_id == 2)
            .expect("clear.mp4 must carry track 2 (audio)")
            .samples
            .iter()
            .map(|s| s.data.to_vec())
            .collect();
        assert_eq!(clear_audio.len(), 95, "clear audio has 95 samples");

        let files = [
            "enc_default_none.mp4",
            "enc_default_explicit.mp4",
            "enc_default_implicit.mp4",
            "enc_base_moof_none.mp4",
            "enc_base_moof_explicit.mp4",
            "enc_base_moof_implicit.mp4",
            "enc_omit_none.mp4",
            "enc_omit_explicit.mp4",
            "enc_omit_implicit.mp4",
        ];
        // Key/KID for these fixtures (their README).
        const FKID: [u8; KEY_LEN] = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ];
        const FKEY: [u8; KEY_LEN] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let keys = KeyMap::new().with_key(FKID, FKEY);
        for f in files {
            let bytes = read(f);
            // Ciphertext samples for track 2, resolved by the shared
            // §8.8.7/§8.8.8 walker (the same code the decryptor and Fmp4Demux
            // use).
            let cipher = collect_fragment_samples(&bytes::Bytes::from(bytes.clone()), 2)
                .unwrap_or_else(|e| panic!("{f}: collect audio: {e}"));
            assert_eq!(
                cipher.len(),
                clear_audio.len(),
                "{f}: audio sample count must match the clear reference"
            );
            // The harvested per-sample IVs/subsample maps for the audio track.
            let dec = CencDecryptor::from_fmp4(&bytes).unwrap_or_else(|e| panic!("{f}: {e}"));
            let crypto = dec
                .tracks
                .iter()
                .find(|c| c.track_id == 2)
                .unwrap_or_else(|| panic!("{f}: no audio crypto record"));
            let key = keys
                .get(&crypto.tenc.default_kid)
                .unwrap_or_else(|| panic!("{f}: no key"));
            assert_eq!(crypto.samples.len(), cipher.len(), "{f}: senc count");
            for (i, (c, entry)) in cipher.iter().zip(crypto.samples.iter()).enumerate() {
                let mut buf = c.data.to_vec();
                crate::cenc_crypto::apply_ctr(
                    &entry.initialization_vector,
                    key,
                    &entry.subsamples,
                    &mut buf,
                )
                .unwrap_or_else(|e| panic!("{f}: audio sample {i}: {e}"));
                assert_eq!(
                    buf, clear_audio[i],
                    "{f}: audio sample {i} must equal the clear plaintext byte-for-byte"
                );
            }
        }
    }
}
