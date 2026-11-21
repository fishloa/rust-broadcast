//! 8.3: the real DVB-T capture recorded and read back byte-exactly.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::MediaPlaylist;
use bytes::Bytes;
use dvb_si::demux::{SiDemux, SiDemuxConfig};
use dvb_si::tables::AnyTableSection;
use dvr_runtime::fs::{FsArchive, FsRecorder};
use dvr_runtime::read::parse_period_playlist;
use dvr_runtime::{ArchiveConfig, ArchiveContainer, ArchiveName, PeriodNum, RetentionLimits};
use media_plane::ingress::{HandshakePolicy, IngestConfig, IngestDriver, ProgramId};
use media_plane::trunk::{SegmentEntry, Trunk, TrunkConfig};
use media_plane::ts_session::TsIngestSession;
use transmux::SegmentMeta;

use crate::support::{cleanup, temp_dir};

const FIXTURE: &str = "ts/tnt-5w-12732v-isi6-10s.ts";
const TS_PACKET: usize = 188;
const SERVICES: [u16; 5] = [0x0601, 0x0606, 0x0608, 0x0609, 0x060A];

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("nonzero")
}

fn trunk_config() -> TrunkConfig {
    TrunkConfig::default()
        .with_timed_capacity(nz(8))
        .with_sparse_capacity(nz(8))
        .with_segment_capacity(nz(64))
        .with_event_capacity(nz(8))
        .with_part_capacity(nz(8))
}

fn dvr_config() -> ArchiveConfig {
    let retention = RetentionLimits::new().with_periods(nz(8));
    ArchiveConfig::new(ArchiveContainer::Ts, Duration::from_secs(2), retention)
        .with_period_duration(Some(Duration::from_secs(4)))
}

fn services_in(ts: &[u8]) -> BTreeSet<u16> {
    let mut demux = SiDemux::new(SiDemuxConfig::default());
    let mut services = BTreeSet::new();
    let mut offset = 0usize;
    while offset + TS_PACKET <= ts.len() {
        for event in demux.feed(&ts[offset..offset + TS_PACKET]) {
            match event.table_section() {
                Ok(AnyTableSection::SdtSection(sdt)) => {
                    for service in &sdt.services {
                        services.insert(service.service_id);
                    }
                }
                Ok(AnyTableSection::PmtSection(pmt)) => {
                    services.insert(pmt.program_number);
                }
                _ => {}
            }
        }
        offset += TS_PACKET;
    }
    services
}

pub struct Recording {
    pub root: std::path::PathBuf,
    pub scan: dvr_runtime::read::ArchiveScan,
    pub closed: BTreeMap<u32, Bytes>,
    pub archive: FsArchive,
    pub programs: usize,
}

pub fn record_into(root: std::path::PathBuf) -> Recording {
    let bytes = test_fixtures::read(FIXTURE);
    let mut driver = IngestDriver::new(
        TsIngestSession::new(),
        IngestConfig::new(HandshakePolicy::establish_by(Timestamp::from_nanos(
            u64::MAX,
        )))
        .with_trunk(trunk_config()),
    );
    let mut now = Timestamp::ZERO;
    let mut offset = 0usize;
    while offset < bytes.len() {
        let end = (offset + TS_PACKET * 64).min(bytes.len());
        driver.feed(&bytes[offset..end], now);
        now = now.saturating_add(Duration::from_millis(4));
        offset = end;
    }
    driver.finish();
    let programs = driver.programs().collect::<Vec<ProgramId>>();
    assert!(!programs.is_empty(), "the MPTS must announce programmes");
    let trunk: Arc<Trunk> = Arc::clone(driver.trunk(programs[0]).expect("program trunk"));
    let mut live = trunk.subscribe_segments();
    let mut recorder = FsRecorder::new(
        &root,
        ArchiveName::new("cam").expect("name"),
        dvr_config(),
        &trunk,
    )
    .expect("recorder");

    let slice = (bytes.len() / SERVICES.len()).div_ceil(TS_PACKET) * TS_PACKET;
    let writer = trunk.segment_writer().expect("segment writer");
    for (index, chunk) in bytes.chunks(slice).enumerate() {
        let seq = u32::try_from(index).expect("index") + 1;
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::copy_from_slice(chunk),
                media_plane::ids::SegmentSeq::new(seq),
                Duration::from_secs(2),
                Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
                SegmentMeta {
                    discontinuous: index == 0,
                },
            ))
            .expect("publish a capture slice");
        now = now.saturating_add(Duration::from_secs(2));
        recorder.poll(now, |_| None, &[]).expect("record the slice");
    }
    recorder.poll(now, |_| None, &[]).expect("final poll");

    let mut closed = BTreeMap::new();
    while let Some(item) = live.poll() {
        if let media_plane::trunk::SegmentCursorItem::Segment(entry) = item {
            closed.insert(entry.sequence_number.value(), entry.bytes);
        }
    }
    let mut archive = FsArchive::open(
        &root,
        &ArchiveName::new("cam").expect("name"),
        ArchiveContainer::Ts,
    );
    let scan = archive.scan();
    Recording {
        root,
        scan,
        closed,
        archive,
        programs: programs.len(),
    }
}

#[test]
fn dvb_capture_records_and_reads_back() {
    let recording = record_into(temp_dir("real-dvbt"));
    assert!(
        recording.scan.unreadable.is_empty(),
        "{:?}",
        recording.scan.unreadable
    );
    assert_eq!(
        recording.programs, 5,
        "the capture announces five programmes"
    );
    assert!(
        !recording.scan.segments.is_empty(),
        "the capture must produce segments"
    );
    assert_eq!(
        recording.closed.len(),
        5,
        "five two-second slices were closed"
    );

    for segment in &recording.scan.segments {
        let expected = recording
            .closed
            .get(&segment.seq.value())
            .unwrap_or_else(|| panic!("seq {} was not closed by the Trunk", segment.seq));
        let archived = recording
            .archive
            .read(segment)
            .expect("read archived bytes");
        assert_eq!(
            &archived, expected,
            "seq {} must be byte-exact",
            segment.seq
        );
    }

    let mut periods = BTreeSet::new();
    for segment in &recording.scan.segments {
        periods.insert(segment.period);
    }
    for period in periods {
        let path = recording.root.join("cam").join(format!("p{period}.m3u8"));
        let text = std::fs::read_to_string(&path).expect("playlist");
        MediaPlaylist::parse(&text).expect("a standard HLS playlist parses");
        parse_period_playlist(period, ArchiveContainer::Ts, text.as_bytes())
            .expect("the archive playlist parses");
    }

    let mut remuxed = Vec::new();
    for segment in &recording.scan.segments {
        remuxed.extend_from_slice(&recording.archive.read(segment).expect("read"));
    }
    let archived_services = services_in(&remuxed);
    let source_services = services_in(&test_fixtures::read(FIXTURE));
    assert_eq!(
        source_services,
        BTreeSet::from(SERVICES),
        "the live path carries the capture's five services"
    );
    assert_eq!(
        archived_services, source_services,
        "re-demuxing the archived bytes finds the same five services"
    );
    cleanup(&recording.root);
    let _ = PeriodNum::new(0);
}

#[test]
fn the_eit_transition_closes_p0_and_opens_p1_with_the_new_event() {
    use broadcast_common::{Parse as WireParse, Serialize as WireSerialize};
    use dvb_si::tables::RunningStatus;
    use dvb_si::tables::eit::{EitKind, EitSection};

    const TF1: u16 = 0x0601;
    let ts_bytes = test_fixtures::read(FIXTURE);
    let mut discover = SiDemux::new(
        SiDemuxConfig::default()
            .with_dvb_si_pids(false)
            .with_pid(dvb_si::tables::eit::PID),
    );
    let mut sections = Vec::new();
    let mut offset = 0usize;
    while offset + TS_PACKET <= ts_bytes.len() {
        for event in discover.feed(&ts_bytes[offset..offset + TS_PACKET]) {
            if let Ok(AnyTableSection::EitSection(section)) = event.table_section()
                && section.kind == EitKind::PresentFollowingActual
                && section.service_id == TF1
            {
                sections.push(event.bytes().clone());
            }
        }
        offset += TS_PACKET;
    }
    let parsed: Vec<EitSection<'_>> = sections
        .iter()
        .map(|bytes| EitSection::parse(bytes).expect("parse a genuine EIT p/f section"))
        .collect();
    let genuine = &parsed[0];
    let mut following = parsed
        .iter()
        .flat_map(|section| section.events.iter())
        .find(|event| event.running_status != RunningStatus::Running)
        .cloned()
        .expect("a genuine following event");
    assert_eq!(
        parsed
            .iter()
            .flat_map(|section| section.events.iter())
            .find(|event| event.running_status == RunningStatus::Running)
            .expect("a genuine present event")
            .event_id,
        0x7857
    );
    assert_eq!(following.event_id, 0x7858);
    following.running_status = RunningStatus::Running;
    let transitioned = EitSection::new(
        genuine.kind,
        genuine.table_id,
        genuine.service_id,
        dvb_si::tables::SectionVersion::new((genuine.version_number + 1) % 32, true, 0, 0),
        genuine.transport_stream_id,
        genuine.original_network_id,
        0,
        genuine.table_id,
        vec![following],
    );
    let mut section_buf = vec![0u8; WireSerialize::serialized_len(&transitioned)];
    WireSerialize::serialize_into(&transitioned, &mut section_buf).expect("serialize");
    let mut packets = Vec::new();
    for packet in
        &mpeg_ts::mux::SectionPacketiser::new(dvb_si::tables::eit::PID).packetise(&[&section_buf])
    {
        packets.extend_from_slice(packet);
    }

    let root = temp_dir("real-eit");
    let trunk = Arc::new(Trunk::new(trunk_config()));
    let writer = trunk.segment_writer().expect("writer");
    let config = ArchiveConfig::new(
        ArchiveContainer::Ts,
        Duration::from_secs(2),
        RetentionLimits::new().with_periods(nz(8)),
    )
    .with_period_duration(Some(Duration::from_secs(3600)))
    .with_programme_service(TF1);
    let mut recorder = FsRecorder::new(
        &root,
        ArchiveName::new("tf1").expect("name"),
        config,
        &trunk,
    )
    .expect("recorder");
    recorder.feed_si(&ts_bytes).expect("baseline");
    assert_eq!(
        recorder.writer().programme().map(|p| p.event.event_id),
        Some(0x7857)
    );
    let slice = (ts_bytes.len() / 5).div_ceil(TS_PACKET) * TS_PACKET;
    let publish = |seq: u32, chunk: &[u8], at: u64| {
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::copy_from_slice(chunk),
                media_plane::ids::SegmentSeq::new(seq),
                Duration::from_secs(2),
                Timestamp::from_nanos(at),
                SegmentMeta {
                    discontinuous: seq == 1,
                },
            ))
            .expect("publish");
    };
    publish(1, &ts_bytes[..slice], 0);
    recorder
        .poll(Timestamp::ZERO, |_| Some(crate::support::ts_at(1)), &[])
        .expect("p0");
    let p0 = std::fs::read_to_string(root.join("tf1").join("p0.m3u8")).expect("p0");
    assert!(p0.contains("X-DVB-EVENT-ID=30807"), "{p0}");

    recorder.feed_si(&packets).expect("the transition");
    assert_eq!(
        recorder.writer().programme().map(|p| p.event.event_id),
        Some(0x7858)
    );
    let p0 = std::fs::read_to_string(root.join("tf1").join("p0.m3u8")).expect("p0");
    assert!(
        p0.ends_with("#EXT-X-ENDLIST\n"),
        "the transition closes p0: {p0}"
    );
    assert!(
        !root.join("tf1").join("p1.m3u8").exists(),
        "p1 waits for its first segment"
    );

    publish(2, &ts_bytes[slice..2 * slice], 2_000_000_000);
    recorder
        .poll(
            Timestamp::from_nanos(2_000_000_000),
            |_| Some(crate::support::ts_at(61)),
            &[],
        )
        .expect("p1");
    let p1 = std::fs::read_to_string(root.join("tf1").join("p1.m3u8")).expect("p1");
    assert!(p1.contains("X-DVB-EVENT-ID=30808"), "{p1}");
    assert!(p1.contains("X-DVB-SERVICE-ID=1537"), "{p1}");
    assert!(
        p1.contains("X-TITLE=\"Plus belle la vie, encore...\""),
        "{p1}"
    );
    assert_eq!(p1.matches("#EXT-X-DATERANGE:").count(), 1);
    assert!(!p1.contains("#EXT-X-ENDLIST"));
    cleanup(&root);
}
