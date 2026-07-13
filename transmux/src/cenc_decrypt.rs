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
use crate::movie_fragment::{MovieFragmentBox, TrackFragmentHeaderBox};

/// Size of a KID / content key / AES-128 key **or block**, in bytes (AES-128's
/// key length and block length coincide).
const KEY_LEN: usize = 16;

/// A map of content keys, keyed by 16-byte Key ID (KID).
///
/// The [`Decrypt::Keys`] material for [`CencDecryptor`]: each protected sample's
/// KID (from `tenc.default_kid`) selects a 16-byte AES-128 content key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyMap {
    keys: BTreeMap<[u8; KEY_LEN], [u8; KEY_LEN]>,
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
#[derive(Debug, Clone)]
pub struct CencDecryptor {
    /// The whole protected fMP4 file (borrowing is avoided so the decryptor is
    /// `'static`-friendly for the trait impl; a `Vec` copy is acceptable here).
    file: Vec<u8>,
    /// Per-track crypto metadata, in `moov` track order.
    tracks: Vec<TrackCrypto>,
}

impl CencDecryptor {
    /// Build a decryptor by harvesting CENC metadata from a protected fMP4.
    ///
    /// Parses each track's `sinf` (`frma` + `schm` + `schi/tenc`) and its
    /// per-sample IV/subsample map (`senc` — a single `stbl`-level box for a
    /// progressive file, or every `moof`'s `traf`-level box, concatenated, for
    /// a fragmented one). Fails with [`Error::UnexpectedBox`] if no protected
    /// track is found.
    pub fn from_fmp4(file: &[u8]) -> Result<Self> {
        let mut tracks = Vec::new();
        harvest_tracks(file, &mut tracks)?;
        if tracks.is_empty() {
            return Err(Error::UnexpectedBox {
                expected: "a protected track (sinf/tenc + senc)",
            });
        }
        Ok(Self {
            file: file.to_vec(),
            tracks,
        })
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
    fn decrypt_sample(
        scheme: CencScheme,
        tenc: &TrackEncryptionBox,
        entry: &SampleEncryptionEntry,
        key: &[u8; KEY_LEN],
        data: &mut [u8],
    ) -> Result<()> {
        match scheme {
            CencScheme::Cenc => {
                cenc_crypto::apply_ctr(&entry.initialization_vector, key, &entry.subsamples, data)
            }
            CencScheme::Cbcs => cenc_crypto::cbcs_sample(tenc, entry, key, data, CbcsOp::Decrypt),
        }
    }
}

impl Decrypt for CencDecryptor {
    type Media = Media;
    type Keys = KeyMap;
    type Error = Error;

    fn decrypt(&self, media: &mut Media, keys: &KeyMap) -> Result<()> {
        // Pair each media track with a recovered crypto record by position
        // (both are in decode/`moov` track order).
        if media.tracks.len() > self.tracks.len() {
            return Err(Error::InvalidInput(
                "media has more tracks than the protected source",
            ));
        }
        for (track, crypto) in media.tracks.iter_mut().zip(self.tracks.iter()) {
            if crypto.tenc.default_is_protected == 0 {
                // Track is not protected — nothing to do.
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
/// `sample_is_non_sync_sample` bit within a 32-bit `sample_flags` word
/// (ISO/IEC 14496-12:2015 §8.8.3.1, bit `[16]`). Set = the sample is **not** a
/// sync sample (random-access point). Mirrors the identical constant in
/// [`crate::media`] (private there); duplicated here rather than exposed
/// cross-module, since both modules independently resolve `trun`/`tfhd`
/// sample flags.
const SAMPLE_FLAG_IS_NON_SYNC: u32 = 0x0001_0000;

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
/// track) or no `senc` at all (should not happen for a genuinely protected
/// track, but tolerated rather than treated as fatal) is skipped.
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
            let Some(senc) = find_box(traf, b"senc") else {
                continue;
            };
            let senc_parsed = parse_senc_box(senc, crypto.tenc.default_per_sample_iv_size)?;
            crypto.samples.extend(senc_parsed.entries);
        }
    }
    Ok(())
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

/// Demux a protected fMP4 into a [`Media`] of encrypted samples.
///
/// Supports both the progressive layout (single `moov`/`mdat`, sample layout
/// from `stsz`/`stsc`/`stco`, e.g. ffmpeg's `-cenc_aes_ctr`) and fragmented
/// CMAF (`moov` + one or more `moof`/`mdat` pairs, sample layout from each
/// fragment's `trun`) — see [`collect_fragment_samples`].
fn demux_protected(file: &[u8]) -> Result<Media> {
    use crate::AVCConfigurationBox;
    use crate::media::{Media, Track};
    use crate::pipeline::{CodecConfig, Sample, TrackSpec};

    let moov = find_top_box(file, b"moov").ok_or(Error::UnexpectedBox { expected: "moov" })?;
    let movie_timescale = mvhd_timescale(moov).unwrap_or(1000);
    let fragmented = find_top_box(file, b"moof").is_some();

    let mut tracks = Vec::new();
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
            continue;
        };
        let sinf_parsed = crate::cenc::ProtectionSchemeInfoBox::parse(sinf)?;
        if &sinf_parsed.original_format.data_format != b"avc1" {
            return Err(Error::UnexpectedBox {
                expected: "avc1 original_format (only protected AVC demux is supported)",
            });
        }
        // Recover the avcC config record from inside the encv entry.
        let avc_config = find_avcc_config(stsd)?;

        let tkhd = find_box(trak, b"tkhd").ok_or(Error::UnexpectedBox { expected: "tkhd" })?;
        let track_id = crate::init_segment::TrackHeaderBox::parse(tkhd)?.track_id;

        let samples = if fragmented {
            collect_fragment_samples(file, track_id)?
        } else {
            // Sample byte layout from stsz + stsc + stco (contiguous chunks).
            let sizes = stsz_sizes(stbl)?;
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
                    data: file[offset..end].to_vec(),
                    duration: 0,
                    is_sync: true,
                    composition_offset: 0,
                    source_timing: None,
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
    Ok(Media::new(tracks, movie_timescale))
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
    file: &[u8],
    target_track_id: u32,
) -> Result<Vec<crate::pipeline::Sample>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut pending_moof: Option<(usize, MovieFragmentBox)> = None;
    while offset + BOX_HEADER_MIN_SIZE <= file.len() {
        let (bx, consumed) = parse_box(&file[offset..])?;
        if bx.header.box_type.is(b"moof") {
            let moof = MovieFragmentBox::parse_body(bx.body)?;
            pending_moof = Some((offset, moof));
        } else if bx.header.box_type.is(b"mdat") {
            if let Some((moof_off, moof)) = pending_moof.take() {
                absorb_protected_fragment(file, moof_off, &moof, target_track_id, &mut out)?;
            }
        }
        if consumed == 0 {
            break;
        }
        offset += consumed;
    }
    Ok(out)
}

/// Resolve one `moof`'s samples for `target_track_id` into `out`, slicing
/// coded bytes from `file` using each `trun`'s `data_offset` (relative to the
/// `moof` start, i.e. `default-base-is-moof` — the near-universal fragmented
/// MP4 convention, ISO/IEC 14496-12:2015 §8.8.7/§8.8.8).
fn absorb_protected_fragment(
    file: &[u8],
    moof_off: usize,
    moof: &MovieFragmentBox,
    target_track_id: u32,
    out: &mut Vec<crate::pipeline::Sample>,
) -> Result<()> {
    use crate::pipeline::Sample;

    for traf in &moof.traf {
        let tfhd = &traf.tfhd;
        if tfhd.track_id != target_track_id {
            continue;
        }
        for trun in &traf.trun {
            let base = moof_off as i64 + trun.data_offset.unwrap_or(0) as i64;
            let mut cursor = base;
            for (i, ts) in trun.samples.iter().enumerate() {
                let size = ts
                    .sample_size
                    .or(tfhd.default_sample_size)
                    .ok_or(Error::InvalidInput(
                    "trun sample has no size (no trun.sample_size, no tfhd default_sample_size)",
                ))? as usize;
                let duration = ts
                    .sample_duration
                    .or(tfhd.default_sample_duration)
                    .unwrap_or(0);
                // Per-sample flags precedence: explicit trun sample_flags, else
                // first_sample_flags for sample 0, else the tfhd default.
                let flags = ts
                    .sample_flags
                    .or(if i == 0 {
                        trun.first_sample_flags
                    } else {
                        None
                    })
                    .or(tfhd.default_sample_flags)
                    .unwrap_or(0);
                let is_sync = flags & SAMPLE_FLAG_IS_NON_SYNC == 0;
                let composition_offset = ts.sample_composition_time_offset.unwrap_or(0);

                let start = usize::try_from(cursor)
                    .map_err(|_| Error::InvalidInput("negative sample data offset"))?;
                let end = start
                    .checked_add(size)
                    .ok_or(Error::InvalidInput("sample offset + size overflow"))?;
                if end > file.len() {
                    return Err(Error::BufferTooShort {
                        need: end,
                        have: file.len(),
                        what: "protected fragment sample data",
                    });
                }
                out.push(Sample {
                    data: file[start..end].to_vec(),
                    duration,
                    is_sync,
                    composition_offset,
                    source_timing: None,
                });
                cursor += size as i64;
            }
        }
    }
    Ok(())
}

/// Parse the avcC record from the (first) encv entry of an stsd.
fn find_avcc_config(stsd: &[u8]) -> Result<crate::avc_config::AVCDecoderConfigurationRecord> {
    let body_start = BOX_HEADER_MIN_SIZE + FULL_HDR + STSD_ENTRY_COUNT;
    for entry in iter_boxes(&stsd[body_start.min(stsd.len())..]) {
        if &entry[4..8] == b"encv" {
            let child_start = BOX_HEADER_MIN_SIZE + VISUAL_SAMPLE_ENTRY_HDR;
            if child_start <= entry.len() {
                if let Some(avcc) = iter_boxes(&entry[child_start..]).find(|b| &b[4..8] == b"avcC")
                {
                    // avcC full bytes → body after the 8-byte box header.
                    let cfg = crate::AVCConfigurationBox::parse_body(&avcc[BOX_HEADER_MIN_SIZE..])?;
                    return Ok(cfg.config);
                }
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
fn stsz_sizes(stbl: &[u8]) -> Result<Vec<usize>> {
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
    let mut sizes = Vec::with_capacity(count);
    if sample_size != 0 {
        for _ in 0..count {
            sizes.push(sample_size as usize);
        }
    } else {
        let table = base + 8;
        let end = table + count * 4;
        if stsz.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: stsz.len(),
                what: "stsz sample_size table",
            });
        }
        for i in 0..count {
            let o = table + i * 4;
            sizes.push(
                u32::from_be_bytes([stsz[o], stsz[o + 1], stsz[o + 2], stsz[o + 3]]) as usize,
            );
        }
    }
    Ok(sizes)
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
    let mut chunk_offsets = Vec::with_capacity(chunk_count);
    let co_table = sc_base + 4;
    if stco.len() < co_table + chunk_count * 4 {
        return Err(Error::BufferTooShort {
            need: co_table + chunk_count * 4,
            have: stco.len(),
            what: "stco chunk offsets",
        });
    }
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
    if stsc.len() < sc_table + entry_count * 12 {
        return Err(Error::BufferTooShort {
            need: sc_table + entry_count * 12,
            have: stsc.len(),
            what: "stsc entries",
        });
    }
    // Expand: samples_per_chunk for each chunk index (1-based).
    let mut samples_per_chunk = Vec::with_capacity(chunk_count);
    for c in 0..chunk_count {
        let chunk_no = (c + 1) as u32;
        // Find the applicable stsc run (last entry whose first_chunk <= chunk_no).
        let mut spc = 0u32;
        for e in 0..entry_count {
            let o = sc_table + e * 12;
            let first_chunk = u32::from_be_bytes([stsc[o], stsc[o + 1], stsc[o + 2], stsc[o + 3]]);
            let per = u32::from_be_bytes([stsc[o + 4], stsc[o + 5], stsc[o + 6], stsc[o + 7]]);
            if first_chunk <= chunk_no {
                spc = per;
            } else {
                break;
            }
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
