//! The bounded-memory streaming file session (`StreamingFileSession`) against
//! the whole-file reference path (`probe_and_demux` + `FileIngestSession`), plus
//! the behaviour decisions the session owns: start window, merge order, hold
//! bounds, loop bookkeeping and 33-bit wrap handling (issue #4057).

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

/// Limits for the whole-file equivalence tests: the merge sees the same
/// sample set the reference path does, so a container that reveals a track
/// late (a capture whose PIDs only resolve at end of input) cannot be
/// reordered by an early start-window close. The fixtures are small; the
/// bounded-memory behaviour is pinned separately by `file_stream_memory`.
fn differential_limits() -> FileStreamLimits {
    FileStreamLimits {
        read_ahead: Duration::MAX,
        start_window: Duration::MAX,
        start_window_bytes: usize::MAX,
        ..FileStreamLimits::default()
    }
}

/// Drive the streaming session from a byte buffer, applying its demands.
fn drive_passes(
    bytes: &[u8],
    container: StreamContainer,
    passes: Option<u32>,
    chunk: usize,
) -> Vec<Emitted> {
    drive_passes_with(bytes, container, passes, chunk, FileStreamLimits::default())
}

fn drive_passes_with(
    bytes: &[u8],
    container: StreamContainer,
    passes: Option<u32>,
    chunk: usize,
    limits: FileStreamLimits,
) -> Vec<Emitted> {
    let mut session =
        StreamingFileSession::new(container, ProgramId(0), true, passes, false, limits);
    drive(&mut session, bytes, chunk)
}

fn drive(session: &mut StreamingFileSession, bytes: &[u8], chunk: usize) -> Vec<Emitted> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut eof_sent = false;
    let mut iterations = 0u64;
    loop {
        iterations += 1;
        if iterations > 5_000_000 {
            panic!(
                "drive stuck: demand {:?} offset {} len {} chunk {} samples {}",
                session.file_demand(),
                offset,
                bytes.len(),
                chunk,
                out.len()
            );
        }
        match session.file_demand() {
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
            FileDemand::Done => break,
            _ => {}
        }
        while let Some(event) = session.poll() {
            if let Some(item) = emitted(event) {
                out.push(item);
            }
        }
    }
    out
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

fn new_path(bytes: &[u8], container: StreamContainer) -> Vec<Emitted> {
    drive_passes_with(bytes, container, Some(PASSES), CHUNK, differential_limits())
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
        assert_same(
            "ts/france-tnt-m6-eac3-zenith.ts",
            &new_path(&bytes, StreamContainer::MpegTs),
            &old_path(&bytes),
        );
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
        let reference = drive_passes_with(&bytes, container, Some(1), CHUNK, differential_limits());
        for chunk in [1usize, 187, 188, 4096, 1 << 20] {
            assert_same(
                &format!("{name} with a {chunk}-byte chunk"),
                &drive_passes_with(&bytes, container, Some(1), chunk, differential_limits()),
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
    loop {
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
            FileDemand::Done => break,
            _ => {}
        }
        while let Some(event) = session.poll() {
            events.push(event);
        }
    }
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
    loop {
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
                passes.push(current);
                break;
            }
            _ => {}
        }
        while let Some(event) = session.poll() {
            if let Some(item) = emitted(event) {
                current.push(item);
            }
        }
    }
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
        &drive_passes_with(
            &bytes,
            StreamContainer::MpegTs,
            Some(PASSES),
            CHUNK,
            differential_limits(),
        ),
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
    loop {
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
            FileDemand::Done => break,
            _ => {}
        }
        while let Some(event) = session.poll() {
            if let SessionEvent::NewProgram { tracks, .. } = event {
                announced.push(tracks);
            }
        }
    }
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
    while samples == 0 {
        match session.file_demand() {
            FileDemand::Bytes => {
                if offset >= bytes.len() {
                    if !eof_sent {
                        session
                            .feed(FileInput::Eof, Timestamp::ZERO)
                            .expect("end of input");
                        eof_sent = true;
                    }
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
    }
    assert_eq!(samples, 1, "the first sample is due immediately");
    let due = session.next_due().expect("a paced session has a deadline");
    assert!(due > Instant::now(), "the next sample is not due yet");
    assert!(
        !matches!(session.poll(), Some(SessionEvent::Sample { .. })),
        "polling before the deadline releases nothing"
    );
}
