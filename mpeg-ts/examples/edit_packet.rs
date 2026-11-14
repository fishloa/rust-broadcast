//! Demonstrate the write/edit side of `mpeg-ts` (mpeg-ts 0.1.2+):
//! set PCR, mutate CC, build null packets, and round-trip packet headers.
//!
//! The committed `m6-single.ts` fixture contains no PCR-bearing packets, so
//! this example constructs one programmatically and mutates it.
//!
//! Run with:
//!   cargo run -p mpeg-ts --example edit_packet

use mpeg_ts::OwnedTsPacket;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::{
    ADAPTATION_FLAG, AF_PCR_FLAG, ContinuityCounter, PAYLOAD_FLAG, Pcr, TS_PACKET_SIZE,
    TS_SYNC_BYTE, TsHeader, TsPacket,
};

/// Build a 188-byte TS packet with a minimal adaptation field carrying a PCR.
fn build_pcr_packet(pid: Pid, cc: ContinuityCounter, pcr: Pcr) -> [u8; TS_PACKET_SIZE] {
    let mut raw = [0xFFu8; TS_PACKET_SIZE];
    let pid = pid.value();
    raw[0] = TS_SYNC_BYTE;
    raw[1] = ((pid >> 8) as u8) & 0x1F;
    raw[2] = (pid & 0xFF) as u8;
    raw[3] = ADAPTATION_FLAG | PAYLOAD_FLAG | cc.value();
    raw[4] = 7; // adaptation_field_length (1 flags + 6 PCR)
    raw[5] = AF_PCR_FLAG;
    raw[6..12].copy_from_slice(&pcr.to_field_bytes());
    raw
}

fn main() {
    // ── 1) Build a PCR-bearing packet ───────────────────────────────────────
    let orig_pcr = Pcr::from_parts(10_000, 0).expect("in range");
    let mut pkt_raw = build_pcr_packet(
        Pid::from_const(0x0100),
        ContinuityCounter::try_new(5).expect("4-bit counter"),
        orig_pcr,
    );

    // Verify via parse
    let pkt = TsPacket::parse(&pkt_raw).unwrap();
    let af = pkt.adaptation_field().unwrap().unwrap();
    assert_eq!(af.pcr, Some(orig_pcr));
    assert_eq!(pkt.header.pid, Pid::from_const(0x0100));
    assert_eq!(pkt.header.continuity_counter.value(), 5);
    println!("--- PCR-bearing packet ---");
    println!(
        "original PCR  : base={} ext={}  ({} ticks @ 27 MHz)",
        orig_pcr.base(),
        orig_pcr.extension(),
        orig_pcr.as_27mhz()
    );

    // ── 2) Set PCR to a shifted value ───────────────────────────────────────
    let shifted_pcr = Pcr::from_27mhz(orig_pcr.as_27mhz() + 27_000_000).expect("in range");
    OwnedTsPacket::set_pcr(&mut pkt_raw, shifted_pcr).expect("set_pcr");

    let re_pkt = TsPacket::parse(&pkt_raw).unwrap();
    let re_af = re_pkt.adaptation_field().unwrap().unwrap();
    let re_pcr = re_af.pcr.unwrap();
    assert_eq!(re_pcr, shifted_pcr);
    println!(
        "shifted PCR   : base={} ext={}  ({} ticks @ 27 MHz)",
        re_pcr.base(),
        re_pcr.extension(),
        re_pcr.as_27mhz()
    );
    assert_eq!(re_pcr.as_27mhz() - orig_pcr.as_27mhz(), 27_000_000);

    // ── 3) set_continuity_counter on a packet ──────────────────────────────
    let mut cc_raw: [u8; TS_PACKET_SIZE] = OwnedTsPacket::serialize_with_payload(
        Pid::from_const(0x0200),
        true,
        ContinuityCounter::ZERO,
        &[],
    )
    .unwrap();
    let orig_cc = TsPacket::parse(&cc_raw).unwrap().header.continuity_counter;
    OwnedTsPacket::set_continuity_counter(
        &mut cc_raw,
        ContinuityCounter::try_new(15).expect("4-bit counter"),
    );
    let new_cc = TsPacket::parse(&cc_raw).unwrap().header.continuity_counter;
    assert_eq!(new_cc.value(), 15);
    println!("\n--- continuity_counter mutation ---");
    println!("original CC: {orig_cc}  →  after set_continuity_counter: {new_cc}");

    // ── 4) Build a null_packet and verify PID ───────────────────────────────
    let null_raw =
        OwnedTsPacket::null_packet(ContinuityCounter::try_new(7).expect("4-bit counter"));
    let null_pkt = TsPacket::parse(&null_raw).unwrap();
    assert_eq!(null_pkt.header.pid, Pid::from_const(0x1FFF));
    assert_eq!(null_pkt.header.continuity_counter.value(), 7);
    println!("\n--- null packet ---");
    println!(
        "null packet PID = 0x{:04X}  (expect 0x1FFF), CC = {}",
        null_pkt.header.pid.value(),
        null_pkt.header.continuity_counter.value()
    );

    // ── 5) Round-trip a TsHeader ────────────────────────────────────────────
    let orig_hdr = TsHeader::parse(&[0x47, 0x40, 0x00, 0x10]).unwrap();
    let mut hdr_buf = [0u8; 4];
    orig_hdr.serialize_into(&mut hdr_buf).unwrap();
    let re_hdr = TsHeader::parse(&hdr_buf).unwrap();
    assert_eq!(re_hdr.pid, orig_hdr.pid);
    assert_eq!(re_hdr.pusi, orig_hdr.pusi);
    assert_eq!(re_hdr.continuity_counter, orig_hdr.continuity_counter);
    println!("\n--- round-trip (TsHeader) ---");
    println!(
        "TsHeader parse → serialize_into → parse: pid=0x{:04X}, pusi={}, cc={}",
        re_hdr.pid.value(),
        re_hdr.pusi,
        re_hdr.continuity_counter.value()
    );

    // ── 6) Round-trip an OwnedTsPacket ──────────────────────────────────────
    let owned = OwnedTsPacket::parse(
        OwnedTsPacket::serialize_with_payload(
            Pid::from_const(0x0100),
            true,
            ContinuityCounter::try_new(3).expect("4-bit counter"),
            &[0xAA, 0xBB],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(owned.pid(), Pid::from_const(0x0100));
    assert_eq!(owned.continuity_counter().value(), 3);
    assert!(owned.payload().is_some());
    assert_eq!(owned.payload().unwrap().len(), 184); // full payload area (serialize_with_payload fills the rest with 0xFF)
    assert_eq!(owned.payload().unwrap()[..2], [0xAA, 0xBB]);
    println!(
        "OwnedTsPacket serialize_with_payload → parse → fields match: pid=0x{:04X}, pusi={}, cc={}",
        owned.pid().value(),
        owned.pusi(),
        owned.continuity_counter().value()
    );

    println!("\nAll write/edit operations completed successfully.");
}
