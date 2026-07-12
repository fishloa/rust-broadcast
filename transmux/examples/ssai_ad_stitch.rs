//! SSAI (server-side ad insertion) ad-stitcher walkthrough — issue #664.
//!
//! Wires four already-shipped pieces of this workspace together end to end:
//!
//! 1. **`scte35-splice`** parses a SCTE-35 `splice_insert` cue reassembled from
//!    a real MPEG-2 TS PID (the same PID-0x01F0 / `SectionReassembler`
//!    extraction pattern `media-doctor`'s `Scte35Check` and `ts-fix`'s
//!    `scte35_preserve` tests already use).
//! 2. **`timed-metadata`**'s [`Timeline`] converts that cue to a
//!    [`TimedEvent`](timed_metadata::TimedEvent), then on to an HLS
//!    [`DateRange`](timed_metadata::DateRange) tag and a DASH `emsg` box —
//!    the crate's whole reason to exist, reused verbatim rather than
//!    reimplemented.
//! 3. **`transmux::splice_insert`** performs the actual sample-level ad
//!    splice on the demuxed [`Media`] IR, returning a
//!    [`SpliceResult`](transmux::SpliceResult) whose
//!    [`SplicePoint`](transmux::SplicePoint)s mark the ad-in and resume cuts.
//! 4. **`transmux`'s HLS/DASH packaging** (`hls::MediaPlaylist`,
//!    `DashPackager`, `build_media_segment_with_events`) renders the spliced
//!    timeline as a real `#EXT-X-DISCONTINUITY` + `#EXT-X-DATERANGE` HLS media
//!    playlist and a real DASH `.mpd` carrying an inband `emsg` at the splice
//!    point.
//!
//! # Fixture choice (documented per the issue's requirement)
//!
//! The base is `fixtures/ts/h264_aac.ts` — a real H.264/AVC + AAC MPEG-2 TS
//! capture (25 fps, 320x240, 3 real keyframes) already used by
//! [`transmux_hub`](../transmux_hub.rs). No fixture in this workspace carries
//! a genuine embedded SCTE-35 cue *and* real audio/video content on a matching
//! PID/timescale profile (the dedicated `fixtures/ts/scte35-*.ts` fixtures are
//! bare section vectors with no AV payload). Per the issue's option (b), this
//! example therefore:
//!
//! - Demuxes the **whole** real capture once (`source`).
//! - Splits it, at its own real keyframe boundaries, into a `base` clip (the
//!   first two GOPs) and a stand-in `ad` clip (the third GOP) — **real,
//!   distinct encoded bytes from the same real capture**, not fabricated
//!   samples. (The workspace has no second h264+AAC fixture at this exact
//!   90 kHz/44.1 kHz profile to serve as genuinely separate ad creative; see
//!   `docs/CRATE-ACCEPTANCE.md`'s "no implementation without a verified
//!   primary source" discipline — this reuses only real bytes, never invents
//!   coded data.)
//! - Hand-builds a spec-correct `splice_insert` SCTE-35 section (via
//!   `scte35-splice`'s own typed builders — the same path its `round_trip_*`
//!   unit tests use) whose `pts_time` is picked to land exactly on the
//!   capture's own second real keyframe, and appends it as a genuine TS
//!   packet on PID 0x01F0 to a *copy* of the real captured bytes.
//! - Extracts that cue back out with the project's standard
//!   `SectionReassembler` pattern, so the splice point genuinely comes from
//!   parsing the stream rather than being threaded through in memory.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p transmux --example ssai_ad_stitch
//! ```
//!
//! and inspect the printed summary plus the manifest/segment files written to
//! the temp directory it reports.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use broadcast_common::{Serialize, Unpackage};
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::{AnyCommand, SpliceInsert};
use scte35_splice::time::{BreakDuration, SpliceTime};
use timed_metadata::convert::{EmsgConfig, SCTE35_SCHEME};
use timed_metadata::{DateRange, TimeAnchor, Timeline};
use transmux::{
    Addressing, CodecConfig, DashPackager, EmsgBox, FragmentTrackData, InbandEventStream, Media,
    MediaPlaylist, MediaSegment, PresentationTime, Sample, SplicePoint, Track, TrackSegments,
    TsDemux, build_init_segment, build_media_segment, build_media_segment_with_events,
    splice_insert,
};

/// This example's base fixture is H.264/AVC video; used to tell the video
/// track from the audio track without depending on transmux's internal
/// (crate-private) `CodecConfig::is_video`.
fn is_avc(config: &CodecConfig) -> bool {
    matches!(config, CodecConfig::Avc { .. })
}

/// SCTE-35 `splice_info_section` PID — the de-facto convention this workspace
/// already uses (`media-doctor::Scte35Check`, `ts-fix`'s `scte35_preserve`
/// tests, ANSI/SCTE 35 §11 example carriage).
const SCTE35_PID: u16 = 0x01F0;
/// Demo `splice_event_id` (ANSI/SCTE 35 §9.9.3 — any unique 32-bit value).
const SPLICE_EVENT_ID: u32 = 100_002;
/// Arbitrary demo wall-clock anchor: 2024-01-15T12:00:00Z, matching the
/// convention already used by `timed-metadata`'s own `scte35_to_hls`/
/// `scte35_to_dash` examples.
const DEMO_EPOCH_MS: i64 = 1_705_320_000_000;
/// `emsg` `value` — SCTE-35 `segmentation_type_id` 0x22, "Provider
/// Advertisement Start" (informational text; this demo's `splice_insert`
/// carries no segmentation descriptor of its own). Matches the convention
/// already used by `timed-metadata/examples/scte35_to_dash.rs`.
const EMSG_VALUE_PROVIDER_AD_START: &str = "34";

/// The full set of rendered outputs, returned so both `main` (which writes
/// them to disk and prints a summary) and the integration test
/// (`tests/ssai_ad_stitch.rs`, which `#[path]`-includes this file as a
/// module) can inspect the exact same run.
pub struct Demo {
    /// The video-rendition HLS media playlist text.
    pub m3u8: String,
    /// The DASH MPD text.
    pub mpd: String,
    /// The serialized DASH `emsg` box embedded in the ad segment.
    pub emsg_bytes: Vec<u8>,
    /// The verbatim `splice_info_section` bytes extracted from the TS.
    pub raw_cue: Vec<u8>,
    /// The `EXT-X-DATERANGE` built from the cue.
    pub daterange: DateRange,
    /// The two splice points (`ad-in`, `resume`) transmux reported.
    pub discontinuity_points: Vec<SplicePoint>,
    /// Where the manifest + segment files were written.
    pub out_dir: PathBuf,
}

/// Locate a fixture under the workspace `fixtures/` directory (a sibling of
/// the crate's `CARGO_MANIFEST_DIR`).
fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(rel)
}

/// Sum of `samples[..upto]`'s durations, in the track's own timescale ticks.
fn cumulative(samples: &[Sample], upto: usize) -> u64 {
    samples[..upto].iter().map(|s| s.duration as u64).sum()
}

/// Sum of `samples[lo..hi]`'s durations.
fn range_duration(samples: &[Sample], lo: usize, hi: usize) -> u64 {
    samples[lo..hi].iter().map(|s| s.duration as u64).sum()
}

/// First sample index whose cumulative duration (from the track start) is at
/// or beyond `offset_ticks`. Mirrors `transmux::splice`'s own (private)
/// `sample_index_at_offset` — used here to align the audio cut to the same
/// wall-clock offset as a video-track keyframe boundary.
fn index_at_offset(samples: &[Sample], offset_ticks: u64) -> usize {
    let mut acc = 0u64;
    for (i, s) in samples.iter().enumerate() {
        if acc >= offset_ticks {
            return i;
        }
        acc += s.duration as u64;
    }
    samples.len()
}

/// Build one 188-byte TS packet carrying `section` on `pid` (PUSI=1,
/// `pointer_field = 0`, adaptation_field_control = payload-only), padded with
/// `0xFF` stuffing bytes — ISO/IEC 13818-1 §2.4.4.2. Errors if `section` does
/// not fit in one packet's payload (184 bytes minus the pointer_field byte).
fn scte35_ts_packet(section: &[u8], pid: u16) -> Vec<u8> {
    let mut pkt = vec![0xFFu8; TS_PACKET_SIZE];
    pkt[0] = 0x47; // sync_byte
    pkt[1] = 0x40 | (((pid >> 8) as u8) & 0x1F); // PUSI=1, PID[12:8]
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10; // adaptation_field_control = 01 (payload only), continuity_counter = 0
    pkt[4] = 0x00; // pointer_field: section starts immediately after
    let end = 5 + section.len();
    assert!(
        end <= TS_PACKET_SIZE,
        "splice_info_section ({} bytes) does not fit in one TS packet payload",
        section.len()
    );
    pkt[5..end].copy_from_slice(section);
    pkt
}

/// Extract the first `splice_info_section` (table_id 0xFC) carried on `pid`,
/// via the same TS-packet walk + `SectionReassembler` pattern already used by
/// `media-doctor::Scte35Check` and `ts-fix`'s `scte35_preserve` tests.
fn extract_scte35_raw(ts: &[u8], pid: u16) -> Vec<u8> {
    let mut reassembler = SectionReassembler::default();
    for raw in ts.chunks_exact(TS_PACKET_SIZE) {
        let Ok(pkt) = TsPacket::parse(raw) else {
            continue;
        };
        if pkt.header.pid != pid {
            continue;
        }
        let Some(payload) = pkt.payload else {
            continue;
        };
        reassembler.feed(payload, pkt.header.pusi);
        while let Some(section) = reassembler.pop_section() {
            if section.first() == Some(&scte35_splice::section::TABLE_ID) {
                return section.to_vec();
            }
        }
    }
    panic!("no splice_info_section found on PID {pid:#06x}");
}

/// Run the full pipeline: fixture -> hand-built cue -> extract -> splice ->
/// HLS + DASH. Writes every manifest/segment file to a temp directory and
/// returns the rendered outputs for inspection.
pub fn run() -> Result<Demo, Box<dyn Error>> {
    // ------------------------------------------------------------------
    // 1. Read the real base fixture and demux it once.
    // ------------------------------------------------------------------
    let base_ts_bytes = fs::read(fixture_path("ts/h264_aac.ts"))?;
    let source: Media = TsDemux::new().unpackage(&base_ts_bytes[..])?;

    let video_idx = source
        .tracks
        .iter()
        .position(|t| is_avc(&t.spec.config))
        .expect("h264_aac.ts must have an H.264/AVC video track");
    let audio_idx = source
        .tracks
        .iter()
        .position(|t| !is_avc(&t.spec.config))
        .expect("h264_aac.ts must have an audio track");
    let video = &source.tracks[video_idx];
    let audio = &source.tracks[audio_idx];

    // ------------------------------------------------------------------
    // 2. Split the real capture, at its own real keyframes, into a `base`
    //    clip and a stand-in `ad` clip (see module docs for why).
    // ------------------------------------------------------------------
    let keyframe_indices: Vec<usize> = video
        .samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_sync)
        .map(|(i, _)| i)
        .collect();
    assert!(
        keyframe_indices.len() >= 3,
        "fixture must carry >= 3 real keyframes for a meaningful splice demo, found {}",
        keyframe_indices.len()
    );
    // Splice point: the capture's own 2nd keyframe (inside the `base` clip).
    let splice_sample_index = keyframe_indices[1];
    // Base/ad content boundary: the capture's own 3rd keyframe.
    let content_split = keyframe_indices[2];

    let splice_dts = video.start_decode_time + cumulative(&video.samples, splice_sample_index);

    let content_split_ticks = cumulative(&video.samples, content_split);
    let content_split_secs = content_split_ticks as f64 / video.spec.timescale as f64;
    let audio_content_split = index_at_offset(
        &audio.samples,
        (content_split_secs * audio.spec.timescale as f64).round() as u64,
    );

    let base_video_samples = video.samples[..content_split].to_vec();
    let ad_video_samples = video.samples[content_split..].to_vec();
    let ad_duration_ticks_90k = ad_video_samples
        .iter()
        .map(|s| s.duration as u64)
        .sum::<u64>();
    let base_audio_samples = audio.samples[..audio_content_split].to_vec();
    let ad_audio_samples = audio.samples[audio_content_split..].to_vec();

    let base_media = Media::new(
        vec![
            Track::new_at(
                video.spec.clone(),
                base_video_samples,
                video.start_decode_time,
            ),
            Track::new_at(
                audio.spec.clone(),
                base_audio_samples,
                audio.start_decode_time,
            ),
        ],
        source.movie_timescale,
    );
    let ad_media = Media::new(
        vec![
            Track::new_at(video.spec.clone(), ad_video_samples, 0),
            Track::new_at(audio.spec.clone(), ad_audio_samples, 0),
        ],
        source.movie_timescale,
    );

    // ------------------------------------------------------------------
    // 3. Hand-build a spec-correct splice_insert() cue at `splice_dts` and
    //    append it as a genuine TS packet to a copy of the real bytes.
    // ------------------------------------------------------------------
    let splice_insert_cmd = SpliceInsert {
        splice_event_id: SPLICE_EVENT_ID,
        splice_event_cancel_indicator: false,
        out_of_network_indicator: true,
        program_splice_flag: true,
        splice_immediate_flag: false,
        event_id_compliance_flag: true,
        splice_time: Some(SpliceTime::with_pts(splice_dts)),
        components: Vec::new(),
        break_duration: Some(BreakDuration {
            auto_return: true,
            duration: ad_duration_ticks_90k,
        }),
        unique_program_id: 1,
        avail_num: 1,
        avails_expected: 1,
    };
    let section = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(splice_insert_cmd), &[]);
    let cue_bytes = section.to_bytes();

    let mut combined_ts = base_ts_bytes;
    combined_ts.extend_from_slice(&scte35_ts_packet(&cue_bytes, SCTE35_PID));

    // ------------------------------------------------------------------
    // 4. Extract the cue back out of the TS bytes (not threaded through in
    //    memory) and convert it via `timed-metadata`.
    // ------------------------------------------------------------------
    let raw_cue = extract_scte35_raw(&combined_ts, SCTE35_PID);
    assert_eq!(
        raw_cue, cue_bytes,
        "extracted cue must match the one embedded"
    );

    let anchor = TimeAnchor {
        pts_90k: video.start_decode_time,
        utc_epoch_ms: DEMO_EPOCH_MS,
    };
    let mut timeline = Timeline::with_anchor(anchor);
    let event = timeline.push_scte35(&raw_cue)?;
    let at_pts_90k = event
        .at
        .expect("program splice_insert with an explicit pts_time")
        .0;

    // Convert the cue's 90 kHz PTS into the base video track's OWN timescale
    // ticks. For this capture the two happen to coincide (TS video is always
    // carried on the 90 kHz PES clock), so this is a no-op multiply here —
    // written generically so it holds for a base with a different timescale.
    let at_ticks =
        (at_pts_90k as u128 * video.spec.timescale as u128 / timed_metadata::PTS_HZ as u128) as u64;

    // ------------------------------------------------------------------
    // 5. The actual splice.
    // ------------------------------------------------------------------
    let result = splice_insert(&base_media, &ad_media, at_ticks)?;

    let mut points_by_track: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for p in &result.discontinuity_points {
        points_by_track
            .entry(p.track_id)
            .or_default()
            .push(p.sample_index);
    }

    let spliced_video = &result.media.tracks[0];
    let spliced_audio = &result.media.tracks[1];
    let video_indices = points_by_track
        .get(&spliced_video.spec.track_id)
        .cloned()
        .unwrap_or_default();
    let audio_indices = points_by_track
        .get(&spliced_audio.spec.track_id)
        .cloned()
        .unwrap_or_default();

    let video_ranges = [
        (0, video_indices[0]),
        (video_indices[0], video_indices[1]),
        (video_indices[1], spliced_video.samples.len()),
    ];
    let audio_ranges = [
        (0, audio_indices[0]),
        (audio_indices[0], audio_indices[1]),
        (audio_indices[1], spliced_audio.samples.len()),
    ];

    // ------------------------------------------------------------------
    // 6. Build the DASH `emsg` for the ad-in point via `timed-metadata`.
    // ------------------------------------------------------------------
    let emsg_cfg = EmsgConfig {
        timescale: spliced_video.spec.timescale,
        presentation: PresentationTime::Absolute(at_ticks),
        event_duration: ad_duration_ticks_90k as u32,
        value: EMSG_VALUE_PROVIDER_AD_START.to_string(),
        id: SPLICE_EVENT_ID,
    };
    let emsg_bytes = timeline.to_emsg(&event, &emsg_cfg)?;
    let emsg_box = EmsgBox::parse(&emsg_bytes)?;

    // ------------------------------------------------------------------
    // 7. Render the CMAF init + media segments (per track — the real
    //    CMAF/DASH representation model), embedding the emsg box at the
    //    start of the video track's ad segment only (ISO/IEC 14496-12 §8.8 +
    //    DASH-IF IOP Part 10 §6.1: it precedes that segment's `moof`).
    // ------------------------------------------------------------------
    let video_init = build_init_segment(
        std::slice::from_ref(&spliced_video.spec),
        result.media.movie_timescale,
    )?;
    let audio_init = build_init_segment(
        std::slice::from_ref(&spliced_audio.spec),
        result.media.movie_timescale,
    )?;

    let mut video_segments = Vec::with_capacity(3);
    for (i, &(lo, hi)) in video_ranges.iter().enumerate() {
        let base_media_decode_time =
            spliced_video.start_decode_time + cumulative(&spliced_video.samples, lo);
        let frag = FragmentTrackData {
            track_id: spliced_video.spec.track_id,
            base_media_decode_time,
            samples: &spliced_video.samples[lo..hi],
        };
        let seg = if i == 1 {
            build_media_segment_with_events(
                (i + 1) as u32,
                &[frag],
                std::slice::from_ref(&emsg_box),
            )?
        } else {
            build_media_segment((i + 1) as u32, &[frag])?
        };
        video_segments.push(seg);
    }

    let mut audio_segments = Vec::with_capacity(3);
    for (i, &(lo, hi)) in audio_ranges.iter().enumerate() {
        let base_media_decode_time =
            spliced_audio.start_decode_time + cumulative(&spliced_audio.samples, lo);
        let frag = FragmentTrackData {
            track_id: spliced_audio.spec.track_id,
            base_media_decode_time,
            samples: &spliced_audio.samples[lo..hi],
        };
        audio_segments.push(build_media_segment((i + 1) as u32, &[frag])?);
    }

    // ------------------------------------------------------------------
    // 8. HLS: a video media playlist with EXT-X-DISCONTINUITY at both splice
    //    points, plus an EXT-X-DATERANGE built from the cue.
    // ------------------------------------------------------------------
    let daterange = timeline.to_daterange(&event)?;
    let video_seg_secs: Vec<f64> = video_ranges
        .iter()
        .map(|&(lo, hi)| {
            range_duration(&spliced_video.samples, lo, hi) as f64
                / spliced_video.spec.timescale as f64
        })
        .collect();

    let playlist = MediaPlaylist {
        version: 7,
        target_duration: video_seg_secs
            .iter()
            .cloned()
            .fold(0.0_f64, f64::max)
            .ceil() as u32,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![
            MediaSegment {
                uri: "seg-1-1.m4s".to_string(),
                duration: video_seg_secs[0],
                discontinuous: false,
                parts: vec![],
            },
            MediaSegment {
                uri: "seg-1-2.m4s".to_string(),
                duration: video_seg_secs[1],
                discontinuous: true,
                parts: vec![],
            },
            MediaSegment {
                uri: "seg-1-3.m4s".to_string(),
                duration: video_seg_secs[2],
                discontinuous: true,
                parts: vec![],
            },
        ],
        endlist: true,
        extra_tags: vec![
            "#EXT-X-MAP:URI=\"init-1.mp4\"".to_string(),
            daterange.to_tag_line(),
        ],
        low_latency: None,
        iframes_only: false,
    };
    let m3u8 = playlist.to_m3u8();

    // ------------------------------------------------------------------
    // 9. DASH: an MPD with a video + audio AdaptationSet, an
    //    InbandEventStream declaration on the video set, and per-segment
    //    SegmentTemplate durations matching the files written below.
    // ------------------------------------------------------------------
    let video_seg_ticks: Vec<u64> = video_ranges
        .iter()
        .map(|&(lo, hi)| range_duration(&spliced_video.samples, lo, hi))
        .collect();
    let audio_seg_ticks: Vec<u64> = audio_ranges
        .iter()
        .map(|&(lo, hi)| range_duration(&spliced_audio.samples, lo, hi))
        .collect();

    let mut dash = DashPackager {
        addressing: Addressing::Number,
        segments: vec![
            TrackSegments {
                track_id: spliced_video.spec.track_id,
                durations: video_seg_ticks,
            },
            TrackSegments {
                track_id: spliced_audio.spec.track_id,
                durations: audio_seg_ticks,
            },
        ],
        init_template: "init-$RepresentationID$.mp4".to_string(),
        media_template: "seg-$RepresentationID$-$Number$.m4s".to_string(),
        inband_event_streams: vec![InbandEventStream {
            scheme_id_uri: SCTE35_SCHEME.to_string(),
            value: None,
        }],
        ..DashPackager::default()
    };
    let mpd = broadcast_common::Package::package(&mut dash, &result.media)?;

    // ------------------------------------------------------------------
    // 10. Write everything to disk.
    // ------------------------------------------------------------------
    let out_dir = std::env::temp_dir().join("transmux-ssai-ad-stitch-demo");
    fs::create_dir_all(&out_dir)?;
    fs::write(out_dir.join("init-1.mp4"), &video_init)?;
    fs::write(out_dir.join("init-2.mp4"), &audio_init)?;
    for (i, seg) in video_segments.iter().enumerate() {
        fs::write(out_dir.join(format!("seg-1-{}.m4s", i + 1)), seg)?;
    }
    for (i, seg) in audio_segments.iter().enumerate() {
        fs::write(out_dir.join(format!("seg-2-{}.m4s", i + 1)), seg)?;
    }
    fs::write(out_dir.join("playlist.m3u8"), &m3u8)?;
    fs::write(out_dir.join("manifest.mpd"), &mpd)?;
    fs::write(out_dir.join("cue.bin"), &raw_cue)?;
    fs::write(out_dir.join("emsg.bin"), &emsg_bytes)?;

    Ok(Demo {
        m3u8,
        mpd,
        emsg_bytes,
        raw_cue,
        daterange,
        discontinuity_points: result.discontinuity_points,
        out_dir,
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let demo = run()?;

    println!(
        "SSAI ad-stitch demo — output written to {}\n",
        demo.out_dir.display()
    );

    println!("splice_event_id  : {SPLICE_EVENT_ID}");
    println!("raw cue          : {} bytes", demo.raw_cue.len());
    println!("discontinuities  : {}", demo.discontinuity_points.len());
    for p in &demo.discontinuity_points {
        println!(
            "  track {} sample {} @ pts {}",
            p.track_id, p.sample_index, p.presentation_time
        );
    }
    println!();
    println!("--- EXT-X-DATERANGE ---");
    println!("{}", demo.daterange.to_tag_line());
    println!();
    println!("--- HLS media playlist (video) ---");
    println!("{}", demo.m3u8);
    println!("--- DASH MPD ---");
    println!("{}", demo.mpd);
    println!("emsg box: {} bytes", demo.emsg_bytes.len());

    Ok(())
}
