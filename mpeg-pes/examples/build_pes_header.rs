//! Build a PES packet from typed fields, serialize, re-parse, and verify
//! PTS/DTS/ESCR/trick-mode round-trip — demonstrating the typed write/edit side
//! of `mpeg-pes` (mpeg-pes 0.1.3+).
//!
//! Because `PesHeader` is `#[non_exhaustive]` (for future optional-field
//! additions), this example assembles the wire bytes using public typed encoders
//! (`Pts::to_field_bytes`, `Dts::to_field_bytes`, `Escr::to_field_bytes`,
//! `TrickMode::to_byte`), then round-trips through `PesPacket::serialize_into`.
//!
//! Run with:
//!   cargo run -p mpeg-pes --example build_pes_header

use mpeg_pes::{Escr, PesPacket, TrickMode, PACKET_START_CODE_PREFIX};

/// Encode a 33-bit value into a 5-byte PTS field with the given 4-bit prefix.
/// Duplicated here because `mpeg_pes::timestamp::write` is `pub(crate)`.
fn encode_ts(ts: u64, prefix: u8) -> [u8; 5] {
    let ts = ts & 0x1_FFFF_FFFF;
    [
        (prefix << 4) | ((((ts >> 30) & 0x07) as u8) << 1) | 0x01,
        ((ts >> 22) & 0xFF) as u8,
        ((((ts >> 15) & 0x7F) as u8) << 1) | 0x01,
        ((ts >> 7) & 0xFF) as u8,
        (((ts & 0x7F) as u8) << 1) | 0x01,
    ]
}

/// Build a complete PES packet (raw bytes) from typed optional-header fields.
fn build_pes(
    stream_id: u8,
    pts: Option<(u64, bool /* has_dts */)>,
    dts: Option<u64>,
    escr: Option<Escr>,
    trick_mode: Option<TrickMode>,
    payload: &[u8],
) -> Vec<u8> {
    let pts_dts_flags = match (pts, dts) {
        (Some(_), Some(_)) => 0b11u8,
        (Some(_), None) => 0b10,
        _ => 0b00,
    };

    // Compute optional-field lengths
    let mut opt_len = 0usize;
    if pts.is_some() {
        opt_len += 5;
    }
    if dts.is_some() {
        opt_len += 5;
    }
    if escr.is_some() {
        opt_len += 6;
    }
    if trick_mode.is_some() {
        opt_len += 1;
    }

    let has_opt = opt_len > 0;
    let hdr_extra = if has_opt { 3 } else { 0 }; // f1 + f2 + hdl

    let f1: u8 = 0x80; // marker "10", all flags clear
    let f2: u8 = (pts_dts_flags << 6)
        | (u8::from(escr.is_some()) << 5)
        | (u8::from(trick_mode.is_some()) << 3);

    let pes_len = (6 + hdr_extra + opt_len + payload.len()) as u16;
    let mut out = vec![0u8; 6 + hdr_extra + opt_len + payload.len()];
    let mut cursor = 0usize;

    // Fixed header
    out[cursor..cursor + 3].copy_from_slice(&PACKET_START_CODE_PREFIX);
    cursor += 3;
    out[cursor] = stream_id;
    cursor += 1;
    out[cursor..cursor + 2].copy_from_slice(&pes_len.to_be_bytes());
    cursor += 2;

    if !has_opt {
        out[cursor..].copy_from_slice(payload);
        return out;
    }

    out[cursor] = f1;
    cursor += 1;
    out[cursor] = f2;
    cursor += 1;
    out[cursor] = opt_len as u8;
    cursor += 1;

    // PTS (and DTS if present)
    if let Some((pts_val, has_dts)) = pts {
        let prefix = if has_dts { 0b0011 } else { 0b0010 };
        out[cursor..cursor + 5].copy_from_slice(&encode_ts(pts_val, prefix));
        cursor += 5;
    }
    if let Some(dts_val) = dts {
        out[cursor..cursor + 5].copy_from_slice(&encode_ts(dts_val, 0b0001));
        cursor += 5;
    }
    // ESCR
    if let Some(escr) = escr {
        out[cursor..cursor + 6].copy_from_slice(&escr.to_field_bytes());
        cursor += 6;
    }
    // DSM trick mode
    if let Some(tm) = trick_mode {
        out[cursor] = tm.to_byte();
        cursor += 1;
    }

    out[cursor..].copy_from_slice(payload);
    out
}

fn main() {
    // ── 1) Build with PTS + DTS ─────────────────────────────────────────────
    let pts_val = 90_000u64; // 1 second @ 90 kHz
    let dts_val = 45_000u64; // 0.5 seconds
    let bytes = build_pes(
        0xE0,
        Some((pts_val, true)),
        Some(dts_val),
        None,
        None,
        &[0xAA, 0xBB],
    );

    let pkt = PesPacket::parse(&bytes).expect("PES with PTS+DTS must parse");
    let hdr = pkt.header.as_ref().expect("video PES has optional header");
    println!("--- PTS + DTS ---");
    println!("stream_id       : 0x{:02X} (video)", pkt.stream_id.0);
    let pts = hdr.pts.unwrap();
    let dts = hdr.dts.unwrap();
    println!(
        "PTS             : {} ticks ({:.6}s)",
        pts.ticks(),
        pts.seconds()
    );
    println!(
        "DTS             : {} ticks ({:.6}s)",
        dts.ticks(),
        dts.seconds()
    );
    assert_eq!(pts.ticks(), 90_000);
    assert_eq!(dts.ticks(), 45_000);

    // ── 2) Round-trip via serialize_into ─────────────────────────────────────
    let mut buf = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut buf).expect("serialize");
    let pkt2 = PesPacket::parse(&buf).expect("re-parse");
    let hdr2 = pkt2.header.unwrap();
    assert_eq!(hdr2.pts.unwrap().ticks(), 90_000);
    assert_eq!(hdr2.dts.unwrap().ticks(), 45_000);
    println!("  round-trip    : PTS+DTS preserved after serialize ✓");

    // ── 3) Build with ESCR ──────────────────────────────────────────────────
    let escr = Escr::from_27mhz(27_000_000); // 1 second @ 27 MHz
    let bytes2 = build_pes(
        0xE0,
        Some((pts_val, false)),
        None,
        Some(escr),
        None,
        &[0xCC],
    );
    let pkt3 = PesPacket::parse(&bytes2).expect("PES with ESCR must parse");
    let hdr3 = pkt3.header.unwrap();
    println!("\n--- ESCR ---");
    println!(
        "ESCR            : base={} ext={}  ({} ticks @ 27 MHz)",
        hdr3.escr.unwrap().base,
        hdr3.escr.unwrap().extension,
        hdr3.escr.unwrap().as_27mhz()
    );
    assert_eq!(hdr3.escr.unwrap().as_27mhz(), 27_000_000);

    // ── 4) Build with trick mode + PTS+DTS ──────────────────────────────────
    let trick = TrickMode::FastForward {
        field_id: 1,
        intra_slice_refresh: true,
        frequency_truncation: 1,
    };
    let bytes3 = build_pes(
        0xE0,
        Some((pts_val, true)),
        Some(dts_val),
        None,
        Some(trick),
        &[0xDD],
    );
    let pkt4 = PesPacket::parse(&bytes3).expect("PES with trick mode must parse");
    let hdr4 = pkt4.header.as_ref().unwrap();
    let tm = hdr4.dsm_trick_mode.unwrap();
    println!("\n--- DSM Trick Mode ---");
    println!("trick_mode      : {tm:?}");
    assert!(matches!(tm, TrickMode::FastForward { field_id: 1, .. }));

    // ── 5) Confirm serialize → re-parse preserves all fields ────────────────
    let mut buf3 = vec![0u8; pkt4.serialized_len()];
    pkt4.serialize_into(&mut buf3).expect("serialize");
    let pkt5 = PesPacket::parse(&buf3).expect("re-parse");
    let hdr5 = pkt5.header.as_ref().unwrap();
    assert_eq!(hdr5.pts.unwrap().ticks(), 90_000);
    assert_eq!(hdr5.dts.unwrap().ticks(), 45_000);
    assert!(hdr5.dsm_trick_mode.is_some());
    println!("  serialize → re-parse: all fields preserved ✓");

    println!("\nAll build/write operations completed successfully.");
}
