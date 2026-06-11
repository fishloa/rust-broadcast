//! Hostility / fuzzing smoke tests for dvb-si.
//!
//! These tests feed garbage, truncations, and random descriptor loops through
//! the dvb-si parsing surface and assert **no panics** (plus basic counter
//! sanity where applicable).  They use a simple deterministic LCG so there are
//! no external fuzzing dependencies and runs are fully reproducible.
//!
//! ## (a) 10 000 random 188-byte TS packets → SiDemux
//! ## (b) Every truncation of a valid SDT section
//! ## (c) 10 000 garbage feeds to T2miPump — lives in dvb-t2mi/tests/chain.rs
//! ## (d) 1 000 random descriptor loops through parse_loop

#![cfg(feature = "ts")]

use dvb_common::Serialize;
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::parse_loop;
use dvb_si::tables::sdt::{SdtKind, SdtSection};
use dvb_si::tables::RunningStatus;

// ── Deterministic LCG (no deps) ───────────────────────────────────────────────

/// Simple 64-bit LCG. Numerical Recipes constants; fully deterministic.
/// Deliberately duplicated (chain.rs <-> hostility.rs): no shared test-helper crate; keep constants in sync.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u8(&mut self) -> u8 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u8
    }
}

// ── Helper: build a valid serialized SDT ─────────────────────────────────────

fn build_sdt_bytes() -> Vec<u8> {
    use dvb_si::tables::sdt::SdtService;

    let sdt = SdtSection {
        kind: SdtKind::Actual,
        transport_stream_id: 0x1234,
        version_number: 3,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        original_network_id: 0x5678,
        services: vec![SdtService {
            service_id: 0x0001,
            eit_schedule_flag: false,
            eit_present_following_flag: true,
            running_status: RunningStatus::Running,
            free_ca_mode: false,
            descriptors: dvb_si::descriptors::DescriptorLoop::new(&[]),
        }],
    };
    let mut buf = vec![0u8; sdt.serialized_len()];
    sdt.serialize_into(&mut buf).expect("SDT serialize");
    buf
}

// ── (a) 10 000 × 188-byte seeded garbage → SiDemux → no panic ────────────────

/// Feeds 10 000 packets of seeded-LCG garbage through `SiDemux::feed`.
/// Asserts no panic.  Asserts that at least `malformed_packets` +
/// `sections_completed` > 0 (i.e. some counter moved, proving the code ran).
#[test]
fn hostility_garbage_ts_packets_no_panic() {
    let mut demux = SiDemux::builder().build();
    let mut lcg = Lcg::new(0xDEAD_CAFE_1234_5678u64);
    const N: usize = 10_000;

    for _ in 0..N {
        let mut pkt = [0u8; 188];
        for b in &mut pkt {
            *b = lcg.next_u8();
        }
        // Drain the iterator — the test drives computation.
        let _ = demux.feed(&pkt).count();
    }

    let s = demux.stats();
    assert_eq!(s.packets, N as u64, "packets counter must equal feed count");
    // LCG garbage rarely has 0x47 sync — nearly all 10k must count as malformed.
    assert!(
        s.malformed_packets > 9_000,
        "LCG garbage rarely has 0x47 sync — nearly all 10k must count as malformed (got {})",
        s.malformed_packets
    );
}

// ── (b) Every truncation of a valid SDT section → no panic ───────────────────

/// For every prefix length 0..=full.len(), both `AnyTableSection::parse` and
/// `parse_loop` over the prefix must return (not panic).
#[test]
fn hostility_sdt_all_truncations_no_panic() {
    use dvb_si::tables::AnyTableSection;

    let full = build_sdt_bytes();

    for len in 0..=full.len() {
        let prefix = &full[..len];

        // AnyTableSection::parse must return Ok or Err — never panic.
        let _ = AnyTableSection::parse(prefix);

        // parse_loop over the same bytes: drain fully — never panic.
        let _ = parse_loop(prefix).count();
    }
}

// ── (d) 1 000 random-length random-byte slices through parse_loop ─────────────

/// Feeds 1 000 random-byte slices of varying length through `parse_loop` and
/// drains each iterator fully. No panic is the acceptance criterion.
#[test]
fn hostility_random_descriptor_loops_no_panic() {
    let mut lcg = Lcg::new(0xABCD_EF01_2345_6789u64);

    for _ in 0..1_000u32 {
        // Lengths 0–255, occasionally up to 511.
        let len_hi = lcg.next_u8();
        let len_lo = lcg.next_u8();
        let len = (len_hi as usize & 0x01) * 256 + len_lo as usize;
        let data: Vec<u8> = (0..len).map(|_| lcg.next_u8()).collect();

        // Drain fully — must not panic on any garbage byte sequence.
        let _ = parse_loop(&data).count();
    }
}

// ── Bonus: parse_loop on valid + truncated short_event loop ──────────────────

/// A well-formed short_event descriptor in a loop parses correctly; then every
/// truncation of the same bytes returns without panicking.
#[test]
fn hostility_descriptor_loop_truncations_no_panic() {
    // A minimal valid short_event descriptor:
    // tag=0x4D, len=0x0C, lang=b"fre", name_len=7, name=b"Journal", text_len=0
    let valid_loop: &[u8] = &[
        0x4D, 0x0C, b'f', b'r', b'e', 0x07, b'J', b'o', b'u', b'r', b'n', b'a', b'l', 0x00,
    ];

    // First, the full loop must parse without error.
    let items: Vec<_> = parse_loop(valid_loop).collect();
    assert_eq!(items.len(), 1, "one descriptor in the loop");
    assert!(items[0].is_ok(), "full descriptor must parse OK");

    // Now every truncation.
    for len in 0..valid_loop.len() {
        let _ = parse_loop(&valid_loop[..len]).count();
    }
}
