//! Shared helpers for the v2 codec-level checks (issue #567).
//!
//! Each codec check needs the PMT-declared `stream_type` (ISO/IEC 13818-1
//! Table 2-34) for every elementary stream PID, plus per-PID access-unit
//! reassembly. This module factors out the bits every check in
//! `codec_signalling`/`param_sets`/`interlace` would otherwise duplicate:
//! PAT/PMT discovery (typed via `dvb-si`) and PES→access-unit reassembly (via
//! `mpeg-pes`), reusing `mpeg-ts`'s `SectionReassembler` the same way
//! `PatPmtVersionCheck` does.
//!
//! No NAL/SPS parsing lives here — that stays in `transmux`
//! (`transmux::nal`, `transmux::annexb`, `transmux::decode_avc_sps`,
//! `transmux::decode_hevc_sps`); this module only locates the elementary
//! streams, their PMT-declared type, and hands each check the PES-stripped
//! elementary-stream bytes for a completed access unit.

use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;

use broadcast_common::Parse;
use dvb_si::tables::pat::PatSection;
use dvb_si::tables::pmt::{PmtSection, StreamType};
use mpeg_pes::PesAssembler;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};

/// One elementary stream declared by a program's PMT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeclaredStream {
    /// Elementary stream PID.
    pub pid: u16,
    /// PMT-declared `stream_type` (ISO/IEC 13818-1 Table 2-34).
    pub stream_type: StreamType,
}

/// Walk `ts` following PAT → every PMT, returning every elementary stream
/// declared across all programs (in PMT wire order, PAT-discovery order).
///
/// Malformed/unparseable PAT or PMT sections are skipped rather than
/// propagated — a codec check degrades to "nothing declared" on a broken PSI
/// layer instead of panicking; other diagnostics (e.g. `PatPmtVersionCheck`)
/// already cover PSI-layer faults.
pub(crate) fn collect_pmt_streams(ts: &[u8]) -> Vec<DeclaredStream> {
    let n_packets = ts.len() / TS_PACKET_SIZE;
    let mut reassemblers: BTreeMap<u16, SectionReassembler> = BTreeMap::new();
    reassemblers.entry(dvb_si::tables::pat::PID).or_default();

    let mut pmt_pids: Vec<u16> = Vec::new();
    let mut declared: Vec<DeclaredStream> = Vec::new();

    for i in 0..n_packets {
        let offset = i * TS_PACKET_SIZE;
        let raw = &ts[offset..offset + TS_PACKET_SIZE];
        let Ok(pkt) = TsPacket::parse(raw) else {
            continue;
        };
        let pid = pkt.header.pid;
        if !reassemblers.contains_key(&pid) {
            continue;
        }
        let Some(payload) = pkt.payload else {
            continue;
        };
        let pusi = pkt.header.pusi;
        reassemblers.get_mut(&pid).unwrap().feed(payload, pusi);

        let mut new_pmt_pids: Vec<u16> = Vec::new();
        while let Some(section) = reassemblers.get_mut(&pid).unwrap().pop_section() {
            if pid == dvb_si::tables::pat::PID {
                if let Ok(pat) = PatSection::parse(&section) {
                    for entry in &pat.entries {
                        if entry.program_number != dvb_si::tables::pat::PROGRAM_NUMBER_NIT
                            && !pmt_pids.contains(&entry.pid)
                            && !new_pmt_pids.contains(&entry.pid)
                        {
                            new_pmt_pids.push(entry.pid);
                        }
                    }
                }
            } else if let Ok(pmt) = PmtSection::parse(&section) {
                for stream in &pmt.streams {
                    declared.push(DeclaredStream {
                        pid: stream.elementary_pid,
                        stream_type: stream.stream_type,
                    });
                }
            }
        }
        for pmt_pid in new_pmt_pids {
            pmt_pids.push(pmt_pid);
            reassemblers.entry(pmt_pid).or_default();
        }
    }

    declared
}

/// Elementary-stream PIDs among `streams` matching `stream_type`, in PMT wire
/// order.
pub(crate) fn pids_with_stream_type(
    streams: &[DeclaredStream],
    stream_type: StreamType,
) -> Vec<u16> {
    streams
        .iter()
        .filter(|s| s.stream_type == stream_type)
        .map(|s| s.pid)
        .collect()
}

/// Reassemble PES access units on every PID accepted by `wanted`, invoking
/// `on_payload` with the PES-header-stripped elementary-stream bytes for each
/// completed unit (ISO/IEC 13818-1 §2.4.3.6), the 0-based index of the TS
/// packet that completed it, and the PID it came from.
///
/// Any PID left with a partially-assembled unit at end of stream is flushed
/// (attributed to the last packet index) — mirrors `PtsCheck`'s flush step.
pub(crate) fn for_each_access_unit(
    ts: &[u8],
    mut wanted: impl FnMut(u16) -> bool,
    mut on_payload: impl FnMut(&[u8], usize, u16),
) {
    let n_packets = ts.len() / TS_PACKET_SIZE;
    let mut assemblers: BTreeMap<u16, PesAssembler> = BTreeMap::new();

    for i in 0..n_packets {
        let offset = i * TS_PACKET_SIZE;
        let raw = &ts[offset..offset + TS_PACKET_SIZE];
        let Ok(pkt) = TsPacket::parse(raw) else {
            continue;
        };
        let pid = pkt.header.pid;
        if !wanted(pid) {
            continue;
        }
        let Some(payload) = pkt.payload else {
            continue;
        };
        if payload.is_empty() {
            continue;
        }
        let pusi = pkt.header.pusi;
        let assembler = assemblers.entry(pid).or_default();
        if let Some(pes_bytes) = assembler.feed(pusi, payload) {
            if let Ok(pes) = mpeg_pes::PesPacket::parse(&pes_bytes) {
                on_payload(pes.payload, i, pid);
            }
        }
    }

    let last = n_packets.saturating_sub(1);
    for (&pid, assembler) in assemblers.iter_mut() {
        if let Some(pes_bytes) = assembler.flush() {
            if let Ok(pes) = mpeg_pes::PesPacket::parse(&pes_bytes) {
                on_payload(pes.payload, last, pid);
            }
        }
    }
}

/// Test-only helpers shared by every codec-check test module: build a minimal
/// but real (typed, CRC-correct) PAT + PMT TS declaring a set of elementary
/// streams, via `dvb-si`'s own section builders + `mpeg-ts`'s
/// `SectionPacketiser` — never hand-rolled bytes.
#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec::Vec;

    use broadcast_common::Serialize;
    use dvb_si::descriptors::any::DescriptorLoop;
    use dvb_si::tables::pat::{PatEntry, PatSection};
    use dvb_si::tables::pmt::{PmtSection, PmtStream, StreamType};
    use mpeg_ts::mux::SectionPacketiser;
    use mpeg_ts::ts::TS_PACKET_SIZE;

    /// PMT PID used by every test fixture built here.
    pub(crate) const TEST_PMT_PID: u16 = 0x0100;

    fn serialize_section<S: Serialize>(section: &S) -> Vec<u8>
    where
        S::Error: core::fmt::Debug,
    {
        let mut buf = alloc::vec![0u8; section.serialized_len()];
        let n = section.serialize_into(&mut buf).expect("serialize section");
        buf.truncate(n);
        buf
    }

    /// Build a single-program TS with only a PAT + PMT declaring the given
    /// `(elementary_pid, stream_type)` pairs — no elementary stream data at
    /// all. Used to test the "PMT declares a stream that never decodes"
    /// signalling-mismatch path.
    pub(crate) fn build_pat_pmt_ts(streams: &[(u16, StreamType)]) -> Vec<u8> {
        let pat = PatSection {
            transport_stream_id: 1,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries: alloc::vec![PatEntry {
                program_number: 1,
                pid: TEST_PMT_PID,
            }],
        };
        let pmt_streams: Vec<PmtStream<'_>> = streams
            .iter()
            .map(|&(pid, stream_type)| PmtStream {
                stream_type,
                elementary_pid: pid,
                es_info: DescriptorLoop::new(&[]),
            })
            .collect();
        let pcr_pid = streams.first().map(|&(pid, _)| pid).unwrap_or(0x1FFF);
        let pmt = PmtSection::new(
            1,
            0,
            true,
            0,
            0,
            pcr_pid,
            DescriptorLoop::new(&[]),
            pmt_streams,
        );

        let pat_bytes = serialize_section(&pat);
        let pmt_bytes = serialize_section(&pmt);

        let mut ts = Vec::new();
        for pkt in SectionPacketiser::new(dvb_si::tables::pat::PID).packetise(&[&pat_bytes]) {
            ts.extend_from_slice(&pkt);
        }
        for pkt in SectionPacketiser::new(TEST_PMT_PID).packetise(&[&pmt_bytes]) {
            ts.extend_from_slice(&pkt);
        }
        assert_eq!(ts.len() % TS_PACKET_SIZE, 0);
        ts
    }

    /// Wrap `payload` in a minimal PES header (no PTS/DTS, `stream_id`
    /// caller-chosen) — ISO/IEC 13818-1 §2.4.3.6/§2.4.3.7 optional-header-less
    /// form (`PTS_DTS_flags = 00`).
    pub(crate) fn build_pes(stream_id: u8, payload: &[u8]) -> Vec<u8> {
        let pes_len = 3 + payload.len();
        let mut pes = alloc::vec![0x00, 0x00, 0x01, stream_id];
        pes.extend_from_slice(&(pes_len as u16).to_be_bytes());
        pes.push(0x80); // flags1: marker + no special flags
        pes.push(0x00); // flags2: PTS_DTS_flags = 00
        pes.push(0x00); // PES_header_data_length = 0
        pes.extend_from_slice(payload);
        pes
    }

    /// Wrap PES bytes in a single 188-byte TS packet (payload-only, no
    /// adaptation field) — the same pattern `pts_check`'s tests use: any bytes
    /// of `pes_bytes` past the 184-byte payload capacity are silently dropped
    /// (fine for these tests since every crafted PES here fits in one
    /// packet), and unused trailing packet bytes are left `0x47`-filled
    /// (harmless — the assembler stops at the PES's own declared length).
    pub(crate) fn make_pes_packet(pid: u16, cc: u8, pes_bytes: &[u8]) -> Vec<u8> {
        let mut pkt = alloc::vec![0x47u8; TS_PACKET_SIZE];
        pkt[1] = 0x40 | (((pid >> 8) as u8) & 0x1F); // PUSI=1
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = 0x10 | (cc & 0x0F); // AFC=01 (payload only)
        let len = pes_bytes.len().min(TS_PACKET_SIZE - 4);
        pkt[4..4 + len].copy_from_slice(&pes_bytes[..len]);
        pkt
    }
}
