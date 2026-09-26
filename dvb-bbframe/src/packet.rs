//! User packet extraction from BBFrame data fields.
//!
//! Supports Normal Mode (NM, variable stride) and High Efficiency Mode
//! (HEM, 187-byte stride) per EN 302 755 §5.1.8.
//!
//! In NM the first byte of each transmitted UP chunk is the CRC-8 of the
//! *previous* UP (see [`nm_stride_bytes`]) and replaces the original sync
//! byte (0x47). In HEM the sync byte is simply absent and must be prepended.
//!
//! ## NM stride (UPL/ISSYI/NPD)
//!
//! For Transport Stream input, EN 302 755 §5.1.8 ("Normal Mode, GFPS and TS",
//! figure 5) transmits each UP as `CRC-8 (1 byte) + original UP without its
//! sync byte (187 bytes) + ISSY (0, 2 or 3 bytes, when ISSYI=1) + DNP (0 or 1
//! byte, when NPD=1)`. The clause's own bit-count walk of the BBHEADER's
//! `UPL` field (O-UPL=1504 bits, +16/+24 for ISSY, -8 for the removed sync
//! byte, +8 for DNP when active, +8 for CRC-8) shows the transmitted UPL
//! already equals this stride in bits, so [`nm_stride_bytes`] reads the
//! stride back from `UPL` directly rather than a fixed 188-byte assumption —
//! a fixed stride corrupts framing whenever ISSYI or NPD is active.
//!
//! ## SYNCD handling
//!
//! `SYNCD` gives the bit offset from the start of the DATA FIELD to the
//! first bit of the CRC-8 byte of the first user packet (NM) or the first
//! byte of the first user packet (HEM).  Callers typically prepend any
//! carry-over from the previous BBFrame and pass the result plus SYNCD.
//!
//! ## NPD/DNP
//!
//! When NPD is active (`matype.npd == true`), each transmitted user packet is
//! followed by a 1-byte DNP counter. [`HemTsIter`] and the NM stride
//! computation both account for this extra byte so framing stays aligned;
//! neither actually reinserts the deleted null packets into the recovered TS
//! (a capability gap tracked separately via [`CarryOverStats::npd_unsupported`]
//! for the HEM carry-over path).

use alloc::vec::Vec;

use crate::crc::crc8;
use crate::header::{BBHEADER_LEN, Bbheader, Mode};

/// User packet size in Normal Mode (188 bytes = full MPEG-2 TS packet).
pub const NM_UP_SIZE: usize = 188;

/// User packet size in High Efficiency Mode (187 bytes = TS minus sync byte).
pub const HEM_UP_SIZE: usize = 187;

/// Longest ISSY field (long form / BUFS-TTO signalling, EN 302 755 Annex C).
const NM_ISSY_MAX_LEN: usize = 3;

/// DNP counter length when NPD is active (EN 302 755 §5.1.5/§5.1.8).
const NM_DNP_LEN: usize = 1;

/// Largest possible NM per-UP stride: CRC-8 + 187-byte UP + long ISSY + DNP.
const NM_MAX_STRIDE: usize = NM_UP_SIZE + NM_ISSY_MAX_LEN + NM_DNP_LEN;

/// MPEG-2 sync byte that CRC-8 replaces in NM.
pub const TS_SYNC_BYTE: u8 = 0x47;

/// Derive the Normal-Mode per-UP stride (bytes) from a parsed BBHEADER, for
/// Transport Stream input — EN 302 755 §5.1.8 ("Normal Mode, GFPS and TS").
///
/// The transmitted UP is `CRC-8 (1) + original UP minus sync (187) + ISSY (0,
/// 2 or 3, per MATYPE ISSYI) + DNP (0 or 1, per MATYPE NPD)`; the clause's bit
/// walk shows this is exactly what the final `UPL` field (in bits) counts, so
/// the stride is read back from `UPL` rather than re-derived from the ISSYI/
/// NPD flags alone (the ISSY short/long form is signalled inside the ISSY
/// field itself, not in the header).
///
/// Returns `None` when `UPL` is zero/not byte-aligned, shorter than a bare
/// `CRC-8 + 187-byte UP`, or implies an ISSY byte count that cannot occur
/// (anything other than 0 with ISSYI=0, or 2/3 with ISSYI=1) — the frame is
/// then treated as unsupported/malformed rather than mis-framed at a fixed
/// stride.
#[must_use]
pub fn nm_stride_bytes(hdr: &Bbheader) -> Option<usize> {
    let upl = hdr.upl;
    if upl == 0 || !upl.is_multiple_of(8) {
        return None;
    }
    let stride = (upl / 8) as usize;
    let extra = stride.checked_sub(NM_UP_SIZE)?;
    let dnp_len = usize::from(hdr.matype.npd);
    let issy_len = extra.checked_sub(dnp_len)?;
    let issy_ok = if hdr.matype.issyi {
        issy_len == 2 || issy_len == 3
    } else {
        issy_len == 0
    };
    if !issy_ok {
        return None;
    }
    Some(stride)
}

/// Iterator over NM TS user packets.
///
/// Each item is a `[u8; 188]` with the sync byte restored to 0x47,
/// replacing the CRC-8 byte that occupies position 0 in the data field. Any
/// trailing ISSY/DNP bytes included in `stride` (see [`nm_stride_bytes`]) are
/// skipped, not copied into the packet.
///
/// This is the lean, no-BBHEADER-overhead iterator: it trusts the caller's
/// `stride` and does not verify the per-UP CRC-8. [`up_iter`] derives `stride`
/// from a [`Bbheader`]; for CRC-8-verified, carry-over-aware extraction with
/// diagnostics use [`CarryOverExtractor`] instead.
#[derive(Clone, Copy)]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct NmTsIter<'a> {
    data: &'a [u8],
    pos: usize,
    stride: usize,
}

impl<'a> NmTsIter<'a> {
    /// Create a new NM TS user packet iterator.
    ///
    /// `data` is the full data field (after skipping SYNCD bytes if already
    /// aligned). The iterator starts at byte 0 of `data`. `stride` is the
    /// per-UP byte stride (see [`nm_stride_bytes`]); a `stride` of 0 makes the
    /// iterator yield nothing, so a caller unable to determine a valid stride
    /// can pass 0 instead of guessing.
    pub fn new(data: &'a [u8], stride: usize) -> Self {
        Self {
            data,
            pos: 0,
            stride,
        }
    }

    /// Return the unconsumed tail of the data field.
    pub fn remaining(self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }
}

impl Iterator for NmTsIter<'_> {
    type Item = [u8; NM_UP_SIZE];

    fn next(&mut self) -> Option<Self::Item> {
        if self.stride == 0 || self.pos + self.stride > self.data.len() {
            return None;
        }
        let mut pkt = [0u8; NM_UP_SIZE];
        pkt[0] = TS_SYNC_BYTE; // Replace CRC-8 byte with sync byte
        pkt[1..].copy_from_slice(&self.data[self.pos + 1..self.pos + NM_UP_SIZE]);
        self.pos += self.stride;
        Some(pkt)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.stride == 0 {
            return (0, Some(0));
        }
        let remaining = self.data.len().saturating_sub(self.pos);
        let count = remaining / self.stride;
        (count, Some(count))
    }
}

/// Iterator over HEM TS user packets.
///
/// Each item is a `[u8; 188]` with the sync byte prepended (0x47).
/// The 187-byte user packets in the data field have no sync byte.
/// If NPD is active, DNP bytes are skipped automatically.
#[derive(Clone, Copy)]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct HemTsIter<'a> {
    data: &'a [u8],
    pos: usize,
    npd: bool,
}

impl<'a> HemTsIter<'a> {
    /// Create a new HEM TS user packet iterator.
    ///
    /// `data` is the full data field (after skipping SYNCD bytes if
    /// already aligned). The iterator starts at byte 0 of `data`.
    pub fn new(data: &'a [u8], npd: bool) -> Self {
        Self { data, pos: 0, npd }
    }

    /// Return the unconsumed tail of the data field.
    pub fn remaining(self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }
}

impl Iterator for HemTsIter<'_> {
    type Item = [u8; NM_UP_SIZE];

    fn next(&mut self) -> Option<Self::Item> {
        let stride = HEM_UP_SIZE + if self.npd { 1 } else { 0 };
        if self.pos + stride > self.data.len() {
            return None;
        }
        let mut pkt = [0u8; NM_UP_SIZE];
        pkt[0] = TS_SYNC_BYTE;
        pkt[1..].copy_from_slice(&self.data[self.pos..self.pos + HEM_UP_SIZE]);
        self.pos += stride;
        Some(pkt)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let stride = HEM_UP_SIZE + if self.npd { 1 } else { 0 };
        let remaining = self.data.len().saturating_sub(self.pos);
        let count = remaining / stride;
        (count, Some(count))
    }
}

/// Concrete user-packet iterator returned by [`up_iter`].
///
/// Selects NM or HEM iteration at runtime without heap allocation or
/// dynamic dispatch — the mode is baked into the variant.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum UpIter<'a> {
    /// Normal Mode iteration.
    Normal(NmTsIter<'a>),
    /// High Efficiency Mode iteration.
    HighEfficiency(HemTsIter<'a>),
}

impl Iterator for UpIter<'_> {
    type Item = [u8; NM_UP_SIZE];

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Normal(it) => it.next(),
            Self::HighEfficiency(it) => it.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Normal(it) => it.size_hint(),
            Self::HighEfficiency(it) => it.size_hint(),
        }
    }
}

/// Build an appropriate user-packet iterator for the given BBHEADER.
///
/// Returns either an NM or HEM iterator depending on the detected mode.
/// The caller must handle SYNCD alignment before calling this — typically
/// by skipping `syncd / 8` bytes at the start of the data field.
///
/// For NM, the stride is derived from `UPL`/ISSYI/NPD via [`nm_stride_bytes`];
/// when that isn't a valid stride (see its docs) the returned iterator yields
/// nothing rather than mis-framing at a fixed 188 bytes.
pub fn up_iter<'a>(data: &'a [u8], bbheader: &Bbheader) -> UpIter<'a> {
    match bbheader.mode {
        Mode::Normal => {
            let stride = nm_stride_bytes(bbheader).unwrap_or(0);
            UpIter::Normal(NmTsIter::new(data, stride))
        }
        Mode::HighEfficiency => UpIter::HighEfficiency(HemTsIter::new(data, bbheader.matype.npd)),
    }
}

/// Stateful UP extractor that carries partial user packets across BBFrame
/// boundaries — a single UP can span multiple frames, especially in HEM
/// where stride=187 bytes.
///
/// Use `feed_nm` / `feed_hem` per received frame; the returned Vec holds
/// whichever 188-byte TS packets completed during that frame.
///
/// Diagnostic counters are accumulated and exposed via [`stats`](CarryOverExtractor::stats).
pub struct CarryOverExtractor {
    /// Partial TS packet being assembled (sync byte position 0). Sized for
    /// the longest possible NM stride (CRC-8 + UP + ISSY + DNP); HEM only
    /// ever fills the first `HEM_UP_SIZE + 1` bytes of it.
    buf: [u8; NM_MAX_STRIDE],
    /// Bytes already written into `buf`.
    pos: usize,
    /// Rolling NM UP-level CRC-8 chain state: the CRC-8 computed over the
    /// last fully-assembled NM UP's content, checked against the next UP's
    /// leading (CRC) byte. `None` when no chain is established yet, or right
    /// after a resync (`partial_discards`) breaks it.
    nm_crc_state: Option<u8>,
    /// Diagnostic counters (see [`CarryOverStats`]).
    stats: CarryOverStats,
}

impl Default for CarryOverExtractor {
    fn default() -> Self {
        Self {
            buf: [0u8; NM_MAX_STRIDE],
            pos: 0,
            nm_crc_state: None,
            stats: CarryOverStats::default(),
        }
    }
}

/// Diagnostic counters for a [`CarryOverExtractor`], read via
/// [`CarryOverExtractor::stats`].
///
/// The extractor stays resilient (it never errors or panics on wire-derived
/// input — bad frames are skipped so a stream keeps flowing). These counters
/// make the otherwise-silent skips observable. Note the distinction:
/// `npd_unsupported` counts **valid data we failed to recover** (a capability
/// gap), whereas the others count malformed/misrouted input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CarryOverStats {
    /// HEM frames skipped because Null-Packet-Deletion (DNP) reinsertion is not
    /// implemented. These carry **valid** user packets that are NOT recovered —
    /// a known capability gap, not wire corruption. **Non-zero means real data
    /// was dropped**; treat it as a signal that NPD-HEM input is unsupported.
    pub npd_unsupported: u64,
    /// Frames whose 10-byte BBHEADER failed to parse.
    pub header_parse_failures: u64,
    /// Frames fed to the wrong mode path (an NM header to `feed_hem_into`, or a
    /// HEM header to `feed_nm_into`).
    pub mode_mismatches: u64,
    /// Carried-over partial user packets discarded on a SYNCD/stride mismatch
    /// (the extractor resynchronised at the frame's SYNCD).
    pub partial_discards: u64,
    /// NM frames whose `UPL` (adjusted for ISSYI/NPD per EN 302 755 §5.1.8)
    /// did not yield a valid per-UP stride — see [`nm_stride_bytes`]. The
    /// frame is skipped rather than mis-framed at a fixed 188-byte stride.
    pub nm_upl_invalid: u64,
    /// NM UP-level CRC-8 mismatches (EN 302 755 §5.1.6): the CRC-8 carried as
    /// the leading byte of a transmitted UP (the *previous* UP's trailer, per
    /// §5.1.8 figure 5) did not match the recomputed CRC-8 of that previous
    /// UP's content. Diagnostic only — the packet is still emitted.
    pub crc8_mismatches: u64,
}

impl CarryOverExtractor {
    /// Create a fresh extractor with no carried-over state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Diagnostic counters accumulated across all `feed_*` calls — see
    /// [`CarryOverStats`]. Check `npd_unsupported` in particular: a non-zero
    /// value means valid HEM frames were dropped (NPD reinsertion unsupported).
    #[must_use]
    pub fn stats(&self) -> CarryOverStats {
        self.stats
    }

    /// Feed a HEM BBFrame's header + data field. Returns any TS packets that
    /// completed during this frame.
    ///
    /// `npd` is the MATYPE-1 NPD flag for the frame — when true the stream
    /// would additionally carry DNP bytes between UPs. NPD reinsertion is
    /// NOT YET implemented here; callers must not pass `npd=true` until
    /// the DNP path lands.
    pub fn feed_hem(
        &mut self,
        bbheader_bytes: &[u8; BBHEADER_LEN],
        data_field: &[u8],
        npd: bool,
    ) -> Vec<[u8; NM_UP_SIZE]> {
        let mut out = Vec::new();
        self.feed_hem_into(bbheader_bytes, data_field, npd, &mut out);
        out
    }

    /// Buffer-reusing variant of [`feed_hem`](Self::feed_hem). Clears `out`,
    /// then appends the TS packets that completed during this frame. Reuse the
    /// same `Vec` across frames to avoid a per-frame heap allocation.
    pub fn feed_hem_into(
        &mut self,
        bbheader_bytes: &[u8; BBHEADER_LEN],
        data_field: &[u8],
        npd: bool,
        out: &mut Vec<[u8; NM_UP_SIZE]>,
    ) {
        out.clear();
        // NPD/DNP reinsertion is not yet implemented; rather than panic on
        // wire-derived input, produce no output (out is already cleared).
        if npd {
            self.stats.npd_unsupported += 1;
            return;
        }
        let hdr = match Bbheader::parse(bbheader_bytes) {
            Ok(h) => h,
            Err(_) => {
                self.stats.header_parse_failures += 1;
                return;
            }
        };
        // Mismatched mode (caller fed a non-HEM header): no output, no panic.
        if hdr.mode != Mode::HighEfficiency {
            self.stats.mode_mismatches += 1;
            return;
        }

        let stride = HEM_UP_SIZE;
        // SYNCD=0xFFFF (65535) means "no UP starts in the DATA FIELD" — the
        // entire data field is a continuation of the carried-over partial UP.
        // EN 302 755 Table 2.
        let all_continuation = hdr.syncd == 0xFFFF;
        let syncd_bytes = if all_continuation {
            0
        } else {
            (hdr.syncd / 8) as usize
        };
        let dfl_bytes = (hdr.dfl / 8) as usize;
        let data = &data_field[..dfl_bytes.min(data_field.len())];

        if all_continuation {
            // The whole data field continues the previous partial UP.
            if self.pos > 0 {
                let space = stride + 1 - self.pos; // bytes still needed to complete
                let take = data.len().min(space);
                self.buf[self.pos..self.pos + take].copy_from_slice(&data[..take]);
                self.pos += take;
                if self.pos == stride + 1 {
                    // UP is now complete.
                    self.buf[0] = TS_SYNC_BYTE;
                    out.push(self.buf[..NM_UP_SIZE].try_into().expect("188 bytes"));
                    self.pos = 0;
                }
            }
            // Any bytes beyond the completed UP are a new partial; in practice
            // SYNCD=0xFFFF means the whole field is the continuation, so there
            // should be nothing left — but buffer any excess defensively.
            return;
        }

        // Complete the partial UP from the previous frame.
        if self.pos > 0 {
            let need = stride + 1 - self.pos; // +1 for the sync byte we'll prepend
            if syncd_bytes == need && data.len() >= need {
                self.buf[self.pos..self.pos + need].copy_from_slice(&data[..need]);
                self.buf[0] = TS_SYNC_BYTE;
                out.push(self.buf[..NM_UP_SIZE].try_into().expect("188 bytes"));
                self.pos = 0;
            } else {
                // Stride mismatch — discard partial and resync to syncd.
                self.stats.partial_discards += 1;
                self.pos = 0;
            }
        }

        // Extract complete UPs at stride.
        let mut i = syncd_bytes;
        while i + stride <= data.len() {
            // HEM: 187 bytes of UP, prepend sync byte.
            self.buf[0] = TS_SYNC_BYTE;
            self.buf[1..1 + stride].copy_from_slice(&data[i..i + stride]);
            out.push(self.buf[..NM_UP_SIZE].try_into().expect("188 bytes"));
            i += stride;
        }

        // Buffer trailing partial packet (may be filled by next frame).
        if i < data.len() {
            let tail = (data.len() - i).min(stride);
            // Store at offset 1 (reserving byte 0 for the sync we prepend later).
            self.buf[1..1 + tail].copy_from_slice(&data[i..i + tail]);
            self.pos = 1 + tail;
        } else {
            self.pos = 0;
        }
    }

    /// Feed an NM BBFrame. The per-UP stride is derived from `UPL`/ISSYI/NPD
    /// (see [`nm_stride_bytes`]); `byte[0]` of each transmitted UP is the
    /// CRC-8 of the previous UP (EN 302 755 §5.1.8 figure 5) and gets
    /// replaced with 0x47 after being checked against the recomputed CRC-8 —
    /// see [`CarryOverStats::crc8_mismatches`].
    pub fn feed_nm(
        &mut self,
        bbheader_bytes: &[u8; BBHEADER_LEN],
        data_field: &[u8],
    ) -> Vec<[u8; NM_UP_SIZE]> {
        let mut out = Vec::new();
        self.feed_nm_into(bbheader_bytes, data_field, &mut out);
        out
    }

    /// Buffer-reusing variant of [`feed_nm`](Self::feed_nm). Clears `out`, then
    /// appends the TS packets that completed during that frame. Reuse the same
    /// `Vec` across frames to avoid a per-frame heap allocation.
    pub fn feed_nm_into(
        &mut self,
        bbheader_bytes: &[u8; BBHEADER_LEN],
        data_field: &[u8],
        out: &mut Vec<[u8; NM_UP_SIZE]>,
    ) {
        out.clear();
        let hdr = match Bbheader::parse(bbheader_bytes) {
            Ok(h) => h,
            Err(_) => {
                self.stats.header_parse_failures += 1;
                return;
            }
        };
        // Mismatched mode (caller fed a non-NM header): no output, no panic.
        if hdr.mode != Mode::Normal {
            self.stats.mode_mismatches += 1;
            return;
        }

        let stride = match nm_stride_bytes(&hdr) {
            Some(s) => s,
            None => {
                // UPL/ISSYI/NPD don't describe a valid stride — skip this
                // frame rather than mis-frame it at a fixed 188 bytes.
                self.stats.nm_upl_invalid += 1;
                return;
            }
        };
        // SYNCD=0xFFFF (65535) means "no UP starts in the DATA FIELD" — the
        // entire data field is a continuation of the carried-over partial UP.
        // EN 302 755 Table 2.
        let all_continuation = hdr.syncd == 0xFFFF;
        let syncd_bytes = if all_continuation {
            0
        } else {
            (hdr.syncd / 8) as usize
        };
        let dfl_bytes = (hdr.dfl / 8) as usize;
        let data = &data_field[..dfl_bytes.min(data_field.len())];

        if all_continuation {
            // The whole data field continues the previous partial UP.
            if self.pos > 0 {
                let space = stride - self.pos; // bytes still needed to complete
                let take = data.len().min(space);
                self.buf[self.pos..self.pos + take].copy_from_slice(&data[..take]);
                self.pos += take;
                if self.pos == stride {
                    self.finish_nm_up(stride, out);
                    self.pos = 0;
                }
            }
            return;
        }

        // Complete partial UP from previous frame.
        if self.pos > 0 {
            let need = stride - self.pos;
            if syncd_bytes == need && data.len() >= need {
                self.buf[self.pos..self.pos + need].copy_from_slice(&data[..need]);
                self.finish_nm_up(stride, out);
                self.pos = 0;
            } else {
                // Stride mismatch — discard partial and resync to syncd. The
                // CRC-8 chain no longer correlates across the gap.
                self.stats.partial_discards += 1;
                self.pos = 0;
                self.nm_crc_state = None;
            }
        }

        // Extract complete UPs at stride.
        let mut i = syncd_bytes;
        while i + stride <= data.len() {
            self.buf[..stride].copy_from_slice(&data[i..i + stride]);
            self.finish_nm_up(stride, out);
            i += stride;
        }

        // Buffer trailing partial.
        if i < data.len() {
            let tail = (data.len() - i).min(stride);
            self.buf[..tail].copy_from_slice(&data[i..i + tail]);
            self.pos = tail;
        } else {
            self.pos = 0;
        }
    }

    /// Finish one fully-buffered NM UP of `stride` bytes in `self.buf`:
    /// check the leading CRC-8 byte (the previous UP's trailer, EN 302 755
    /// §5.1.8 figure 5) against the CRC-8 recomputed from that previous UP's
    /// content when a chain is established, emit the reconstructed 188-byte
    /// TS packet (sync byte restored, any ISSY/DNP trailer dropped), then
    /// extend the chain with this UP's own CRC-8 for the next call.
    fn finish_nm_up(&mut self, stride: usize, out: &mut Vec<[u8; NM_UP_SIZE]>) {
        let raw_crc_byte = self.buf[0];
        if let Some(expected) = self.nm_crc_state
            && raw_crc_byte != expected
        {
            self.stats.crc8_mismatches += 1;
        }
        // CRC-8 covers the UPL-8 bits of the UP after sync-byte removal (EN
        // 302 755 §5.1.6), i.e. everything in this stride after the leading
        // CRC-8 byte: the 187-byte UP plus any ISSY/DNP trailer.
        self.nm_crc_state = Some(crc8(&self.buf[1..stride]));

        let mut pkt = [0u8; NM_UP_SIZE];
        pkt[0] = TS_SYNC_BYTE; // replace CRC-8 with sync byte
        pkt[1..].copy_from_slice(&self.buf[1..NM_UP_SIZE]);
        out.push(pkt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{Bbheader, Matype, Mode, TsGs};

    /// Plain NM header: ISSYI=0, NPD=0, so UPL=188*8=1504 and the stride is
    /// the bare `CRC-8 + 187-byte UP` (EN 302 755 §5.1.8 figure 5).
    fn make_nm_header(syncd: u16) -> Bbheader {
        Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0,
            },
            upl: (NM_UP_SIZE * 8) as u16,
            sync: 0x47,
            dfl: 0,
            syncd,
            mode: Mode::Normal,
            issy_in_header: None,
        }
    }

    fn make_hem_header(npd: bool) -> Bbheader {
        Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: false,
                npd,
                ext: 0,
                isi: 0,
            },
            upl: 0,
            sync: 0,
            dfl: 0,
            syncd: 0,
            mode: Mode::HighEfficiency,
            issy_in_header: None,
        }
    }

    #[test]
    fn nm_extracts_single_complete_up() {
        let mut data = vec![0xAA; NM_UP_SIZE];
        data[0] = 0xFF; // CRC-8 byte (will be replaced with sync)
        for (i, byte) in data.iter_mut().enumerate().skip(1) {
            *byte = i as u8;
        }

        let _hdr = make_nm_header(0);
        let pkts: Vec<_> = up_iter(&data, &_hdr).collect();

        assert_eq!(pkts.len(), 1);
        assert_eq!(pkts[0][0], TS_SYNC_BYTE);
        assert_eq!(&pkts[0][1..], &data[1..]);
    }

    #[test]
    fn nm_multiple_back_to_back_ups() {
        let num_ups = 3;
        let mut data = Vec::with_capacity(num_ups * NM_UP_SIZE);

        for i in 0..num_ups {
            data.push(0x00); // CRC-8 byte
            for j in 1..NM_UP_SIZE {
                data.push((i * 10 + j) as u8);
            }
        }

        let _hdr = make_nm_header(0);
        let pkts: Vec<_> = up_iter(&data, &_hdr).collect();

        assert_eq!(pkts.len(), num_ups);
        for pkt in &pkts {
            assert_eq!(pkt[0], TS_SYNC_BYTE);
        }
    }

    #[test]
    fn nm_partial_tail_does_not_yield() {
        let mut data = vec![0xAA; NM_UP_SIZE + 50];
        data[0] = 0xFF; // CRC-8
        for (i, byte) in data.iter_mut().enumerate().skip(1) {
            *byte = i as u8;
        }

        let _hdr = make_nm_header(0);
        let pkts: Vec<_> = up_iter(&data, &_hdr).collect();

        assert_eq!(pkts.len(), 1); // Only one complete UP
    }

    #[test]
    fn nm_stride_reflects_issyi_and_npd() {
        // ISSYI=1 (short form, 2 bytes), NPD=0 → 188+2=190.
        let mut hdr = make_nm_header(0);
        hdr.matype.issyi = true;
        hdr.upl = 190 * 8;
        assert_eq!(nm_stride_bytes(&hdr), Some(190));

        // ISSYI=1 (long form, 3 bytes), NPD=1 → 188+3+1=192.
        hdr.matype.npd = true;
        hdr.upl = 192 * 8;
        assert_eq!(nm_stride_bytes(&hdr), Some(192));

        // ISSYI=0, NPD=1 → 188+1=189.
        hdr.matype.issyi = false;
        hdr.upl = 189 * 8;
        assert_eq!(nm_stride_bytes(&hdr), Some(189));
    }

    #[test]
    fn nm_stride_rejects_upl_inconsistent_with_matype() {
        // ISSYI=0 but UPL implies a 1-byte "ISSY" (no such length exists).
        let mut hdr = make_nm_header(0);
        hdr.upl = 189 * 8;
        assert_eq!(nm_stride_bytes(&hdr), None);

        // ISSYI=1 but UPL implies a 4-byte ISSY (only 2 or 3 exist).
        hdr.matype.issyi = true;
        hdr.upl = 192 * 8;
        assert_eq!(nm_stride_bytes(&hdr), None);

        // UPL shorter than a bare CRC-8 + 187-byte UP.
        hdr.matype.issyi = false;
        hdr.upl = 100 * 8;
        assert_eq!(nm_stride_bytes(&hdr), None);

        // UPL not byte-aligned, or zero.
        hdr.upl = 1;
        assert_eq!(nm_stride_bytes(&hdr), None);
        hdr.upl = 0;
        assert_eq!(nm_stride_bytes(&hdr), None);
    }

    #[test]
    fn nm_invalid_upl_bumps_stat_not_fixed_188_stride() {
        // BUG (#1033): a header whose UPL doesn't describe a valid stride
        // must be skipped, not silently mis-framed at a hardcoded 188 bytes.
        let mut hdr = make_nm_header(0);
        hdr.upl = 1; // not byte-aligned
        hdr.dfl = (NM_UP_SIZE * 8) as u16;
        let header_bytes = hdr.serialize();

        let mut extractor = CarryOverExtractor::new();
        let pkts = extractor.feed_nm(&header_bytes, &[0xAAu8; NM_UP_SIZE]);
        assert!(pkts.is_empty());
        assert_eq!(extractor.stats().nm_upl_invalid, 1);
    }

    #[test]
    fn nm_crc8_mismatch_detected_across_two_ups() {
        // Two back-to-back plain (no ISSY/DNP) UPs. The second UP's leading
        // byte should be crc8 of the first UP's 187-byte content; corrupt it
        // and confirm the mismatch is flagged (but the packet is still
        // emitted — diagnostic only).
        let up_a = [0x11u8; NM_UP_SIZE - 1];
        let up_b = [0x22u8; NM_UP_SIZE - 1];
        let correct_crc = crc8(&up_a);

        let mut data = Vec::with_capacity(NM_UP_SIZE * 2);
        data.push(0xEE); // first UP's leading byte: no predecessor, unchecked
        data.extend_from_slice(&up_a);
        data.push(correct_crc ^ 0x01); // deliberately wrong
        data.extend_from_slice(&up_b);

        let mut hdr = make_nm_header(0);
        hdr.dfl = (data.len() * 8) as u16;
        let header_bytes = hdr.serialize();
        let mut extractor = CarryOverExtractor::new();
        let pkts = extractor.feed_nm(&header_bytes, &data);

        assert_eq!(pkts.len(), 2);
        assert_eq!(&pkts[0][1..], &up_a[..]);
        assert_eq!(&pkts[1][1..], &up_b[..]);
        assert_eq!(extractor.stats().crc8_mismatches, 1);
    }

    #[test]
    fn nm_crc8_matches_when_correct() {
        let up_a = [0x33u8; NM_UP_SIZE - 1];
        let up_b = [0x44u8; NM_UP_SIZE - 1];
        let correct_crc = crc8(&up_a);

        let mut data = Vec::with_capacity(NM_UP_SIZE * 2);
        data.push(0xEE);
        data.extend_from_slice(&up_a);
        data.push(correct_crc);
        data.extend_from_slice(&up_b);

        let mut hdr = make_nm_header(0);
        hdr.dfl = (data.len() * 8) as u16;
        let header_bytes = hdr.serialize();
        let mut extractor = CarryOverExtractor::new();
        let pkts = extractor.feed_nm(&header_bytes, &data);

        assert_eq!(pkts.len(), 2);
        assert_eq!(extractor.stats().crc8_mismatches, 0);
    }

    #[test]
    fn nm_crc_byte_replaced_with_sync_only() {
        let mut data = vec![0u8; NM_UP_SIZE];
        data[0] = 0x42; // Some CRC value
        data[1] = 0x47; // Actual sync byte in payload position 1
        for (i, byte) in data.iter_mut().enumerate().skip(2) {
            *byte = i as u8;
        }

        let _hdr = make_nm_header(0);
        let pkt = up_iter(&data, &_hdr).next().unwrap();

        assert_eq!(pkt[0], TS_SYNC_BYTE); // CRC replaced
        assert_eq!(pkt[1], 0x47); // Original byte preserved
        assert_eq!(&pkt[2..], &data[2..]);
    }

    #[test]
    fn hem_extracts_up_with_sync_prepend() {
        let data: Vec<u8> = (0..HEM_UP_SIZE as u8).cycle().take(HEM_UP_SIZE).collect();
        let mut expected = [0u8; NM_UP_SIZE];
        expected[0] = TS_SYNC_BYTE;
        expected[1..].copy_from_slice(&data[..HEM_UP_SIZE]);

        let _hdr = make_hem_header(false);
        let pkt = up_iter(&data, &_hdr).next().unwrap();

        assert_eq!(pkt, expected);
    }

    #[test]
    fn hem_multiple_ups_without_npd() {
        let num_ups = 3;
        let data = vec![0xAB; num_ups * HEM_UP_SIZE];
        let expected: [u8; NM_UP_SIZE] = {
            let mut e = [0xAB; NM_UP_SIZE];
            e[0] = TS_SYNC_BYTE;
            e
        };

        let _hdr = make_hem_header(false);
        let pkts: Vec<_> = up_iter(&data, &_hdr).collect();

        assert_eq!(pkts.len(), num_ups);
        for pkt in &pkts {
            assert_eq!(*pkt, expected);
        }
    }

    #[test]
    fn hem_with_npd_skips_dnp_bytes() {
        let up_size = HEM_UP_SIZE;
        let num_ups = 2;
        // Each UP followed by a DNP byte
        let stride = up_size + 1;
        let mut data = Vec::with_capacity(num_ups * stride);

        for i in 0..num_ups {
            for j in 0..up_size {
                data.push((i * up_size + j) as u8);
            }
            data.push(i as u8); // DNP counter
        }

        let _hdr = make_hem_header(true);
        let pkts: Vec<_> = up_iter(&data, &_hdr).collect();

        assert_eq!(pkts.len(), num_ups);
        for (i, pkt) in pkts.iter().enumerate() {
            assert_eq!(pkt[0], TS_SYNC_BYTE);
            // Verify the 187 bytes match the expected slice
            let offset = i * stride;
            assert_eq!(&pkt[1..], &data[offset..offset + up_size]);
        }
    }

    #[test]
    fn nm_remaining_returns_unconsumed_tail() {
        let data = vec![0xAA; NM_UP_SIZE * 2 + 50];
        let _hdr = make_nm_header(0);
        let mut iter = NmTsIter::new(&data, NM_UP_SIZE);

        let _p1 = iter.next().unwrap();
        let _p2 = iter.next().unwrap();
        let remaining = iter.remaining();

        assert_eq!(remaining.len(), 50);
    }

    #[test]
    fn hem_remaining_returns_unconsumed_tail() {
        let data = vec![0xAA; HEM_UP_SIZE * 2 + 30];
        let _hdr = make_hem_header(false);
        let mut iter = HemTsIter::new(&data, false);

        let _p1 = iter.next().unwrap();
        let _p2 = iter.next().unwrap();
        let remaining = iter.remaining();

        assert_eq!(remaining.len(), 30);
    }

    #[test]
    fn empty_data_yields_nothing() {
        let _hdr = make_nm_header(0);
        let pkts: Vec<_> = up_iter(&[], &_hdr).collect();
        assert!(pkts.is_empty());
    }

    #[test]
    fn data_shorter_than_one_up_yields_nothing() {
        let data = vec![0xAA; 100]; // Less than NM_UP_SIZE or HEM_UP_SIZE
        let _hdr = make_nm_header(0);
        let pkts: Vec<_> = up_iter(&data, &_hdr).collect();
        assert!(pkts.is_empty());
    }

    #[test]
    fn carry_over_extractor_emits_ts_across_two_bbframes_hem() {
        // Two HEM BBFrames where the first ends with a partial UP (70 bytes) and
        // the second completes it (117 bytes) then starts a new UP.
        let make_hem_header = |syncd_bits: u16, dfl_bits: u16| -> [u8; 10] {
            let mut h = [0u8; 10];
            // MATYPE-1: TS=0b11 → 0xC0, SIS=1 → 0x20, CCM=1 → 0x10, ISSYI=0, NPD=0, EXT=0
            h[0] = 0xF0;
            h[1] = 0x00; // ISI=0
            // UPL=0 (ignored in HEM)
            h[2] = 0x00;
            h[3] = 0x00;
            h[4] = (dfl_bits >> 8) as u8;
            h[5] = (dfl_bits & 0xFF) as u8;
            h[6] = 0x00; // sync (ignored in HEM)
            h[7] = (syncd_bits >> 8) as u8;
            h[8] = (syncd_bits & 0xFF) as u8;
            // byte 9 = CRC-8 XOR MODE with MODE=1 for HEM
            let crc = crate::crc::crc8(&h[..9]);
            h[9] = crc ^ 1;
            h
        };

        // Two BBFrames, each 70 data bytes. Pattern: each data byte is (frame << 4) | offset_lo.
        // We expect CarryOverExtractor to produce 0 packets on frame1 (partial tail),
        // then 1 on frame2 (187-byte completion across the boundary).
        let frame1_data = (0..70u8).map(|i| 0xA0 | (i & 0x0F)).collect::<Vec<u8>>();
        let frame2_data = (0..200u8).map(|i| 0xB0 | (i & 0x0F)).collect::<Vec<u8>>();
        let hdr1 = make_hem_header(0, (frame1_data.len() * 8) as u16);
        let hdr2 = make_hem_header(0, (frame2_data.len() * 8) as u16);

        let mut extractor = CarryOverExtractor::new();
        let packets1 = extractor.feed_hem(&hdr1, &frame1_data, false);
        assert_eq!(packets1.len(), 0, "70 bytes (< 187) must not yet emit a UP");

        let packets2 = extractor.feed_hem(&hdr2, &frame2_data, false);
        assert!(!packets2.is_empty(), "boundary UP should complete");
        assert_eq!(
            packets2[0][0], 0x47,
            "first emitted packet has sync byte prepended"
        );
    }

    #[test]
    fn carry_over_hem_completion_success_path() {
        // Exercises the carry-over COMPLETION path (feed_hem_into success branch
        // `syncd_bytes == need`): frame2's syncd must point exactly past the bytes
        // that finish frame1's partial UP. The sibling test above uses syncd=0, so
        // it hits the discard branch instead — this one covers the success branch.
        let make_hem_header = |syncd_bits: u16, dfl_bits: u16| -> [u8; 10] {
            let mut h = [0u8; 10];
            h[0] = 0xF0; // TS, SIS, CCM (HEM via byte-9 MODE xor)
            h[4] = (dfl_bits >> 8) as u8;
            h[5] = (dfl_bits & 0xFF) as u8;
            h[7] = (syncd_bits >> 8) as u8;
            h[8] = (syncd_bits & 0xFF) as u8;
            h[9] = crate::crc::crc8(&h[..9]) ^ 1; // MODE=1 (HEM)
            h
        };

        // Frame1: 70-byte partial UP → after it, extractor.pos = 1 + 70 = 71.
        let frame1: Vec<u8> = (0..70u8).map(|i| 0xA0 | (i & 0x0F)).collect();
        // need = HEM_UP_SIZE(187) + 1 - pos(71) = 117. syncd must equal `need`.
        let need = 117usize;
        // Frame2: `need` completion bytes, then one fresh 187-byte UP.
        let frame2: Vec<u8> = (0..(need + HEM_UP_SIZE) as u16)
            .map(|i| 0xB0 | (i & 0x0F) as u8)
            .collect();
        let h1 = make_hem_header(0, (frame1.len() * 8) as u16);
        let h2 = make_hem_header((need * 8) as u16, (frame2.len() * 8) as u16);

        let mut ex = CarryOverExtractor::new();
        let p1 = ex.feed_hem(&h1, &frame1, false);
        assert_eq!(p1.len(), 0, "frame1's 70-byte partial emits nothing yet");

        let p2 = ex.feed_hem(&h2, &frame2, false);
        assert_eq!(
            ex.stats().partial_discards,
            0,
            "completion path must be taken, NOT the discard branch"
        );
        assert_eq!(p2.len(), 2, "completed boundary UP + one fresh UP");
        // Completed UP = sync + frame1's 70 carried bytes + frame2's 117 completion bytes.
        assert_eq!(p2[0][0], TS_SYNC_BYTE);
        assert_eq!(&p2[0][1..71], &frame1[..]);
        assert_eq!(&p2[0][71..188], &frame2[..need]);
        // Fresh UP = sync + frame2[need..need+187].
        assert_eq!(p2[1][0], TS_SYNC_BYTE);
        assert_eq!(&p2[1][1..188], &frame2[need..need + HEM_UP_SIZE]);
    }

    #[test]
    fn feed_into_matches_allocating_api() {
        // Run the same two-frame HEM sequence (partial UP carried across the
        // boundary) through the allocating `feed_hem` and the buffer-reusing
        // `feed_hem_into` (one Vec reused across both frames). Identical output
        // proves `_into` clears + appends equivalently — if it failed to clear,
        // frame 2's buffer would still hold frame 1's packets and diverge.
        let make_hem_header = |syncd_bits: u16, dfl_bits: u16| -> [u8; 10] {
            let mut h = [0u8; 10];
            h[0] = 0xF0;
            h[4] = (dfl_bits >> 8) as u8;
            h[5] = (dfl_bits & 0xFF) as u8;
            h[7] = (syncd_bits >> 8) as u8;
            h[8] = (syncd_bits & 0xFF) as u8;
            let crc = crate::crc::crc8(&h[..9]);
            h[9] = crc ^ 1;
            h
        };
        let f1 = (0..70u8).map(|i| 0xA0 | (i & 0x0F)).collect::<Vec<u8>>();
        let f2 = (0..200u8).map(|i| 0xB0 | (i & 0x0F)).collect::<Vec<u8>>();
        let h1 = make_hem_header(0, (f1.len() * 8) as u16);
        let h2 = make_hem_header(0, (f2.len() * 8) as u16);

        let mut alloc = CarryOverExtractor::new();
        let a1 = alloc.feed_hem(&h1, &f1, false);
        let a2 = alloc.feed_hem(&h2, &f2, false);

        let mut reuse = CarryOverExtractor::new();
        let mut buf = Vec::new();
        reuse.feed_hem_into(&h1, &f1, false, &mut buf);
        let b1 = buf.clone();
        reuse.feed_hem_into(&h2, &f2, false, &mut buf);
        let b2 = buf.clone();

        assert_eq!(a1, b1, "frame 1 output matches across APIs");
        assert_eq!(
            a2, b2,
            "frame 2 (carry-over) output matches; buffer was cleared"
        );
    }

    #[test]
    fn remaining_safe_when_pos_equals_len() {
        let data = vec![0xAA; NM_UP_SIZE];
        let mut iter = NmTsIter::new(&data, NM_UP_SIZE);
        let _p = iter.next().unwrap();
        // pos == data.len() — must not panic
        let remaining = iter.remaining();
        assert!(remaining.is_empty());
    }

    #[test]
    fn remaining_safe_when_pos_exceeds_len() {
        // Construct an iterator and manually set pos beyond data length.
        // This cannot happen through normal iteration, but the safe
        // get().unwrap_or(&[]) handles it gracefully.
        let data = vec![0xAA; 10];
        let iter = NmTsIter {
            data: &data,
            pos: 20,
            stride: NM_UP_SIZE,
        };
        let remaining = iter.remaining();
        assert!(remaining.is_empty());
    }

    /// Build a serialised HEM BBHEADER with given syncd (bits) and dfl (bits).
    fn make_hem_hdr_bytes(syncd_bits: u16, dfl_bits: u16) -> [u8; 10] {
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0,
            },
            upl: 0,
            sync: 0,
            dfl: dfl_bits,
            syncd: syncd_bits,
            mode: crate::header::Mode::HighEfficiency,
            issy_in_header: None,
        };
        hdr.serialize()
    }

    #[test]
    fn syncd_65535_hem_continues_carry_over_without_partial_discard() {
        // BUG 2 regression: SYNCD=65535 means "no UP starts in this DATA FIELD" —
        // the entire data field is a continuation of the carried-over partial UP.
        // The old code computed syncd_bytes = 65535/8 = 8191, which never matched
        // `need`, and took the discard branch (partial_discards++).
        //
        // Test sequence (HEM, stride=187):
        //   Frame A: data field = first 100 bytes of a 187-byte UP.
        //            SYNCD=0 (UP starts at byte 0).  Extractor must carry 100 bytes.
        //   Frame B: data field = next 87 bytes of the SAME UP.
        //            SYNCD=0xFFFF (no new UP starts).  Must APPEND to partial, emit UP.
        //   No partial_discards should occur.

        // Build recognisable UP content: bytes 0..186 = 0x00..0xBA (distinct from sync)
        let up_payload: Vec<u8> = (0u8..187).collect(); // 187 bytes

        // Frame A: send the first 100 bytes; DFL = 100*8 bits; SYNCD=0.
        let frame_a_data: Vec<u8> = up_payload[..100].to_vec();
        let hdr_a = make_hem_hdr_bytes(0, (frame_a_data.len() * 8) as u16);

        // Frame B: send the remaining 87 bytes; SYNCD=0xFFFF (no new UP starts).
        // DFL = 87*8 bits.
        let frame_b_data: Vec<u8> = up_payload[100..].to_vec();
        let hdr_b = make_hem_hdr_bytes(0xFFFF, (frame_b_data.len() * 8) as u16);

        let mut extractor = CarryOverExtractor::new();

        let pkts_a = extractor.feed_hem(&hdr_a, &frame_a_data, false);
        assert_eq!(
            pkts_a.len(),
            0,
            "frame A: 100 bytes < 187, must not emit yet"
        );

        let pkts_b = extractor.feed_hem(&hdr_b, &frame_b_data, false);
        assert_eq!(
            extractor.stats().partial_discards,
            0,
            "SYNCD=0xFFFF must NOT trigger a partial discard"
        );
        assert_eq!(
            pkts_b.len(),
            1,
            "SYNCD=0xFFFF: the UP must complete and be emitted"
        );
        assert_eq!(pkts_b[0][0], TS_SYNC_BYTE, "sync byte prepended correctly");
        // Verify the 187 payload bytes are correct: [sync][up_payload[0..187]].
        assert_eq!(
            &pkts_b[0][1..],
            up_payload.as_slice(),
            "completed UP must contain the exact original 187-byte payload"
        );
    }

    #[test]
    fn stats_count_npd_skip_and_mode_mismatch() {
        let mut ext = CarryOverExtractor::new();
        let mut out = Vec::new();

        // Valid HEM header but NPD set: unsupported → no output, counted as a
        // dropped-valid-data event (not wire corruption).
        let hem = make_hem_header(true).serialize();
        ext.feed_hem_into(&hem, &[0u8; NM_UP_SIZE], true, &mut out);
        assert!(out.is_empty());
        assert_eq!(ext.stats().npd_unsupported, 1);

        // NM header fed to the HEM path → mode mismatch, counted.
        let nm = make_nm_header(0).serialize();
        ext.feed_hem_into(&nm, &[0u8; NM_UP_SIZE], false, &mut out);
        assert!(out.is_empty());
        assert_eq!(ext.stats().mode_mismatches, 1);
        // Earlier counter is unchanged.
        assert_eq!(ext.stats().npd_unsupported, 1);
    }
}
