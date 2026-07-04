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

use crate::nal::{NalCodec, nal_unit_type};

// ── NAL-type classification (VCL ranges + AUD), per each codec's Table 7-1 / 5 ──

/// AVC lowest VCL `nal_unit_type` (coded slice) — ITU-T H.264 Table 7-1.
const AVC_VCL_MIN: u8 = 1;
/// AVC highest VCL `nal_unit_type` (IDR slice) — ITU-T H.264 Table 7-1.
const AVC_VCL_MAX: u8 = 5;
/// AVC access-unit-delimiter `nal_unit_type` — ITU-T H.264 Table 7-1.
const AVC_AUD: u8 = 9;

/// HEVC highest VCL `nal_unit_type` (`RSV_VCL31`) — ITU-T H.265 Table 7-1
/// (VCL types are 0..=31).
const HEVC_VCL_MAX: u8 = 31;
/// HEVC access-unit-delimiter `nal_unit_type` (`AUD_NUT`) — ITU-T H.265 Table 7-1.
const HEVC_AUD: u8 = 35;

/// VVC highest VCL `nal_unit_type` (`RSV_VCL_11`) — ITU-T H.266 Table 5
/// (VCL types are 0..=11).
const VVC_VCL_MAX: u8 = 11;
/// VVC access-unit-delimiter `nal_unit_type` (`AUD_NUT`) — ITU-T H.266 Table 5.
const VVC_AUD: u8 = 20;

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

/// Positions of every start code's first `00` (of the trailing `00 00 01`).
///
/// A 4-byte start code (`00 00 00 01`) is reported at the last three bytes; the
/// extra leading `00` is left in the preceding slice, which does not affect NAL
/// classification (only the header bytes after the code are read) and keeps
/// concatenation byte-exact.
fn start_code_positions(data: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    let n = data.len();
    let mut p = 0usize;
    while p + 3 <= n {
        if data[p] == 0 && data[p + 1] == 0 && data[p + 2] == 1 {
            positions.push(p);
            p += 3;
        } else {
            p += 1;
        }
    }
    positions
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
    /// Bytes of the access unit currently being assembled (Annex B, verbatim).
    au: Vec<u8>,
    /// Whether `au` already contains a VCL NAL.
    au_has_vcl: bool,
    /// Completed access units awaiting [`pop`](Self::pop).
    ready: VecDeque<Vec<u8>>,
}

impl AccessUnitSplitter {
    /// Create a splitter for `codec`.
    pub fn new(codec: NalCodec) -> Self {
        Self {
            codec,
            buf: Vec::new(),
            primed: false,
            au: Vec::new(),
            au_has_vcl: false,
            ready: VecDeque::new(),
        }
    }

    /// Append `bytes` of the Annex B stream and split off any access units that
    /// became complete. Completed units are queued for [`pop`](Self::pop).
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        self.drain_complete_nals();
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
    fn drain_complete_nals(&mut self) {
        // A NAL owns its own start code *including* any leading `zero_byte`s
        // (`start_code_positions` reports the `00 00 01`; the preceding zeros of a
        // 4-byte `00 00 00 01` code belong to that same NAL). `au_starts` gives
        // each NAL's true first byte so concatenation is byte-exact.
        let au_starts = self.nal_starts();
        if !self.primed {
            match au_starts.first() {
                // Drop non-zero junk before the first start code (its leading
                // zeros are already folded into `au_starts[0]`).
                Some(&first) => {
                    if first > 0 {
                        self.buf.drain(..first);
                    }
                    self.primed = true;
                }
                None => return, // no start code yet — keep buffering
            }
        }
        // Re-derive against the (possibly trimmed) buffer.
        let au_starts = self.nal_starts();
        // Each NAL i spans [au_starts[i] .. au_starts[i+1]); the last is still open.
        if au_starts.len() < 2 {
            return;
        }
        let mut consumed = 0usize;
        for w in au_starts.windows(2) {
            let (start, end) = (w[0], w[1]);
            let nal = self.buf[start..end].to_vec();
            self.process_nal(&nal);
            consumed = end;
        }
        // Retain from the last start code (the still-incomplete NAL) onward.
        self.buf.drain(..consumed);
    }

    /// First-byte offset of every NAL in `buf`: each start code's `00 00 01`
    /// position pulled back over any immediately-preceding `zero_byte`s.
    fn nal_starts(&self) -> Vec<usize> {
        start_code_positions(&self.buf)
            .into_iter()
            .map(|cp| {
                let mut s = cp;
                while s > 0 && self.buf[s - 1] == 0 {
                    s -= 1;
                }
                s
            })
            .collect()
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
            self.au_has_vcl
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
    s.push(annexb);
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
            s.push(&[b]);
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
}
