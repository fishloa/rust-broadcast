//! A sequential media segment closed by `Mp4Input::SegmentEnd`: a damaged
//! segment must cost only itself.
//!
//! Every multi-segment group under the committed DASH, CMAF-HLS and SCTE-35
//! DASH fixture directories is walked. For each segment in turn the test
//! damages it three ways, feeds init and then every segment followed by
//! `SegmentEnd`, and compares each track with `Mp4Demux` over
//! `init ++ segment` — the per-segment behaviour a consumer relied on before
//! one long-lived stage. Undamaged segments must be identical, the damaged one
//! must match what `Mp4Demux` recovers from it, and the segment after it must
//! be complete.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use broadcast_common::stage::{Stage, Timestamp};
use test_harness::collect::Collected as Media;
use transmux::ir::SampleTiming;
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use transmux::{InputDegradation, TrackSpec};

/// Groups the walk must compare (a checkout with no usable files fails rather
/// than passing vacuously).
const MIN_SEGMENT_GROUPS: usize = 6;
/// Damage cases the walk must exercise.
const MIN_DAMAGE_CASES: usize = 20;
/// Largest chunk handed to one `feed`.
const FEED_CHUNK: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
struct SampleDump {
    data: Vec<u8>,
    timing: Option<SampleTiming>,
    sync: bool,
}

fn sample_dump(sample: &transmux::Sample) -> SampleDump {
    SampleDump {
        data: sample.data.to_vec(),
        timing: sample.timing,
        sync: sample.flags.is_sync,
    }
}

/// One track's expected spec and samples over a whole group.
struct Expected {
    spec: TrackSpec,
    samples: Vec<SampleDump>,
}

fn add_tracks(expected: &mut BTreeMap<u32, Expected>, media: &Media) {
    for track in &media.tracks {
        let id = track.spec.track_id.value();
        let entry = expected.entry(id).or_insert_with(|| Expected {
            spec: track.spec.clone(),
            samples: Vec::new(),
        });
        entry.samples.extend(track.samples.iter().map(sample_dump));
    }
}

/// A fresh stage over the init plus one segment: the per-segment result the
/// long-lived stage must reproduce.
fn fmp4_segment(init: &[u8], segment: &[u8]) -> Media {
    let mut demux = Mp4Demux::sequential(Mp4DemuxConfig::default());
    feed_bytes(&mut demux, init);
    demux
        .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
        .expect("init segment end");
    feed_bytes(&mut demux, segment);
    demux
        .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
        .expect("segment end");
    demux.finish().expect("finish");
    let mut events = Vec::new();
    while let Some(event) = demux.poll() {
        events.push(event);
    }
    test_harness::collect::media_from_events(events)
}

fn feed_bytes(demux: &mut Mp4Demux, bytes: &[u8]) {
    for chunk in bytes.chunks(FEED_CHUNK) {
        demux
            .feed(Mp4Input::Next(chunk), Timestamp::ZERO)
            .unwrap_or_else(|err| panic!("feed: {err:?}"));
    }
}

/// Feed the init once, then each segment followed by `SegmentEnd`.
fn drive(init: &[u8], segments: &[Vec<u8>]) -> Media {
    let mut demux = Mp4Demux::sequential(Mp4DemuxConfig::default());
    feed_bytes(&mut demux, init);
    demux
        .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
        .expect("init segment end");
    for segment in segments {
        feed_bytes(&mut demux, segment);
        demux
            .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
            .expect("segment end");
    }
    demux.finish().expect("finish");
    let mut events = Vec::new();
    while let Some(event) = demux.poll() {
        events.push(event);
    }
    test_harness::collect::media_from_events(events)
}

enum Damage {
    TruncateHalf,
    MoofSizePastSegment,
    TruncateInsideMdat,
}

impl Damage {
    const ALL: [Damage; 3] = [
        Damage::TruncateHalf,
        Damage::MoofSizePastSegment,
        Damage::TruncateInsideMdat,
    ];

    fn name(&self) -> &'static str {
        match self {
            Damage::TruncateHalf => "truncate-half",
            Damage::MoofSizePastSegment => "moof-size-past-segment",
            Damage::TruncateInsideMdat => "truncate-inside-mdat",
        }
    }
}

/// `(start, size, header size)` for each top-level box of `bytes`.
fn top_boxes(bytes: &[u8]) -> Vec<(usize, usize, usize, [u8; 4])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        let raw = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        let size = usize::try_from(raw).unwrap_or(usize::MAX);
        let fourcc: [u8; 4] = bytes[at + 4..at + 8].try_into().expect("four-CC");
        let (header, total) = match size {
            0 => (8usize, bytes.len() - at),
            1 => {
                if at + 16 > bytes.len() {
                    break;
                }
                let mut large = [0u8; 8];
                large.copy_from_slice(&bytes[at + 8..at + 16]);
                (
                    16usize,
                    usize::try_from(u64::from_be_bytes(large)).unwrap_or(usize::MAX),
                )
            }
            value => (8usize, value),
        };
        if total < header || total > bytes.len() - at {
            out.push((at, bytes.len() - at, header, fourcc));
            break;
        }
        out.push((at, total, header, fourcc));
        at += total;
    }
    out
}

fn box_of(bytes: &[u8], fourcc: &[u8; 4]) -> (usize, usize, usize) {
    for (start, size, header, ty) in top_boxes(bytes) {
        if ty == *fourcc {
            return (start, size, header);
        }
    }
    panic!("no {fourcc:?} box in the segment")
}

fn damage(segment: &[u8], damage: &Damage) -> Vec<u8> {
    match damage {
        Damage::TruncateHalf => segment[..segment.len() / 2].to_vec(),
        Damage::MoofSizePastSegment => {
            let mut damaged = segment.to_vec();
            let (start, _, _) = box_of(segment, b"moof");
            let beyond = u32::try_from(segment.len() + 1).expect("segment fits u32");
            damaged[start..start + 4].copy_from_slice(&beyond.to_be_bytes());
            damaged
        }
        Damage::TruncateInsideMdat => {
            let mut damaged = segment.to_vec();
            let (start, size, header) = box_of(segment, b"mdat");
            let payload = size - header;
            damaged.truncate(start + header + payload / 2);
            damaged
        }
    }
}

struct Group {
    name: String,
    init: Vec<u8>,
    segments: Vec<Vec<u8>>,
}

fn media_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "mp4" | "m4a" | "mov" | "m4s"))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if media_extension(&path) {
            out.push(path);
        }
    }
}

/// The marker an init's segments carry: the part before or after `init` in the
/// file stem (`p1_dash_track1_init` and `init-stream0` both work).
fn init_tag(stem: &str) -> Option<String> {
    let pos = stem.find("init")?;
    let prefix = stem[..pos].trim_end_matches(['-', '_']);
    let suffix = stem[pos + "init".len()..].trim_start_matches(['-', '_']);
    if !prefix.is_empty() {
        Some(prefix.to_string())
    } else if !suffix.is_empty() {
        Some(suffix.to_string())
    } else {
        None
    }
}

fn digits(name: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in name.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(current.parse().unwrap_or(0));
            current.clear();
        }
    }
    if !current.is_empty() {
        out.push(current.parse().unwrap_or(0));
    }
    out
}

fn belongs(segment_stem: &str, tag: &str) -> bool {
    if tag.chars().all(|ch| ch.is_ascii_digit()) {
        segment_stem.contains(&format!("-{tag}-"))
    } else {
        segment_stem.contains(tag)
    }
}

fn groups() -> Vec<Group> {
    let root = test_fixtures::root();
    let roots = [
        root.join("dash"),
        root.join("hls").join("cmaf-fmp4"),
        root.join("scte35").join("dash"),
    ];
    let mut groups = Vec::new();
    for dir_root in roots {
        let mut files = Vec::new();
        collect_files(&dir_root, &mut files);
        let mut dirs: Vec<PathBuf> = Vec::new();
        for file in &files {
            if let Some(parent) = file.parent()
                && !dirs.contains(&parent.to_path_buf())
            {
                dirs.push(parent.to_path_buf());
            }
        }
        dirs.sort();
        for dir in dirs {
            let mut here: Vec<PathBuf> = files
                .iter()
                .filter(|file| file.parent() == Some(dir.as_path()))
                .cloned()
                .collect();
            here.sort();
            let inits: Vec<&PathBuf> = here
                .iter()
                .filter(|path| {
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .is_some_and(|stem| stem.contains("init"))
                })
                .collect();
            if inits.is_empty() {
                continue;
            }
            for init in inits {
                let Some(init_stem) = init.file_stem().and_then(|stem| stem.to_str()) else {
                    continue;
                };
                let tag = init_tag(init_stem);
                let mut pieces: Vec<(Vec<u64>, PathBuf)> = Vec::new();
                for file in &here {
                    if file == init {
                        continue;
                    }
                    let Some(stem) = file.file_stem().and_then(|stem| stem.to_str()) else {
                        continue;
                    };
                    if stem.contains("init") {
                        continue;
                    }
                    let matches = match &tag {
                        Some(tag) => belongs(stem, tag),
                        None => true,
                    };
                    if matches {
                        pieces.push((digits(stem), file.clone()));
                    }
                }
                if pieces.len() < 2 {
                    continue;
                }
                pieces.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
                let Ok(init_bytes) = std::fs::read(init) else {
                    continue;
                };
                let mut segments = Vec::new();
                for (_, path) in pieces {
                    if let Ok(bytes) = std::fs::read(&path) {
                        segments.push(bytes);
                    }
                }
                if segments.len() >= 2 {
                    groups.push(Group {
                        name: init.display().to_string(),
                        init: init_bytes,
                        segments,
                    });
                }
            }
        }
    }
    groups
}

#[test]
fn a_damaged_segment_costs_only_itself() {
    let groups = groups();
    assert!(
        groups.len() >= MIN_SEGMENT_GROUPS,
        "only {} multi-segment groups found",
        groups.len()
    );
    let mut damage_cases = 0usize;
    for group in &groups {
        for victim in 0..group.segments.len() {
            for harm in Damage::ALL {
                damage_cases += 1;
                let mut segments = group.segments.clone();
                segments[victim] = damage(&group.segments[victim], &harm);

                let mut expected: BTreeMap<u32, Expected> = BTreeMap::new();
                let mut damaged_reference = None;
                for (index, segment) in segments.iter().enumerate() {
                    let media = fmp4_segment(&group.init, segment);
                    if index == victim {
                        damaged_reference = Some(media.degraded.clone());
                    }
                    add_tracks(&mut expected, &media);
                }

                let stage = drive(&group.init, &segments);
                assert_eq!(
                    stage.tracks.len(),
                    expected.len(),
                    "{} segment {victim} {}: track count",
                    group.name,
                    harm.name()
                );
                for track in &stage.tracks {
                    let id = track.spec.track_id.value();
                    let Some(want) = expected.get(&id) else {
                        panic!(
                            "{} segment {victim} {}: unexpected track {id}",
                            group.name,
                            harm.name()
                        );
                    };
                    assert_eq!(
                        format!("{:?}", track.spec),
                        format!("{:?}", want.spec),
                        "{} segment {victim} {}: track {id} spec",
                        group.name,
                        harm.name()
                    );
                    let got: Vec<SampleDump> = track.samples.iter().map(sample_dump).collect();
                    assert_eq!(
                        got,
                        want.samples,
                        "{} segment {victim} {}: track {id} samples (damaged segment and every \
                         later one)",
                        group.name,
                        harm.name()
                    );
                }
                for kind in &stage.degraded {
                    assert!(
                        matches!(kind, InputDegradation::MalformedUnit { .. }),
                        "{} segment {victim} {}: a damaged unit is MalformedUnit, got {kind:?}",
                        group.name,
                        harm.name()
                    );
                }
                if damaged_reference
                    .as_ref()
                    .is_some_and(|degraded| !degraded.is_empty())
                {
                    assert!(
                        !stage.degraded.is_empty(),
                        "{} segment {victim} {}: Mp4Demux reported a degraded unit the stage \
                         did not",
                        group.name,
                        harm.name()
                    );
                }
            }
        }
    }
    assert!(
        damage_cases >= MIN_DAMAGE_CASES,
        "only {damage_cases} damage cases exercised"
    );
    eprintln!(
        "mp4 segment-end equivalence: {} groups, {damage_cases} damage cases",
        groups.len()
    );
}

#[test]
fn feeding_segment_end_to_a_seekable_stage_is_a_typed_error() {
    let mut demux = Mp4Demux::seekable(Mp4DemuxConfig::default());
    let err = demux
        .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
        .expect_err("SegmentEnd to a seekable stage");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "got {err:?}"
    );
}
