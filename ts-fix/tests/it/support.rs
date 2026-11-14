//! Test support helpers for `ts-fix` tests.

use broadcast_common::traits::Serialize;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{PmtSection, PmtStream, StreamType};
use mpeg_ts::mux::SectionPacketiser;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::TsPacket;

/// Deterministically corrupt the continuity counter in a TS packet.
///
/// The continuity counter is the 4-bit field in byte 3, bits [3:0].
/// This function zeros all continuity counters in the input stream.
pub fn zero_continuity_counters(data: &mut [u8]) {
    const CC_MASK: u8 = 0x0F;
    for chunk in data.chunks_mut(188) {
        if chunk.len() >= 4 {
            // Byte 3, bits [3:0] is the CC.
            chunk[3] &= !CC_MASK;
        }
    }
}

/// Corrupt the continuity counter in a TS packet using XOR.
///
/// Applies a different XOR value based on packet index (to create varied corruption).
pub fn xor_continuity_counters(data: &mut [u8], pattern: u8) {
    const CC_MASK: u8 = 0x0F;
    for (idx, chunk) in data.chunks_mut(188).enumerate() {
        if chunk.len() >= 4 {
            let xor_val = ((idx as u8) ^ pattern) & CC_MASK;
            chunk[3] = (chunk[3] & !CC_MASK) | ((chunk[3] ^ xor_val) & CC_MASK);
        }
    }
}

/// Add null packets (PID 0x1FFF) at regular intervals to a stream.
///
/// This creates a synthetic stream with null packets for testing drop/pad operations.
pub fn add_null_packets(data: &[u8], every_n_packets: usize) -> Vec<u8> {
    let mut output = Vec::new();
    for (idx, chunk) in data.chunks(188).enumerate() {
        output.extend_from_slice(chunk);
        if (idx + 1) % every_n_packets == 0 {
            // Insert a null packet after every `every_n_packets` real packets.
            output.extend_from_slice(&make_null_packet());
        }
    }
    output
}

/// Construct a standard null TS packet (PID 0x1FFF, all padding).
fn make_null_packet() -> [u8; 188] {
    let mut pkt = [0xFF_u8; 188];
    pkt[0] = 0x47; // Sync byte
    pkt[1] = 0x1F; // PID upper 5 bits
    pkt[2] = 0xFF; // PID lower 8 bits
    pkt[3] = 0x10; // Adaptation field control = 01, CC = 0
    pkt
}

/// Extract the PID from bytes 1-2 of a TS packet header.
pub fn extract_pid(pkt: &[u8]) -> Pid {
    if pkt.len() < 3 {
        return Pid::from_const(0);
    }
    let b1 = pkt[1];
    let b2 = pkt[2];
    Pid::from_wire_masked((((b1 & 0x1F) as u16) << 8) | (b2 as u16))
}

/// The adaptation field's PCR on the 27 MHz clock and its `discontinuity_indicator`, if the
/// packet carries a PCR.
pub fn pcr_and_disc(p: &[u8]) -> Option<(u64, bool)> {
    let af = TsPacket::parse(p).ok()?.adaptation_field()?.ok()?;
    Some((
        af.pcr?.to_clock27().ok()?.ticks(),
        af.discontinuity_indicator,
    ))
}

/// The packet's PCR on the 27 MHz clock, if it carries one.
pub fn pcr_27mhz(p: &[u8]) -> Option<u64> {
    pcr_and_disc(p).map(|(pcr, _)| pcr)
}

// ── PID constants ────────────────────────────────────────────────────────────

pub const PAT_PID: Pid = Pid::from_const(0x0000);
pub const PMT1_PID: Pid = Pid::from_const(0x0100);
pub const PMT2_PID: Pid = Pid::from_const(0x0200);

pub const P1_PCR_PID: Pid = Pid::from_const(0x0101);
pub const P1_VIDEO_PID: Pid = Pid::from_const(0x0101);
pub const P1_AUDIO_PID: Pid = Pid::from_const(0x0102);

pub const P2_PCR_PID: Pid = Pid::from_const(0x0201);
pub const P2_VIDEO_PID: Pid = Pid::from_const(0x0201);
pub const P2_AUDIO_PID: Pid = Pid::from_const(0x0202);

pub const NULL_PID: Pid = Pid::from_const(0x1FFF);

// ── Synthetic 2-program mux builder ─────────────────────────────────────────

/// Build a 188-byte TS packet carrying a dummy payload on `pid`.
///
/// Sets PUSI=false, adaptation_field_control=01 (payload only), CC=`cc`.
pub fn dummy_es_packet(pid: Pid, cc: u8) -> [u8; 188] {
    let mut pkt = [0u8; 188];
    pkt[0] = 0x47; // sync byte
    pkt[1] = ((pid.value() >> 8) as u8) & 0x1F; // PUSI=0, TEI=0, transport_priority=0
    pkt[2] = (pid.value() & 0xFF) as u8;
    pkt[3] = 0x10 | (cc & 0x0F); // adaptation_field_control=01 (payload only), CC
    // payload: fill with a recognisable pattern
    for (i, b) in pkt[4..].iter_mut().enumerate() {
        *b = (i as u8).wrapping_add(pid.value() as u8);
    }
    pkt
}

/// Build a 188-byte null packet (PID 0x1FFF).
pub fn null_packet() -> [u8; 188] {
    let mut pkt = [0u8; 188];
    pkt[0] = 0x47;
    pkt[1] = 0x1F;
    pkt[2] = 0xFF;
    pkt[3] = 0x10; // payload only
    for b in &mut pkt[4..] {
        *b = 0xFF;
    }
    pkt
}

/// Serialize a `PatSection` to bytes.
pub fn serialize_pat(pat: &PatSection) -> Vec<u8> {
    let mut buf = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut buf).expect("PAT serialize");
    buf
}

/// Serialize a `PmtSection` to bytes.
pub fn serialize_pmt(pmt: &PmtSection<'_>) -> Vec<u8> {
    let mut buf = vec![0u8; pmt.serialized_len()];
    pmt.serialize_into(&mut buf).expect("PMT serialize");
    buf
}

/// Build the synthetic 2-program TS as a flat `Vec<u8>`.
///
/// The stream contains, in interleaved order per cycle:
///   PAT | PMT1 | PMT2 | P1-video | P1-audio | P2-video | P2-audio | null
///
/// We repeat the cycle `cycles` times so that every PID appears multiple
/// times and packets from different programs are interleaved.
fn build_two_program_core(cycles: usize, psi_each_cycle: bool) -> Vec<u8> {
    // ── PAT ─────────────────────────────────────────────────────────────────
    let pat = PatSection::new(
        1,
        0,
        true,
        0,
        0,
        vec![PatEntry::new(1, PMT1_PID), PatEntry::new(2, PMT2_PID)],
    );
    let pat_section_bytes = serialize_pat(&pat);
    let mut pat_pktz = SectionPacketiser::new(PAT_PID);
    let pat_pkts = pat_pktz.packetise(&[&pat_section_bytes]);

    // ── PMT1 ─────────────────────────────────────────────────────────────────
    let pmt1 = PmtSection::new(
        /* program_number */ 1,
        dvb_si::tables::SectionVersion::new(
            /* version_number */ 0, /* current_next_indicator */ true,
            /* section_number */ 0, /* last_section_number */ 0,
        ),
        /* pcr_pid */ P1_PCR_PID,
        /* program_info */ DescriptorLoop::new(&[]),
        /* streams */
        vec![
            PmtStream::new(
                StreamType::Mpeg2Video,
                P1_VIDEO_PID,
                DescriptorLoop::new(&[]),
            ),
            PmtStream::new(
                StreamType::Mpeg2Audio,
                P1_AUDIO_PID,
                DescriptorLoop::new(&[]),
            ),
        ],
    );
    let pmt1_section_bytes = serialize_pmt(&pmt1);
    let mut pmt1_pktz = SectionPacketiser::new(PMT1_PID);
    let pmt1_pkts = pmt1_pktz.packetise(&[&pmt1_section_bytes]);

    // ── PMT2 ─────────────────────────────────────────────────────────────────
    let pmt2 = PmtSection::new(
        /* program_number */ 2,
        dvb_si::tables::SectionVersion::new(
            /* version_number */ 0, /* current_next_indicator */ true,
            /* section_number */ 0, /* last_section_number */ 0,
        ),
        /* pcr_pid */ P2_PCR_PID,
        /* program_info */ DescriptorLoop::new(&[]),
        /* streams */
        vec![
            PmtStream::new(
                StreamType::Mpeg2Video,
                P2_VIDEO_PID,
                DescriptorLoop::new(&[]),
            ),
            PmtStream::new(
                StreamType::Mpeg2Audio,
                P2_AUDIO_PID,
                DescriptorLoop::new(&[]),
            ),
        ],
    );
    let pmt2_section_bytes = serialize_pmt(&pmt2);
    let mut pmt2_pktz = SectionPacketiser::new(PMT2_PID);
    let pmt2_pkts = pmt2_pktz.packetise(&[&pmt2_section_bytes]);

    // ── Assemble interleaved stream ──────────────────────────────────────────
    let mut stream: Vec<u8> = Vec::new();

    // PSI packets appear first (once — they're short enough to fit in one packet).
    for pkt in &pat_pkts {
        stream.extend_from_slice(pkt);
    }
    for pkt in &pmt1_pkts {
        stream.extend_from_slice(pkt);
    }
    for pkt in &pmt2_pkts {
        stream.extend_from_slice(pkt);
    }

    // Then interleave ES packets and nulls over `cycles` iterations.
    for i in 0..cycles {
        let cc = (i as u8) & 0x0F;
        if psi_each_cycle {
            for pkt in pat_pkts.iter().chain(&pmt1_pkts).chain(&pmt2_pkts) {
                stream.extend_from_slice(pkt);
            }
        }
        // Interleave both programs' ES so a prefix cut fails.
        stream.extend_from_slice(&dummy_es_packet(P1_VIDEO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P2_VIDEO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P1_AUDIO_PID, cc));
        stream.extend_from_slice(&dummy_es_packet(P2_AUDIO_PID, cc));
        stream.extend_from_slice(&null_packet());
    }

    stream
}

/// The PSI is emitted once at the top, then `cycles` ES/null cycles.
pub fn build_two_program_ts(cycles: usize) -> Vec<u8> {
    build_two_program_core(cycles, false)
}

/// As [`build_two_program_ts`], but the PAT and PMTs are re-emitted every
/// cycle (as a real mux does), so PAT slots also appear after the PMTs.
pub fn build_two_program_ts_with_cycled_psi(cycles: usize) -> Vec<u8> {
    build_two_program_core(cycles, true)
}
