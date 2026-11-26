//! Differential: the streaming `Mp4Demux`'s two input paths, on every fixture
//! they accept (the seekable range path against the forward-only one).
//!
//! Both modes are compared with a canonical per-track dump: the track spec,
//! then per sample its byte length, a 64-bit FNV of its bytes, decode and
//! presentation time, duration, sync flag and display tag. Skipped tracks and
//! the degraded list are compared too.
//!
//! Seekable mode runs over every fixture. Sequential mode runs over every
//! fragmented fixture and every `moov`-first progressive one, fed in chunks of
//! 1, 7 and 64 KiB. A final test checks that an init segment plus N media
//! segments, fed as one sequential input, equals demuxing `init ++ segment`
//! afresh for each segment.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use test_harness::collect::Collected as Media;
use test_harness::collect::{CollectMode, media_from_mp4};
use transmux::{InputDegradation, TrackId};

/// Fixtures the walk must compare (a checkout with no usable files fails
/// rather than passing vacuously).
const MIN_DIFFERENTIAL_FIXTURES: usize = 40;
/// Tracks the walk must compare.
const MIN_DIFFERENTIAL_TRACKS: usize = 80;
/// Samples the walk must compare.
const MIN_DIFFERENTIAL_SAMPLES: usize = 10_000;
/// Media files the init-plus-segments equivalence must compare.
const MIN_SEGMENT_GROUPS: usize = 2;

/// 64-bit FNV-1a of a sample's coded bytes.
fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn media_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "mp4" | "m4a" | "mov"))
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

/// Every media fixture under `fixtures/mp4` and `private/fixtures/mp4`.
fn fixtures() -> Vec<(String, PathBuf)> {
    let mut roots = vec![("public", test_fixtures::root())];
    if let Some(private) = test_fixtures::private("mp4") {
        roots.push(("private", private));
    }
    let mut out = Vec::new();
    for (tier, root) in roots {
        let mut files = Vec::new();
        collect_files(&root, &mut files);
        files.sort();
        for file in files {
            if let Ok(rel) = file.strip_prefix(&root) {
                out.push((format!("{tier}/{}", rel.display()), file));
            }
        }
    }
    out
}

/// Whether the file's top-level boxes carry a `moof`.
fn fragmented(bytes: &[u8]) -> bool {
    isobmff::BoxIter::new(bytes)
        .flatten()
        .any(|b| b.four_cc() == isobmff::segments::MOOF)
}

/// Whether the file's `moov` precedes its `mdat` (the only progressive layout a
/// forward-only source can read).
fn moov_first(bytes: &[u8]) -> bool {
    let mut moov = None;
    let mut mdat = None;
    for (index, item) in isobmff::BoxIter::new(bytes).flatten().enumerate() {
        if item.four_cc() == isobmff::init_segment::MOOV && moov.is_none() {
            moov = Some(index);
        }
        if item.four_cc() == isobmff::segments::MDAT && mdat.is_none() {
            mdat = Some(index);
        }
    }
    matches!((moov, mdat), (Some(m), Some(d)) if m < d)
}

/// The reference view of a file: sequential where the forward-only reader can
/// follow it, seekable otherwise. Each test compares the other path with it.
fn reference_media(bytes: &[u8]) -> Option<Media> {
    let mode = if fragmented(bytes) || moov_first(bytes) {
        CollectMode::Sequential { chunk: 64 * 1024 }
    } else {
        CollectMode::Seekable
    };
    media_from_mp4(bytes, mode).ok()
}

#[derive(Debug, PartialEq, Eq)]
struct SampleDump {
    len: usize,
    fnv: u64,
    dts: Option<i64>,
    pts: Option<i64>,
    duration: Option<u32>,
    sync: bool,
    display: String,
}

#[derive(Debug, PartialEq, Eq)]
struct TrackDump {
    id: u32,
    spec: String,
    start_decode_time: u64,
    samples: Vec<SampleDump>,
}

#[derive(Debug, PartialEq, Eq)]
struct Dump {
    movie_timescale: u32,
    tracks: Vec<TrackDump>,
    skipped: Vec<(String, String, Option<u32>)>,
    degraded: Vec<InputDegradation>,
}

fn track_dump(track: &test_harness::collect::CollectedTrack) -> TrackDump {
    let samples = track
        .samples
        .iter()
        .map(|sample| {
            let timing = sample.timing;
            SampleDump {
                len: sample.data.len(),
                fnv: fnv(&sample.data),
                dts: timing.and_then(|t| t.dts()),
                pts: timing.and_then(|t| t.pts()),
                duration: timing.and_then(|t| t.duration()),
                sync: sample.flags.is_sync,
                display: format!("{:?}", sample.display),
            }
        })
        .collect();
    TrackDump {
        id: track.spec.track_id.value(),
        spec: format!("{:?}", track.spec),
        start_decode_time: track.start_decode_time,
        samples,
    }
}

fn dump(media: &Media) -> Dump {
    let mut tracks: Vec<TrackDump> = media.tracks.iter().map(track_dump).collect();
    tracks.sort_by_key(|track| track.id);
    let mut skipped: Vec<(String, String, Option<u32>)> = media
        .skipped
        .iter()
        .map(|skip| {
            (
                skip.fourcc.clone(),
                skip.reason.clone(),
                skip.track_id.map(TrackId::value),
            )
        })
        .collect();
    skipped.sort();
    Dump {
        movie_timescale: media.movie_timescale.get(),
        tracks,
        skipped,
        degraded: comparable_degradations(media, &[]),
    }
}

/// The degradations both demuxers can model: a fault on a top-level `moof` or
/// `mdat`. The reader's permissive mode also reports `moov`-structure faults
/// (a QuickTime `meta` box clamped to its parent, an unparseable sample
/// entry) that the whole-input demuxers tolerate without a record; those are
/// not media-unit damage and have no counterpart to compare against.
fn comparable_degradations(media: &Media, bytes: &[u8]) -> Vec<InputDegradation> {
    let units: Vec<u64> = if bytes.is_empty() {
        Vec::new()
    } else {
        isobmff::BoxIter::new(bytes)
            .flatten()
            .filter(|item| {
                item.four_cc() == isobmff::segments::MOOF
                    || item.four_cc() == isobmff::segments::MDAT
            })
            .map(|item| item.start as u64)
            .collect()
    };
    media
        .degraded
        .iter()
        .filter(|kind| match kind {
            InputDegradation::MalformedUnit { position }
            | InputDegradation::TruncatedUnit { position } => units.contains(position),
            _ => true,
        })
        .cloned()
        .collect()
}

fn compare(key: &str, mode: &str, old: &Media, new: &Media, bytes: &[u8]) {
    let mut a = dump(old);
    let mut b = dump(new);
    a.degraded = comparable_degradations(old, bytes);
    b.degraded = comparable_degradations(new, bytes);
    if a == b {
        return;
    }
    eprintln!(
        "{key} [{mode}] MISMATCH movie {} vs {}; ids old {:?} new {:?}; skipped old {:?} new {:?}; degraded old {:?} new {:?}",
        a.movie_timescale,
        b.movie_timescale,
        a.tracks.iter().map(|t| t.id).collect::<Vec<_>>(),
        b.tracks.iter().map(|t| t.id).collect::<Vec<_>>(),
        a.skipped,
        b.skipped,
        a.degraded,
        b.degraded
    );
    for track in &a.tracks {
        let Some(other) = b.tracks.iter().find(|t| t.id == track.id) else {
            eprintln!("{key} [{mode}]: track {} missing on the new side", track.id);
            continue;
        };
        if track.samples.len() != other.samples.len() {
            eprintln!(
                "{key} [{mode}]: track {} sample count {} vs {}",
                track.id,
                track.samples.len(),
                other.samples.len()
            );
        }
        for index in 0..track.samples.len().min(other.samples.len()) {
            if track.samples[index] != other.samples[index] {
                eprintln!(
                    "{key} [{mode}]: track {} first differing sample {index}:\n  old {:?}\n  new {:?}",
                    track.id, track.samples[index], other.samples[index]
                );
                break;
            }
        }
        if track.spec != other.spec {
            eprintln!(
                "{key} [{mode}]: track {} spec differs:\n  old {}\n  new {}",
                track.id, track.spec, other.spec
            );
        }
        if track.start_decode_time != other.start_decode_time {
            eprintln!(
                "{key} [{mode}]: track {} start_decode_time {} vs {}",
                track.id, track.start_decode_time, other.start_decode_time
            );
        }
    }
    assert_eq!(a, b, "{key} [{mode}]: canonical dump differs");
}

#[test]
fn seekable_mode_matches_the_reference_path() {
    let mut compared_fixtures = 0usize;
    let mut compared_tracks = 0usize;
    let mut compared_samples = 0usize;
    for (key, path) in fixtures() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Some(old) = reference_media(&bytes) else {
            continue;
        };
        let new = match media_from_mp4(&bytes, CollectMode::Seekable) {
            Ok(media) => media,
            Err(err) => panic!("{key}: seekable demux failed: {err:?}"),
        };
        compare(&key, "seekable", &old, &new, &bytes);
        compared_fixtures += 1;
        compared_tracks += old.tracks.len();
        compared_samples += old.tracks.iter().map(|t| t.samples.len()).sum::<usize>();
    }
    assert!(
        compared_fixtures >= MIN_DIFFERENTIAL_FIXTURES,
        "only {compared_fixtures} fixtures compared"
    );
    assert!(
        compared_tracks >= MIN_DIFFERENTIAL_TRACKS,
        "only {compared_tracks} tracks compared"
    );
    assert!(
        compared_samples >= MIN_DIFFERENTIAL_SAMPLES,
        "only {compared_samples} samples compared"
    );
    eprintln!(
        "mp4 differential: {compared_fixtures} fixtures, {compared_tracks} tracks, \
         {compared_samples} samples"
    );
}

#[test]
fn sequential_mode_matches_the_reference_path_at_every_chunk_size() {
    let mut compared_fixtures = 0usize;
    for (key, path) in fixtures() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if !fragmented(&bytes) && !moov_first(&bytes) {
            continue;
        }
        let Some(old) = reference_media(&bytes) else {
            continue;
        };
        for chunk in [1usize, 7, 65_536] {
            let new = match media_from_mp4(&bytes, CollectMode::Sequential { chunk }) {
                Ok(media) => media,
                Err(err) => panic!("{key}: sequential ({chunk}) demux failed: {err:?}"),
            };
            compare(&key, &format!("sequential/{chunk}"), &old, &new, &bytes);
        }
        compared_fixtures += 1;
    }
    assert!(
        compared_fixtures >= MIN_DIFFERENTIAL_FIXTURES / 2,
        "only {compared_fixtures} fixtures compared sequentially"
    );
    eprintln!("mp4 sequential differential: {compared_fixtures} fixtures");
}

/// `(name, init bytes, media segment bytes)` groups under the committed DASH
/// and CMAF-HLS fixture directories.
fn segment_groups() -> Vec<(String, Vec<u8>, Vec<Vec<u8>>)> {
    let roots = [
        test_fixtures::root().join("dash"),
        test_fixtures::root().join("hls/cmaf-fmp4"),
    ];
    let segment_extension = |path: &Path| {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "mp4" | "m4a" | "mov" | "m4s"))
    };
    let mut files = Vec::new();
    for root in roots {
        let mut found = Vec::new();
        collect_files(&root, &mut found);
        files.extend(found.into_iter().filter(|path| segment_extension(path)));
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for file in &files {
        if let Some(parent) = file.parent()
            && !dirs.contains(&parent.to_path_buf())
        {
            dirs.push(parent.to_path_buf());
        }
    }
    dirs.sort();
    let mut groups = Vec::new();
    for dir in dirs {
        let mut here: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| segment_extension(path))
                    .collect()
            })
            .unwrap_or_default();
        here.sort();
        let inits: Vec<&PathBuf> = here
            .iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains("init"))
            })
            .collect();
        for init in inits {
            let Some(init_name) = init.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let tag = init_name
                .split("init")
                .nth(1)
                .unwrap_or("")
                .trim_start_matches('-')
                .trim_end_matches(".m4s")
                .trim_end_matches(".mp4")
                .to_string();
            let Ok(init_bytes) = std::fs::read(init) else {
                continue;
            };
            let mut pieces = Vec::new();
            for file in &here {
                if file == init {
                    continue;
                }
                let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if name.contains("init") {
                    continue;
                }
                if !tag.is_empty() && !name.contains(&tag) {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(file) {
                    pieces.push(bytes);
                }
            }
            if !pieces.is_empty() {
                groups.push((format!("{}", init.display()), init_bytes, pieces));
            }
        }
    }
    groups
}

#[test]
fn init_plus_segments_equals_one_demux_per_segment() {
    let mut compared_groups = 0usize;
    for (name, init, segments) in segment_groups() {
        let mut whole = init.clone();
        for segment in &segments {
            whole.extend_from_slice(segment);
        }
        let combined = match media_from_mp4(&whole, CollectMode::Sequential { chunk: 4096 }) {
            Ok(media) => media,
            Err(err) => panic!("{name}: combined sequential demux failed: {err:?}"),
        };
        let combined = dump(&combined);
        let mut per_track: BTreeMap<u32, (String, Vec<SampleDump>)> = BTreeMap::new();
        for segment in &segments {
            let mut input = init.clone();
            input.extend_from_slice(segment);
            let media = match media_from_mp4(&input, CollectMode::Sequential { chunk: 4096 }) {
                Ok(media) => media,
                Err(err) => panic!("{name}: per-segment sequential demux failed: {err:?}"),
            };
            for track in &media.tracks {
                let dumped = track_dump(track);
                let entry = per_track
                    .entry(dumped.id)
                    .or_insert((dumped.spec, Vec::new()));
                entry.1.extend(dumped.samples);
            }
        }
        assert_eq!(
            per_track.len(),
            combined.tracks.len(),
            "{name}: track count"
        );
        for track in &combined.tracks {
            let Some((spec, samples)) = per_track.get(&track.id) else {
                panic!(
                    "{name}: combined demux has track {} the per-segment demux lacks",
                    track.id
                );
            };
            assert_eq!(spec, &track.spec, "{name}: track {} spec", track.id);
            assert_eq!(
                samples.len(),
                track.samples.len(),
                "{name}: track {} sample count",
                track.id
            );
            for (index, (a, b)) in samples.iter().zip(&track.samples).enumerate() {
                assert_eq!(
                    a, b,
                    "{name}: track {} first differing sample {index}",
                    track.id
                );
            }
        }
        compared_groups += 1;
    }
    assert!(
        compared_groups >= MIN_SEGMENT_GROUPS,
        "only {compared_groups} init-plus-segments groups compared"
    );
    eprintln!("mp4 init-plus-segments equivalence: {compared_groups} groups");
}
