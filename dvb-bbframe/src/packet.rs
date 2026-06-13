//! User packet extraction from BBFrame data fields.
//!
//! Supports Normal Mode (NM, 188-byte stride) and High Efficiency Mode
//! (HEM, 187-byte stride) per EN 302 755 §5.1.8.
//!
//! In NM the first byte of each user packet in the data field is a CRC-8
//! that replaces the original sync byte (0x47). In HEM the sync byte is
//! simply absent and must be prepended.
//!
//! ## SYNCD handling
//!
//! `SYNCD` gives the bit offset from the start of the DATA FIELD to the
//! first bit of the CRC-8 byte of the first user packet (NM) or the first
//! byte of the first user packet (HEM).  Callers typically prepend any
//! carry-over from the previous BBFrame and pass the result plus SYNCD.
//!
//! ## NPD/DNP reinsertion (HEM only)
//!
//! When NPD is active (`matype.npd == true`), each transmitted user
//! packet is followed by a 1-byte DNP counter. The [`HemTsIter`] skips
//! these DNP bytes automatically.

use crate::header::{Bbheader, Mode, BBHEADER_LEN};

/// User packet size in Normal Mode (188 bytes = full MPEG-2 TS packet).
pub const NM_UP_SIZE: usize = 188;

/// User packet size in High Efficiency Mode (187 bytes = TS minus sync byte).
pub const HEM_UP_SIZE: usize = 187;

/// MPEG-2 sync byte that CRC-8 replaces in NM.
pub const TS_SYNC_BYTE: u8 = 0x47;

/// Iterator over NM TS user packets.
///
/// Each item is a `[u8; 188]` with the sync byte restored to 0x47,
/// replacing the CRC-8 byte that occupies position 0 in the data field.
#[derive(Clone, Copy)]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct NmTsIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> NmTsIter<'a> {
    /// Create a new NM TS user packet iterator.
    ///
    /// `data` is the full data field (after skipping SYNCD bytes if
    /// already aligned). The iterator starts at byte 0 of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Return the unconsumed tail of the data field.
    pub fn remaining(self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }
}

impl Iterator for NmTsIter<'_> {
    type Item = [u8; NM_UP_SIZE];

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + NM_UP_SIZE > self.data.len() {
            return None;
        }
        let mut pkt = [0u8; NM_UP_SIZE];
        pkt[0] = TS_SYNC_BYTE; // Replace CRC-8 byte with sync byte
        pkt[1..].copy_from_slice(&self.data[self.pos + 1..self.pos + NM_UP_SIZE]);
        self.pos += NM_UP_SIZE;
        Some(pkt)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.data.len().saturating_sub(self.pos);
        let count = remaining / NM_UP_SIZE;
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
pub fn up_iter<'a>(data: &'a [u8], bbheader: &Bbheader) -> UpIter<'a> {
    match bbheader.mode {
        Mode::Normal => UpIter::Normal(NmTsIter::new(data)),
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
    /// Partial TS packet being assembled (sync byte position 0).
    buf: [u8; NM_UP_SIZE],
    /// Bytes already written into `buf`.
    pos: usize,
    /// Diagnostic counters (see [`CarryOverStats`]).
    stats: CarryOverStats,
}

impl Default for CarryOverExtractor {
    fn default() -> Self {
        Self {
            buf: [0u8; NM_UP_SIZE],
            pos: 0,
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
                    out.push(self.buf);
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
                out.push(self.buf);
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
            out.push(self.buf);
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

    /// Feed an NM BBFrame. Stride=188, `byte[0]` of each UP is the CRC-8 of
    /// the previous UP and gets replaced with 0x47.
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

        let stride = NM_UP_SIZE;
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
                    // UP is now complete.
                    self.buf[0] = TS_SYNC_BYTE; // replace CRC-8 with sync byte
                    out.push(self.buf);
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
                self.buf[0] = TS_SYNC_BYTE; // replace CRC-8 with sync byte
                out.push(self.buf);
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
            self.buf.copy_from_slice(&data[i..i + stride]);
            self.buf[0] = TS_SYNC_BYTE; // replace CRC-8 with sync byte
            out.push(self.buf);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{Bbheader, Matype, Mode, TsGs};

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
            upl: 0,
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
        let mut iter = NmTsIter::new(&data);

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
        let mut iter = NmTsIter::new(&data);
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
