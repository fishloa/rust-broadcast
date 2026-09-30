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
/// declared, one entry per PID.
///
/// A PMT repeats on the air (typically about every 100 ms — ISO/IEC
/// 13818-1 §2.4.4.2's PSI repetition-rate advice), so a stream's PID list is
/// walked far more than once end to end; without deduping, both this
/// function's own `Vec` and every caller's `.contains()` scan over it grow
/// linearly with repetitions rather than with the number of actually
/// declared PIDs — quadratic overall (issue #1069 / audit MD-C2). Dedup by
/// elementary PID, keeping the latest PMT generation seen for that PID (a
/// version bump legitimately changes a PID's declared `stream_type`, and the
/// most recent one is authoritative); returned in PID order.
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
    let mut declared: BTreeMap<u16, StreamType> = BTreeMap::new();

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
                    declared.insert(stream.elementary_pid, stream.stream_type);
                }
            }
        }
        for pmt_pid in new_pmt_pids {
            pmt_pids.push(pmt_pid);
            reassemblers.entry(pmt_pid).or_default();
        }
    }

    declared
        .into_iter()
        .map(|(pid, stream_type)| DeclaredStream { pid, stream_type })
        .collect()
}

/// Scan `payload` for a byte offset at which a well-formed ADTS header parses
/// (ISO/IEC 13818-7 §6.2, via [`transmux::parse_adts_header`]) — not assumed
/// to sit at offset 0, so this tolerates leading PES stuffing.
///
/// Shared by every check that asks whether an AAC stream is really
/// ADTS-framed (`CodecSignallingCheck`, `media-doctor watch`); it lived as two
/// identical copies before (audit MD-W10).
pub(crate) fn has_adts_sync(payload: &[u8]) -> bool {
    const ADTS_MIN: usize = 7;
    if payload.len() < ADTS_MIN {
        return false;
    }
    (0..=payload.len() - ADTS_MIN).any(|off| {
        payload[off] == 0xFF
            && (payload[off + 1] & 0xF0) == 0xF0
            && transmux::parse_adts_header(&payload[off..]).is_ok()
    })
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
        if let Some(pes_bytes) = assembler.feed(pusi, payload)
            && let Ok(pes) = mpeg_pes::PesPacket::parse(&pes_bytes)
        {
            on_payload(pes.payload, i, pid);
        }
    }

    let last = n_packets.saturating_sub(1);
    for (&pid, assembler) in assemblers.iter_mut() {
        if let Some(pes_bytes) = assembler.flush()
            && let Ok(pes) = mpeg_pes::PesPacket::parse(&pes_bytes)
        {
            on_payload(pes.payload, last, pid);
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
        build_pat_pmt_ts_versioned(streams, 0, true)
    }

    /// As [`build_pat_pmt_ts`], with an explicit `version_number` and
    /// `current_next_indicator`, so version-change tests can build a real
    /// second generation (or an ISO/IEC 13818-1 §2.4.4.11 next-generation
    /// section) through `dvb-si`'s own builders rather than hand-rolled bytes.
    pub(crate) fn build_pat_pmt_ts_versioned(
        streams: &[(u16, StreamType)],
        version_number: u8,
        current_next_indicator: bool,
    ) -> Vec<u8> {
        let pat = PatSection {
            transport_stream_id: 1,
            version_number,
            current_next_indicator,
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
            version_number,
            current_next_indicator,
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

    /// A PAT declaring several programs whose PMTs share one PSI PID (legal —
    /// ISO/IEC 13818-1 §2.4.4.8; the sub-tables are distinguished by
    /// `table_id_extension`), each PMT carrying one elementary stream
    /// `(program_number, elementary_pid, stream_type, version_number)`.
    pub(crate) fn build_shared_pid_pmt_ts(programs: &[(u16, u16, StreamType, u8)]) -> Vec<u8> {
        let pat = PatSection {
            transport_stream_id: 1,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries: programs
                .iter()
                .map(|&(program_number, _, _, _)| PatEntry {
                    program_number,
                    pid: TEST_PMT_PID,
                })
                .collect(),
        };
        let pat_bytes = serialize_section(&pat);

        let mut ts = Vec::new();
        for pkt in SectionPacketiser::new(dvb_si::tables::pat::PID).packetise(&[&pat_bytes]) {
            ts.extend_from_slice(&pkt);
        }
        for &(program_number, elementary_pid, stream_type, version_number) in programs {
            let pmt = PmtSection::new(
                program_number,
                version_number,
                true,
                0,
                0,
                elementary_pid,
                DescriptorLoop::new(&[]),
                alloc::vec![PmtStream {
                    stream_type,
                    elementary_pid,
                    es_info: DescriptorLoop::new(&[]),
                }],
            );
            let pmt_bytes = serialize_section(&pmt);
            for pkt in SectionPacketiser::new(TEST_PMT_PID).packetise(&[&pmt_bytes]) {
                ts.extend_from_slice(&pkt);
            }
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

    /// A single real PMT section, on `pid`, for program 1 declaring one H.264
    /// elementary stream — used to plant a PMT on a PID a PAT does *not*
    /// declare (a corrupted/aliased multiplex) without involving the PAT.
    pub(crate) fn build_pmt_ts_on_pid(pid: u16, version_number: u8) -> Vec<u8> {
        let pmt = PmtSection::new(
            1,
            version_number,
            true,
            0,
            0,
            ELEMENTARY_PID,
            DescriptorLoop::new(&[]),
            alloc::vec![PmtStream {
                stream_type: StreamType::H264,
                elementary_pid: ELEMENTARY_PID,
                es_info: DescriptorLoop::new(&[]),
            }],
        );
        let pmt_bytes = serialize_section(&pmt);
        let mut ts = Vec::new();
        for pkt in SectionPacketiser::new(pid).packetise(&[&pmt_bytes]) {
            ts.extend_from_slice(&pkt);
        }
        ts
    }

    /// Elementary-stream PID used by every test fixture built here.
    pub(crate) const ELEMENTARY_PID: u16 = 0x0101;
}

#[cfg(test)]
mod dedup_tests {
    use super::tests::build_pat_pmt_ts;
    use super::*;
    use dvb_si::tables::pmt::StreamType;

    /// A real PMT repeats on the air about every 100 ms (issue #1069 / audit
    /// MD-C2): concatenating the SAME PAT+PMT generation `N` times mirrors
    /// that repetition. `collect_pmt_streams` must record each declared PID
    /// **once**, not once per repetition — the un-deduped Vec is what made
    /// `pids_with_stream_type`'s `Vec` (and every `.contains()` scan over it)
    /// grow without bound over stream length.
    #[test]
    fn repeated_pmt_generations_are_deduped() {
        let one_generation = build_pat_pmt_ts(&[(0x0101, StreamType::H264)]);
        let mut ts = Vec::new();
        for _ in 0..50 {
            ts.extend_from_slice(&one_generation);
        }

        let declared = collect_pmt_streams(&ts);
        assert_eq!(
            declared,
            alloc::vec![DeclaredStream {
                pid: 0x0101,
                stream_type: StreamType::H264,
            }],
            "50 repetitions of the same PMT generation must dedup to exactly \
             one declared stream, got {} entries: {declared:?}",
            declared.len(),
        );
    }

    /// A PMT version change mid-stream (e.g. a PID's `stream_type` changed
    /// across a version bump) must leave the LATEST declaration standing,
    /// not both — otherwise a caller like `Scte35Check`/`CodecSignallingCheck`
    /// would treat a PID as two stream types at once.
    #[test]
    fn later_pmt_generation_overrides_earlier_declaration_for_same_pid() {
        let first = build_pat_pmt_ts(&[(0x0101, StreamType::H264)]);
        let second = build_pat_pmt_ts(&[(0x0101, StreamType::Hevc)]);
        let mut ts = Vec::new();
        ts.extend_from_slice(&first);
        ts.extend_from_slice(&second);

        let declared = collect_pmt_streams(&ts);
        assert_eq!(
            declared,
            alloc::vec![DeclaredStream {
                pid: 0x0101,
                stream_type: StreamType::Hevc,
            }],
            "the later PMT generation's stream_type must win for a repeated \
             PID, got {declared:?}",
        );
    }
}
