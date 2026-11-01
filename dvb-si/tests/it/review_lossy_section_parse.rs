//! Adversarial review (dvb-si tables/core batch): sections that `parse`
//! ACCEPTS but cannot re-serialize byte-identically, against the crate's own
//! policy (lib.rs "RFU policy": "Serialize preserves what was parsed").
//!
//! A strict single-bit-flip sweep (every flip that still parses must
//! re-serialize to the flipped bytes; the committed `table_reserved_bits.rs`
//! only counts a flip as "lost" when exactly that one bit comes back and
//! ignores flips that change the length) found these:
//!
//! 1. NIT/BAT: a `transport_stream_loop_length` SHORTER than the bytes left
//!    before the CRC is accepted; the trailing bytes are silently dropped and
//!    the section re-serializes shorter (AIT rejects the same shape with
//!    `app_loop_actual_end != app_loop_end`).
//! 2. SAT: the typed loop bodies (`satellite_table_id` 0, 1, 3, 4) stop when
//!    fewer bits than one entry remain and drop them; the section
//!    re-serializes shorter.
//! 3. `section_syntax_indicator` is neither validated nor preserved: a PAT
//!    with SSI = 0 parses and re-serializes with SSI = 1 (and a TDT/RST/DIT
//!    with SSI = 1 re-serializes with 0).
#![cfg(feature = "ts")]

use broadcast_common::{Parse, Serialize};
use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTableSection;
use dvb_si::tables::bat::BatSection;
use dvb_si::tables::nit::NitSection;
use dvb_si::tables::pat::PatSection;
use dvb_si::tables::sat::SatSection;
use mpeg_ts::ts::TS_PACKET_SIZE;

fn crc(b: &[u8]) -> [u8; 4] {
    broadcast_common::crc32_mpeg2::compute(b).to_be_bytes()
}

fn reseal(s: &mut [u8]) {
    let e = s.len() - 4;
    let c = crc(&s[..e]);
    s[e..].copy_from_slice(&c);
}

fn ser<T: Serialize>(t: &T) -> Vec<u8>
where
    T::Error: core::fmt::Debug,
{
    let mut b = vec![0u8; t.serialized_len()];
    let n = t.serialize_into(&mut b).expect("serialize");
    b.truncate(n);
    b
}

/// First CRC-valid section with `table_id` from a committed real capture.
fn real_section(file: &str, tid: u8) -> Vec<u8> {
    let path = format!("{}/../fixtures/{file}", env!("CARGO_MANIFEST_DIR"));
    let data = std::fs::read(&path).expect("committed fixture");
    let mut demux = SiDemux::builder().build();
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE {
            continue;
        }
        for ev in demux.feed(chunk) {
            let b: &[u8] = ev.bytes().as_ref();
            if b[0] == tid && ev.crc_ok() {
                return b.to_vec();
            }
        }
    }
    panic!("no table_id 0x{tid:02X} in {file}");
}

/// Real NIT_actual from `tnt-5w-12732v-isi6-10s.ts` with its
/// `transport_stream_loop_length` reduced by 64 (section_length unchanged,
/// CRC recomputed): the last 64 loop bytes are now outside the declared loop.
fn nit_with_short_ts_loop() -> Vec<u8> {
    let mut s = real_section("dvb-si/tnt-5w-12732v-isi6-10s.ts", 0x40);
    let ndl = (usize::from(s[8] & 0x0F) << 8) | usize::from(s[9]);
    let at = 10 + ndl;
    let tsl = (usize::from(s[at] & 0x0F) << 8) | usize::from(s[at + 1]);
    assert!(tsl > 64, "fixture loop long enough");
    let short = tsl - 64;
    s[at] = (s[at] & 0xF0) | (short >> 8) as u8;
    s[at + 1] = short as u8;
    reseal(&mut s);
    s
}

#[test]
fn nit_short_transport_stream_loop_is_rejected_or_round_trips() {
    let s = nit_with_short_ts_loop();
    if let Ok(nit) = NitSection::parse(&s) {
        assert_eq!(
            ser(&nit).len(),
            s.len(),
            "NIT parsed with trailing bytes outside transport_stream_loop_length; \
             they were silently dropped on re-serialize"
        );
    }
}

#[test]
fn bat_short_transport_stream_loop_is_rejected_or_round_trips() {
    // BAT has the NIT layout (bouquet_id in place of network_id).
    let mut s = nit_with_short_ts_loop();
    s[0] = 0x4A;
    reseal(&mut s);
    if let Ok(bat) = BatSection::parse(&s) {
        assert_eq!(
            ser(&bat).len(),
            s.len(),
            "BAT parsed with trailing bytes outside transport_stream_loop_length; \
             they were silently dropped on re-serialize"
        );
    }
}

/// SAT, `satellite_table_id = 1` (Cell Fragment, Table 11d) with a 3-byte
/// body `00 01 02`: shorter than one `cell_fragment_id(32)+flags(2)` entry.
/// Parses as an empty Cell Fragment body and re-serializes without the 3
/// bytes.
#[test]
fn sat_trailing_body_bytes_are_rejected_or_round_trip() {
    let mut s = vec![
        0x4D, 0xF0, 0x0C, // table_id, flags | section_length = 12
        0x04, 0x01, // satellite_table_id = 1, table_count = 1
        0xC3, 0x00, 0x00, // reserved | version | cni, section_number, last
        0x00, 0x01, 0x02, // body (3 bytes)
        0, 0, 0, 0,
    ];
    reseal(&mut s);
    if let Ok(sat) = SatSection::parse(&s) {
        assert_eq!(
            ser(&sat),
            s,
            "SAT body bytes after the last whole entry were silently dropped"
        );
    }
}

/// Real PAT (`m6-single.ts`) with `section_syntax_indicator` cleared.
#[test]
fn pat_section_syntax_indicator_is_rejected_or_preserved() {
    let mut s = real_section("ts/m6-single.ts", 0x00);
    s[1] &= 0x7F;
    reseal(&mut s);
    if let Ok(pat) = PatSection::parse(&s) {
        assert_eq!(
            ser(&pat),
            s,
            "PAT with section_syntax_indicator = 0 parsed, re-serialized with SSI = 1"
        );
    }
    // Through the dispatcher too.
    if let Ok(AnyTableSection::PatSection(pat)) = AnyTableSection::parse(&s) {
        assert_eq!(ser(&pat), s);
    }
}
