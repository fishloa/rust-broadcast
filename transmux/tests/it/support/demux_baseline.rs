//! Shared parser and summariser for the per-container whole-input demux
//! baselines (`fixtures/{ts,flv,rtmp}/demux-baseline.txt`).
//!
//! Each baseline was recorded from `origin/main`'s whole-input demuxer before
//! Phase 6 slice 2 deleted it; the streaming stage must reproduce every line at
//! three feed granularities. The per-track line pins the codec, timescale,
//! sample count, first/last dts and pts, sync count, an FNV-1a 64 over the
//! concatenated coded sample bytes, an FNV-1a 64 over the per-sample
//! `(dts, pts, duration)` sequence, and the track's `start_decode_time`; the
//! summary line pins the movie timescale and the PCR, degraded and skipped
//! reports.

use transmux::Media;

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
pub struct Section {
    pub file: String,
    /// Byte offset the recorded demuxer input starts at (RTMP handshake
    /// prefix; `0` for every other container).
    pub skip: usize,
    pub tracks: Vec<String>,
    pub summary: String,
    pub error: Option<String>,
}

/// Parse a baseline text into its sections.
pub fn parse(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(file) = line.strip_prefix("file=") {
            sections.push(Section {
                file: file.to_string(),
                skip: 0,
                tracks: Vec::new(),
                summary: String::new(),
                error: None,
            });
            continue;
        }
        let Some(section) = sections.last_mut() else {
            panic!("golden line before a file section: {line}");
        };
        if let Some(skip) = line.strip_prefix("skip=") {
            section.skip = skip.parse().expect("skip offset");
        } else if let Some(error) = line.strip_prefix("error=") {
            section.error = Some(error.to_string());
        } else if line.starts_with("movie_timescale=") {
            section.summary = line.to_string();
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

/// The per-track lines plus the summary line, in the recorded order and shape.
pub fn summarize(media: &Media) -> (Vec<String>, String) {
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
        let stamp = |sample: Option<&transmux::Sample>, pts: bool| {
            optional(
                sample
                    .and_then(|sample| sample.timing)
                    .and_then(|timing| if pts { timing.pts() } else { timing.dts() }),
            )
        };
        lines.push(format!(
            "track={index} codec={} timescale={} samples={} first_dts={} first_pts={} last_dts={} last_pts={} syncs={} bytes_fnv={bytes_hash:016x} timing_fnv={timing_hash:016x} start_decode_time={}",
            track.spec.config.name().replace(' ', "-"),
            track.spec.timescale.get(),
            track.samples.len(),
            stamp(first, false),
            stamp(first, true),
            stamp(last, false),
            stamp(last, true),
            syncs,
            track.start_decode_time,
        ));
    }
    let degraded: Vec<String> = media.degraded.iter().map(ToString::to_string).collect();
    let skipped: Vec<String> = media
        .skipped
        .iter()
        .map(|skipped| format!("{}:{}", skipped.fourcc, skipped.reason))
        .collect();
    let summary = format!(
        "movie_timescale={} pcr={} degraded={} skipped={}",
        media.movie_timescale,
        media.pcr.len(),
        degraded.join(","),
        skipped.join(","),
    );
    (lines, summary)
}
