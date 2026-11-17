//! The `SiDemux` version/CRC gate: change detection, eviction fairness and
//! the counters/accessors downstream consumers depend on.
//!
//! Sections are built here from the ISO/IEC 13818-1 syntax (independent CRC)
//! rather than through the production serializer.

use core::num::NonZeroUsize;

use dvb_si::demux::{SiDemux, SiDemuxConfig};
use mpeg_ts::pid::Pid;
use test_fixtures::psi::{crc32_mpeg2, ts_packet};

const CRC_LEN: usize = 4;

/// A long-form section with an explicit `current_next_indicator`,
/// `section_number` and `last_section_number`.
fn long_section_full(
    table_id: u8,
    ext: u16,
    version: u8,
    current: bool,
    section_number: u8,
    last_section_number: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut after = Vec::new();
    after.extend_from_slice(&ext.to_be_bytes());
    after.push(0xC0 | ((version & 0x1F) << 1) | u8::from(current));
    after.push(section_number);
    after.push(last_section_number);
    after.extend_from_slice(payload);
    let section_length = after.len() + CRC_LEN;
    let mut section = vec![
        table_id,
        0xB0 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
    ];
    section.extend_from_slice(&after);
    section.extend_from_slice(&[0; CRC_LEN]);
    let end = section.len() - CRC_LEN;
    let crc = crc32_mpeg2(&section[..end]).to_be_bytes();
    section[end..].copy_from_slice(&crc);
    section
}

/// A short-form (`section_syntax_indicator == 0`) CRC-less section.
fn short_form(table_id: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len();
    let mut section = vec![table_id, ((len >> 8) as u8) & 0x0F, (len & 0xFF) as u8];
    section.extend_from_slice(body);
    section
}

/// A PAT section over `(program_number, program_map_PID)` entries.
fn pat_section_full(
    tsid: u16,
    version: u8,
    current: bool,
    section_number: u8,
    last_section_number: u8,
    entries: &[(u16, u16)],
) -> Vec<u8> {
    let mut payload = Vec::new();
    for &(program, pid) in entries {
        payload.extend_from_slice(&program.to_be_bytes());
        payload.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
        payload.push((pid & 0xFF) as u8);
    }
    long_section_full(
        0x00,
        tsid,
        version,
        current,
        section_number,
        last_section_number,
        &payload,
    )
}

fn pmt_payload(pcr_pid: u16) -> Vec<u8> {
    vec![
        0xE0 | ((pcr_pid >> 8) as u8 & 0x1F),
        (pcr_pid & 0xFF) as u8,
        0xF0,
        0x00,
    ]
}

/// A demux watching only `pid` (no DVB SI PIDs), so the watch set is exactly
/// what the test builds.
fn demux_for(pid: u16) -> SiDemux {
    SiDemux::new(
        SiDemuxConfig::default()
            .with_dvb_si_pids(false)
            .with_pid(Pid::from_const(pid)),
    )
}

/// The public accessors carry the wire header: version 21, extension 0xABCD,
/// section_number 7 on a long-form section.
#[test]
fn long_form_event_accessors_carry_the_wire_header() {
    let mut demux = demux_for(0x0042);
    let section = long_section_full(0x42, 0xABCD, 21, true, 7, 7, &[0xAA]);
    let events: Vec<_> = demux.feed(&ts_packet(0x0042, &section)).collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].table_id(), 0x42);
    assert_eq!(events[0].version(), Some(21));
    assert_eq!(events[0].table_id_extension(), Some(0xABCD));
    assert_eq!(events[0].section_number(), Some(7));
    assert!(events[0].crc_ok());
}

/// A short-form section carries no version, extension or section number.
#[test]
fn short_form_event_accessors_are_none() {
    let mut demux = demux_for(0x0070);
    let section = short_form(0x70, &[0x11, 0x22, 0x33, 0x44, 0x55]);
    let events: Vec<_> = demux.feed(&ts_packet(0x0070, &section)).collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].version(), None);
    assert_eq!(events[0].table_id_extension(), None);
    assert_eq!(events[0].section_number(), None);
}

/// A six-byte long-form section (the shortest that carries a passing CRC):
/// the extension is readable but there is no `section_number` byte at all.
#[test]
fn six_byte_long_form_section_has_no_section_number() {
    let mut demux = demux_for(0x0060);
    let section = [0x9Bu8, 0x80, 0x03, 0xC5, 0xDB, 0x99];
    let events: Vec<_> = demux.feed(&ts_packet(0x0060, &section)).collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].table_id_extension(), Some(0xC5DB));
    assert_eq!(events[0].section_number(), None);
}

/// A short-form TOT is the one short-form table that carries a CRC: a good
/// one emits, a corrupted one is dropped and counted.
fn tot_section(body: &[u8]) -> Vec<u8> {
    let len = body.len() + CRC_LEN;
    let mut section = vec![0x73, ((len >> 8) as u8) & 0x0F, (len & 0xFF) as u8];
    section.extend_from_slice(body);
    section.extend_from_slice(&[0; CRC_LEN]);
    let end = section.len() - CRC_LEN;
    let crc = crc32_mpeg2(&section[..end]).to_be_bytes();
    section[end..].copy_from_slice(&crc);
    section
}

#[test]
fn a_tot_is_crc_checked_despite_being_short_form() {
    let mut demux = demux_for(0x0014);
    let good = tot_section(&[0xE0, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(demux.feed(&ts_packet(0x0014, &good)).count(), 1);

    let mut bad = tot_section(&[0xE0, 0x10, 0x00, 0x00, 0x00]);
    bad[3] ^= 0xFF;
    assert_eq!(demux.feed(&ts_packet(0x0014, &bad)).count(), 0);
    assert_eq!(demux.stats().crc_failures, 1);
    assert_eq!(demux.stats().emitted, 1);
}

/// The minimum-length guard accepts a header-only short-form section and
/// counts a header-only long-form one as a CRC failure (it cannot carry a
/// CRC). `[0x70, 0x00, 0x00]` is exactly 3 bytes, the smallest possible.
#[test]
fn a_three_byte_section_is_measured_against_the_minimum_exactly() {
    let mut demux = demux_for(0x0070);
    let header_only_short = [0x70u8, 0x00, 0x00];
    assert_eq!(demux.feed(&ts_packet(0x0070, &header_only_short)).count(), 1);
    assert_eq!(demux.stats().crc_failures, 0);

    let header_only_long = [0x42u8, 0x80, 0x00];
    assert_eq!(demux.feed(&ts_packet(0x0070, &header_only_long)).count(), 0);
    assert_eq!(demux.stats().crc_failures, 1);
    assert_eq!(demux.stats().emitted, 1);
}

/// A long-form section uses its `table_id_extension` in the gate key, so two
/// twelve-byte sections differing only there are separate changes and each
/// repeat stays suppressed.
#[test]
fn a_long_form_section_keys_the_gate_by_its_extension() {
    let mut demux = demux_for(0x0042);
    let first = long_section_full(0x42, 0x0001, 0, true, 0, 0, &[]);
    let second = long_section_full(0x42, 0x0002, 0, true, 0, 0, &[]);
    assert_eq!(first.len(), 12);
    assert_eq!(demux.feed(&ts_packet(0x0042, &first)).count(), 1);
    assert_eq!(demux.feed(&ts_packet(0x0042, &second)).count(), 1);
    assert_eq!(demux.feed(&ts_packet(0x0042, &first)).count(), 0);
    assert_eq!(demux.feed(&ts_packet(0x0042, &second)).count(), 0);
}

/// Two short-form sections that agree everywhere a long-form header would
/// look (extension, `section_number`, the `current_next` bit and the trailing
/// four bytes) but differ in the middle are still distinct changes: a
/// short-form section is gated by a hash of its whole bytes.
#[test]
fn a_short_form_section_is_gated_by_its_whole_bytes() {
    let mut demux = demux_for(0x0070);
    let make = |differing: u8| {
        let body = [
            0x11u8, 0x22, differing, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD,
            0xEE, 0x0F, 0x1F, 0x2F,
        ];
        short_form(0x70, &body)
    };
    let first = make(0x30);
    let second = make(0x34);
    assert_eq!(demux.feed(&ts_packet(0x0070, &first)).count(), 1);
    assert_eq!(
        demux.feed(&ts_packet(0x0070, &second)).count(),
        1,
        "a different short-form body is a different change"
    );
}

/// The gate key includes the PID: the identical section on two PIDs is two
/// distinct changes and each repeat is suppressed on its own PID.
#[test]
fn gate_keys_are_distinct_across_pids() {
    const FIRST: u16 = 0x0100;
    const SECOND: u16 = 0x0200;
    let mut demux = SiDemux::new(
        SiDemuxConfig::default()
            .with_dvb_si_pids(false)
            .with_pid(Pid::from_const(FIRST))
            .with_pid(Pid::from_const(SECOND)),
    );
    let section = long_section_full(0x4E, 0x0001, 0, true, 0, 0, &[0xAA]);
    assert_eq!(demux.feed(&ts_packet(FIRST, &section)).count(), 1);
    assert_eq!(demux.feed(&ts_packet(SECOND, &section)).count(), 1);
    assert_eq!(demux.feed(&ts_packet(FIRST, &section)).count(), 0);
    assert_eq!(demux.feed(&ts_packet(SECOND, &section)).count(), 0);
}

/// A next-version PAT is its own gate entry (`current_next_indicator` is part
/// of the key) and its PMT PIDs are not followed: only a current PAT moves
/// the followed set.
#[test]
fn a_next_pat_keeps_its_own_entry_and_does_not_move_the_pmt_pids() {
    let mut demux = SiDemux::new(SiDemuxConfig::default());
    let current = pat_section_full(0x0001, 3, true, 0, 0, &[(1, 0x0100)]);
    let next = pat_section_full(0x0001, 4, false, 0, 0, &[(1, 0x0300)]);

    let current_events: Vec<_> = demux.feed(&ts_packet(0, &current)).collect();
    assert_eq!(current_events.len(), 1);
    assert_eq!(current_events[0].version(), Some(3));

    let next_events: Vec<_> = demux.feed(&ts_packet(0, &next)).collect();
    assert_eq!(next_events.len(), 1, "a next PAT is a distinct gate key");
    assert_eq!(next_events[0].version(), Some(4));

    let pmt = long_section_full(0x02, 1, 0, true, 0, 0, &pmt_payload(0x0100));
    assert_eq!(
        demux.feed(&ts_packet(0x0300, &pmt)).count(),
        0,
        "the next PAT's PMT PID is not followed"
    );
    assert_eq!(
        demux.feed(&ts_packet(0x0100, &pmt)).count(),
        1,
        "the current PAT's PMT PID is still followed"
    );

    assert_eq!(demux.feed(&ts_packet(0, &current)).count(), 0);
    let stats = demux.stats();
    assert_eq!(stats.suppressed, 1);
    assert_eq!(stats.emitted, 3);
    assert_eq!(stats.crc_failures, 0);
}

/// A gate at capacity with two equal shares: the incoming PID is at its
/// share, so the victim is the *other* PID's oldest key.
#[test]
fn an_equal_share_insert_evicts_the_other_pids_oldest_key() {
    const CAPACITY: usize = 4;
    const FIRST: u16 = 0x0100;
    const SECOND: u16 = 0x0200;
    let mut demux = SiDemux::new(
        SiDemuxConfig::default()
            .with_dvb_si_pids(false)
            .with_pid(Pid::from_const(FIRST))
            .with_pid(Pid::from_const(SECOND))
            .with_gate_capacity(NonZeroUsize::new(CAPACITY).expect("a non-zero capacity")),
    );
    let first = [
        long_section_full(0x4E, 0x0001, 0, true, 0, 0, &[0x01]),
        long_section_full(0x4E, 0x0002, 0, true, 0, 0, &[0x02]),
    ];
    let second = [
        long_section_full(0x4E, 0x0011, 0, true, 0, 0, &[0x11]),
        long_section_full(0x4E, 0x0012, 0, true, 0, 0, &[0x12]),
    ];
    for section in &first {
        assert_eq!(demux.feed(&ts_packet(FIRST, section)).count(), 1);
    }
    for section in &second {
        assert_eq!(demux.feed(&ts_packet(SECOND, section)).count(), 1);
    }

    let extra = long_section_full(0x4E, 0x0013, 0, true, 0, 0, &[0x13]);
    assert_eq!(demux.feed(&ts_packet(SECOND, &extra)).count(), 1);
    assert_eq!(demux.stats().gate_evictions, 1);

    assert_eq!(
        demux.feed(&ts_packet(SECOND, &second[0])).count(),
        0,
        "the other PID paid: its keys are still gated"
    );
    assert_eq!(
        demux.feed(&ts_packet(FIRST, &first[0])).count(),
        1,
        "the first PID's oldest key was the victim"
    );
}

/// A PID over its equal share pays for its own insert even when another PID
/// holds the same number of keys: the tie goes to the incoming PID.
#[test]
fn an_over_share_pid_evicts_its_own_oldest_key_on_a_tie() {
    const CAPACITY: usize = 9;
    const A: u16 = 0x0100;
    const B: u16 = 0x0200;
    const C: u16 = 0x0300;
    let mut demux = SiDemux::new(
        SiDemuxConfig::default()
            .with_dvb_si_pids(false)
            .with_pid(Pid::from_const(A))
            .with_pid(Pid::from_const(B))
            .with_pid(Pid::from_const(C))
            .with_gate_capacity(NonZeroUsize::new(CAPACITY).expect("a non-zero capacity")),
    );
    let a_keys: Vec<_> = (1..=4u16)
        .map(|n| long_section_full(0x4E, 0x0100 + n, 0, true, 0, 0, &[n as u8]))
        .collect();
    let b_keys: Vec<_> = (1..=4u16)
        .map(|n| long_section_full(0x4E, 0x0200 + n, 0, true, 0, 0, &[n as u8]))
        .collect();
    let c_key = long_section_full(0x4E, 0x0301, 0, true, 0, 0, &[0x33]);
    for section in &a_keys {
        assert_eq!(demux.feed(&ts_packet(A, section)).count(), 1);
    }
    for section in &b_keys {
        assert_eq!(demux.feed(&ts_packet(B, section)).count(), 1);
    }
    assert_eq!(demux.feed(&ts_packet(C, &c_key)).count(), 1);

    let fifth = long_section_full(0x4E, 0x0105, 0, true, 0, 0, &[0x05]);
    assert_eq!(demux.feed(&ts_packet(A, &fifth)).count(), 1);

    assert_eq!(
        demux.feed(&ts_packet(B, &b_keys[0])).count(),
        0,
        "the tied PID B did not pay"
    );
    assert_eq!(
        demux.feed(&ts_packet(A, &a_keys[0])).count(),
        1,
        "the over-share PID A paid"
    );
}

/// A PID flooding distinct keys is charged for its own growth: the legitimate
/// PID's three keys stay gated and the flood's oldest keys are the ones that
/// disappear.
#[test]
fn a_flooding_pid_cannot_evict_a_legitimate_pids_keys() {
    const CAPACITY: usize = 8;
    const LEGIT: u16 = 0x0100;
    const FLOOD: u16 = 0x0200;
    let mut demux = SiDemux::new(
        SiDemuxConfig::default()
            .with_dvb_si_pids(false)
            .with_pid(Pid::from_const(LEGIT))
            .with_pid(Pid::from_const(FLOOD))
            .with_gate_capacity(NonZeroUsize::new(CAPACITY).expect("a non-zero capacity")),
    );
    let legit: Vec<_> = (1..=3u16)
        .map(|n| long_section_full(0x4E, 0x0001 + n, 0, true, 0, 0, &[n as u8]))
        .collect();
    for section in &legit {
        assert_eq!(demux.feed(&ts_packet(LEGIT, section)).count(), 1);
    }
    let flood: Vec<_> = (0..20u16)
        .map(|n| long_section_full(0x4E, 0x0100 + n, 0, true, 0, 0, &[n as u8]))
        .collect();
    for section in &flood {
        demux.feed(&ts_packet(FLOOD, section)).for_each(drop);
    }
    assert!(demux.stats().gate_evictions > 0);

    for section in &legit {
        assert_eq!(
            demux.feed(&ts_packet(LEGIT, section)).count(),
            0,
            "the legitimate PID's unchanged keys stay gated"
        );
    }
    assert_eq!(
        demux.feed(&ts_packet(FLOOD, &flood[0])).count(),
        1,
        "the flood's oldest key was evicted"
    );
}

/// `with_follow_pat(false)` means a PAT is emitted but no PMT PID is added to
/// the watch set.
#[test]
fn follow_pat_can_be_disabled() {
    let mut demux = SiDemux::new(SiDemuxConfig::default().with_follow_pat(false));
    let pat = pat_section_full(0x0001, 0, true, 0, 0, &[(1, 0x0100)]);
    assert_eq!(demux.feed(&ts_packet(0, &pat)).count(), 1);
    let pmt = long_section_full(0x02, 1, 0, true, 0, 0, &pmt_payload(0x0100));
    assert_eq!(
        demux.feed(&ts_packet(0x0100, &pmt)).count(),
        0,
        "the PMT PID must not be followed"
    );
}

/// The twelve-byte minimum PAT (no programmes) still empties the followed PMT
/// set: a shorter-than-minimum guard must not swallow it.
#[test]
fn a_minimum_length_empty_pat_drops_the_followed_pmt_pid() {
    let mut demux = SiDemux::new(SiDemuxConfig::default());
    let with_programme = pat_section_full(0x0001, 0, true, 0, 0, &[(1, 0x0100)]);
    assert_eq!(demux.feed(&ts_packet(0, &with_programme)).count(), 1);

    let empty = pat_section_full(0x0001, 1, true, 0, 0, &[]);
    assert_eq!(empty.len(), 12);
    assert_eq!(demux.feed(&ts_packet(0, &empty)).count(), 1);

    let pmt = long_section_full(0x02, 1, 0, true, 0, 0, &pmt_payload(0x0100));
    assert_eq!(
        demux.feed(&ts_packet(0x0100, &pmt)).count(),
        0,
        "the empty current PAT dropped the PMT PID"
    );
}

/// A two-section PAT version change is the drop that matters: the old
/// generation's sections must not be unioned into the new one.
#[test]
fn a_two_section_pat_version_change_drops_the_old_pmt_pid() {
    let mut demux = SiDemux::new(SiDemuxConfig::default());
    let v0_s0 = pat_section_full(0x0001, 0, true, 0, 1, &[(1, 0x0100)]);
    let v0_s1 = pat_section_full(0x0001, 0, true, 1, 1, &[(2, 0x0101)]);
    let v1_s0 = pat_section_full(0x0001, 1, true, 0, 1, &[(1, 0x0100)]);
    let v1_s1 = pat_section_full(0x0001, 1, true, 1, 1, &[(3, 0x0102)]);
    for section in [&v0_s0, &v0_s1, &v1_s0, &v1_s1] {
        assert_eq!(demux.feed(&ts_packet(0, section)).count(), 1);
    }

    let stale = long_section_full(0x02, 1, 0, true, 0, 0, &pmt_payload(0x0101));
    assert_eq!(
        demux.feed(&ts_packet(0x0101, &stale)).count(),
        0,
        "0x0101 is not in the current PAT generation"
    );
    let fresh = long_section_full(0x02, 1, 0, true, 0, 0, &pmt_payload(0x0102));
    assert_eq!(demux.feed(&ts_packet(0x0102, &fresh)).count(), 1);
}

/// A version change to a PAT with *fewer* sections must clear the previous
/// generation's accumulated sections: otherwise the stale sections are
/// unioned into the new (shorter) sub-table and an old PMT PID stays watched.
#[test]
fn a_shorter_pat_version_change_drops_the_old_pmt_pid() {
    let mut demux = SiDemux::new(SiDemuxConfig::default());
    let v0_s0 = pat_section_full(0x0001, 0, true, 0, 1, &[(1, 0x0100)]);
    let v0_s1 = pat_section_full(0x0001, 0, true, 1, 1, &[(2, 0x0101)]);
    for section in [&v0_s0, &v0_s1] {
        assert_eq!(demux.feed(&ts_packet(0, section)).count(), 1);
    }

    // v1 is a single-section PAT: section 0 is also the last section.
    let v1 = pat_section_full(0x0001, 1, true, 0, 0, &[(3, 0x0102)]);
    assert_eq!(demux.feed(&ts_packet(0, &v1)).count(), 1);

    let stale = long_section_full(0x02, 1, 0, true, 0, 0, &pmt_payload(0x0101));
    assert_eq!(
        demux.feed(&ts_packet(0x0101, &stale)).count(),
        0,
        "0x0101 is not in the shorter current PAT"
    );
    let fresh = long_section_full(0x02, 1, 0, true, 0, 0, &pmt_payload(0x0102));
    assert_eq!(demux.feed(&ts_packet(0x0102, &fresh)).count(), 1);
}
