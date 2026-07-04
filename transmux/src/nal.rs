//! NAL-unit-type classification + IDR/IRAP keyframe detection for AVC/HEVC/VVC.
//!
//! One reusable helper the demuxers share instead of each hand-rolling keyframe
//! detection. Works on a single NAL unit (header already stripped of any
//! start-code prefix or length prefix) and on a whole access unit carried as an
//! **Annex B** byte stream (start code `00 00 01`) or a **4-byte length-prefixed**
//! buffer, reusing [`crate::annexb`].
//!
//! NAL header layouts and the keyframe/IRAP tables (all cited):
//!
//! - **H.264 / AVC** — ITU-T H.264 §7.3.1 (`nal_unit()`): 1-byte header,
//!   `nal_unit_type` = bits `[4:0]` of byte 0. Table 7-1: a coded slice of an
//!   **IDR** picture is type **5** (the AVC keyframe VCL type); a non-IDR coded
//!   slice is type 1.
//! - **H.265 / HEVC** — ITU-T H.265 §7.3.1.2 (`nal_unit_header()`): 2-byte
//!   header, `nal_unit_type` = bits `[6:1]` of byte 0, i.e. `(byte0 >> 1) & 0x3F`.
//!   Table 7-1: the **IRAP** VCL types are **16..=23** (BLA_W_LP … CRA_NUT,
//!   covering BLA / IDR_W_RADL=19 / IDR_N_LP=20 / CRA_NUT=21) — any of these
//!   starts a random-access point / keyframe.
//! - **H.266 / VVC** — ITU-T H.266 §7.3.1.2 (`nal_unit_header()`): 2-byte header,
//!   `nal_unit_type` = bits `[7:3]` of byte 1, i.e. `(byte1 >> 3) & 0x1F`.
//!   Table 5: the **IRAP** VCL types are **IDR_W_RADL = 7**, **IDR_N_LP = 8**,
//!   and **CRA_NUT = 9** — any of these starts a random-access point / keyframe.
//!
//! # Open-GOP AVC random-access points (issue #595)
//!
//! Broadcast H.264 is frequently **open-GOP**: it never codes an IDR (type 5)
//! at all. Each GOP instead opens with an SPS(7)/PPS(8) pair and a non-IDR
//! I-slice, usually announced by a `recovery_point` SEI message — ITU-T H.264
//! Annex D.1.7 (`recovery_point` syntax) / D.2.7 (semantics): the decoder is
//! guaranteed exact reconstruction `recovery_frame_cnt` pictures after this
//! one, which is exactly what a segmentation anchor needs. [`is_keyframe_nal`]
//! stays IDR-only (existing per-NAL callers keep that strict meaning), but
//! [`access_unit_is_rap`] additionally recognises an AVC access unit as a RAP
//! when it carries a `recovery_point` SEI ([`recovery_point_sei`]) or an SPS —
//! see that function's doc for the full policy. Segments opened this way are
//! non-IDR: correct for open-GOP decode and DASH-IF/CMAF-acceptable, but not a
//! "closed GOP" clean random-access point in the strict sense.

use alloc::vec::Vec;

use crate::annexb::{iter_annexb_nals, iter_length_prefixed_nals};

// ── AVC (ITU-T H.264 §7.3.1, Table 7-1) ─────────────────────────────────────

/// Mask for the H.264 5-bit `nal_unit_type` in header byte 0 (bits `[4:0]`).
const AVC_NAL_TYPE_MASK: u8 = 0x1F;
/// H.264 `nal_unit_type` for a coded slice of an **IDR** picture (Table 7-1).
const AVC_NAL_IDR: u8 = 5;
/// H.264 `nal_unit_type` for a supplemental enhancement information (SEI)
/// message (Table 7-1).
const AVC_NAL_SEI: u8 = 6;
/// H.264 `nal_unit_type` for a sequence parameter set (SPS) (Table 7-1).
const AVC_NAL_SPS: u8 = 7;

/// `payloadType` for the `recovery_point` SEI message — ITU-T H.264 Annex D
/// Table D-1 (syntax D.1.7, semantics D.2.7). Broadcast open-GOP H.264 sends
/// this instead of an IDR to mark a random-access point.
const AVC_SEI_PAYLOAD_TYPE_RECOVERY_POINT: u32 = 6;

// ── HEVC (ITU-T H.265 §7.3.1.2, Table 7-1) ──────────────────────────────────

/// Right shift to reach the HEVC 6-bit `nal_unit_type` in header byte 0.
const HEVC_NAL_TYPE_SHIFT: u8 = 1;
/// Mask for the HEVC 6-bit `nal_unit_type` after the shift.
const HEVC_NAL_TYPE_MASK: u8 = 0x3F;
/// First HEVC IRAP `nal_unit_type` (`BLA_W_LP`) — Table 7-1.
const HEVC_IRAP_FIRST: u8 = 16;
/// Last HEVC IRAP `nal_unit_type` (`RSV_IRAP_VCL23`) — Table 7-1.
const HEVC_IRAP_LAST: u8 = 23;
/// HEVC `nal_unit_type` for a prefix SEI NAL unit (`PREFIX_SEI_NUT`) — Table 7-1.
const HEVC_NAL_SEI_PREFIX: u8 = 39;
/// HEVC `nal_unit_type` for a suffix SEI NAL unit (`SUFFIX_SEI_NUT`) — Table 7-1.
const HEVC_NAL_SEI_SUFFIX: u8 = 40;

// ── VVC (ITU-T H.266 §7.3.1.2, Table 5) ─────────────────────────────────────

/// Right shift to reach the VVC 5-bit `nal_unit_type` in header byte 1.
const VVC_NAL_TYPE_SHIFT: u8 = 3;
/// Mask for the VVC 5-bit `nal_unit_type` after the shift.
const VVC_NAL_TYPE_MASK: u8 = 0x1F;
/// VVC `IDR_W_RADL` `nal_unit_type` — Table 5.
const VVC_NAL_IDR_W_RADL: u8 = 7;
/// VVC `IDR_N_LP` `nal_unit_type` — Table 5.
const VVC_NAL_IDR_N_LP: u8 = 8;
/// VVC `CRA_NUT` `nal_unit_type` — Table 5.
const VVC_NAL_CRA: u8 = 9;

/// The video codec family a NAL unit belongs to, selecting the NAL-header layout
/// and keyframe/IRAP table used to classify it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum NalCodec {
    /// H.264 / AVC — ITU-T H.264 (ISO/IEC 14496-10).
    Avc,
    /// H.265 / HEVC — ITU-T H.265 (ISO/IEC 23008-2).
    Hevc,
    /// H.266 / VVC — ITU-T H.266 (ISO/IEC 23090-3).
    Vvc,
}

impl NalCodec {
    /// The short spec token for this codec family.
    pub fn name(&self) -> &'static str {
        match self {
            NalCodec::Avc => "AVC",
            NalCodec::Hevc => "HEVC",
            NalCodec::Vvc => "VVC",
        }
    }

    /// Minimum NAL-header size in bytes: AVC has a 1-byte header, HEVC and VVC
    /// each have a 2-byte header.
    fn header_len(self) -> usize {
        match self {
            NalCodec::Avc => 1,
            NalCodec::Hevc | NalCodec::Vvc => 2,
        }
    }
}

broadcast_common::impl_spec_display!(NalCodec);

/// The raw `nal_unit_type` from a single NAL unit's header, or `None` if the
/// slice is shorter than that codec's NAL header.
///
/// `nal` must be one NAL unit with any start-code / length prefix already
/// removed (its first byte is the NAL header). Bit layout per
/// [module docs](crate::nal): AVC `byte0[4:0]`, HEVC `(byte0 >> 1) & 0x3F`,
/// VVC `(byte1 >> 3) & 0x1F`.
pub fn nal_unit_type(codec: NalCodec, nal: &[u8]) -> Option<u8> {
    if nal.len() < codec.header_len() {
        return None;
    }
    Some(match codec {
        NalCodec::Avc => nal[0] & AVC_NAL_TYPE_MASK,
        NalCodec::Hevc => (nal[0] >> HEVC_NAL_TYPE_SHIFT) & HEVC_NAL_TYPE_MASK,
        NalCodec::Vvc => (nal[1] >> VVC_NAL_TYPE_SHIFT) & VVC_NAL_TYPE_MASK,
    })
}

/// Whether a single NAL unit is an IDR/IRAP VCL type (a random-access keyframe
/// slice) for `codec`.
///
/// - AVC: type 5 (IDR slice).
/// - HEVC: types 16..=23 (IRAP: BLA / IDR / CRA).
/// - VVC: types 7 (IDR_W_RADL), 8 (IDR_N_LP), 9 (CRA_NUT).
///
/// Returns `false` for a slice too short to carry a NAL header.
pub fn is_keyframe_nal(codec: NalCodec, nal: &[u8]) -> bool {
    match nal_unit_type(codec, nal) {
        None => false,
        Some(t) => match codec {
            NalCodec::Avc => t == AVC_NAL_IDR,
            NalCodec::Hevc => (HEVC_IRAP_FIRST..=HEVC_IRAP_LAST).contains(&t),
            NalCodec::Vvc => t == VVC_NAL_IDR_W_RADL || t == VVC_NAL_IDR_N_LP || t == VVC_NAL_CRA,
        },
    }
}

/// Whether an access unit contains any keyframe/IRAP VCL NAL unit for `codec`.
///
/// Iterates the NAL units of `au` — as a 4-byte **length-prefixed** buffer when
/// `length_prefixed` is `true`, otherwise as an **Annex B** byte stream — and
/// returns `true` if any is a keyframe per [`is_keyframe_nal`]. A malformed
/// length-prefixed buffer (a declared length running past the end) yields
/// `false` rather than an error, since "no detectable keyframe" is the safe
/// answer for keyframe gating.
pub fn access_unit_is_keyframe(codec: NalCodec, au: &[u8], length_prefixed: bool) -> bool {
    if length_prefixed {
        match iter_length_prefixed_nals(au) {
            Ok(nals) => nals.iter().any(|nal| is_keyframe_nal(codec, nal)),
            Err(_) => false,
        }
    } else {
        iter_annexb_nals(au).any(|nal| is_keyframe_nal(codec, nal))
    }
}

/// Iterator over the emulation-prevention-unescaped bytes of an H.264 NAL body
/// (ITU-T H.264 §7.4.1 `emulation_prevention_three_byte`): every `00 00 03`
/// triplet in the input collapses to `00 00`, matching `unescape()` in
/// [`crate::bitreader`] but streamed byte-at-a-time so a SEI walk can stop
/// early without allocating a copy of the whole NAL.
struct EbspBytes<'a> {
    nal: &'a [u8],
    pos: usize,
    zero_run: u8,
}

impl<'a> EbspBytes<'a> {
    fn new(nal: &'a [u8]) -> Self {
        Self {
            nal,
            pos: 0,
            zero_run: 0,
        }
    }
}

impl Iterator for EbspBytes<'_> {
    type Item = u8;

    fn next(&mut self) -> Option<u8> {
        while self.pos < self.nal.len() {
            let b = self.nal[self.pos];
            self.pos += 1;
            if self.zero_run >= 2 && b == 0x03 {
                // emulation_prevention_three_byte: dropped, run resets.
                self.zero_run = 0;
                continue;
            }
            self.zero_run = if b == 0 { self.zero_run + 1 } else { 0 };
            return Some(b);
        }
        None
    }
}

/// Read one SEI `payloadType`/`payloadSize` varint (ITU-T H.264 §7.3.2.3.1
/// `sei_payload()`): a run of `0xFF` bytes (each worth 255) terminated by a
/// final byte `< 0xFF` that is added to the running total. Returns `None` if
/// the byte stream runs out before the terminating byte.
fn read_sei_varint(bytes: &mut impl Iterator<Item = u8>) -> Option<u32> {
    let mut value: u32 = 0;
    loop {
        let b = bytes.next()?;
        value += u32::from(b);
        if b != 0xFF {
            return Some(value);
        }
    }
}

/// Whether a single H.264 SEI NAL unit (type 6) carries a `recovery_point` SEI
/// message — ITU-T H.264 Annex D.1.7 (syntax) / D.2.7 (semantics): the
/// open-GOP random-access point signal broadcast encoders send instead of an
/// IDR.
///
/// Walks each `sei_message()` in the NAL's RBSP (§7.3.2.3 `sei_rbsp()`;
/// `payloadType` then `payloadSize`, each coded as a run of `0xFF` bytes
/// terminated by a final byte `< 0xFF`), skipping `payloadSize` bytes to
/// reach the next message — real broadcast
/// streams commonly pack `buffering_period`/`pic_timing`/`recovery_point`
/// together in one SEI NAL, so every message in the NAL is checked, not just
/// the first. Returns `false` for anything that isn't a type-6 NAL, is too
/// short, or fails to parse — SEI is non-VCL, so a malformed one is simply not
/// a recognised RAP signal rather than an error.
pub fn recovery_point_sei(nal: &[u8]) -> bool {
    if nal_unit_type(NalCodec::Avc, nal) != Some(AVC_NAL_SEI) {
        return false;
    }
    let mut bytes = EbspBytes::new(&nal[NalCodec::Avc.header_len()..]).peekable();
    loop {
        let Some(payload_type) = read_sei_varint(&mut bytes) else {
            return false;
        };
        let Some(payload_size) = read_sei_varint(&mut bytes) else {
            return false;
        };
        if payload_type == AVC_SEI_PAYLOAD_TYPE_RECOVERY_POINT {
            return true;
        }
        for _ in 0..payload_size {
            if bytes.next().is_none() {
                return false;
            }
        }
        // No more sei_message()s to try (whatever remains, if anything, is
        // rbsp_trailing_bits — ITU-T H.264 §7.3.2.3 `sei_rbsp()`).
        if bytes.peek().is_none() {
            return false;
        }
    }
}

/// Whether an access unit is a random-access point (RAP) — the segmentation
/// anchor signal — for `codec`.
///
/// - **HEVC / VVC**: identical to [`access_unit_is_keyframe`] (any IRAP NAL —
///   HEVC's IRAP range already covers the open-GOP CRA/BLA types, so no
///   extra signal is needed).
/// - **AVC**: broadcast H.264 is frequently open-GOP (issue #595): an AVC
///   access unit is a RAP when it contains ANY of:
///   1. an IDR NAL (type 5) — unchanged closed-GOP behaviour;
///   2. a `recovery_point` SEI ([`recovery_point_sei`]) — the spec-correct
///      open-GOP RAP signal (ITU-T H.264 Annex D.1.7/D.2.7); or
///   3. an SPS (type 7) — a pragmatic open-GOP fallback for streams that open
///      a GOP with SPS/PPS + a non-IDR I-slice but omit the recovery-point
///      SEI: a GOP-opening SPS in a broadcast video ES reliably marks a RAP
///      in practice.
///
///   Segments cut on case 2 or 3 open on a non-IDR access unit — correct for
///   open-GOP decode and DASH-IF/CMAF-acceptable, but not a "closed GOP"
///   clean random-access point in the strict ISO/IEC 14496-12 sync-sample
///   sense.
pub fn access_unit_is_rap(codec: NalCodec, au: &[u8], length_prefixed: bool) -> bool {
    match codec {
        NalCodec::Hevc | NalCodec::Vvc => access_unit_is_keyframe(codec, au, length_prefixed),
        NalCodec::Avc => {
            let is_rap_nal = |nal: &[u8]| {
                nal_unit_type(NalCodec::Avc, nal) == Some(AVC_NAL_SPS)
                    || is_keyframe_nal(NalCodec::Avc, nal)
                    || recovery_point_sei(nal)
            };
            if length_prefixed {
                match iter_length_prefixed_nals(au) {
                    Ok(nals) => nals.iter().any(|nal| is_rap_nal(nal)),
                    Err(_) => false,
                }
            } else {
                iter_annexb_nals(au).any(is_rap_nal)
            }
        }
    }
}

// ── ATSC A/53 caption SEI (issue #599) ──────────────────────────────────────

/// SEI `payloadType` for `user_data_registered_itu_t_t35` — ITU-T H.264 Annex D
/// Table D-1 (syntax D.1.6, semantics D.2.6) / ITU-T H.265 Annex D (same
/// registry: the SEI `payloadType` namespace is shared across H.264/H.265/H.266).
const SEI_PAYLOAD_TYPE_USER_DATA_REGISTERED_ITU_T_T35: u32 = 4;

/// `itu_t_t35_country_code` for the United States (ITU-T T.35 country code
/// table) — the fixed first byte of an ATSC caption SEI payload.
const ITU_T_T35_COUNTRY_CODE_USA: u8 = 0xB5;

/// `itu_t_t35_provider_code` identifying ATSC as the T.35 payload provider
/// (registered with the SMPTE RA). Well-established industry convention for
/// H.264/H.265 caption SEI (matches e.g. ffmpeg's `ff_alloc_a53_sei`); ATSC
/// A/53 Part 4 itself only specifies the MPEG-2 `user_data()` embedding
/// (§6.2.2/§6.2.3, no ITU-T T.35 wrapper — MPEG-2 has no SEI mechanism), so
/// this and the two constants below are the H.264/H.265 SEI-specific framing
/// that carries the same `ATSC_user_data()` payload (Table 6.7/6.8) inside it.
const ATSC_T35_PROVIDER_CODE: u16 = 0x0031;

/// `user_identifier` selecting `ATSC_user_data()` — ATSC A/53 Part 4 §6.2.3
/// Table 6.7, value `0x47413934` (`"GA94"`).
const ATSC_USER_IDENTIFIER_GA94: u32 = 0x4741_3934;

/// `user_data_type_code` selecting `MPEG_cc_data()` (i.e. `cc_data()` plus a
/// trailing `marker_bits` byte) — ATSC A/53 Part 4 §6.2.3 Table 6.9/6.10.
const ATSC_USER_DATA_TYPE_CODE_CC_DATA: u8 = 0x03;

/// Length in bytes of the T.35/ATSC header preceding `MPEG_cc_data()`:
/// `itu_t_t35_country_code`(1) + `itu_t_t35_provider_code`(2) +
/// `user_identifier`(4) + `user_data_type_code`(1).
const ATSC_T35_HEADER_LEN: usize = 8;

/// Whether `nal` is a SEI NAL unit for `codec` (AVC type 6; HEVC prefix/suffix
/// types 39/40). VVC has no caption SEI convention defined here, so it never
/// matches.
fn is_sei_nal(codec: NalCodec, nal: &[u8]) -> bool {
    match codec {
        NalCodec::Avc => nal_unit_type(codec, nal) == Some(AVC_NAL_SEI),
        NalCodec::Hevc => matches!(
            nal_unit_type(codec, nal),
            Some(HEVC_NAL_SEI_PREFIX) | Some(HEVC_NAL_SEI_SUFFIX)
        ),
        NalCodec::Vvc => false,
    }
}

/// If `payload` (one SEI message's `sei_payload()` bytes, already
/// EBSP-unescaped) is an ATSC A/53 caption SEI — `itu_t_t35_country_code`
/// 0xB5, provider 0x0031, `user_identifier` "GA94", `user_data_type_code`
/// 0x03 (§6.2.3 Tables 6.7-6.10) — append its `MPEG_cc_data()` bytes
/// (`cc_data()` + trailing `marker_bits`, sized from the `cc_count` in the
/// embedded `cc_data()` header) to `out`. Anything else — wrong signature,
/// too short, or a declared `cc_count` that doesn't fit in the payload — is
/// silently ignored: SEI is non-VCL, so a message that doesn't match is
/// simply not a recognised caption carrier rather than an error.
fn append_if_atsc_cc_data(payload: &[u8], out: &mut Vec<u8>) {
    if payload.len() < ATSC_T35_HEADER_LEN {
        return;
    }
    if payload[0] != ITU_T_T35_COUNTRY_CODE_USA {
        return;
    }
    let provider_code = u16::from_be_bytes([payload[1], payload[2]]);
    if provider_code != ATSC_T35_PROVIDER_CODE {
        return;
    }
    let user_identifier = u32::from_be_bytes([payload[3], payload[4], payload[5], payload[6]]);
    if user_identifier != ATSC_USER_IDENTIFIER_GA94 {
        return;
    }
    if payload[7] != ATSC_USER_DATA_TYPE_CODE_CC_DATA {
        return;
    }
    let cc = &payload[ATSC_T35_HEADER_LEN..];
    // cc_data() header (ETSI TS 101 154 Table B.9): byte0 low 5 bits = cc_count,
    // byte1 reserved; cc_count triplets of 3 bytes each; 1 trailing marker byte.
    const CC_DATA_HEADER_LEN: usize = 2;
    const CC_TRIPLET_LEN: usize = 3;
    const CC_DATA_MARKER_LEN: usize = 1;
    const CC_COUNT_MASK: u8 = 0x1F;
    if cc.len() < CC_DATA_HEADER_LEN {
        return;
    }
    let cc_count = usize::from(cc[0] & CC_COUNT_MASK);
    let total = CC_DATA_HEADER_LEN + cc_count * CC_TRIPLET_LEN + CC_DATA_MARKER_LEN;
    if cc.len() < total {
        return;
    }
    out.extend_from_slice(&cc[..total]);
}

/// Walk every `sei_message()` in one SEI NAL (§7.3.2.3 `sei_rbsp()` for AVC;
/// the equivalent HEVC/H.265 Annex D grammar), appending the `MPEG_cc_data()`
/// bytes of any ATSC A/53 caption message ([`append_if_atsc_cc_data`]) found
/// to `out`, in the order they appear in the NAL.
///
/// Reuses the `payloadType`/`payloadSize` varint walk and EBSP-unescape from
/// [`recovery_point_sei`] (issue #595) — real broadcast SEI NALs commonly pack
/// several `sei_message()`s (e.g. `buffering_period`/`pic_timing`/caption)
/// into one NAL, so every message is checked. Stops silently (no panic) on a
/// truncated or malformed NAL, matching that function's convention.
fn append_atsc_cc_data_from_sei_nal(codec: NalCodec, nal: &[u8], out: &mut Vec<u8>) {
    if !is_sei_nal(codec, nal) {
        return;
    }
    let mut bytes = EbspBytes::new(&nal[codec.header_len()..]).peekable();
    loop {
        let Some(payload_type) = read_sei_varint(&mut bytes) else {
            return;
        };
        let Some(payload_size) = read_sei_varint(&mut bytes) else {
            return;
        };
        let payload_size = payload_size as usize;
        if payload_type == SEI_PAYLOAD_TYPE_USER_DATA_REGISTERED_ITU_T_T35 {
            let payload: Vec<u8> = (&mut bytes).take(payload_size).collect();
            if payload.len() != payload_size {
                // Truncated mid-payload: no more bytes to find further
                // messages either.
                return;
            }
            append_if_atsc_cc_data(&payload, out);
        } else {
            for _ in 0..payload_size {
                if bytes.next().is_none() {
                    return;
                }
            }
        }
        // No more sei_message()s to try (whatever remains, if anything, is
        // rbsp_trailing_bits).
        if bytes.peek().is_none() {
            return;
        }
    }
}

/// Extract every ATSC A/53 caption SEI's `MPEG_cc_data()` bytes from an access
/// unit, concatenated in AU order (issue #599).
///
/// `au` is one access unit — a 4-byte **length-prefixed** buffer when
/// `length_prefixed` is `true` (the form [`crate::pipeline::Sample::data`]
/// uses for AVC/HEVC), otherwise an **Annex B** byte stream. Walks every NAL,
/// finds each `user_data_registered_itu_t_t35` SEI message (H.264 type 6 /
/// HEVC prefix/suffix types 39/40, `payloadType` 4) matching the ATSC A/53
/// signature (§6.2.3: country 0xB5, provider 0x0031, `user_identifier`
/// "GA94", `user_data_type_code` 0x03), and appends its `MPEG_cc_data()`
/// bytes — ready to feed straight into [`cc_data::CcData::parse`](https://docs.rs/cc-data)
/// (the crate `timed-metadata`'s `Cea608CueExtractor`/`Cea708CueExtractor`
/// consume) exactly as the PES-carried `cc_data()` path does.
///
/// Non-caption SEI messages (`recovery_point`, `pic_timing`,
/// `buffering_period`, …) and non-SEI NALs are ignored. A malformed
/// length-prefixed buffer, an empty AU, or an AU with no caption SEI all
/// yield an empty `Vec` rather than an error — "no captions in this AU" is
/// the expected common case, not a failure. VVC always yields empty: no
/// caption SEI convention is defined for it here.
pub fn caption_cc_data(codec: NalCodec, au: &[u8], length_prefixed: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if length_prefixed {
        if let Ok(nals) = iter_length_prefixed_nals(au) {
            for nal in nals {
                append_atsc_cc_data_from_sei_nal(codec, nal, &mut out);
            }
        }
    } else {
        for nal in iter_annexb_nals(au) {
            append_atsc_cc_data_from_sei_nal(codec, nal, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avc_nal_type_extraction() {
        // AVC SPS header byte: forbidden_zero(0) nal_ref_idc(11) type(00111=7).
        assert_eq!(nal_unit_type(NalCodec::Avc, &[0x67, 0x42]), Some(7));
        // AVC IDR slice: type 5 (00101), nal_ref_idc 11 → 0x65.
        assert_eq!(nal_unit_type(NalCodec::Avc, &[0x65, 0x88]), Some(5));
        assert!(is_keyframe_nal(NalCodec::Avc, &[0x65, 0x88]));
        assert!(!is_keyframe_nal(NalCodec::Avc, &[0x67, 0x42])); // SPS is not VCL keyframe
        // Non-IDR slice type 1 (0x41) is not a keyframe.
        assert!(!is_keyframe_nal(NalCodec::Avc, &[0x41, 0x9a]));
        // Empty slice → None / false.
        assert_eq!(nal_unit_type(NalCodec::Avc, &[]), None);
        assert!(!is_keyframe_nal(NalCodec::Avc, &[]));
    }

    #[test]
    fn hevc_nal_type_extraction() {
        // HEVC 2-byte header. type = (byte0 >> 1) & 0x3f.
        // SPS = 33 → byte0 = 33<<1 = 0x42.
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x42, 0x01]), Some(33));
        assert!(!is_keyframe_nal(NalCodec::Hevc, &[0x42, 0x01]));
        // IDR_W_RADL = 19 → 0x26; IDR_N_LP = 20 → 0x28.
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x26, 0x01]), Some(19));
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x28, 0x01]), Some(20));
        assert!(is_keyframe_nal(NalCodec::Hevc, &[0x26, 0x01]));
        assert!(is_keyframe_nal(NalCodec::Hevc, &[0x28, 0x01]));
        // TRAIL_R = 1 → 0x02 is not IRAP.
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x02, 0x01]), Some(1));
        assert!(!is_keyframe_nal(NalCodec::Hevc, &[0x02, 0x01]));
        // A 1-byte slice is too short for HEVC's 2-byte header.
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x26]), None);
    }

    #[test]
    fn vvc_nal_type_extraction() {
        // VVC 2-byte header. type = (byte1 >> 3) & 0x1f.
        // A real vvenc SPS NAL header pair is [0x00, byte1] with type SPS=15 →
        // byte1 = 15<<3 = 0x78.
        assert_eq!(nal_unit_type(NalCodec::Vvc, &[0x00, 0x78]), Some(15));
        assert!(!is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x78]));
        // IDR_W_RADL = 7 → byte1 = 7<<3 = 0x38; IDR_N_LP = 8 → 0x40; CRA = 9 → 0x48.
        assert_eq!(nal_unit_type(NalCodec::Vvc, &[0x00, 0x38]), Some(7));
        assert_eq!(nal_unit_type(NalCodec::Vvc, &[0x00, 0x40]), Some(8));
        assert_eq!(nal_unit_type(NalCodec::Vvc, &[0x00, 0x48]), Some(9));
        assert!(is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x38]));
        assert!(is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x40]));
        assert!(is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x48]));
        // TRAIL = 0 → byte1 = 0x00 is not IRAP.
        assert!(!is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x00]));
    }

    #[test]
    fn access_unit_annexb_and_length_prefixed_agree() {
        // Annex B AU: SPS (67) + non-IDR slice (41) → no keyframe.
        let non_kf = [0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x01, 0x41, 0x9a];
        assert!(!access_unit_is_keyframe(NalCodec::Avc, &non_kf, false));
        // Annex B AU: SPS (67) + IDR slice (65) → keyframe.
        let kf = [0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x01, 0x65, 0x88];
        assert!(access_unit_is_keyframe(NalCodec::Avc, &kf, false));

        // Length-prefixed form of the same AUs yields the same verdict.
        let lp_non_kf = crate::annexb::annexb_to_length_prefixed(&non_kf);
        let lp_kf = crate::annexb::annexb_to_length_prefixed(&kf);
        assert!(!access_unit_is_keyframe(NalCodec::Avc, &lp_non_kf, true));
        assert!(access_unit_is_keyframe(NalCodec::Avc, &lp_kf, true));
    }

    #[test]
    fn malformed_length_prefixed_is_not_keyframe() {
        // Declares a 99-byte NAL in a 6-byte buffer.
        let lp = [0x00, 0x00, 0x00, 0x63, 0x65, 0x88];
        assert!(!access_unit_is_keyframe(NalCodec::Avc, &lp, true));
    }

    #[test]
    fn nal_codec_display() {
        assert_eq!(NalCodec::Avc.to_string(), "AVC");
        assert_eq!(NalCodec::Hevc.to_string(), "HEVC");
        assert_eq!(NalCodec::Vvc.to_string(), "VVC");
    }

    // ── Open-GOP RAP detection (issue #595) ─────────────────────────────────

    #[test]
    fn recovery_point_sei_matches_payload_type_6() {
        // SEI NAL: header(0x06), payloadType=6 (recovery_point), payloadSize=0.
        let nal = [0x06, 0x06, 0x00];
        assert!(recovery_point_sei(&nal));
    }

    #[test]
    fn other_sei_payload_type_is_not_recovery_point() {
        // SEI NAL: header(0x06), payloadType=1 (pic_timing), payloadSize=0.
        let nal = [0x06, 0x01, 0x00];
        assert!(!recovery_point_sei(&nal));
    }

    #[test]
    fn recovery_point_sei_found_among_multiple_messages() {
        // One SEI NAL packing buffering_period(0, size 1, payload 0xAA), then
        // recovery_point(6, size 0) — matches real broadcast streams that pack
        // several sei_message()s into a single NAL.
        let nal = [0x06, 0x00, 0x01, 0xAA, 0x06, 0x00];
        assert!(recovery_point_sei(&nal));
    }

    #[test]
    fn recovery_point_sei_rejects_non_sei_nal() {
        // An SPS (type 7), not a SEI (type 6).
        let nal = [0x67, 0x42, 0x00];
        assert!(!recovery_point_sei(&nal));
        // Too short to even carry a payloadType byte.
        let short_sei = [0x06];
        assert!(!recovery_point_sei(&short_sei));
    }

    #[test]
    fn recovery_point_sei_unescapes_emulation_prevention() {
        // payloadSize's 0xFF-run byte is followed by an emulation-prevention
        // triplet (00 00 03) that must collapse to (00 00) before summing.
        // payloadType=6, payloadSize = 0xFF + 0x00 = 255 (leading run byte
        // 0xFF, terminator 0x00 straight after — no EPB needed for the size
        // itself, but the EPB in the trailing zero run must still unescape
        // correctly for later bytes to align). Use a simpler direct check:
        // a `00 00 03 00` sequence inside the payload must not desync the
        // byte count used to find the *next* message.
        let nal = [
            0x06, // SEI NAL header
            0x00, 0x02, 0x00, 0x00,
            0x03, // buffering_period: type 0, size 2, payload [00 00] (EPB'd as 00 00 03)
            0x06, 0x00, // recovery_point: type 6, size 0
        ];
        assert!(recovery_point_sei(&nal));
    }

    #[test]
    fn access_unit_is_rap_recognises_open_gop_signals() {
        // AVC AU with SPS(7) + PPS(8) + non-IDR I-slice(1), no IDR at all: the
        // SPS-present fallback marks it a RAP.
        let sps_led = [
            0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x0A, // SPS
            0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80, // PPS
            0x00, 0x00, 0x01, 0x41, 0x9A, // non-IDR slice
        ];
        assert!(access_unit_is_rap(NalCodec::Avc, &sps_led, false));
        // The strict IDR-only helper does NOT consider this a keyframe —
        // proves the two helpers now diverge for open-GOP input.
        assert!(!access_unit_is_keyframe(NalCodec::Avc, &sps_led, false));

        // AVC AU with only a recovery-point SEI + non-IDR slice (no SPS in
        // this AU — e.g. SPS was only sent once at stream start): still a RAP.
        // The SEI NAL ends with the `rbsp_trailing_bits` stop-bit byte (0x80)
        // — a real `sei_rbsp()` always does, and `iter_annexb_nals` strips
        // trailing `zero_byte` padding, so a NAL ending in the `payloadSize`
        // byte's `0x00` would otherwise be (correctly) trimmed away.
        let sei_led = [
            0x00, 0x00, 0x01, 0x06, 0x06, 0x00, 0x80, // SEI: recovery_point, size 0
            0x00, 0x00, 0x01, 0x41, 0x9A, // non-IDR slice
        ];
        assert!(access_unit_is_rap(NalCodec::Avc, &sei_led, false));
        assert!(!access_unit_is_keyframe(NalCodec::Avc, &sei_led, false));

        // A plain non-IDR AU with neither signal is not a RAP.
        let plain = [0x00, 0x00, 0x01, 0x41, 0x9A];
        assert!(!access_unit_is_rap(NalCodec::Avc, &plain, false));

        // IDR-only AU is still recognised (closed-GOP behaviour unchanged).
        let idr = [0x00, 0x00, 0x01, 0x65, 0x88];
        assert!(access_unit_is_rap(NalCodec::Avc, &idr, false));
        assert!(access_unit_is_keyframe(NalCodec::Avc, &idr, false));
    }

    #[test]
    fn access_unit_is_rap_hevc_matches_keyframe_helper() {
        // HEVC/VVC are untouched by #595: access_unit_is_rap must agree with
        // access_unit_is_keyframe exactly.
        let cra = [0x00, 0x00, 0x01, 0x2A, 0x01]; // type 21 (CRA_NUT) = 21<<1=0x2A
        assert!(access_unit_is_keyframe(NalCodec::Hevc, &cra, false));
        assert!(access_unit_is_rap(NalCodec::Hevc, &cra, false));

        let trail = [0x00, 0x00, 0x01, 0x02, 0x01]; // type 1 (TRAIL_R)
        assert!(!access_unit_is_keyframe(NalCodec::Hevc, &trail, false));
        assert!(!access_unit_is_rap(NalCodec::Hevc, &trail, false));
    }

    // ── ATSC A/53 caption SEI (issue #599) ──────────────────────────────────

    /// One complete H.264 SEI NAL, byte-for-byte extracted from the real
    /// `samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2885/transformers_EIA608_H264.ts`
    /// capture (first `user_data_registered_itu_t_t35` SEI in the stream):
    /// `00 00 01` start code, NAL header `06` (SEI), `payloadType` `04`
    /// (user_data_registered_itu_t_t35), `payloadSize` `0x47` (71), the
    /// 71-byte payload — T.35 header (country `0xB5`, provider `0x0031`,
    /// `"GA94"`, type `0x03`) + a 63-byte `MPEG_cc_data()` (`cc_count` = 20) —
    /// then the real stream's trailing bytes through the NAL's actual
    /// `rbsp_trailing_bits` stop byte (`0x80`): a second, uninteresting
    /// `sei_message()` this encoder packs into the same NAL. Extending the
    /// fixture that far (rather than cutting it exactly at the T.35 payload
    /// boundary) matters: [`iter_annexb_nals`] strips trailing `zero_byte`
    /// padding from a NAL (real `rbsp_trailing_bits` always ends non-zero),
    /// and this capture's raw bytes happen to end the T.35 payload on `00 00`
    /// — cutting there would make the fixture *look* real while silently
    /// exercising a code path the real stream never hits.
    #[rustfmt::skip]
    const REAL_A53_SEI_NAL: [u8; 82] = [
        0x00, 0x00, 0x01, 0x06, 0x04, 0x47, 0xb5, 0x00, 0x31, 0x47, 0x41, 0x39,
        0x34, 0x03, 0xd4, 0xff, 0xfc, 0x80, 0x80, 0xfd, 0x80, 0x80, 0xfa, 0x00,
        0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00,
        0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00,
        0x00, 0xfa, 0x47, 0x01, 0xe1, 0x12, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa,
        0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa,
        0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xff, 0x80,
    ];

    /// The expected `MPEG_cc_data()` bytes: `REAL_A53_SEI_NAL`'s last 63 bytes
    /// (everything after the 8-byte T.35/`GA94`/type-code header).
    #[rustfmt::skip]
    const REAL_A53_CC_DATA: [u8; 63] = [
        0xd4, 0xff, 0xfc, 0x80, 0x80, 0xfd, 0x80, 0x80, 0xfa, 0x00, 0x00, 0xfa,
        0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa,
        0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa,
        0x47, 0x01, 0xe1, 0x12, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00,
        0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00, 0xfa, 0x00, 0x00,
        0xfa, 0x00, 0x00,
    ];

    #[test]
    fn caption_cc_data_extracts_real_atsc_a53_sei() {
        let extracted = caption_cc_data(NalCodec::Avc, &REAL_A53_SEI_NAL, false);
        assert_eq!(extracted, REAL_A53_CC_DATA);
    }

    #[test]
    fn caption_cc_data_finds_sei_alongside_other_nals() {
        // A non-IDR slice before the SEI, and another after — the extractor
        // must not be thrown off by surrounding VCL NALs, and must return
        // exactly the caption bytes (not, e.g., duplicated).
        let mut au = alloc::vec::Vec::new();
        au.extend_from_slice(&[0x00, 0x00, 0x01, 0x41, 0x9a]); // non-IDR slice
        au.extend_from_slice(&REAL_A53_SEI_NAL);
        au.extend_from_slice(&[0x00, 0x00, 0x01, 0x41, 0x9b]); // non-IDR slice
        let extracted = caption_cc_data(NalCodec::Avc, &au, false);
        assert_eq!(extracted, REAL_A53_CC_DATA);
    }

    #[test]
    fn caption_cc_data_length_prefixed_matches_annexb() {
        let lp = crate::annexb::annexb_to_length_prefixed(&REAL_A53_SEI_NAL);
        let extracted = caption_cc_data(NalCodec::Avc, &lp, true);
        assert_eq!(extracted, REAL_A53_CC_DATA);
    }

    #[test]
    fn caption_cc_data_ignores_non_caption_sei() {
        // recovery_point SEI (payloadType 6) — a real, common SEI message —
        // must not be mistaken for a caption SEI (bites the payloadType
        // discrimination, not just the T.35 signature check).
        let recovery_point = [0x00, 0x00, 0x01, 0x06, 0x06, 0x00];
        assert!(caption_cc_data(NalCodec::Avc, &recovery_point, false).is_empty());

        // pic_timing SEI (payloadType 1) with an arbitrary payload.
        let pic_timing = [0x00, 0x00, 0x01, 0x06, 0x01, 0x02, 0xAA, 0xBB];
        assert!(caption_cc_data(NalCodec::Avc, &pic_timing, false).is_empty());

        // user_data_registered_itu_t_t35 (payloadType 4) but the wrong
        // provider/country signature (not ATSC/GA94) — must not match.
        let wrong_signature = [
            0x00, 0x00, 0x01, 0x06, // start code + SEI header
            0x04, 0x08, // payloadType 4, payloadSize 8
            0xB5, 0x00, 0x99, b'X', b'X', b'X', b'X', 0x03, // wrong provider
        ];
        assert!(caption_cc_data(NalCodec::Avc, &wrong_signature, false).is_empty());
    }

    #[test]
    fn caption_cc_data_rejects_declared_cc_count_overrunning_payload() {
        // A syntactically-valid T.35/GA94 header whose cc_data() declares
        // more triplets (cc_count) than fit in the SEI payload — must be
        // ignored, not read out of bounds / truncated silently as if valid.
        let truncated = [
            0x00, 0x00, 0x01, 0x06, // start code + SEI header
            0x04, 0x0A, // payloadType 4, payloadSize 10
            0xB5, 0x00, 0x31, 0x47, 0x41, 0x39, 0x34, 0x03, // T.35 + GA94 + type 0x03
            0x9F, 0xFF, // cc_data() header: cc_count = 31 (needs 96 more bytes, has 0)
        ];
        assert!(caption_cc_data(NalCodec::Avc, &truncated, false).is_empty());
    }

    #[test]
    fn caption_cc_data_hevc_prefix_and_suffix_sei() {
        // Re-wrap the real payload's T.35 body in a 2-byte HEVC SEI header
        // (prefix type 39, then suffix type 40) instead of the 1-byte AVC
        // header — proves the codec dispatch, not just the AVC path.
        let t35_and_cc_data = &REAL_A53_SEI_NAL[6..]; // payload bytes only (after payloadSize)
        for (label, nal_type) in [
            ("prefix", HEVC_NAL_SEI_PREFIX),
            ("suffix", HEVC_NAL_SEI_SUFFIX),
        ] {
            let mut au = alloc::vec::Vec::new();
            au.extend_from_slice(&[0x00, 0x00, 0x01]);
            au.push(nal_type << HEVC_NAL_TYPE_SHIFT); // HEVC NAL header byte 0
            au.push(0x01); // HEVC NAL header byte 1 (layer_id/temporal_id, arbitrary)
            au.push(0x04); // payloadType 4
            au.push(0x47); // payloadSize 71
            au.extend_from_slice(t35_and_cc_data);
            let extracted = caption_cc_data(NalCodec::Hevc, &au, false);
            assert_eq!(extracted, REAL_A53_CC_DATA, "HEVC {label} SEI");
        }
    }

    #[test]
    fn caption_cc_data_vvc_is_always_empty() {
        // No caption SEI convention is defined for VVC here — even a byte
        // sequence that would match on AVC/HEVC must yield nothing.
        let extracted = caption_cc_data(NalCodec::Vvc, &REAL_A53_SEI_NAL, false);
        assert!(extracted.is_empty());
    }

    #[test]
    fn caption_cc_data_no_panic_on_arbitrary_and_truncated_bytes() {
        // A grab-bag of adversarial inputs: empty, single bytes, runs of
        // 0xFF (pathological SEI varints), a truncated real SEI NAL at every
        // possible cut point, and a malformed length-prefixed declaration.
        // None of these may panic; every one is expected to yield "no
        // captions found" (possibly non-empty in a way we don't assert here —
        // the property under test is "doesn't panic", not a specific value).
        for codec in [NalCodec::Avc, NalCodec::Hevc, NalCodec::Vvc] {
            let _ = caption_cc_data(codec, &[], false);
            let _ = caption_cc_data(codec, &[], true);
            let _ = caption_cc_data(codec, &[0x06], false);
            let _ = caption_cc_data(codec, &[0xFF; 32], false);
            let _ = caption_cc_data(codec, &[0x00, 0x00, 0x01, 0x06, 0xFF, 0xFF, 0xFF], false);
            for cut in 0..=REAL_A53_SEI_NAL.len() {
                let _ = caption_cc_data(codec, &REAL_A53_SEI_NAL[..cut], false);
            }
            // Malformed length-prefixed buffer: declares a length past the
            // end of the buffer.
            let bad_lp = [0x00, 0x00, 0x00, 0x63, 0x06, 0x04];
            let _ = caption_cc_data(codec, &bad_lp, true);
        }
    }
}
