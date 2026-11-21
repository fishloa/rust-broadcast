#[cfg(feature = "fs")]
use std::path::{Path, PathBuf};
#[cfg(feature = "fs")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::epg::{PresentEventChange, PresentEventTracker};
use dvb_si::tables::RunningStatus;
use dvb_si::tables::SectionVersion;
use dvb_si::tables::eit::{EitEvent, EitKind, EitSection};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::SegmentEntry;

#[cfg(feature = "fs")]
pub fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/dvr-runtime-tests")
        .join(format!("{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[cfg(feature = "fs")]
pub fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn short_event(name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(0x4Du8);
    let name = name.as_bytes();
    bytes.push((3 + 1 + name.len() + 1) as u8);
    bytes.extend_from_slice(b"eng");
    bytes.push(name.len() as u8);
    bytes.extend_from_slice(name);
    bytes.push(0);
    bytes
}

pub fn present_event(
    tracker: &mut PresentEventTracker,
    service: u16,
    event: u16,
    title: &str,
    start: jiff::Timestamp,
    duration: Duration,
) -> PresentEventChange {
    let descriptors = short_event(title);
    let mut eit_event = EitEvent::new(
        event,
        [0; 5],
        [0; 3],
        RunningStatus::Running,
        false,
        DescriptorLoop::new(&descriptors),
    );
    eit_event.set_start_time(start).expect("start time");
    eit_event.set_duration(duration).expect("duration");
    let section = EitSection::new(
        EitKind::PresentFollowingActual,
        0x4E,
        service,
        SectionVersion::new(0, true, 0, 0),
        1,
        1,
        0,
        0x4E,
        vec![eit_event],
    );
    tracker.on_section(&section).expect("a present event")
}

pub fn segment_at(seq: u32, len: usize, at_ns: u64, discontinuous: bool) -> SegmentEntry {
    SegmentEntry::new(
        Bytes::from(vec![0xABu8; len]),
        SegmentSeq::new(seq),
        Duration::from_secs(2),
        Timestamp::from_nanos(at_ns),
        transmux::SegmentMeta { discontinuous },
    )
}

pub fn ts_at(secs: u64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(1_716_055_200 + i64::try_from(secs).expect("secs fit"))
        .expect("timestamp")
}

use dvr_runtime::writer::{PlanOutcome, Step, StoreOp, WriterEvent};
use dvr_runtime::{ArchiveConfig, ArchiveContainer, ArchiveWriter, RetentionLimits};

pub fn config(
    container: ArchiveContainer,
    target_secs: u64,
    retention: RetentionLimits,
) -> ArchiveConfig {
    ArchiveConfig::new(container, Duration::from_secs(target_secs), retention)
        .with_period_duration(Some(Duration::from_secs(86400)))
}

pub fn periods(n: usize) -> RetentionLimits {
    RetentionLimits::new().with_periods(std::num::NonZeroUsize::new(n).expect("nonzero"))
}

pub fn bytes_limit(n: u64) -> RetentionLimits {
    RetentionLimits::new().with_bytes(std::num::NonZeroU64::new(n).expect("nonzero"))
}

pub fn plan(step: Step) -> dvr_runtime::writer::Plan {
    match step {
        Step::Execute(plan) => plan,
        other => panic!("expected a plan, got {other:?}"),
    }
}

pub fn run(writer: &mut ArchiveWriter, step: Step) -> Vec<StoreOp> {
    let plan = plan(step);
    let ops = plan.ops.clone();
    writer
        .complete(plan.id, PlanOutcome::Done)
        .expect("complete");
    ops
}

pub fn events(writer: &mut ArchiveWriter) -> Vec<WriterEvent> {
    std::iter::from_fn(|| writer.poll_event()).collect()
}

pub fn playlist_text(ops: &[StoreOp]) -> String {
    let mut text = String::new();
    for op in ops {
        match op {
            StoreOp::CreatePlaylist { text: body, .. }
            | StoreOp::AppendPlaylist { text: body, .. } => text.push_str(body),
            _ => {}
        }
    }
    text
}

pub fn append_playlist_index(ops: &[StoreOp]) -> usize {
    ops.iter()
        .position(|op| matches!(op, StoreOp::AppendPlaylist { .. }))
        .expect("an append")
}

pub fn opened_events(
    events: &[WriterEvent],
) -> Vec<(dvr_runtime::PeriodNum, dvr_runtime::RollReason)> {
    events
        .iter()
        .filter_map(|event| match event {
            WriterEvent::PeriodOpened { period, reason } => Some((*period, *reason)),
            _ => None,
        })
        .collect()
}

pub fn evicted(events: &[WriterEvent]) -> Vec<dvr_runtime::PeriodNum> {
    events
        .iter()
        .filter_map(|event| match event {
            WriterEvent::Evicted { period } => Some(*period),
            _ => None,
        })
        .collect()
}

pub fn step_item(
    writer: &mut ArchiveWriter,
    now: Timestamp,
    wall: Option<jiff::Timestamp>,
    item: media_plane::trunk::SegmentCursorItem,
) -> (dvr_runtime::writer::Plan, Vec<StoreOp>) {
    let step = writer.on_item(now, wall, &item).expect("on_item");
    let plan = plan(step);
    let ops = plan.ops.clone();
    writer
        .complete(plan.id, PlanOutcome::Done)
        .expect("complete");
    (plan, ops)
}

pub fn step_item_fail(
    writer: &mut ArchiveWriter,
    now: Timestamp,
    wall: Option<jiff::Timestamp>,
    item: media_plane::trunk::SegmentCursorItem,
    at_op: usize,
    failure: dvr_runtime::writer::StoreFailure,
) -> (dvr_runtime::writer::Plan, Vec<StoreOp>) {
    let step = writer.on_item(now, wall, &item).expect("on_item");
    let plan = plan(step);
    let ops = plan.ops.clone();
    writer
        .complete(plan.id, PlanOutcome::Failed { at_op, failure })
        .expect("complete");
    (plan, ops)
}
