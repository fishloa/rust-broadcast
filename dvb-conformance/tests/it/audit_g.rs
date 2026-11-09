use std::time::Duration;

use broadcast_common::Serialize;
use dvb_conformance::{ConformanceEvent, ConformanceMonitor, Indicator};
use dvb_si::descriptors::any::DescriptorLoop;
use dvb_si::tables::eit::{EitKind, EitSection, TABLE_ID_PF_ACTUAL};
use dvb_si::tables::nit::{NitKind, NitSection};
use dvb_si::tables::pat::{PatEntry, PatSection};
use mpeg_ts::mux::SectionPacketiser;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::TS_PACKET_SIZE;

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

struct Rig {
    m: ConformanceMonitor,
    events: Vec<ConformanceEvent>,
    null_cc: u8,
    t: u64,
}

impl Rig {
    fn new() -> Self {
        let mut r = Self {
            m: ConformanceMonitor::new(),
            events: Vec::new(),
            null_cc: 0,
            t: 0,
        };
        for _ in 0..6 {
            r.null();
        }
        r
    }

    fn feed(&mut self, pkt: &[u8], at_ms: u64) {
        self.t = self.t.max(at_ms);
        let ev = self.m.feed(pkt, ms(at_ms));
        self.events.extend(ev.iter().cloned());
    }

    fn null(&mut self) {
        let mut p = [0xFFu8; TS_PACKET_SIZE];
        p[0] = 0x47;
        p[1] = 0x1F;
        p[2] = 0xFF;
        p[3] = 0x10 | (self.null_cc & 0x0F);
        self.null_cc = self.null_cc.wrapping_add(1);
        let t = self.t;
        self.feed(&p, t);
    }

    fn idle_until(&mut self, end_ms: u64) {
        while self.t + 50 <= end_ms {
            let t = self.t + 50;
            let mut p = [0xFFu8; TS_PACKET_SIZE];
            p[0] = 0x47;
            p[1] = 0x1F;
            p[2] = 0xFF;
            p[3] = 0x10 | (self.null_cc & 0x0F);
            self.null_cc = self.null_cc.wrapping_add(1);
            self.feed(&p, t);
        }
    }

    fn count(&self, i: Indicator) -> usize {
        self.events.iter().filter(|e| e.indicator == i).count()
    }
}

fn pat_bytes(programs: &[(u16, u16)]) -> Vec<u8> {
    let pat = PatSection::new(
        1,
        0,
        true,
        0,
        0,
        programs
            .iter()
            .map(|&(n, p)| PatEntry::new(n, Pid::new(p)))
            .collect(),
    );
    let mut b = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut b).unwrap();
    b
}

fn nit_bytes(kind: NitKind, section_number: u8) -> Vec<u8> {
    let nit = NitSection::new(
        kind,
        1,
        0,
        true,
        section_number,
        0,
        DescriptorLoop::new(&[]),
        vec![],
    );
    let mut b = vec![0u8; nit.serialized_len()];
    nit.serialize_into(&mut b).unwrap();
    b
}

fn eit_pf_bytes(section_number: u8, service_id: u16) -> Vec<u8> {
    let eit = EitSection::new(
        EitKind::PresentFollowingActual,
        TABLE_ID_PF_ACTUAL,
        service_id,
        0,
        true,
        section_number,
        0,
        1,
        1,
        0,
        TABLE_ID_PF_ACTUAL,
        vec![],
    );
    let mut b = vec![0u8; eit.serialized_len()];
    eit.serialize_into(&mut b).unwrap();
    b
}

fn corrupt_crc(s: &mut [u8]) {
    let n = s.len();
    s[n - 1] ^= 0xFF;
}

fn send(rig: &mut Rig, pz: &mut SectionPacketiser, section: &[u8], at_ms: u64) {
    for p in pz.packetise(&[section]) {
        rig.feed(&p, at_ms);
    }
}

#[test]
fn crc_corrupt_pat_must_not_start_tracking_a_bogus_pmt_pid() {
    let mut rig = Rig::new();
    let mut pz = SectionPacketiser::new(Pid::new(0));
    send(&mut rig, &mut pz, &pat_bytes(&[(1, 0x100)]), 10);
    let mut bad = pat_bytes(&[(1, 0x200)]);
    corrupt_crc(&mut bad);
    send(&mut rig, &mut pz, &bad, 100);
    rig.idle_until(1_000);
    let bogus: Vec<_> = rig
        .events
        .iter()
        .filter(|e| e.indicator == Indicator::PmtError2 && e.pid == Some(Pid::new(0x200)))
        .collect();
    assert!(
        bogus.is_empty(),
        "PMT_error_2 raised for PID 0x200, which no CRC-valid PAT ever referenced: {bogus:?}"
    );
}

#[test]
fn crc_corrupt_nit_actual_must_not_satisfy_si_repetition() {
    let mut rig = Rig::new();
    let mut pz = SectionPacketiser::new(Pid::new(0x10));
    send(&mut rig, &mut pz, &nit_bytes(NitKind::Actual, 0), 10);
    let mut bad = nit_bytes(NitKind::Actual, 0);
    corrupt_crc(&mut bad);
    for k in 1..=13u64 {
        rig.idle_until(k * 1_000 - 100);
        send(&mut rig, &mut pz, &bad, k * 1_000);
    }
    assert!(rig.count(Indicator::CrcError) >= 13);
    assert_eq!(
        rig.count(Indicator::SiRepetitionError),
        1,
        "no CRC-valid NIT_actual for 13 s (limit 10 s): 3.2 must fire"
    );
}

#[test]
fn crc_corrupt_eit_pf_must_not_create_a_phantom_sub_table() {
    let mut rig = Rig::new();
    let mut pz = SectionPacketiser::new(Pid::new(0x12));
    send(&mut rig, &mut pz, &eit_pf_bytes(0, 1), 10);
    send(&mut rig, &mut pz, &eit_pf_bytes(1, 1), 20);
    let mut bad = eit_pf_bytes(0, 99);
    corrupt_crc(&mut bad);
    send(&mut rig, &mut pz, &bad, 100);
    rig.idle_until(4_000);
    assert_eq!(rig.count(Indicator::CrcError), 1);
    assert_eq!(
        rig.count(Indicator::EitPfError),
        0,
        "EIT_PF_error for service 99 that only exists in a CRC-failed section: {:?}",
        rig.events
            .iter()
            .filter(|e| e.indicator == Indicator::EitPfError)
            .collect::<Vec<_>>()
    );
}

#[test]
fn nit_other_repetition_is_reported_again_after_the_table_recovers() {
    let mut rig = Rig::new();
    let mut pz = SectionPacketiser::new(Pid::new(0x10));
    let s = nit_bytes(NitKind::Other, 0);
    for t in [0u64, 1_000, 20_000, 21_000, 22_000, 45_000] {
        send(&mut rig, &mut pz, &s, t);
    }
    assert_eq!(
        rig.count(Indicator::NitOtherError),
        2,
        "gaps of 19 s and 23 s against a 10 s limit are two separate failures"
    );
}

fn pcr_packet(pid: u16, cc: u8, pcr_ticks: u64) -> [u8; TS_PACKET_SIZE] {
    let base = pcr_ticks / 300;
    let ext = (pcr_ticks % 300) as u16;
    let mut p = [0xFFu8; TS_PACKET_SIZE];
    p[0] = 0x47;
    p[1] = ((pid >> 8) as u8) & 0x1F;
    p[2] = pid as u8;
    p[3] = 0x20 | (cc & 0x0F);
    p[4] = 183;
    p[5] = 0x10;
    p[6] = (base >> 25) as u8;
    p[7] = (base >> 17) as u8;
    p[8] = (base >> 9) as u8;
    p[9] = (base >> 1) as u8;
    p[10] = (((base & 1) << 7) as u8) | 0x7E | ((ext >> 8) & 1) as u8;
    p[11] = (ext & 0xFF) as u8;
    p
}

#[test]
fn pcr_step_of_100_5_ms_is_outside_the_0_to_100_ms_range() {
    let mut rig = Rig::new();
    let t0 = 27_000_000u64 * 10;
    rig.feed(&pcr_packet(0x100, 0, t0), 20);
    rig.feed(&pcr_packet(0x100, 0, t0 + 2_713_500), 40);
    assert_eq!(
        rig.count(Indicator::PcrDiscontinuityError),
        1,
        "PCR advanced 100.5 ms with discontinuity_indicator=0"
    );
}

#[test]
fn legal_duplicate_continuation_packet_must_not_corrupt_the_section() {
    let programs: Vec<(u16, u16)> = (0..100u16).map(|i| (i + 1, 0x100 + i)).collect();
    let section = pat_bytes(&programs);
    let mut pz = SectionPacketiser::new(Pid::new(0));
    let pkts = pz.packetise(&[&section[..]]);
    assert_eq!(pkts.len(), 3);
    let mut rig = Rig::new();
    rig.feed(&pkts[0], 10);
    rig.feed(&pkts[1], 11);
    rig.feed(&pkts[1], 12);
    rig.feed(&pkts[2], 13);
    assert_eq!(
        rig.count(Indicator::ContinuityCountError),
        0,
        "a single byte-identical repeat is a legal duplicate (H.222.0 2.4.3.3)"
    );
    assert_eq!(
        rig.count(Indicator::CrcError),
        0,
        "the PAT carried by the three packets is intact; the duplicate must not be appended into the section"
    );
}
