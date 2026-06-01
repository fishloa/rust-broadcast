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
        &self.data[self.pos..]
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
        &self.data[self.pos..]
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

/// Build an appropriate user-packet iterator for the given BBHEADER.
///
/// Returns either an NM or HEM iterator depending on the detected mode.
/// The caller must handle SYNCD alignment before calling this — typically
/// by skipping `syncd / 8` bytes at the start of the data field.
pub fn up_iter<'a>(
    data: &'a [u8],
    bbheader: &Bbheader,
) -> Box<dyn Iterator<Item = [u8; NM_UP_SIZE]> + 'a> {
    match bbheader.mode {
        Mode::Normal => Box::new(NmTsIter::new(data)),
        Mode::HighEfficiency => Box::new(HemTsIter::new(data, bbheader.matype.npd)),
    }
}

/// Stateful UP extractor that carries partial user packets across BBFrame
/// boundaries — a single UP can span multiple frames, especially in HEM
/// where stride=187 bytes.
///
/// Use `feed_nm` / `feed_hem` per received frame; the returned Vec holds
/// whichever 188-byte TS packets completed during that frame.
pub struct CarryOverExtractor {
    /// Partial TS packet being assembled (sync byte position 0).
    buf: [u8; NM_UP_SIZE],
    /// Bytes already written into `buf`.
    pos: usize,
}

impl Default for CarryOverExtractor {
    fn default() -> Self {
        Self {
            buf: [0u8; NM_UP_SIZE],
            pos: 0,
        }
    }
}

impl CarryOverExtractor {
    /// Create a fresh extractor with no carried-over state.
    pub fn new() -> Self {
        Self::default()
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
        assert!(
            !npd,
            "CarryOverExtractor does not yet implement NPD/DNP reinsertion"
        );
        let hdr = match Bbheader::parse(bbheader_bytes) {
            Ok(h) => h,
            Err(_) => return Vec::new(),
        };
        assert_eq!(
            hdr.mode,
            Mode::HighEfficiency,
            "feed_hem called on non-HEM header"
        );

        let stride = HEM_UP_SIZE;
        let syncd_bytes = (hdr.syncd / 8) as usize;
        let dfl_bytes = (hdr.dfl / 8) as usize;
        let data = &data_field[..dfl_bytes.min(data_field.len())];

        let mut out = Vec::new();

        // Complete the partial UP from the previous frame.
        if self.pos > 0 {
            let need = stride + 1 - self.pos; // +1 for the sync byte we'll prepend
            if syncd_bytes == need && data.len() >= need {
                self.buf[self.pos..self.pos + need].copy_from_slice(&data[..need]);
                self.buf[0] = 0x47;
                out.push(self.buf);
                self.pos = 0;
            } else {
                // Stride mismatch — discard partial and resync to syncd.
                self.pos = 0;
            }
        }

        // Extract complete UPs at stride.
        let mut i = syncd_bytes;
        while i + stride <= data.len() {
            // HEM: 187 bytes of UP, prepend 0x47.
            self.buf[0] = 0x47;
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

        out
    }

    /// Feed an NM BBFrame. Stride=188, byte[0] of each UP is the CRC-8 of
    /// the previous UP and gets replaced with 0x47.
    pub fn feed_nm(
        &mut self,
        bbheader_bytes: &[u8; BBHEADER_LEN],
        data_field: &[u8],
    ) -> Vec<[u8; NM_UP_SIZE]> {
        let hdr = match Bbheader::parse(bbheader_bytes) {
            Ok(h) => h,
            Err(_) => return Vec::new(),
        };
        assert_eq!(hdr.mode, Mode::Normal, "feed_nm called on non-NM header");

        let stride = NM_UP_SIZE;
        let syncd_bytes = (hdr.syncd / 8) as usize;
        let dfl_bytes = (hdr.dfl / 8) as usize;
        let data = &data_field[..dfl_bytes.min(data_field.len())];

        let mut out = Vec::new();

        // Complete partial UP from previous frame.
        if self.pos > 0 {
            let need = stride - self.pos;
            if syncd_bytes == need && data.len() >= need {
                self.buf[self.pos..self.pos + need].copy_from_slice(&data[..need]);
                self.buf[0] = 0x47; // replace CRC-8 with sync byte
                out.push(self.buf);
                self.pos = 0;
            } else {
                self.pos = 0;
            }
        }

        // Extract complete UPs at stride.
        let mut i = syncd_bytes;
        while i + stride <= data.len() {
            self.buf.copy_from_slice(&data[i..i + stride]);
            self.buf[0] = 0x47; // replace CRC-8 with sync byte
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

        out
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
}
