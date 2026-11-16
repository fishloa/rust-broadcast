//! The per-PID PCR gauges must be exported only for PIDs the stream's own
//! PMT designates as its `PCR_PID`, not for every PID that happens to carry a
//! PCR field (audit a20-37). Every distinct `pid` label becomes a series the
//! host's exporter keeps and renders until it expires, so a source that puts
//! a PCR on many PIDs would otherwise fill the registry for the process
//! lifetime.

use core::time::Duration;
use std::sync::OnceLock;

use broadcast_common::Serialize;
use broadcast_common::clock33::PCR_HZ;
use compliance_probe::Probe;
use dvb_si::tables::SectionVersion;
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::pmt::{PmtSection, PmtStream, StreamType};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use mpeg_ts::mux::SectionPacketiser;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::{Pcr, TS_PACKET_SIZE};

const PAT_PID: u16 = 0x0000;
const PMT_PID: u16 = 0x0100;
const DECLARED_PCR_PID: u16 = 0x0200;
const UNDECLARED_PCR_PID: u16 = 0x0210;

fn install() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| {
            let recorder = PrometheusBuilder::new().build_recorder();
            let handle = recorder.handle();
            metrics::set_global_recorder(recorder)
                .expect("the process-wide recorder is installed exactly once");
            handle
        })
        .clone()
}

fn null_packet(cc: u8) -> [u8; TS_PACKET_SIZE] {
    let mut packet = [0xFFu8; TS_PACKET_SIZE];
    packet[0] = 0x47;
    packet[1] = 0x1F;
    packet[2] = 0xFF;
    packet[3] = 0x10 | (cc & 0x0F);
    packet
}

fn pcr_packet(pid: u16, ticks: u64, cc: u8) -> [u8; TS_PACKET_SIZE] {
    let mut packet = [0xFFu8; TS_PACKET_SIZE];
    packet[0] = 0x47;
    packet[1] = (pid >> 8) as u8;
    packet[2] = pid as u8;
    packet[3] = 0x20 | (cc & 0x0F);
    packet[4] = 183;
    packet[5] = 0x10;
    packet[6..12].copy_from_slice(
        &Pcr::from_27mhz(ticks)
            .expect("a tick count on the 27 MHz clock")
            .to_field_bytes(),
    );
    packet
}

fn pat_section() -> Vec<u8> {
    let pat = PatSection::new(
        1,
        0,
        true,
        0,
        0,
        vec![PatEntry::new(1, Pid::from_const(PMT_PID))],
    );
    let mut bytes = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut bytes).expect("PAT serializes");
    bytes
}

fn pmt_section() -> Vec<u8> {
    let pmt = PmtSection::new(
        1,
        SectionVersion::new(0, true, 0, 0),
        Pid::from_const(DECLARED_PCR_PID),
        Default::default(),
        vec![PmtStream::new(
            StreamType::Mpeg2Video,
            Pid::from_const(0x0201),
            Default::default(),
        )],
    );
    let mut bytes = vec![0u8; pmt.serialized_len()];
    pmt.serialize_into(&mut bytes).expect("PMT serializes");
    bytes
}

#[test]
fn pcr_gauges_are_exported_only_for_pmt_declared_pcr_pids() {
    let handle = install();
    let mut probe = Probe::new();

    for cc in 0..6u8 {
        probe.feed_ts_packet(&null_packet(cc), Duration::from_millis(u64::from(cc)));
    }
    for packet in SectionPacketiser::new(Pid::from_const(PAT_PID)).packetise(&[&pat_section()]) {
        probe.feed_ts_packet(&packet, Duration::from_millis(10));
    }
    for packet in SectionPacketiser::new(Pid::from_const(PMT_PID)).packetise(&[&pmt_section()]) {
        probe.feed_ts_packet(&packet, Duration::from_millis(20));
    }

    let step = PCR_HZ / 10;
    for (index, pid) in [DECLARED_PCR_PID, UNDECLARED_PCR_PID]
        .into_iter()
        .enumerate()
    {
        for (sample, cc) in [0u8, 1].into_iter().enumerate() {
            let ticks = step * (sample as u64 + 1);
            let at = Duration::from_millis(100 + 100 * sample as u64 + index as u64);
            probe.feed_ts_packet(&pcr_packet(pid, ticks, cc), at);
        }
    }

    let exposition = handle.render();
    let declared = format!("pid=\"0x{DECLARED_PCR_PID:04X}\"");
    let undeclared = format!("pid=\"0x{UNDECLARED_PCR_PID:04X}\"");
    assert!(
        exposition.contains(&declared),
        "the PMT-declared PCR PID must still get its drift/jitter series"
    );
    assert!(
        !exposition.contains(&undeclared),
        "a PCR on a PID no PMT declares must not create a labelled series"
    );
    assert!(
        exposition.contains("compliance_probe_pcr_undeclared_pid_total"),
        "the suppressed sample must be counted"
    );
}
