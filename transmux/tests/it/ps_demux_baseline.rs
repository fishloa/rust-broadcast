//! Streaming-first Phase 5 slice 1 gate: the streaming `StreamingPsDemux`
//! reproduces the whole-buffer demuxer's output byte for byte.
//!
//! `fixtures/ps/demux-baseline.txt` was recorded from `origin/main`'s
//! `PsDemux` (see `fixtures/ps/PROVENANCE.md`). Every public Program Stream
//! fixture is demuxed here at three feed granularities — 1 byte, 188 bytes and
//! 64 KiB — and its per-track sample count, first/last timestamps, sync count,
//! coded-byte hash and timing hash must equal the recorded lines exactly.

use test_harness::collect::Collected as Media;

/// The FNV-1a 64 offset basis.
const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
/// The FNV-1a 64 prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// One golden file section.
struct Section {
    file: String,
    movie_timescale: u32,
    tracks: Vec<String>,
}

fn parse_golden() -> Vec<Section> {
    let text = test_fixtures::read_to_string("ps/demux-baseline.txt");
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(file) = line.strip_prefix("file=") {
            sections.push(Section {
                file: file.to_string(),
                movie_timescale: 0,
                tracks: Vec::new(),
            });
            continue;
        }
        let Some(section) = sections.last_mut() else {
            panic!("golden line before a file section: {line}");
        };
        if let Some(timescale) = line.strip_prefix("movie_timescale=") {
            section.movie_timescale = timescale.parse().expect("movie timescale");
        } else if line.starts_with("track=") {
            section.tracks.push(line.to_string());
        } else {
            panic!("unknown golden line: {line}");
        }
    }
    assert!(!sections.is_empty(), "the golden must hold a section");
    sections
}

fn optional(value: Option<i64>) -> String {
    value.map_or_else(|| "none".to_string(), |value| value.to_string())
}

fn summarize(media: &Media) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, track) in media.tracks.iter().enumerate() {
        let first = track.samples.first();
        let last = track.samples.last();
        let mut bytes_hash = FNV_OFFSET;
        let mut timing_hash = FNV_OFFSET;
        let mut syncs = 0u32;
        for sample in &track.samples {
            bytes_hash = fnv1a(&sample.data, bytes_hash);
            let (dts, pts, duration) = match sample.timing {
                Some(timing) => (timing.dts(), timing.pts(), timing.duration()),
                None => (None, None, None),
            };
            timing_hash = fnv1a(&dts.unwrap_or(i64::MIN).to_le_bytes(), timing_hash);
            timing_hash = fnv1a(&pts.unwrap_or(i64::MIN).to_le_bytes(), timing_hash);
            timing_hash = fnv1a(&duration.unwrap_or(u32::MAX).to_le_bytes(), timing_hash);
            if sample.flags.is_sync {
                syncs += 1;
            }
        }
        let dts = |sample: Option<&transmux::Sample>, pts: bool| {
            optional(
                sample
                    .and_then(|sample| sample.timing)
                    .and_then(|timing| if pts { timing.pts() } else { timing.dts() }),
            )
        };
        lines.push(format!(
            "track={index} codec={} timescale={} samples={} first_dts={} first_pts={} last_dts={} last_pts={} syncs={} bytes_fnv={bytes_hash:016x} timing_fnv={timing_hash:016x}",
            track.spec.config.name().replace(' ', "-"),
            track.spec.timescale.get(),
            track.samples.len(),
            dts(first, false),
            dts(first, true),
            dts(last, false),
            dts(last, true),
            syncs,
        ));
    }
    lines
}

#[test]
fn every_ps_fixture_reproduces_the_whole_buffer_baseline() {
    let sections = parse_golden();
    assert!(
        sections.len() >= 7,
        "the golden must cover every PS fixture"
    );
    for section in &sections {
        let bytes = test_fixtures::read(&format!("ps/{}", section.file));
        for chunk in [1usize, 188, 64 * 1024] {
            let media = test_harness::collect::media_from_ps(&bytes, chunk)
                .unwrap_or_else(|error| panic!("{} at chunk {chunk}: {error}", section.file));
            assert_eq!(
                media.movie_timescale.get(),
                section.movie_timescale,
                "{} at chunk {chunk}: movie timescale",
                section.file
            );
            assert_eq!(
                summarize(&media),
                section.tracks,
                "{} at chunk {chunk}: per-track samples differ from the baseline",
                section.file
            );
        }
    }
}
