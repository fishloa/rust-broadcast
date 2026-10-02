//! Streaming Annex B byte stream → access-unit splitter — ITU-T H.264 §7.4.1.2
//! (access unit detection), ITU-T H.265 §7.4.2.4.4, ITU-T H.266 §7.4.2.4.3.
//!
//! An IP-camera SoC encoder emits a **continuous Annex B byte stream** (NAL
//! units separated by `00 00 01` start codes) with no TS/PES framing. To feed it
//! into the neutral IR one access unit (one coded picture, with its leading
//! parameter-set / SEI / AUD NALs) at a time, the byte stream has to be split at
//! access-unit boundaries incrementally, as bytes arrive.
//!
//! [`AccessUnitSplitter`] buffers pushed bytes and emits each complete access
//! unit as soon as the *next* AU's first NAL is seen (a NAL is only complete once
//! the following start code arrives, so emission always lags by one AU until
//! [`AccessUnitSplitter::finish`]). Unlike the one-shot AUD-only splitter in
//! `ps_demux`, this is public, streaming, codec-aware, and does not require the
//! stream to carry access-unit delimiters.
//!
//! ## Boundary rule
//!
//! A new access unit begins at the first of these NAL units, once the current
//! access unit already contains a VCL (coded-slice) NAL (H.264 §7.4.1.2.4):
//!
//! - an **access unit delimiter** (AVC type 9 / HEVC `AUD_NUT` 35 / VVC
//!   `AUD_NUT` 20) — by definition the first NAL of an AU;
//! - a **VCL** NAL that is the **first slice of a new picture** (AVC
//!   `first_mb_in_slice` == 0 / HEVC `first_slice_segment_in_pic_flag` == 1);
//! - any **non-VCL** NAL (SPS/PPS/VPS/SEI …) — the leading NALs of the next AU.
//!
//! The emitted AU bytes are the verbatim Annex B slice (start codes included), so
//! concatenating every emitted AU reproduces the input byte stream from its first
//! start code onward (leading bytes before the first start code are dropped, as
//! they carry no NAL). VVC first-slice detection is not implemented (its slice
//! header layout differs); VVC streams split on AUD / non-VCL boundaries only.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::error::{Error, Result as AuResult};
use crate::nal::{NalCodec, nal_unit_type};

// ── NAL-type classification (VCL ranges + AUD), per each codec's Table 7-1 / 5 ──

/// AVC lowest VCL `nal_unit_type` (coded slice) — ITU-T H.264 Table 7-1.
const AVC_VCL_MIN: u8 = 1;
/// AVC highest VCL `nal_unit_type` (IDR slice) — ITU-T H.264 Table 7-1.
const AVC_VCL_MAX: u8 = 5;
/// AVC access-unit-delimiter `nal_unit_type` — ITU-T H.264 Table 7-1.
const AVC_AUD: u8 = 9;
/// AVC `end_of_sequence` — ITU-T H.264 Table 7-1. May trail the last VCL of an
/// access unit (H.264 §7.4.1.2.3).
const AVC_END_OF_SEQ: u8 = 10;
/// AVC `end_of_stream` — ITU-T H.264 Table 7-1; likewise a trailing NAL.
const AVC_END_OF_STREAM: u8 = 11;
/// AVC `filler_data` — ITU-T H.264 Table 7-1; likewise a trailing NAL.
const AVC_FILLER_DATA: u8 = 12;

/// HEVC highest VCL `nal_unit_type` (`RSV_VCL31`) — ITU-T H.265 Table 7-1
/// (VCL types are 0..=31).
const HEVC_VCL_MAX: u8 = 31;
/// HEVC access-unit-delimiter `nal_unit_type` (`AUD_NUT`) — ITU-T H.265 Table 7-1.
const HEVC_AUD: u8 = 35;
/// HEVC `EOS_NUT` — ITU-T H.265 Table 7-1; may trail the last VCL (H.265
/// §7.4.2.4.4).
const HEVC_EOS: u8 = 36;
/// HEVC `EOB_NUT` — ITU-T H.265 Table 7-1; likewise trailing.
const HEVC_EOB: u8 = 37;
/// HEVC `FD_NUT` (filler data) — ITU-T H.265 Table 7-1; likewise trailing.
const HEVC_FD: u8 = 38;
/// HEVC `SUFFIX_SEI_NUT` — ITU-T H.265 Table 7-1; belongs to the picture that
/// precedes it.
const HEVC_SUFFIX_SEI: u8 = 40;
/// HEVC lowest reserved non-VCL `nal_unit_type` (`RSV_NVCL45`); 45..=47 are
/// suffix-classified (H.265 §7.4.2.4.4).
const HEVC_RSV_NVCL_SUFFIX_MIN: u8 = 45;
/// HEVC highest reserved non-VCL `nal_unit_type` (`RSV_NVCL47`).
const HEVC_RSV_NVCL_SUFFIX_MAX: u8 = 47;
/// HEVC lowest unspecified `nal_unit_type` (`UNSPEC56`); 56..=63 are
/// suffix-classified when they follow a VCL (H.265 §7.4.2.4.4).
const HEVC_UNSPEC_SUFFIX_MIN: u8 = 56;

/// VVC highest VCL `nal_unit_type` (`RSV_VCL_11`) — ITU-T H.266 Table 5
/// (VCL types are 0..=11).
const VVC_VCL_MAX: u8 = 11;
/// VVC access-unit-delimiter `nal_unit_type` (`AUD_NUT`) — ITU-T H.266 Table 5.
const VVC_AUD: u8 = 20;
/// VVC `EOS_NUT` — ITU-T H.266 Table 5; may trail the last VCL (H.266
/// §7.4.2.4.3).
const VVC_EOS: u8 = 21;
/// VVC `EOB_NUT` — ITU-T H.266 Table 5; likewise trailing.
const VVC_EOB: u8 = 22;
/// VVC `SUFFIX_APS_NUT` — ITU-T H.266 Table 5; belongs to the preceding picture.
/// (`PREFIX_APS_NUT` 17 is deliberately absent: a prefix APS starts an AU.)
const VVC_SUFFIX_APS: u8 = 18;
/// VVC `SUFFIX_SEI_NUT` — ITU-T H.266 Table 5; belongs to the preceding picture.
/// (`PREFIX_SEI_NUT` 23 is deliberately absent: a prefix SEI starts an AU.)
const VVC_SUFFIX_SEI: u8 = 24;
/// VVC `FD_NUT` (filler data) — ITU-T H.266 Table 5; likewise trailing.
const VVC_FD: u8 = 25;

/// Upper bound on bytes buffered for a single still-open NAL, and on the size
/// of one assembled access unit.
///
/// A NAL only completes when the *next* start code arrives, so a stream that
/// never emits one — or one whose declared picture runs to gigabytes — would
/// otherwise grow `buf`/`au` until the process runs out of memory. The bound is
/// far above any real coded picture (the largest H.264 level allows ~139 MiB
/// per frame at the top level, and broadcast pictures are orders of magnitude
/// smaller), so a conformant stream never reaches it.
const MAX_OPEN_NAL_BYTES: usize = 64 * 1024 * 1024;

/// NAL-header length in bytes: AVC 1, HEVC/VVC 2.
fn header_len(codec: NalCodec) -> usize {
    match codec {
        NalCodec::Avc => 1,
        NalCodec::Hevc | NalCodec::Vvc => 2,
    }
}

/// Whether `t` is a VCL (coded-slice) `nal_unit_type` for `codec`.
fn is_vcl(codec: NalCodec, t: u8) -> bool {
    match codec {
        NalCodec::Avc => (AVC_VCL_MIN..=AVC_VCL_MAX).contains(&t),
        NalCodec::Hevc => t <= HEVC_VCL_MAX,
        NalCodec::Vvc => t <= VVC_VCL_MAX,
    }
}

/// Whether `t` is the access-unit-delimiter `nal_unit_type` for `codec`.
fn is_aud(codec: NalCodec, t: u8) -> bool {
    match codec {
        NalCodec::Avc => t == AVC_AUD,
        NalCodec::Hevc => t == HEVC_AUD,
        NalCodec::Vvc => t == VVC_AUD,
    }
}

/// Whether a non-VCL `t` may **trail** the last VCL of the access unit it
/// follows, i.e. belongs to that same picture rather than starting the next one.
///
/// Without this, a trailing NAL after the last slice opens a new access unit
/// that the next AUD closes immediately, so a CBR AVC stream with filler emits
/// an alternating AU-per-picture / AU-of-filler sequence, and every HEVC suffix
/// SEI (decoded-picture-hash, …) is attributed to the *next* picture.
///
/// - H.264 §7.4.1.2.3: `end_of_sequence` (10), `end_of_stream` (11) and
///   `filler_data` (12) are the trailing non-VCL NALs.
/// - H.265 §7.4.2.4.4: `EOS_NUT` (36), `EOB_NUT` (37), `FD_NUT` (38),
///   `SUFFIX_SEI_NUT` (40), `RSV_NVCL45..47` and `UNSPEC56..63`.
/// - H.266 §7.4.2.4.3: `EOS_NUT` (21), `EOB_NUT` (22), `SUFFIX_APS_NUT` (18),
///   `SUFFIX_SEI_NUT` (24) and `FD_NUT` (25). VVC's prefix/suffix APS and SEI
///   are *both* explicitly allowed after a VCL, so the two prefix types
///   (`PREFIX_APS_NUT` 17, `PREFIX_SEI_NUT` 23) are excluded here — they always
///   begin an access unit.
fn is_suffix(codec: NalCodec, t: u8) -> bool {
    match codec {
        NalCodec::Avc => matches!(t, AVC_END_OF_SEQ | AVC_END_OF_STREAM | AVC_FILLER_DATA),
        NalCodec::Hevc => {
            matches!(t, HEVC_EOS | HEVC_EOB | HEVC_FD | HEVC_SUFFIX_SEI)
                || (HEVC_RSV_NVCL_SUFFIX_MIN..=HEVC_RSV_NVCL_SUFFIX_MAX).contains(&t)
                || t >= HEVC_UNSPEC_SUFFIX_MIN
        }
        NalCodec::Vvc => matches!(
            t,
            VVC_EOS | VVC_EOB | VVC_SUFFIX_APS | VVC_SUFFIX_SEI | VVC_FD
        ),
    }
}

/// Whether a VCL NAL is the first slice of a new coded picture.
///
/// AVC `first_mb_in_slice` is the leading `ue(v)` of the slice header; value 0
/// (a new picture's first slice) encodes as a single `1` bit, so it is the top
/// bit of the first RBSP byte. HEVC `first_slice_segment_in_pic_flag` is the
/// leading `u(1)` of the slice-segment header — likewise the top bit of the
/// first RBSP byte. VVC uses a different slice-header layout, so its first-slice
/// flag is not derived here (returns `false`).
fn first_slice_of_picture(codec: NalCodec, nal_body: &[u8]) -> bool {
    let hl = header_len(codec);
    match codec {
        NalCodec::Avc | NalCodec::Hevc => nal_body.get(hl).is_some_and(|b| b & 0x80 != 0),
        NalCodec::Vvc => false,
    }
}

// ── start-code scanning ────────────────────────────────────────────────────────

/// First-byte offset of the first NAL in `data`: the position of a `00 00 01`
/// code pulled back over any immediately-preceding `zero_byte`s (which belong to
/// that NAL). `None` when `data` holds no start code.
pub(crate) fn first_nal_start(data: &[u8]) -> Option<usize> {
    let n = data.len();
    let mut p = 0usize;
    while p + 3 <= n {
        if data[p] == 0 && data[p + 1] == 0 && data[p + 2] == 1 {
            let mut s = p;
            while s > 0 && data[s - 1] == 0 {
                s -= 1;
            }
            return Some(s);
        }
        p += 1;
    }
    None
}

// ── the splitter ───────────────────────────────────────────────────────────────

/// Incremental Annex B → access-unit splitter (see [module docs](self)).
///
/// Push bytes with [`push`](Self::push); drain completed access units with
/// [`pop`](Self::pop). Call [`finish`](Self::finish) at end of stream to complete
/// the trailing NAL and flush the final access unit.
pub struct AccessUnitSplitter {
    codec: NalCodec,
    /// Unconsumed Annex B bytes; begins at a start code once primed.
    buf: Vec<u8>,
    /// Whether the first start code has been located (leading junk dropped).
    primed: bool,
    /// Start-code scan cursor into `buf` (r04-W3): positions below it have
    /// already been proven not to begin a start code, so a push only walks the
    /// new bytes (plus the last 2, where a split start code can complete).
    scanned: usize,
    /// Bytes of the access unit currently being assembled (Annex B, verbatim).
    au: Vec<u8>,
    /// Whether `au` already contains a VCL NAL.
    au_has_vcl: bool,
    /// Completed access units awaiting [`pop`](Self::pop).
    ready: VecDeque<Vec<u8>>,
    /// Total bytes walked by the start-code scan, across every `push`.
    ///
    /// Present only in test builds, where it pins the r04-W3 guarantee: the
    /// scan is O(bytes fed), never O(pushes x bytes buffered).
    #[cfg(test)]
    scanned_bytes: usize,
}

impl AccessUnitSplitter {
    /// Create a splitter for `codec`.
    pub fn new(codec: NalCodec) -> Self {
        Self {
            codec,
            buf: Vec::new(),
            primed: false,
            scanned: 0,
            au: Vec::new(),
            au_has_vcl: false,
            ready: VecDeque::new(),
            #[cfg(test)]
            scanned_bytes: 0,
        }
    }

    /// Append `bytes` of the Annex B stream and split off any access units that
    /// became complete. Completed units are queued for [`pop`](Self::pop).
    pub fn push(&mut self, bytes: &[u8]) -> AuResult<()> {
        self.buf.extend_from_slice(bytes);
        self.drain_complete_nals()
    }

    /// Pop the next completed access unit, if any.
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        self.ready.pop_front()
    }

    /// Flush at end of stream: complete the trailing NAL and emit the final
    /// access unit. After this, [`pop`](Self::pop) drains everything remaining.
    pub fn finish(&mut self) {
        // The trailing bytes in `buf` (from the last start code) are now a
        // complete NAL — process it as one final range.
        if self.primed && !self.buf.is_empty() {
            let range = core::mem::take(&mut self.buf);
            self.process_nal(&range);
        }
        if !self.au.is_empty() {
            self.ready.push_back(core::mem::take(&mut self.au));
            self.au_has_vcl = false;
        }
    }

    /// Split every NAL whose following start code is already buffered, retaining
    /// the trailing (incomplete) NAL for the next push.
    ///
    /// The start-code scan resumes at [`Self::scanned`] instead of rewalking the
    /// whole buffer (r04-W3): a 1 MiB IDR fed in 1316-byte datagrams used to be
    /// rescanned ~800 times. Each complete NAL is handed to
    /// [`Self::process_nal`] as a slice of the buffer, so it is copied once
    /// (the pre-fix code copied it into a temporary `Vec` first).
    fn drain_complete_nals(&mut self) -> AuResult<()> {
        if !self.primed {
            // Drop everything up to the first start code: leading junk, and the
            // `zero_byte`s of a 4-byte `00 00 00 01` code (they belong to that
            // NAL, so the retained offset is pulled back over them).
            match first_nal_start(&self.buf) {
                Some(first) => {
                    if first > 0 {
                        self.buf.drain(..first);
                    }
                    self.primed = true;
                    self.scanned = 0;
                }
                None => {
                    // No start code anywhere yet: bound the junk we hold.
                    return self.check_open_nal_cap();
                }
            }
        }
        // Take the buffer locally so the NAL slices can be borrowed while
        // `process_nal` mutates the rest of the splitter; the retained tail is
        // appended back afterwards.
        let buf = core::mem::take(&mut self.buf);
        let keep_from = self.scan(&buf);
        self.buf.extend_from_slice(&buf[keep_from..]);
        // `scanned` indexes the *pre-drain* buffer, so it must move down by the
        // same amount the front was trimmed — otherwise the next push resumes
        // past the retained bytes, skips their start codes, and merges NALs
        // (losing access-unit boundaries) (r04-W3).
        self.scanned = self.scanned.saturating_sub(keep_from);
        self.check_open_nal_cap()
    }

    /// Reject a stream whose open NAL or assembled access unit has grown past
    /// [`MAX_OPEN_NAL_BYTES`] without completing.
    fn check_open_nal_cap(&self) -> AuResult<()> {
        for (len, what) in [
            (self.buf.len(), "Annex B NAL"),
            (self.au.len(), "access unit"),
        ] {
            if len > MAX_OPEN_NAL_BYTES {
                return Err(Error::InvalidValue {
                    field: what,
                    value: len as u64,
                    reason: "exceeds the maximum buffered Annex B NAL/access unit size",
                });
            }
        }
        Ok(())
    }

    /// Feed every complete NAL in `buf` to [`Self::process_nal`] and return the
    /// offset the caller must retain: the start of the one still-incomplete
    /// trailing NAL.
    ///
    /// `buf` must already be primed (start at a start code). The scan resumes at
    /// [`Self::scanned`], which only ever moves forward past positions already
    /// proven not to begin a start code, so the newest bytes are walked once.
    fn scan(&mut self, buf: &[u8]) -> usize {
        // Every start code ends the NAL before it; the last one opens a NAL that
        // is still incomplete.
        let mut starts: Vec<usize> = Vec::new();
        let n = buf.len();
        let mut p = self.scanned.min(n.saturating_sub(3));
        #[cfg(test)]
        let scan_start = p;
        while p + 3 <= n {
            if buf[p] == 0 && buf[p + 1] == 0 && buf[p + 2] == 1 {
                starts.push(p);
                p += 3;
            } else {
                p += 1;
            }
        }
        #[cfg(test)]
        {
            self.scanned_bytes += p.saturating_sub(scan_start);
        }
        // Positions at or past `n - 2` could still be the first two bytes of a
        // start code completed by the next push, so the cursor stops short of
        // them — but it must never move backwards.
        self.scanned = self.scanned.max(p.min(n.saturating_sub(2)));
        if starts.is_empty() {
            // A single start code with nothing after it: that NAL is the whole
            // buffer, still open.
            return 0;
        }
        // Fold every NAL start back over the `zero_byte`s that belong to it.
        // Boundary 0 is the first NAL, which always begins at the buffer start
        // once primed — the scan cursor may have skipped past it, so it can
        // never be derived from `starts`.
        let mut boundaries: Vec<usize> = Vec::with_capacity(starts.len() + 1);
        boundaries.push(0);
        for &cp in &starts {
            // A start code at offset 0 is the first NAL's own code; its boundary
            // is already recorded.
            if cp == 0 {
                continue;
            }
            let mut s = cp;
            while s > 0 && buf[s - 1] == 0 {
                s -= 1;
            }
            boundaries.push(s);
        }
        // Every boundary but the last ends a complete NAL.
        for i in 0..boundaries.len() - 1 {
            self.process_nal(&buf[boundaries[i]..boundaries[i + 1]]);
        }
        boundaries[boundaries.len() - 1]
    }

    /// Classify one complete Annex B NAL (start code included) and either append
    /// it to the current access unit or start a new one.
    fn process_nal(&mut self, nal_with_code: &[u8]) {
        // NAL body begins after the start code (any leading zeros + `00 00 01`);
        // classification only reads the header bytes at its front.
        let body = &nal_with_code[start_code_len(nal_with_code)..];
        let Some(t) = nal_unit_type(self.codec, body) else {
            // Too short to carry a NAL header — attach to the current AU verbatim.
            self.au.extend_from_slice(nal_with_code);
            return;
        };

        let vcl = is_vcl(self.codec, t);
        let starts_new_au = if is_aud(self.codec, t) {
            true
        } else if vcl {
            self.au_has_vcl && first_slice_of_picture(self.codec, body)
        } else {
            // A trailing non-VCL NAL (filler, suffix SEI, EOS/EOB) still belongs
            // to the picture whose VCL precedes it (r04-W2); only a genuine
            // prefix NAL starts the next access unit.
            self.au_has_vcl && !is_suffix(self.codec, t)
        };

        if starts_new_au && !self.au.is_empty() {
            self.ready.push_back(core::mem::take(&mut self.au));
            self.au_has_vcl = false;
        }
        self.au.extend_from_slice(nal_with_code);
        if vcl {
            self.au_has_vcl = true;
        }
    }
}

/// Length of the start-code prefix at the front of `nal_with_code` (any number
/// of leading `zero_byte`s followed by `00 00 01`): the offset of the first NAL
/// header byte. Falls back to the full length if no `00 00 01` is present.
fn start_code_len(nal_with_code: &[u8]) -> usize {
    let n = nal_with_code.len();
    let mut i = 0;
    while i + 3 <= n {
        if nal_with_code[i] == 0 && nal_with_code[i + 1] == 0 && nal_with_code[i + 2] == 1 {
            return i + 3;
        }
        i += 1;
    }
    n
}

/// Split a complete Annex B byte stream into access units in one call.
///
/// Convenience wrapper over [`AccessUnitSplitter`] for a buffer already held in
/// full (feeds it, finishes, and collects every access unit).
pub fn split_access_units(codec: NalCodec, annexb: &[u8]) -> Vec<Vec<u8>> {
    let mut s = AccessUnitSplitter::new(codec);
    // The one-shot convenience wrapper has no way to surface a mid-stream
    // error, and its buffer is the caller's whole stream — cap by simply
    // stopping the feed; `finish` still flushes what was assembled.
    if s.push(annexb).is_err() {
        return Vec::new();
    }
    s.finish();
    let mut out = Vec::new();
    while let Some(au) = s.pop() {
        out.push(au);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // Realistic-ish NAL headers. AVC: [type], first RBSP byte encodes
    // first_mb_in_slice (0x80 → ==0). HEVC: [type<<1, layer/tid], then RBSP.
    fn avc_sps() -> Vec<u8> {
        vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1e]
    }
    fn avc_pps() -> Vec<u8> {
        vec![0x00, 0x00, 0x00, 0x01, 0x68, 0xce, 0x38, 0x80]
    }
    fn avc_aud() -> Vec<u8> {
        vec![0x00, 0x00, 0x00, 0x01, 0x09, 0xf0]
    }
    /// IDR slice, first_mb_in_slice==0 (top bit of byte after header set).
    fn avc_idr_first() -> Vec<u8> {
        vec![0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00]
    }
    /// Non-IDR slice, first_mb_in_slice==0.
    fn avc_p_first() -> Vec<u8> {
        vec![0x00, 0x00, 0x01, 0x41, 0x9a, 0x00]
    }
    /// Non-IDR slice, first_mb_in_slice != 0 (top bit clear → continuation).
    fn avc_p_cont() -> Vec<u8> {
        vec![0x00, 0x00, 0x01, 0x41, 0x00, 0x11]
    }

    fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut v = Vec::new();
        for p in parts {
            v.extend_from_slice(p);
        }
        v
    }

    #[test]
    fn splits_aud_delimited_stream() {
        // AU1: AUD SPS PPS IDR ; AU2: AUD P
        let stream = concat(&[
            avc_aud(),
            avc_sps(),
            avc_pps(),
            avc_idr_first(),
            avc_aud(),
            avc_p_first(),
        ]);
        let aus = split_access_units(NalCodec::Avc, &stream);
        assert_eq!(aus.len(), 2, "two AUDs → two access units");
        assert_eq!(concat(&aus), stream, "AU concatenation is byte-exact");
    }

    #[test]
    fn splits_audless_stream_on_first_slice_and_config() {
        // No AUDs. AU1: SPS PPS IDR ; AU2: P(first) ; AU3: P(first)
        let stream = concat(&[
            avc_sps(),
            avc_pps(),
            avc_idr_first(),
            avc_p_first(),
            avc_p_first(),
        ]);
        let aus = split_access_units(NalCodec::Avc, &stream);
        assert_eq!(aus.len(), 3);
        assert_eq!(concat(&aus), stream);
        // First AU carries the parameter sets.
        assert!(aus[0].windows(1).any(|b| b[0] == 0x67));
    }

    #[test]
    fn multi_slice_picture_stays_one_au() {
        // One picture split into two slices: first_mb==0 then first_mb!=0.
        let stream = concat(&[avc_sps(), avc_idr_first(), avc_p_cont()]);
        // Note: avc_p_cont has first_mb!=0, so it must NOT open a new AU.
        let aus = split_access_units(NalCodec::Avc, &stream);
        assert_eq!(aus.len(), 1, "continuation slice stays in the same AU");
        assert_eq!(concat(&aus), stream);
    }

    #[test]
    fn streaming_matches_whole_buffer_at_every_split_point() {
        let stream = concat(&[
            avc_aud(),
            avc_sps(),
            avc_pps(),
            avc_idr_first(),
            avc_aud(),
            avc_p_first(),
            avc_p_cont(),
            avc_aud(),
            avc_p_first(),
        ]);
        let whole = split_access_units(NalCodec::Avc, &stream);
        // Feed one byte at a time — the hardest chunk boundary case.
        let mut s = AccessUnitSplitter::new(NalCodec::Avc);
        let mut chunked = Vec::new();
        for &b in &stream {
            s.push(&[b]).unwrap();
            while let Some(au) = s.pop() {
                chunked.push(au);
            }
        }
        s.finish();
        while let Some(au) = s.pop() {
            chunked.push(au);
        }
        assert_eq!(chunked, whole, "byte-by-byte streaming == whole-buffer");
        assert_eq!(concat(&chunked), stream);
    }

    #[test]
    fn hevc_aud_boundaries() {
        // HEVC AUD_NUT=35 → (35<<1)=0x46 ; VPS=32→0x40 ; IDR_W_RADL=19→0x26.
        let aud = vec![0x00u8, 0x00, 0x00, 0x01, 0x46, 0x01, 0x50];
        let vps = vec![0x00u8, 0x00, 0x00, 0x01, 0x40, 0x01, 0x0c];
        let idr = vec![0x00u8, 0x00, 0x01, 0x26, 0x01, 0x80]; // first_slice=1 (0x80)
        let stream = concat(&[aud.clone(), vps, idr, aud]);
        let aus = split_access_units(NalCodec::Hevc, &stream);
        assert_eq!(aus.len(), 2);
        assert_eq!(concat(&aus), stream);
    }

    #[test]
    fn leading_junk_before_first_start_code_is_dropped() {
        let mut stream = vec![0xaa, 0xbb, 0xcc];
        stream.extend_from_slice(&concat(&[avc_aud(), avc_idr_first()]));
        let aus = split_access_units(NalCodec::Avc, &stream);
        assert_eq!(aus.len(), 1);
        // Junk dropped: AU begins at the first start code.
        assert_eq!(&aus[0][..4], &[0x00, 0x00, 0x00, 0x01]);
    }

    /// r04-W2: H.264 §7.4.1.2.3 lets `filler_data` (12), `end_of_sequence` (10)
    /// and `end_of_stream` (11) trail the last VCL of the *same* access unit.
    /// Unfixed, each opened a new AU that the next AUD closed at once, so a CBR
    /// stream produced an alternating picture-AU / filler-AU sequence and every
    /// second AU carried no coded slice.
    #[test]
    fn avc_filler_and_eos_trail_their_picture() {
        let filler = vec![0x00u8, 0x00, 0x00, 0x01, 0x0c, 0xff, 0x80];
        let eos = vec![0x00u8, 0x00, 0x00, 0x01, 0x0a];
        let stream = concat(&[
            avc_aud(),
            avc_idr_first(),
            filler.clone(),
            eos,
            avc_aud(),
            avc_p_first(),
            filler,
        ]);
        let aus = split_access_units(NalCodec::Avc, &stream);
        assert_eq!(aus.len(), 2, "filler/EOS must not open an access unit");
        assert_eq!(concat(&aus), stream, "concatenation stays byte-exact");
        // Every emitted AU carries a coded slice (the pre-fix output alternated).
        for au in &aus {
            assert!(
                contains_nal_type(au, NalCodec::Avc, &[1, 5]),
                "each AU must contain a VCL NAL"
            );
        }
    }

    /// r04-W2 (HEVC §7.4.2.4.4): suffix SEI (40), FD (38), EOS/EOB (36/37) and
    /// the reserved/unspecified suffix ranges belong to the picture before them.
    /// Unfixed, a decoded-picture-hash suffix SEI was attributed to the *next*
    /// picture.
    #[test]
    fn hevc_suffix_nals_trail_their_picture() {
        let aud = vec![0x00u8, 0x00, 0x00, 0x01, 0x46, 0x01, 0x50];
        let idr = vec![0x00u8, 0x00, 0x01, 0x26, 0x01, 0x80]; // first_slice_segment=1
        let p = vec![0x00u8, 0x00, 0x01, 0x02, 0x01, 0x00]; // TRAIL_R, first slice (0x00)
        // SUFFIX_SEI_NUT = 40 → (40<<1) = 0x50.
        let suffix_sei = vec![0x00u8, 0x00, 0x00, 0x01, 0x50, 0x01, 0xaa];
        // FD_NUT = 38 → 0x4c.
        let fd = vec![0x00u8, 0x00, 0x00, 0x01, 0x4c, 0x01, 0xff];
        // PREFIX_SEI_NUT = 39 → 0x4e; still starts the next AU.
        let prefix_sei = vec![0x00u8, 0x00, 0x00, 0x01, 0x4e, 0x01, 0xbb];
        let stream = concat(&[aud.clone(), idr, suffix_sei, fd, prefix_sei, p, aud]);
        let aus = split_access_units(NalCodec::Hevc, &stream);
        // AU 1 = picture + its suffix SEI/FD; AU 2 = the prefix SEI and the next
        // slice; AU 3 = the trailing AUD alone (an AUD always opens an AU).
        assert_eq!(aus.len(), 3, "only the prefix SEI starts the second AU");
        assert_eq!(concat(&aus), stream);
        // The first AU ends with the suffix SEI + filler data (in stream order).
        let first = &aus[0];
        assert_eq!(
            &first[first.len() - 14..],
            &[
                0x00, 0x00, 0x00, 0x01, 0x50, 0x01, 0xaa, // SUFFIX_SEI_NUT
                0x00, 0x00, 0x00, 0x01, 0x4c, 0x01, 0xff, // FD_NUT
            ][..]
        );
        assert!(contains_nal_type(first, NalCodec::Hevc, &[38, 40]));
        assert!(contains_nal_type(&aus[1], NalCodec::Hevc, &[2, 39]));
    }

    /// Whether `annexb` holds a NAL whose type is one of `types`.
    fn contains_nal_type(annexb: &[u8], codec: NalCodec, types: &[u8]) -> bool {
        for (i, w) in annexb.windows(3).enumerate() {
            if w == [0x00, 0x00, 0x01] {
                let body = &annexb[i + 3..];
                if let Some(t) = nal_unit_type(codec, body)
                    && types.contains(&t)
                {
                    return true;
                }
            }
        }
        false
    }

    /// r04-W3 regression: `scanned` is an offset into the *pre-drain* buffer, so
    /// after `drain_complete_nals` trims `keep_from` bytes off the front it must
    /// be rebased by the same amount. Unfixed, the cursor pointed past the
    /// retained bytes, the next push skipped the start codes in them, and NALs
    /// merged — losing access-unit boundaries.
    ///
    /// The stream is shaped so the bug bites: each AU's *last* NAL is long, so
    /// every drain leaves a large tail and the stale cursor is far too big.
    #[test]
    fn chunked_pushes_keep_au_boundaries_after_a_drain() {
        // AU1 .. AU4, each ending in a long non-VCL filler NAL so the retained
        // tail after each drain is measured in thousands of bytes.
        let mut stream = Vec::new();
        for i in 0..4u8 {
            stream.extend_from_slice(&avc_aud());
            stream.extend_from_slice(&avc_idr_first());
            let mut filler = alloc::vec![0x00, 0x00, 0x00, 0x01, 0x0c]; // filler_data NAL
            filler.extend_from_slice(&alloc::vec![0xffu8; 3000 + i as usize]);
            stream.extend_from_slice(&filler);
        }
        let whole = split_access_units(NalCodec::Avc, &stream);
        assert_eq!(whole.len(), 4, "four AUD-led access units");

        // Feed in small chunks: 512 bytes is well inside the long filler NALs,
        // so a stale cursor lands mid-NAL on the next push.
        for chunk_size in [1usize, 2, 3, 7, 64, 511, 512, 513, 1316] {
            let mut s = AccessUnitSplitter::new(NalCodec::Avc);
            let mut chunked = Vec::new();
            for chunk in stream.chunks(chunk_size) {
                s.push(chunk).unwrap();
                while let Some(au) = s.pop() {
                    chunked.push(au);
                }
            }
            s.finish();
            while let Some(au) = s.pop() {
                chunked.push(au);
            }
            assert_eq!(
                chunked, whole,
                "chunk size {chunk_size}: AU boundaries must match the one-shot split"
            );
            assert_eq!(concat(&chunked), stream, "chunk size {chunk_size}");
        }
    }

    /// r04-W3: the splitter must not rescan bytes it has already walked. Feeding
    /// the same stream byte-by-byte and in one push must agree (the cursor is
    /// the only difference), and a large single NAL pushed in many small chunks
    /// must not consume time quadratic in the chunk count.
    #[test]
    fn scan_cursor_does_not_change_the_result() {
        let mut big = avc_idr_first();
        big.extend_from_slice(&vec![0x55u8; 200_000]);
        let stream = concat(&[avc_aud(), big, avc_p_first(), avc_aud()]);
        let whole = split_access_units(NalCodec::Avc, &stream);

        // 1316 bytes is the classic TS/RTMP payload size; the pre-fix splitter
        // rescanned the whole buffer on every one of these pushes.
        let mut s = AccessUnitSplitter::new(NalCodec::Avc);
        let mut chunked = Vec::new();
        for chunk in stream.chunks(1316) {
            s.push(chunk).unwrap();
            while let Some(au) = s.pop() {
                chunked.push(au);
            }
        }
        s.finish();
        while let Some(au) = s.pop() {
            chunked.push(au);
        }
        assert_eq!(chunked, whole);
        assert_eq!(concat(&chunked), stream);
    }

    /// r04-W3: the start-code scan must be O(bytes fed), not O(pushes x bytes
    /// buffered). The pre-fix code called `start_code_positions` over the whole
    /// buffer on every push, so a large IDR delivered in many small chunks was
    /// rescanned once per chunk. This asserts the *total* bytes walked stays
    /// within a small constant factor of the stream length — a monotonic-cursor
    /// assertion cannot catch the regression, since a rescan-from-zero cursor is
    /// also monotonic.
    #[test]
    fn total_bytes_scanned_is_linear_in_the_stream_length() {
        let mut big = avc_idr_first();
        big.extend_from_slice(&vec![0x55u8; 200_000]);
        let stream = concat(&[avc_aud(), big, avc_p_first(), avc_aud()]);

        // 1316 bytes is the classic TS/RTMP payload size: ~150 pushes over a
        // 200 KB buffer. Rescanning from zero each time walks ~15 MB.
        let mut s = AccessUnitSplitter::new(NalCodec::Avc);
        for chunk in stream.chunks(1316) {
            s.push(chunk).unwrap();
            while s.pop().is_some() {}
        }
        s.finish();
        assert!(
            s.scanned_bytes <= 2 * stream.len(),
            "scan walked {} bytes for a {}-byte stream; must stay O(n)",
            s.scanned_bytes,
            stream.len()
        );
        // And the O(n) scan produces exactly the one-shot result.
        let mut s2 = AccessUnitSplitter::new(NalCodec::Avc);
        let mut chunked = Vec::new();
        for chunk in stream.chunks(1316) {
            s2.push(chunk).unwrap();
            while let Some(au) = s2.pop() {
                chunked.push(au);
            }
        }
        s2.finish();
        while let Some(au) = s2.pop() {
            chunked.push(au);
        }
        assert_eq!(chunked, split_access_units(NalCodec::Avc, &stream));
    }

    /// r04-W3: a stream that never emits another start code (or whose single NAL
    /// runs past the cap) must be rejected, not buffered without limit.
    #[test]
    fn an_unterminated_nal_is_rejected_at_the_cap() {
        let mut s = AccessUnitSplitter::new(NalCodec::Avc);
        s.push(&avc_aud()).unwrap();
        // Feed well past the cap in modest chunks: the open NAL never closes.
        let chunk = vec![0x55u8; 1 << 20];
        let mut err = None;
        for _ in 0..(MAX_OPEN_NAL_BYTES / chunk.len() + 4) {
            if let Err(e) = s.push(&chunk) {
                err = Some(e);
                break;
            }
        }
        let err = err.expect("an unterminated NAL must eventually be rejected");
        assert!(
            matches!(
                err,
                Error::InvalidValue {
                    field: "Annex B NAL",
                    ..
                }
            ),
            "expected the buffered-NAL cap error, got {err:?}"
        );
        // A conformant stream of the same total size, split into real NALs, is
        // unaffected: nothing is rejected and no AU is lost.
        let mut ok = Vec::new();
        for _ in 0..64 {
            ok.extend_from_slice(&avc_aud());
            ok.extend_from_slice(&avc_idr_first());
            let mut filler = alloc::vec![0x00, 0x00, 0x00, 0x01, 0x0c];
            filler.extend_from_slice(&alloc::vec![0xffu8; 2000]);
            ok.extend_from_slice(&filler);
        }
        let mut s2 = AccessUnitSplitter::new(NalCodec::Avc);
        for chunk in ok.chunks(4096) {
            s2.push(chunk)
                .expect("conformant stream must not be rejected");
        }
        s2.finish();
        let mut n = 0;
        while s2.pop().is_some() {
            n += 1;
        }
        assert_eq!(n, 64);
    }
}
