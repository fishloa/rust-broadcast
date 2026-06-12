//! Inner-TS recovery — the single driver from a T2-MI PID to the inner MPEG-TS.
//!
//! ETSI TS 102 773 carries a DVB-T2 modulator feed as T2-MI packets inside an
//! outer MPEG-TS; the payload TS that a receiver ultimately decodes lives inside
//! the BBFrames of those T2-MI packets (EN 302 755 §5.1, baseband framing).
//! Recovering it is a fixed three-stage pipeline — [`T2miPump`] (PID filter +
//! CRC-validated T2-MI packets) → [`AnyPayload::Bbframe`] → `dvb_bbframe`
//! `Bbheader` + [`CarryOverExtractor`] (BBHEADER parse, mode handling, SYNCD
//! carry-over across frames). [`InnerTsRecovery`] folds that whole chain into
//! one feed-and-collect type so callers don't re-wire it.
//!
//! ```no_run
//! # #[cfg(feature = "ts")] {
//! use dvb_t2mi::inner_ts::InnerTsRecovery;
//! let mut rec = InnerTsRecovery::new(0x1000); // the T2-MI PID
//! for ts_packet in outer_stream() {            // 188-byte outer TS packets
//!     for inner in rec.feed(&ts_packet) {       // recovered inner TS packets
//!         feed_to_si_demux(inner);
//!     }
//! }
//! # fn outer_stream() -> Vec<[u8; 188]> { vec![] }
//! # fn feed_to_si_demux(_p: &[u8; 188]) {}
//! # }
//! ```

use dvb_bbframe::header::{Bbheader, Mode, BBHEADER_LEN};
use dvb_bbframe::packet::{CarryOverExtractor, NM_UP_SIZE};

use crate::payload::AnyPayload;
use crate::pump::{Stats, T2miPump};

/// Recovers the inner MPEG-TS carried inside a T2-MI stream.
///
/// Feed outer 188-byte TS packets from the T2-MI PID with [`feed`](Self::feed);
/// each call returns the inner TS packets recovered from that input packet
/// (often empty — a BBFrame spans several T2-MI packets). The driver owns the
/// pump, the carry-over extractor, and the NM/HEM mode handling.
///
/// Normal Mode and High-Efficiency Mode (without Null-Packet-Deletion) frames
/// are recovered; HEM frames with `MATYPE.NPD` set are skipped (DNP-byte
/// reinsertion is not modelled by the extractor) rather than mis-decoded.
pub struct InnerTsRecovery {
    pump: T2miPump,
    extractor: CarryOverExtractor,
    out: Vec<[u8; NM_UP_SIZE]>,
    up_buf: Vec<[u8; NM_UP_SIZE]>,
    target_plp: Option<u8>,
    filtered_out: u64,
}

impl InnerTsRecovery {
    /// Create a recovery driver filtering the outer TS for `t2mi_pid`.
    /// All PLPs are unfiltered.
    #[must_use]
    pub fn new(t2mi_pid: u16) -> Self {
        Self::build(t2mi_pid, None)
    }

    /// Create a recovery driver that only recovers inner TS from the given
    /// baseband frame PLP (`plp_id`). BBFrames from other PLPs are counted
    /// by [`filtered_bbframes`](Self::filtered_bbframes).
    #[must_use]
    pub fn new_for_plp(t2mi_pid: u16, plp_id: u8) -> Self {
        Self::build(t2mi_pid, Some(plp_id))
    }

    fn build(t2mi_pid: u16, target_plp: Option<u8>) -> Self {
        Self {
            pump: T2miPump::new(t2mi_pid),
            extractor: CarryOverExtractor::new(),
            out: Vec::new(),
            up_buf: Vec::new(),
            target_plp,
            filtered_out: 0,
        }
    }

    /// Feed one outer 188-byte TS packet; returns the inner TS packets recovered
    /// from it. The returned slice borrows an internal buffer that is cleared on
    /// every call, so copy out anything you need to keep.
    pub fn feed(&mut self, ts_packet: &[u8]) -> &[[u8; NM_UP_SIZE]] {
        self.out.clear();
        // Collect the pump's events first: `feed_ts` borrows `self.pump`, and the
        // loop body mutates `self.extractor`/`self.up_buf`/`self.out`. T2miEvent
        // owns its bytes, so collecting is cheap and releases the pump borrow.
        let events: Vec<_> = self.pump.feed_ts(ts_packet).collect();
        for event in events {
            let Ok(AnyPayload::Bbframe(bb)) = event.payload() else {
                continue;
            };
            if self.target_plp.is_some_and(|t| bb.plp_id != t) {
                self.filtered_out += 1;
                continue;
            }
            if bb.bbframe.len() < BBHEADER_LEN {
                continue;
            }
            let Ok(hdr) = Bbheader::parse(bb.bbframe) else {
                continue;
            };
            let header_bytes: [u8; BBHEADER_LEN] = match bb.bbframe[..BBHEADER_LEN].try_into() {
                Ok(b) => b,
                Err(_) => continue,
            };
            let data_field = &bb.bbframe[BBHEADER_LEN..];
            match hdr.mode {
                Mode::Normal => {
                    self.extractor
                        .feed_nm_into(&header_bytes, data_field, &mut self.up_buf);
                }
                Mode::HighEfficiency if !hdr.matype.npd => {
                    self.extractor.feed_hem_into(
                        &header_bytes,
                        data_field,
                        false,
                        &mut self.up_buf,
                    );
                }
                // HEM with NPD, or any future mode: skip (not recoverable here).
                _ => continue,
            }
            self.out.append(&mut self.up_buf);
        }
        &self.out
    }

    /// Pump statistics (packets seen, CRC failures, …) — passthrough to the
    /// underlying [`T2miPump::stats`].
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.pump.stats()
    }

    /// Number of BBFrames filtered out because their PLP did not match the
    /// target set via [`new_for_plp`](Self::new_for_plp). Always zero when
    /// constructed with [`new`](Self::new).
    #[must_use]
    pub fn filtered_bbframes(&self) -> u64 {
        self.filtered_out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_bbframe::crc::crc8;
    use dvb_bbframe::header::{Matype, TsGs};
    use dvb_common::crc32_mpeg2;

    const TS_SYNC: u8 = 0x47;
    const TS_LEN: usize = 188;

    /// One inner TS packet: PID 0x0100, PUSI, all-0xAA payload (distinguishable).
    fn inner_packet() -> [u8; TS_LEN] {
        let mut p = [0xAAu8; TS_LEN];
        p[0] = TS_SYNC;
        p[1] = 0x41; // PUSI | PID hi = 0x0100
        p[2] = 0x00;
        p[3] = 0x10; // payload only
        p
    }

    /// Wrap one inner TS packet in a Normal-Mode BBFrame (mirrors tests/chain.rs).
    fn nm_bbframe(inner: &[u8; TS_LEN]) -> Vec<u8> {
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
            upl: 1504,
            sync: TS_SYNC,
            dfl: 1504,
            syncd: 0,
            mode: Mode::Normal,
            issy_in_header: None,
        };
        let mut frame = hdr.serialize().to_vec();
        let mut data = [0u8; TS_LEN];
        data[0] = crc8(&[0u8; TS_LEN]); // CRC-8 of the (all-zero) previous UP
        data[1..].copy_from_slice(&inner[1..]);
        frame.extend_from_slice(&data);
        frame
    }

    /// Wrap a BBFrame in a T2-MI BBFrame packet (type 0x00) with CRC-32.
    fn t2mi_packet(bbframe: &[u8]) -> Vec<u8> {
        let mut payload = vec![0x00, 0x05, 0x80]; // frame_idx, plp_id, intl_frame_start
        payload.extend_from_slice(bbframe);
        let mut pkt = vec![0x00u8, 0x01, 0x00, 0x00];
        pkt.extend_from_slice(&((payload.len() * 8) as u16).to_be_bytes());
        pkt.extend_from_slice(&payload);
        let crc = crc32_mpeg2::compute(&pkt);
        pkt.extend_from_slice(&crc.to_be_bytes());
        pkt
    }

    /// Wrap T2-MI data in outer TS packets on `pid` (PUSI + continuation).
    fn outer_ts(pid: u16, data: &[u8]) -> Vec<[u8; TS_LEN]> {
        let mut out = Vec::new();
        let first_cap = TS_LEN - 5;
        let cont_cap = TS_LEN - 4;
        let mut off = 0;
        let mut first = true;
        while off < data.len() {
            let mut pkt = [0xFFu8; TS_LEN];
            pkt[0] = TS_SYNC;
            let cap = if first { first_cap } else { cont_cap };
            pkt[1] = (if first { 0x40 } else { 0x00 }) | (((pid >> 8) as u8) & 0x1F);
            pkt[2] = (pid & 0xFF) as u8;
            pkt[3] = 0x10;
            let hdr_len = if first {
                pkt[4] = 0x00; // pointer_field
                5
            } else {
                4
            };
            let n = (data.len() - off).min(cap);
            pkt[hdr_len..hdr_len + n].copy_from_slice(&data[off..off + n]);
            out.push(pkt);
            off += n;
            first = false;
        }
        out
    }

    #[test]
    fn recovers_inner_ts_from_nm_bbframe_chain() {
        let pid = 0x1000;
        let inner = inner_packet();
        let outer = outer_ts(pid, &t2mi_packet(&nm_bbframe(&inner)));

        let mut rec = InnerTsRecovery::new(pid);
        let mut recovered: Vec<[u8; TS_LEN]> = Vec::new();
        for pkt in &outer {
            recovered.extend_from_slice(rec.feed(pkt));
        }

        assert_eq!(recovered.len(), 1, "exactly one inner TS packet expected");
        assert_eq!(recovered[0][0], TS_SYNC, "sync byte restored");
        // Bytes 1..188 survive the NM round-trip verbatim (byte 0 is re-synced).
        assert_eq!(&recovered[0][1..], &inner[1..]);
    }

    #[test]
    fn wrong_pid_yields_nothing() {
        let inner = inner_packet();
        let outer = outer_ts(0x1000, &t2mi_packet(&nm_bbframe(&inner)));
        let mut rec = InnerTsRecovery::new(0x0064); // different PID
        let mut n = 0;
        for pkt in &outer {
            n += rec.feed(pkt).len();
        }
        assert_eq!(n, 0);
    }

    #[test]
    fn garbage_packet_no_panic_no_output() {
        let mut rec = InnerTsRecovery::new(0x1000);
        let junk = [0u8; TS_LEN];
        assert!(rec.feed(&junk).is_empty());
    }

    /// Wrap a BBFrame in a T2-MI BBFrame packet with a chosen PLP.
    fn t2mi_packet_for_plp(bbframe: &[u8], plp_id: u8) -> Vec<u8> {
        let mut payload = vec![0x00, plp_id, 0x80]; // frame_idx=0, plp_id, intl_frame_start
        payload.extend_from_slice(bbframe);
        let mut pkt = vec![0x00u8, 0x01, 0x00, 0x00];
        pkt.extend_from_slice(&((payload.len() * 8) as u16).to_be_bytes());
        pkt.extend_from_slice(&payload);
        let crc = crc32_mpeg2::compute(&pkt);
        pkt.extend_from_slice(&crc.to_be_bytes());
        pkt
    }

    /// A distinguishable inner TS packet with a marker byte at offset 4.
    fn tagged_inner_packet(marker: u8) -> [u8; TS_LEN] {
        let mut p = [0xAAu8; TS_LEN];
        p[0] = TS_SYNC;
        p[1] = 0x41;
        p[2] = 0x00;
        p[3] = 0x10;
        p[4] = marker;
        p
    }

    #[test]
    fn plp_filter_keeps_only_target_plp() {
        let pid = 0x1000;
        let inner_plp0 = tagged_inner_packet(0xA0);
        let inner_plp1 = tagged_inner_packet(0xB0);

        // Build two BBFrames (one per PLP), each carrying one inner TS packet.
        let bb_plp0 = nm_bbframe(&inner_plp0);
        let bb_plp1 = nm_bbframe(&inner_plp1);

        // Wrap each in a T2-MI packet with the correct PLP id.
        let t2mi_plp0 = t2mi_packet_for_plp(&bb_plp0, 0);
        let t2mi_plp1 = t2mi_packet_for_plp(&bb_plp1, 1);

        // Interleave: PLP 0 then PLP 1 (two separate T2-MI packets).
        let mut combined = t2mi_plp0;
        combined.extend_from_slice(&t2mi_plp1);
        let outer = outer_ts(pid, &combined);

        // --- Test: new_for_plp(pid, 0) should only get plp 0 ---
        let mut rec0 = InnerTsRecovery::new_for_plp(pid, 0);
        let mut recovered_0: Vec<[u8; TS_LEN]> = Vec::new();
        for pkt in &outer {
            recovered_0.extend_from_slice(rec0.feed(pkt));
        }
        assert_eq!(
            recovered_0.len(),
            1,
            "plp 0 filter should recover exactly one inner packet"
        );
        assert_eq!(recovered_0[0][4], 0xA0, "should be the plp 0 packet");
        assert_eq!(
            rec0.filtered_bbframes(),
            1,
            "one BBFRAME (plp 1) filtered out"
        );

        // --- Test: new_for_plp(pid, 1) should only get plp 1 ---
        let mut rec1 = InnerTsRecovery::new_for_plp(pid, 1);
        let mut recovered_1: Vec<[u8; TS_LEN]> = Vec::new();
        for pkt in &outer {
            recovered_1.extend_from_slice(rec1.feed(pkt));
        }
        assert_eq!(
            recovered_1.len(),
            1,
            "plp 1 filter should recover exactly one inner packet"
        );
        assert_eq!(recovered_1[0][4], 0xB0, "should be the plp 1 packet");
        assert_eq!(
            rec1.filtered_bbframes(),
            1,
            "one BBFRAME (plp 0) filtered out"
        );

        // --- Test: new(pid) should get BOTH ---
        let mut all = InnerTsRecovery::new(pid);
        let mut recovered_all: Vec<[u8; TS_LEN]> = Vec::new();
        for pkt in &outer {
            recovered_all.extend_from_slice(all.feed(pkt));
        }
        assert_eq!(
            recovered_all.len(),
            2,
            "unfiltered should recover both inner packets"
        );
        assert_eq!(
            all.filtered_bbframes(),
            0,
            "no filtering when target is None"
        );
    }
}
