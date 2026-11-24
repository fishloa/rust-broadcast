//! The bounded-memory streaming file session (`StreamingFileSession`) against
//! the whole-file reference path (`probe_and_demux` + `FileIngestSession`), plus
//! the behaviour decisions the session owns: start window, merge order, hold
//! bounds, loop bookkeeping and 33-bit wrap handling (issue #4057).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use broadcast_common::timescale::Timescale;
use broadcast_common::{Package, Stage, Timestamp};
use media_plane::file_source::{
    FileIngestSession, FileReaderError, PassRange, advance_offsets, probe_and_demux,
};
use media_plane::file_stream::{
    FileDemand, FileInput, FileStreamLimits, StreamContainer, StreamingFileSession,
};
use media_plane::ingress::{ProgramId, SessionEvent};
use transmux::ir::{Track, TrackId, TrackSpec};
use transmux::pipeline::{CodecConfig, DataCarriage};
use transmux::{Media, Sample, Track as IrTrack};

const CHUNK: usize = 64 << 10;
const PASSES: u32 = 3;
/// The whole-file session needs two drain calls per pass: setup, then data.
const DRAINS_PER_PASS: usize = 2;

/// One emitted sample, in the exact shape both paths are compared on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Emitted {
    track: u32,
    pts: i64,
    dts: i64,
    duration: Option<u32>,
    sync: bool,
    data: Vec<u8>,
}

fn emitted(event: SessionEvent) -> Option<Emitted> {
    let SessionEvent::Sample {
        track_id, sample, ..
    } = event
    else {
        return None;
    };
    let timing = sample.timing?;
    Some(Emitted {
        track: track_id.value(),
        pts: timing.pts()?,
        dts: timing.dts()?,
        duration: timing.duration(),
        sync: sample.flags.is_sync,
        data: sample.data.to_vec(),
    })
}

/// The reference path: demux the whole buffer, then replay it three passes.
fn old_path(bytes: &[u8]) -> Vec<Emitted> {
    let parsed = probe_and_demux(bytes).expect("whole-file demux");
    let mut session = FileIngestSession::new(parsed, ProgramId(0), true, false)
        .expect("the fixture has playable samples");
    let mut out = Vec::new();
    for _pass in 0..PASSES {
        session
            .feed((), Timestamp::ZERO)
            .expect("whole-file session feed");
        // The first poll call ends after `NewProgram` (the driver's
        // `advance_route` step sits between it and the first sample), so the
        // data is in the second call.
        for _drain in 0..DRAINS_PER_PASS {
            while let Some(event) = session.poll() {
                if let Some(item) = emitted(event) {
                    out.push(item);
                }
            }
        }
    }
    out
}

/// Drive the streaming session from a byte buffer, applying its demands.
fn drive_passes(
    bytes: &[u8],
    container: StreamContainer,
    passes: Option<u32>,
    chunk: usize,
) -> Vec<Emitted> {
    let mut session = StreamingFileSession::new(
        container,
        ProgramId(0),
        true,
        passes,
        false,
        FileStreamLimits::default(),
    );
    drive(&mut session, bytes, chunk)
}

/// Drive the session until `Done`. Every iteration must make progress: feed
/// bytes, send end of input, rewind, drain an event, or change the session's
/// demand; anything else is the stall the loop budget catches.
fn drive(session: &mut StreamingFileSession, bytes: &[u8], chunk: usize) -> Vec<Emitted> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut eof_sent = false;
    let budget = (bytes.len() + bytes.len() / chunk.max(1) + 8) * 4;
    for _iteration in 0..budget {
        let demand = session.file_demand();
        if demand == FileDemand::Done {
            return out;
        }
        let before_offset = offset;
        let before_eof = eof_sent;
        let before_events = out.len();
        match demand {
            FileDemand::Bytes => {
                if offset >= bytes.len() {
                    session
                        .feed(FileInput::Eof, Timestamp::ZERO)
                        .expect("end of input");
                } else {
                    offset = feed_chunk(session, bytes, offset, chunk);
                }
            }
            FileDemand::Rewind => {
                offset = feed_chunk(session, bytes, 0, chunk);
            }
            FileDemand::Wait => {
                eof_sent = eof_when_exhausted(session, offset, bytes.len(), eof_sent);
            }
            _ => {}
        }
        while let Some(event) = session.poll() {
            if let Some(item) = emitted(event) {
                out.push(item);
            }
        }
        let progressed = offset != before_offset
            || eof_sent != before_eof
            || out.len() != before_events
            || session.file_demand() != demand;
        assert!(
            progressed,
            "drive stalled: demand {demand:?}, offset {offset} of {}, chunk {chunk}, \
             samples {}",
            bytes.len(),
            out.len()
        );
    }
    panic!(
        "drive exceeded its budget for {} bytes in {chunk}-byte chunks ({} samples)",
        bytes.len(),
        out.len()
    );
}

/// The session's hold can fill before its start window closes (a short
/// container whose tracks all appear at the start); the caller's only
/// remaining input is then end of file.
fn eof_when_exhausted(
    session: &mut StreamingFileSession,
    offset: usize,
    len: usize,
    sent: bool,
) -> bool {
    if offset >= len && !sent {
        session
            .feed(FileInput::Eof, Timestamp::ZERO)
            .expect("end of input");
        return true;
    }
    sent
}

fn feed_chunk(
    session: &mut StreamingFileSession,
    bytes: &[u8],
    offset: usize,
    chunk: usize,
) -> usize {
    let end = offset.saturating_add(chunk).min(bytes.len());
    session
        .feed(FileInput::Bytes(&bytes[offset..end]), Timestamp::ZERO)
        .expect("session feed");
    end
}

/// Compare two emitted sequences, reporting the first divergence compactly
/// (the payload bytes are too large to print).
fn assert_same(label: &str, new: &[Emitted], old: &[Emitted]) {
    let mut index = 0usize;
    while index < new.len() && index < old.len() && new[index] == old[index] {
        index += 1;
    }
    if index == new.len() && index == old.len() {
        return;
    }
    let summary = |items: &[Emitted]| {
        items.get(index).map(|item| {
            (
                item.track,
                item.pts,
                item.dts,
                item.duration,
                item.data.len(),
            )
        })
    };
    panic!(
        "{label}: {index} equal of new {} / old {}; new {:?}, old {:?}",
        new.len(),
        old.len(),
        summary(new),
        summary(old)
    );
}

type TrackSequence = Vec<(i64, i64, Option<u32>, Vec<u8>)>;

/// Each track's emitted samples in output order, keyed by track id.
fn per_track(items: &[Emitted]) -> BTreeMap<u32, TrackSequence> {
    let mut out: BTreeMap<u32, TrackSequence> = BTreeMap::new();
    for item in items {
        out.entry(item.track).or_default().push((
            item.pts,
            item.dts,
            item.duration,
            item.data.clone(),
        ));
    }
    out
}

/// Compare the per-track sequences without printing their payload bytes.
fn assert_per_track_same(label: &str, new: &[Emitted], old: &[Emitted]) {
    let new_map = per_track(new);
    let old_map = per_track(old);
    assert_eq!(
        new_map.keys().collect::<Vec<_>>(),
        old_map.keys().collect::<Vec<_>>(),
        "{label}: track set"
    );
    for (track, new_seq) in &new_map {
        let old_seq = old_map.get(track).expect("track present in both");
        if new_seq == old_seq {
            continue;
        }
        let index = new_seq
            .iter()
            .zip(old_seq)
            .position(|(a, b)| a != b)
            .unwrap_or(new_seq.len().min(old_seq.len()));
        let summary = |seq: &[(i64, i64, Option<u32>, Vec<u8>)]| {
            seq.get(index)
                .map(|item| (item.0, item.1, item.2, item.3.len()))
        };
        panic!(
            "{label}: track {track} differs at {index} of {} new / {} old: \
             new {:?}, old {:?}",
            new_seq.len(),
            old_seq.len(),
            summary(new_seq),
            summary(old_seq)
        );
    }
}

fn new_path(bytes: &[u8], container: StreamContainer) -> Vec<Emitted> {
    drive_passes(bytes, container, Some(PASSES), CHUNK)
}

#[test]
fn ts_streaming_session_matches_the_whole_file_session() {
    for name in ["ts/h264-aac.ts", "ts/h264-aac-40s.ts", "ts/m6-single.ts"] {
        let bytes = test_fixtures::read(name);
        assert_same(
            name,
            &new_path(&bytes, StreamContainer::MpegTs),
            &old_path(&bytes),
        );
    }
    if let Some((path, _)) = test_fixtures::find("ts/france-tnt-m6-eac3-zenith.ts") {
        let bytes = std::fs::read(path).expect("read private fixture");
        let new = new_path(&bytes, StreamContainer::MpegTs);
        let old = old_path(&bytes);
        // This capture carries track 5 at pts 0 beside media at real 90 kHz
        // timestamps: the whole-file sort puts those samples first, while a
        // streaming merge cannot know a late-arriving zero belongs at the
        // front. Per-track order and the timed-track interleave are the
        // invariants that survive every ordering.
        assert_per_track_same("zenith", &new, &old);
        let timed_new: Vec<&Emitted> = new.iter().filter(|item| item.pts != 0).collect();
        let timed_old: Vec<&Emitted> = old.iter().filter(|item| item.pts != 0).collect();
        assert_eq!(timed_new, timed_old, "zenith: timed-track interleave");
    }
}

#[test]
fn flv_streaming_session_matches_the_whole_file_session() {
    for name in ["flv/av.flv", "flv/aac-7-1.flv"] {
        let bytes = test_fixtures::read(name);
        assert_same(
            name,
            &new_path(&bytes, StreamContainer::Flv),
            &old_path(&bytes),
        );
    }
}

#[test]
fn chunk_size_does_not_change_the_output() {
    for (name, container) in [
        ("ts/h264-aac.ts", StreamContainer::MpegTs),
        ("flv/av.flv", StreamContainer::Flv),
    ] {
        let bytes = test_fixtures::read(name);
        let reference = drive_passes(&bytes, container, Some(1), CHUNK);
        for chunk in [1usize, 187, 188, 4096, 1 << 20] {
            assert_same(
                &format!("{name} with a {chunk}-byte chunk"),
                &drive_passes(&bytes, container, Some(1), chunk),
                &reference,
            );
        }
    }
}

fn timescale(hz: u32) -> Timescale {
    Timescale::try_new(hz).expect("a non-zero timescale")
}

/// A `CodecConfig::Data` track carrying `samples` on a 90 kHz grid.
fn data_track(id: u32, samples: Vec<Sample>) -> IrTrack {
    Track::new(
        TrackSpec::new(
            TrackId::new(id),
            timescale(90_000),
            CodecConfig::Data {
                stream_type: 0x06,
                descriptors: Vec::new(),
                carriage: DataCarriage::Pes,
            },
        ),
        samples,
    )
}

fn timed_sample(tag: u8, index: usize, dts: i64, duration: u32) -> Sample {
    Sample::new(
        vec![tag, index as u8, 0xAB, 0xCD],
        Some(dts),
        Some(dts),
        Some(duration),
        true,
    )
    .expect("valid timing")
}

/// A TS whose first track runs at 25 fps and whose second carries one sample
/// every 60 s, so the merge must not wait for the sparse track.
fn sparse_ts() -> Vec<u8> {
    let frame = 3_600u32;
    let video = (0..1_875)
        .map(|i| timed_sample(1, i, i as i64 * i64::from(frame), frame))
        .collect::<Vec<_>>();
    let sparse = [0i64, 5_400_000, 10_800_000]
        .iter()
        .enumerate()
        .map(|(i, &dts)| timed_sample(2, i, dts, frame))
        .collect::<Vec<_>>();
    let media = Media::new(
        vec![data_track(1, video), data_track(2, sparse)],
        timescale(90_000),
    );
    transmux::TsMux::new()
        .package(&media)
        .expect("mux sparse TS")
}

/// A TS whose video runs at real 90 kHz presentation times (0..5 s) and whose
/// data track carries pts 0 while arriving at 1 s, 2 s and 6 s in decode
/// order: a timestamp far behind the presentation frontier by the time the
/// demuxer releases it.
fn out_of_range_pts_ts() -> Vec<u8> {
    let frame = 3_600u32;
    let start = 0i64;
    let video = (0..125)
        .map(|i| timed_sample(1, i, start + i as i64 * i64::from(frame), frame))
        .collect::<Vec<_>>();
    let data = (0..10u32)
        .map(|i| {
            let dts = 90_000 + i64::from(i) * 45_000;
            Sample::new(
                vec![7, i as u8, 0xAB, 0xCD],
                Some(start + dts),
                Some(0),
                Some(1_024),
                true,
            )
            .expect("valid timing")
        })
        .collect::<Vec<_>>();
    let media = Media::new(
        vec![data_track(1, video), data_track(2, data)],
        timescale(90_000),
    );
    transmux::TsMux::new()
        .package(&media)
        .expect("mux out-of-range TS")
}

/// A TS with a PAT/PMT (one declared track) and no access units at all.
fn psi_only_ts() -> Vec<u8> {
    let media = Media::new(vec![data_track(1, Vec::new())], timescale(90_000));
    transmux::TsMux::new()
        .package(&media)
        .expect("mux PSI-only TS")
}

/// The PES timestamp fields in `bytes` as `(byte offset, 5-byte field)`.
fn pes_stamps(bytes: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::new();
    for (index, packet) in bytes.chunks_exact(188).enumerate() {
        let base = index * 188;
        if packet.first() != Some(&0x47) {
            continue;
        }
        let payload_start = packet[1] & 0x40 != 0;
        let afc = (packet[3] >> 4) & 0x3;
        if !payload_start || afc & 0x1 == 0 {
            continue;
        }
        let mut payload = 4;
        if afc & 0x2 != 0 {
            payload += 1 + usize::from(packet[4]);
        }
        let pes = payload;
        let Some(header) = packet.get(pes..pes + 9) else {
            continue;
        };
        if header[..3] != [0, 0, 1] {
            continue;
        }
        match header[7] & 0xC0 {
            0x80 => offsets.push(base + pes + 9),
            0xC0 => {
                offsets.push(base + pes + 9);
                offsets.push(base + pes + 14);
            }
            _ => {}
        }
    }
    offsets
}

/// Shift every PES PTS/DTS so each track's presentation span straddles the
/// 33-bit wrap, exercising the demuxer's unroll in both paths.
fn restamp_across_wrap(bytes: &[u8]) -> Vec<u8> {
    use broadcast_common::clock33::{WRAP_33BIT, add, unpack_marked};
    let offsets = pes_stamps(bytes);
    let stamps: Vec<u64> = offsets
        .iter()
        .map(|offset| {
            unpack_marked(bytes[*offset..*offset + 5].try_into().expect("5 bytes")).timestamp
        })
        .collect();
    let min = stamps.iter().copied().min().unwrap_or(0);
    let max = stamps.iter().copied().max().unwrap_or(min);
    let half = (max - min) / 2;
    let delta = (i128::from(WRAP_33BIT) - i128::from(half) - i128::from(min % WRAP_33BIT))
        .rem_euclid(i128::from(WRAP_33BIT));
    let delta = u64::try_from(delta).expect("a value below the wrap");
    let mut out = bytes.to_vec();
    for offset in offsets {
        let field: [u8; 5] = out[offset..offset + 5].try_into().expect("5 bytes");
        let marked = unpack_marked(&field);
        let shifted = add(marked.timestamp, delta);
        let packed =
            broadcast_common::clock33::pack_marked_with(shifted, marked.prefix, marked.markers);
        out[offset..offset + 5].copy_from_slice(&packed);
    }
    out
}

#[test]
fn events_start_established_then_new_program_then_samples() {
    let bytes = test_fixtures::read("ts/h264-aac.ts");
    let mut session = StreamingFileSession::new(
        StreamContainer::MpegTs,
        ProgramId(7),
        false,
        None,
        false,
        FileStreamLimits::default(),
    );
    let mut events = Vec::new();
    let mut offset = 0usize;
    let mut eof_sent = false;
    let mut done = false;
    let budget = (bytes.len() + bytes.len() / CHUNK + 8) * 4;
    for _iteration in 0..budget {
        let before = (offset, eof_sent, events.len(), session.file_demand());
        match session.file_demand() {
            FileDemand::Bytes => {
                if offset >= bytes.len() {
                    session
                        .feed(FileInput::Eof, Timestamp::ZERO)
                        .expect("end of input");
                } else {
                    offset = feed_chunk(&mut session, &bytes, offset, CHUNK);
                }
            }
            FileDemand::Wait => {
                eof_sent = eof_when_exhausted(&mut session, offset, bytes.len(), eof_sent);
            }
            FileDemand::Rewind => panic!("a non-looping session never rewinds"),
            FileDemand::Done => {
                done = true;
                break;
            }
            _ => {}
        }
        while let Some(event) = session.poll() {
            events.push(event);
        }
        assert!(
            offset != before.0
                || eof_sent != before.1
                || events.len() != before.2
                || session.file_demand() != before.3,
            "events drive stalled at offset {offset}"
        );
    }
    assert!(done, "events drive exhausted its budget");
    assert!(matches!(events.first(), Some(SessionEvent::Established)));
    let Some(SessionEvent::NewProgram { program, tracks }) = events.get(1) else {
        panic!(
            "second event must be the program announcement, got {:?}",
            events.get(1)
        );
    };
    assert_eq!(*program, ProgramId(7));
    assert!(!tracks.is_empty());
    assert!(
        events[2..]
            .iter()
            .any(|event| matches!(event, SessionEvent::Sample { .. })),
        "samples follow the announcement"
    );
}

#[test]
fn sparse_track_does_not_stall_the_merge_and_stays_in_order() {
    let bytes = sparse_ts();
    let out = drive_passes(&bytes, StreamContainer::MpegTs, Some(1), 4096);
    let video: Vec<i64> = out.iter().filter(|e| e.track == 1).map(|e| e.pts).collect();
    assert_eq!(video.len(), 1_875, "every video sample is released");
    assert!(video.windows(2).all(|pair| pair[0] < pair[1]));
    let sparse: Vec<i64> = out.iter().filter(|e| e.track == 2).map(|e| e.pts).collect();
    assert_eq!(sparse.len(), 3, "every sparse sample is released");
    assert!(
        sparse.windows(2).all(|pair| pair[1] - pair[0] == 5_400_000),
        "sparse samples keep their one-per-60-s cadence: {sparse:?}"
    );
    assert_eq!(
        out.len(),
        1_878,
        "every video sample is released without waiting for the sparse track"
    );
}

#[test]
fn an_untimed_data_track_is_released_in_arrival_order_and_does_not_stall() {
    let bytes = out_of_range_pts_ts();
    let limits = FileStreamLimits {
        max_queued_bytes: 4 << 10,
        read_ahead: Duration::from_secs(600),
        start_window: Duration::from_secs(600),
        start_window_bytes: 4 << 10,
        ..FileStreamLimits::default()
    };
    let chunk = 1 << 10;
    let mut session = StreamingFileSession::new(
        StreamContainer::MpegTs,
        ProgramId(0),
        false,
        None,
        false,
        limits.clone(),
    );
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut eof_sent = false;
    let budget = (bytes.len() + bytes.len() / chunk + 8) * 4;
    for _iteration in 0..budget {
        let demand = session.file_demand();
        if demand == FileDemand::Done {
            break;
        }
        let before_offset = offset;
        let before_eof = eof_sent;
        let before_events = out.len();
        match demand {
            FileDemand::Bytes => {
                if offset >= bytes.len() {
                    session
                        .feed(FileInput::Eof, Timestamp::ZERO)
                        .expect("end of input");
                } else {
                    offset = feed_chunk(&mut session, &bytes, offset, chunk);
                    assert!(
                        session.queued_sample_bytes() <= limits.max_queued_bytes + chunk,
                        "held {} bytes after one {chunk}-byte chunk",
                        session.queued_sample_bytes()
                    );
                }
            }
            FileDemand::Wait => {
                eof_sent = eof_when_exhausted(&mut session, offset, bytes.len(), eof_sent);
            }
            _ => {}
        }
        while let Some(event) = session.poll() {
            if let Some(item) = emitted(event) {
                out.push(item);
            }
        }
        let progressed = offset != before_offset
            || eof_sent != before_eof
            || out.len() != before_events
            || session.file_demand() != demand;
        assert!(
            progressed,
            "stalled: demand {demand:?}, offset {offset}, {} samples",
            out.len()
        );
    }

    let video = out.iter().filter(|item| item.track == 1).count();
    assert_eq!(video, 125, "every video sample is released");
    let data: Vec<(usize, i64, i64)> = out
        .iter()
        .enumerate()
        .filter(|(_, item)| item.track == 2)
        .map(|(index, item)| (index, item.pts, item.dts))
        .collect();
    assert_eq!(data.len(), 10, "every data sample is released");
    assert!(
        data.iter().all(|entry| entry.1 == data[0].1),
        "every data sample keeps its out-of-range presentation time"
    );
    assert!(
        data.windows(2).all(|pair| pair[0].2 < pair[1].2),
        "data samples keep arrival (decode) order"
    );
    for pair in data.windows(2) {
        let (position, _, _) = pair[0];
        let arrival = pair[1].2;
        if let Some(after) = out
            .iter()
            .position(|later| later.track == 1 && later.dts > arrival)
        {
            assert!(
                position < after,
                "data sample at {position} was held past video arriving at {after}"
            );
        }
    }
}

#[test]
fn no_playable_samples_is_a_typed_error_after_the_first_pass() {
    let bytes = psi_only_ts();
    let mut session = StreamingFileSession::new(
        StreamContainer::MpegTs,
        ProgramId(0),
        true,
        Some(2),
        false,
        FileStreamLimits::default(),
    );
    session
        .feed(FileInput::Bytes(&bytes), Timestamp::ZERO)
        .expect("feed PSI-only TS");
    while session.poll().is_some() {}
    let err = session
        .feed(FileInput::Eof, Timestamp::ZERO)
        .expect_err("a file with no timed samples fails");
    assert!(matches!(err, FileReaderError::NoPlayableSamples));
    assert_ne!(session.file_demand(), FileDemand::Rewind);
}

#[test]
fn truncated_tail_plays_whole_samples_and_loops_cleanly() {
    let bytes = test_fixtures::read("ts/h264-aac.ts");
    let cut = bytes.len() - 100;
    let mut session = StreamingFileSession::new(
        StreamContainer::MpegTs,
        ProgramId(0),
        true,
        Some(2),
        false,
        FileStreamLimits::default(),
    );
    let mut passes: Vec<Vec<Emitted>> = Vec::new();
    let mut current = Vec::new();
    let mut offset = 0usize;
    let mut eof_sent = false;
    let mut done = false;
    let budget = (cut + cut / 4096 + 8) * 8;
    for _iteration in 0..budget {
        let before = (
            offset,
            eof_sent,
            current.len(),
            passes.len(),
            session.file_demand(),
        );
        match session.file_demand() {
            FileDemand::Bytes => {
                if offset >= cut {
                    session
                        .feed(FileInput::Eof, Timestamp::ZERO)
                        .expect("end of input");
                } else {
                    offset = feed_chunk(&mut session, &bytes[..cut], offset, 4096);
                }
            }
            FileDemand::Rewind => {
                passes.push(std::mem::take(&mut current));
                offset = feed_chunk(&mut session, &bytes[..cut], 0, 4096);
                eof_sent = false;
            }
            FileDemand::Wait => {
                eof_sent = eof_when_exhausted(&mut session, offset, cut, eof_sent);
            }
            FileDemand::Done => {
                passes.push(std::mem::take(&mut current));
                done = true;
                break;
            }
            _ => {}
        }
        while let Some(event) = session.poll() {
            if let Some(item) = emitted(event) {
                current.push(item);
            }
        }
        assert!(
            offset != before.0
                || eof_sent != before.1
                || current.len() != before.2
                || passes.len() != before.3
                || session.file_demand() != before.4,
            "looped drive stalled at offset {offset}"
        );
    }
    assert!(done, "looped drive exhausted its budget");
    assert_eq!(passes.len(), 2, "two passes");
    let (first, second) = (&passes[0], &passes[1]);
    assert!(!first.is_empty() && !second.is_empty());
    let mut tracks: Vec<u32> = first.iter().map(|e| e.track).collect();
    tracks.sort_unstable();
    tracks.dedup();
    for track in tracks {
        let mut pts: Vec<i64> = first
            .iter()
            .filter(|e| e.track == track)
            .map(|e| e.pts)
            .collect();
        pts.sort_unstable();
        pts.dedup();
        let frame = pts
            .windows(2)
            .filter_map(|pair| pair[1].checked_sub(pair[0]))
            .filter(|gap| *gap > 0)
            .min()
            .expect("a timeline with a positive gap");
        let first_second = second
            .iter()
            .find(|e| e.track == track)
            .expect("the track plays pass 2")
            .pts;
        assert_eq!(
            first_second,
            pts.last().copied().expect("pass 1 has samples") + frame,
            "pass 2 continues one frame past pass 1 on track {track}"
        );
    }
}

#[test]
fn pts_wrap_inside_a_pass_keeps_the_timeline_increasing() {
    let bytes = restamp_across_wrap(&test_fixtures::read("ts/h264-aac.ts"));
    assert_same(
        "wrap restamp",
        &drive_passes(&bytes, StreamContainer::MpegTs, Some(PASSES), CHUNK),
        &old_path(&bytes),
    );
    let out = drive_passes(&bytes, StreamContainer::MpegTs, Some(1), CHUNK);
    let wrap = broadcast_common::clock33::WRAP_33BIT as i64;
    let mut tracks: Vec<u32> = out.iter().map(|e| e.track).collect();
    tracks.sort_unstable();
    tracks.dedup();
    let mut crossed = Vec::new();
    for track in tracks {
        let pts: Vec<i64> = out
            .iter()
            .filter(|e| e.track == track)
            .map(|e| e.pts)
            .collect();
        let min = pts.iter().copied().min().expect("samples");
        let max = pts.iter().copied().max().expect("samples");
        if min < wrap && max > wrap {
            crossed.push(track);
        }
    }
    assert!(
        !crossed.is_empty(),
        "the restamped 90 kHz timeline crosses 2^33"
    );
}

#[test]
fn demand_is_wait_once_the_read_ahead_is_full_and_bytes_after_draining() {
    let bytes = test_fixtures::read("ts/h264-aac-40s.ts");
    let mut session = StreamingFileSession::new(
        StreamContainer::MpegTs,
        ProgramId(0),
        false,
        None,
        false,
        FileStreamLimits::default(),
    );
    let mut offset = 0usize;
    while session.file_demand() == FileDemand::Bytes {
        if offset >= bytes.len() {
            break;
        }
        offset = feed_chunk(&mut session, &bytes, offset, 4096);
    }
    assert_eq!(
        session.file_demand(),
        FileDemand::Wait,
        "the read-ahead hold stops the feed before the file is exhausted"
    );
    while session.poll().is_some() {}
    assert_eq!(
        session.file_demand(),
        FileDemand::Bytes,
        "draining the queues re-opens the demand"
    );
}

#[test]
fn a_track_set_change_between_passes_is_reannounced() {
    let first = test_fixtures::read("ts/h264-aac.ts");
    let second = test_fixtures::read("ts/m6-single.ts");
    let mut session = StreamingFileSession::new(
        StreamContainer::MpegTs,
        ProgramId(0),
        true,
        Some(2),
        false,
        FileStreamLimits::default(),
    );
    let mut announced: Vec<Vec<TrackSpec>> = Vec::new();
    let mut bytes: &[u8] = &first;
    let mut offset = 0usize;
    let mut eof_sent = false;
    let mut done = false;
    let budget = (first.len() + second.len() + 8) * 4;
    for _iteration in 0..budget {
        let before = (offset, eof_sent, announced.len(), session.file_demand());
        match session.file_demand() {
            FileDemand::Bytes => {
                if offset >= bytes.len() {
                    session
                        .feed(FileInput::Eof, Timestamp::ZERO)
                        .expect("end of input");
                } else {
                    offset = feed_chunk(&mut session, bytes, offset, CHUNK);
                }
            }
            FileDemand::Rewind => {
                bytes = &second;
                offset = feed_chunk(&mut session, bytes, 0, CHUNK);
                eof_sent = false;
            }
            FileDemand::Wait => {
                eof_sent = eof_when_exhausted(&mut session, offset, bytes.len(), eof_sent);
            }
            FileDemand::Done => {
                done = true;
                break;
            }
            _ => {}
        }
        while let Some(event) = session.poll() {
            if let SessionEvent::NewProgram { tracks, .. } = event {
                announced.push(tracks);
            }
        }
        assert!(
            offset != before.0
                || eof_sent != before.1
                || announced.len() != before.2
                || session.file_demand() != before.3,
            "track-set drive stalled at offset {offset}"
        );
    }
    assert!(done, "track-set drive exhausted its budget");
    assert!(announced.len() >= 2, "the second pass re-announces");
    assert_ne!(announced.first(), announced.last());
}

#[test]
fn loop_offsets_overflow_is_a_typed_error() {
    let huge = PassRange {
        span_ticks: i64::MAX,
        frame_ticks: 1,
        span: Duration::ZERO,
        frame: Duration::ZERO,
    };
    let mut offsets = [0i64];
    assert!(matches!(
        advance_offsets(&mut offsets, std::slice::from_ref(&huge)),
        Err(FileReaderError::OffsetOverflow)
    ));
    let step = PassRange {
        span_ticks: 1,
        frame_ticks: 1,
        span: Duration::ZERO,
        frame: Duration::ZERO,
    };
    let mut offsets = [i64::MAX];
    assert!(matches!(
        advance_offsets(&mut offsets, std::slice::from_ref(&step)),
        Err(FileReaderError::OffsetOverflow)
    ));
}

#[test]
fn paced_sample_is_not_released_before_it_is_due() {
    let bytes = test_fixtures::read("ts/h264-aac.ts");
    let mut session = StreamingFileSession::new(
        StreamContainer::MpegTs,
        ProgramId(0),
        false,
        None,
        true,
        FileStreamLimits::default(),
    );
    let mut offset = 0usize;
    let mut eof_sent = false;
    let mut samples = 0usize;
    let budget = (bytes.len() + bytes.len() / CHUNK + 8) * 4;
    for _iteration in 0..budget {
        if samples > 0 {
            break;
        }
        let before = (offset, eof_sent, samples, session.file_demand());
        match session.file_demand() {
            FileDemand::Bytes => {
                if offset >= bytes.len() {
                    session
                        .feed(FileInput::Eof, Timestamp::ZERO)
                        .expect("end of input");
                } else {
                    offset = feed_chunk(&mut session, &bytes, offset, CHUNK);
                }
            }
            FileDemand::Wait => {
                eof_sent = eof_when_exhausted(&mut session, offset, bytes.len(), eof_sent);
            }
            FileDemand::Rewind | FileDemand::Done => panic!("a one-pass paced session"),
            _ => {}
        }
        while let Some(event) = session.poll() {
            if matches!(event, SessionEvent::Sample { .. }) {
                samples += 1;
                break;
            }
        }
        assert!(
            offset != before.0
                || eof_sent != before.1
                || samples != before.2
                || session.file_demand() != before.3,
            "paced drive stalled at offset {offset}"
        );
    }
    assert_eq!(samples, 1, "the first sample is due immediately");
    let due = session.next_due().expect("a paced session has a deadline");
    assert!(due > Instant::now(), "the next sample is not due yet");
    assert!(
        !matches!(session.poll(), Some(SessionEvent::Sample { .. })),
        "polling before the deadline releases nothing"
    );
}
