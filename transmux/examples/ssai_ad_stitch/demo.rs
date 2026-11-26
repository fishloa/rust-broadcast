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
//! 3. **`test_harness::collect::splice_insert`** drives `transmux`'s
//!    `SpliceStage` over a `SplicePlan::insert` to perform the actual
//!    sample-level ad splice on the demuxed [`Media`] IR, returning a
//!    `SpliceOutput` whose [`SplicePoint`](transmux::SplicePoint)s mark the
//!    ad-in and resume cuts.
//! 4. **`transmux`'s HLS/DASH packaging** (`broadcast_hls::MediaPlaylist`,
//!    `DashPackager`, `build_media_segment_with_events`) renders the spliced
//!    timeline as a real `#EXT-X-DISCONTINUITY` + `#EXT-X-DATERANGE` HLS media
//!    playlist and a real DASH `.mpd` carrying an inband `emsg` at the splice
//!    point.
//!
//! # Fixture choice (documented per the issue's requirement)
//!
//! The base is `fixtures/ts/h264-aac.ts` — a real H.264/AVC + AAC MPEG-2 TS
//! capture (25 fps, 320x240, 3 real keyframes) already used by
//! [`transmux_hub`](../../README.md). No fixture in this workspace carries
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

use mpeg_ts::pid::Pid;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use transmux::{SpliceIndexBuilder, SplicePlan, SpliceSample, SpliceStage, TrackId, TrackSpec};

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp as StageTimestamp;
use broadcast_core::Serialize;
use broadcast_hls::{DecimalSeconds, MediaPlaylist, MediaSegment};
use isobmff::emsg::{EmsgBox, PresentationTime};
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::{AnyCommand, ProgramSplice, SpliceInsert, SpliceMode};
use scte35_splice::time::{BreakDuration, SpliceTime};
use timed_metadata::convert::{EmsgConfig, SCTE35_SCHEME};
use timed_metadata::{DateRange, TimeAnchor, Timeline};
use transmux::ir::SampleTiming;
use transmux::{
    Addressing, CodecConfig, DashPackager, DemuxEvent, FragmentTrackData, InbandEventStream,
    Sample, SpliceIndex, SplicePoint, build_init_segment, build_media_segment,
    build_media_segment_with_events,
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
pub const SPLICE_EVENT_ID: u32 = 100_002;
/// Arbitrary demo wall-clock anchor: 2024-01-15T12:00:00Z, matching the
/// convention already used by `timed-metadata`'s own `scte35_to_hls`/
/// `scte35_to_dash` examples.
const DEMO_EPOCH_MS: i64 = 1_705_320_000_000;
/// `emsg` `value` — SCTE-35 `segmentation_type_id` 0x22, "Provider
/// Advertisement Start" (informational text; this demo's `splice_insert`
/// carries no segmentation descriptor of its own). Matches the convention
/// already used by `timed-metadata/examples/scte35_to_dash.rs`.
const EMSG_VALUE_PROVIDER_AD_START: &str = "34";
/// Chunk size for every pass that reads the capture from disk, so neither pass
/// holds the whole file.
const INPUT_CHUNK: usize = 64 * 1024;

/// The full set of rendered outputs, returned so both `main` (which writes
/// them to disk and prints a summary) and the integration test
/// (`tests/it/ssai_ad_stitch.rs`, which `#[path]`-includes this file as a
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
    test_fixtures::path(rel)
}

/// Sum of `samples[lo..hi]`'s durations.
fn range_duration(samples: &[Sample], lo: usize, hi: usize) -> u64 {
    samples[lo..hi]
        .iter()
        .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64)
        .sum()
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
        if pkt.header.pid != Pid::from_wire_masked(pid) {
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
/// Drive the TS demux over `path` in [`INPUT_CHUNK`] pieces, handing every
/// event to `on_event`, so neither pass collects the capture in one buffer.
fn drive_stream<F>(path: &std::path::Path, mut on_event: F) -> Result<(), Box<dyn Error>>
where
    F: FnMut(&DemuxEvent) -> Result<(), Box<dyn Error>>,
{
    let mut demux = transmux::StreamingTsDemux::new().with_strict_pmt_order();
    let mut file = fs::File::open(path)?;
    let mut buf = vec![0u8; INPUT_CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        demux.feed(&buf[..n])?;
        while let Some(event) = demux.poll_event() {
            on_event(&event)?;
        }
    }
    demux.finish()?;
    while let Some(event) = demux.poll_event() {
        on_event(&event)?;
    }
    Ok(())
}

/// Rebuild a [`SampleTiming`] from an index sample's planner fields.
fn timing_of(sample: &SpliceSample) -> Option<SampleTiming> {
    match sample.dts {
        Some(dts) => Some(SampleTiming::Timed {
            dts,
            pts: sample.pts.unwrap_or(dts),
            duration: sample.duration,
        }),
        None => sample
            .duration
            .map(|duration| SampleTiming::DurationOnly { duration }),
    }
}

/// Sum of `samples[..upto]`'s durations, in the track's own timescale ticks.
fn cumulative_index(samples: &[SpliceSample], upto: usize) -> u64 {
    samples[..upto]
        .iter()
        .map(|s| s.duration.map_or(0, |d| u64::from(d.get())))
        .sum()
}

/// First index sample whose cumulative duration (from the track start) is at
/// or beyond `offset_ticks`.
fn index_at_offset_index(samples: &[SpliceSample], offset_ticks: u64) -> usize {
    let mut acc = 0u64;
    for (i, s) in samples.iter().enumerate() {
        if acc >= offset_ticks {
            return i;
        }
        acc += s.duration.map_or(0, |d| u64::from(d.get()));
    }
    samples.len()
}

/// Split the source index into the base and insert indexes the planner needs,
/// carrying only timing (never sample bytes), in `order`.
fn split_index(
    index: &SpliceIndex,
    order: &[usize],
    content_split: &BTreeMap<TrackId, usize>,
) -> (SpliceIndex, SpliceIndex) {
    let mut base = SpliceIndexBuilder::new();
    let mut ad = SpliceIndexBuilder::new();
    for &source in order {
        let track = &index.tracks()[source];
        let split = content_split
            .get(&track.spec.track_id)
            .copied()
            .unwrap_or(track.samples.len());
        let bi = base.add_track(track.spec.clone(), track.start_decode_time);
        for sample in &track.samples[..split] {
            base.push_sample(bi, timing_of(sample), sample.is_sync);
        }
        let ai = ad.add_track(track.spec.clone(), 0);
        for sample in &track.samples[split..] {
            ad.push_sample(ai, timing_of(sample), sample.is_sync);
        }
    }
    (base.build(), ad.build())
}

/// One track's streaming splice state: the stage, the buffered tail (fed only
/// after the inserted run), and the output segments being filled.
struct TrackStream {
    track_id: TrackId,
    split: usize,
    content_split: usize,
    stage: SpliceStage,
    source_index: usize,
    tail: Vec<Sample>,
    current: Vec<Sample>,
    targets: [usize; 3],
    segment: usize,
    segments: Vec<Vec<Sample>>,
}

impl TrackStream {
    fn drain(&mut self) {
        while let Some((_, sample)) = self.stage.poll() {
            self.current.push(sample);
            if self.segment < self.targets.len() && self.current.len() == self.targets[self.segment]
            {
                self.segments.push(std::mem::take(&mut self.current));
                self.segment += 1;
            }
        }
    }

    fn feed(&mut self, sample: &Sample) -> Result<(), Box<dyn Error>> {
        self.stage
            .feed((self.track_id, sample.clone()), StageTimestamp::ZERO)?;
        self.drain();
        Ok(())
    }
}

/// The per-track output of the splice: its spec, first decode time, and one
/// segment's samples per resulting range.
struct SplicedTrack {
    spec: TrackSpec,
    start: u64,
    segments: Vec<Vec<Sample>>,
}

/// Run the full pipeline: fixture -> hand-built cue -> extract -> splice ->
/// HLS + DASH. Writes every manifest/segment file to a temp directory and
/// returns the rendered outputs for inspection.
///
/// Two bounded passes over the capture: the first builds the payload-free
/// splice index, the second streams samples through [`SpliceStage`] and
/// builds each segment as its range fills.
pub fn run() -> Result<Demo, Box<dyn Error>> {
    let fixture = fixture_path("ts/h264-aac.ts");
    let movie_timescale = transmux::rtp::VIDEO_CLOCK_TIMESCALE;

    // ------------------------------------------------------------------
    // 1. Pass 1: build the source splice index in bounded chunks.
    // ------------------------------------------------------------------
    let mut builder = SpliceIndexBuilder::new();
    drive_stream(&fixture, |event| {
        builder.push_event(event);
        Ok(())
    })?;
    let index = builder.build();

    let video_idx = index
        .tracks()
        .iter()
        .position(|t| is_avc(&t.spec.config))
        .expect("h264-aac.ts must have an H.264/AVC video track");
    let audio_idx = index
        .tracks()
        .iter()
        .position(|t| !is_avc(&t.spec.config))
        .expect("h264-aac.ts must have an audio track");
    let order = [video_idx, audio_idx];
    let video = &index.tracks()[video_idx];
    let audio = &index.tracks()[audio_idx];

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
    let splice_sample_index = keyframe_indices[1];
    let content_split = keyframe_indices[2];

    let splice_dts =
        video.start_decode_time + cumulative_index(&video.samples, splice_sample_index);

    let content_split_ticks = cumulative_index(&video.samples, content_split);
    let content_split_secs = content_split_ticks as f64 / video.spec.timescale.get() as f64;
    let audio_content_split = index_at_offset_index(
        &audio.samples,
        (content_split_secs * audio.spec.timescale.get() as f64).round() as u64,
    );

    let ad_duration_ticks_90k = (content_split..video.samples.len())
        .map(|i| video.samples[i].duration.map_or(0, |d| u64::from(d.get())))
        .sum::<u64>();

    let mut content_split_by_track = BTreeMap::new();
    content_split_by_track.insert(video.spec.track_id, content_split);
    content_split_by_track.insert(audio.spec.track_id, audio_content_split);
    let (base_index, ad_index) = split_index(&index, &order, &content_split_by_track);

    // ------------------------------------------------------------------
    // 3. Hand-build a spec-correct splice_insert() cue at `splice_dts` and
    //    append it as a genuine TS packet to a copy of the real bytes.
    // ------------------------------------------------------------------
    let splice_insert_cmd = SpliceInsert {
        splice_event_id: SPLICE_EVENT_ID,
        cancel: None,
        out_of_network_indicator: true,
        event_id_compliance_flag: true,
        mode: SpliceMode::Program(ProgramSplice::At(SpliceTime::at(
            broadcast_common::clock33::Clock33::try_new(splice_dts).expect("in range"),
        ))),
        break_duration: Some(BreakDuration {
            auto_return: true,
            duration: ad_duration_ticks_90k,
        }),
        unique_program_id: 1,
        avail_num: 1,
        avails_expected: 1,
    };
    let section = SpliceInfoSection::new_clear(
        AnyCommand::SpliceInsert(splice_insert_cmd),
        scte35_splice::descriptor_loop::SpliceDescriptorLoop::new(&[]),
    );
    let cue_bytes = section.try_to_bytes().unwrap();

    // ------------------------------------------------------------------
    // 4. Extract the cue back out of the TS packet (not threaded through in
    //    memory) and convert it via `timed-metadata`.
    // ------------------------------------------------------------------
    let cue_packet = scte35_ts_packet(&cue_bytes, SCTE35_PID);
    let raw_cue = extract_scte35_raw(&cue_packet, SCTE35_PID);
    assert_eq!(
        raw_cue, cue_bytes,
        "extracted cue must match the one embedded"
    );

    let anchor = TimeAnchor {
        pts_90k: video.start_decode_time,
        utc_epoch_ms: DEMO_EPOCH_MS,
    };
    let mut timeline = Timeline::new(anchor);
    let event = timeline.push_scte35(&raw_cue)?;
    let at_pts_90k = event
        .at
        .expect("program splice_insert with an explicit pts_time")
        .0;

    let at_ticks = (at_pts_90k as u128 * video.spec.timescale.get() as u128
        / timed_metadata::PTS_HZ as u128) as u64;

    // ------------------------------------------------------------------
    // 5. The actual splice plan, then pass 2 streams the samples through it.
    // ------------------------------------------------------------------
    let plan = SplicePlan::insert(&base_index, &ad_index, movie_timescale, at_ticks)?;
    let discontinuity_points = plan.discontinuity_points().to_vec();

    let mut streams: Vec<TrackStream> = plan
        .tracks()
        .iter()
        .enumerate()
        .map(|(slice_index, slice)| {
            let content_split = content_split_by_track[&slice.track_id];
            TrackStream {
                track_id: slice.track_id,
                split: slice.split_index,
                content_split,
                stage: SpliceStage::new(
                    plan.only(slice_index)
                        .expect("the plan names this track slice"),
                ),
                source_index: 0,
                tail: Vec::new(),
                current: Vec::new(),
                targets: [
                    slice.split_index,
                    slice.insert_count,
                    content_split.saturating_sub(slice.split_index),
                ],
                segment: 0,
                segments: Vec::new(),
            }
        })
        .collect();
    let by_id: BTreeMap<TrackId, usize> = streams
        .iter()
        .enumerate()
        .map(|(i, s)| (s.track_id, i))
        .collect();

    drive_stream(&fixture, |event| {
        if let DemuxEvent::Sample {
            track_id, sample, ..
        } = event
            && let Some(&si) = by_id.get(track_id)
        {
            let stream = &mut streams[si];
            let i = stream.source_index;
            stream.source_index += 1;
            if i < stream.split || i >= stream.content_split {
                stream.feed(sample)?;
            } else {
                stream.tail.push(sample.clone());
            }
        }
        Ok(())
    })?;
    // The base tail plays after the inserted run, so it is buffered until last.
    for stream in &mut streams {
        for sample in std::mem::take(&mut stream.tail) {
            stream.feed(&sample)?;
        }
        stream.stage.finish()?;
        stream.drain();
        if !stream.current.is_empty() {
            stream.segments.push(std::mem::take(&mut stream.current));
        }
    }

    let spliced: Vec<SplicedTrack> = streams
        .into_iter()
        .enumerate()
        .map(|(slice_index, stream)| SplicedTrack {
            spec: index.tracks()[order[slice_index]].spec.clone(),
            start: index.tracks()[order[slice_index]].start_decode_time,
            segments: stream.segments,
        })
        .collect();
    let spliced_video = &spliced[0];
    let spliced_audio = &spliced[1];

    // ------------------------------------------------------------------
    // 6. Build the DASH `emsg` for the ad-in point via `timed-metadata`.
    // ------------------------------------------------------------------
    let emsg_cfg = EmsgConfig::new(
        spliced_video.spec.timescale,
        PresentationTime::Absolute(at_ticks),
        ad_duration_ticks_90k as u32,
        EMSG_VALUE_PROVIDER_AD_START.to_string(),
        SPLICE_EVENT_ID,
    );
    let emsg_bytes = timeline.to_emsg(&event, &emsg_cfg)?;
    let emsg_box = EmsgBox::parse(&emsg_bytes)?;

    // ------------------------------------------------------------------
    // 7. Render the CMAF init + media segments (per track — the real
    //    CMAF/DASH representation model), embedding the emsg box at the
    //    start of the video track's ad segment only (DASH-IF IOP Part 10 §6.1: it
    //    precedes that segment's `moof`).
    // ------------------------------------------------------------------
    let video_init =
        build_init_segment(std::slice::from_ref(&spliced_video.spec), movie_timescale)?;
    let audio_init =
        build_init_segment(std::slice::from_ref(&spliced_audio.spec), movie_timescale)?;

    let video_segments = build_segments(spliced_video, Some(&emsg_box))?;
    let audio_segments = build_segments(spliced_audio, None)?;

    // ------------------------------------------------------------------
    // 8. HLS: a video media playlist with EXT-X-DISCONTINUITY at both splice
    //    points, plus an EXT-X-DATERANGE built from the cue.
    // ------------------------------------------------------------------
    let daterange = timeline.to_daterange(&event)?;
    let video_seg_secs: Vec<f64> = spliced_video
        .segments
        .iter()
        .map(|samples| {
            range_duration(samples, 0, samples.len()) as f64
                / spliced_video.spec.timescale.get() as f64
        })
        .collect();

    let playlist = MediaPlaylist {
        version: 7,
        target_duration: video_seg_secs
            .iter()
            .cloned()
            .fold(0.0_f64, f64::max)
            .ceil() as u32,
        msn: broadcast_hls::MediaSequence::new(0),
        discontinuity_sequence: 0,
        segments: vec![
            MediaSegment {
                uri: "seg-1-1.m4s".to_string(),
                duration: DecimalSeconds::new(video_seg_secs[0])
                    .expect("segment duration is finite, >= 0"),
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg-1-2.m4s".to_string(),
                duration: DecimalSeconds::new(video_seg_secs[1])
                    .expect("segment duration is finite, >= 0"),
                discontinuous: true,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg-1-3.m4s".to_string(),
                duration: DecimalSeconds::new(video_seg_secs[2])
                    .expect("segment duration is finite, >= 0"),
                discontinuous: true,
                parts: vec![],
                ..Default::default()
            },
        ],
        endlist: true,
        extra_tags: vec![
            "#EXT-X-MAP:URI=\"init-1.mp4\"".to_string(),
            daterange.to_tag_line()?,
        ],
        low_latency: None,
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };
    let m3u8 = playlist.to_m3u8().unwrap();

    // ------------------------------------------------------------------
    // 9. DASH: an MPD with a video + audio AdaptationSet, an
    //    InbandEventStream declaration on the video set, and per-segment
    //    SegmentTemplate durations matching the files written below.
    // ------------------------------------------------------------------
    let video_seg_ticks: Vec<u64> = spliced_video
        .segments
        .iter()
        .map(|samples| range_duration(samples, 0, samples.len()))
        .collect();
    let audio_seg_ticks: Vec<u64> = spliced_audio
        .segments
        .iter()
        .map(|samples| range_duration(samples, 0, samples.len()))
        .collect();

    let mut log = transmux::segment_log::SegmentSummaryLog::new();
    for track in &spliced {
        let duration: u64 = track
            .segments
            .iter()
            .flatten()
            .map(|s| u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)))
            .sum();
        let bytes: u64 = track
            .segments
            .iter()
            .flatten()
            .map(|s| s.data.len() as u64)
            .sum();
        let samples: usize = track.segments.iter().map(Vec::len).sum();
        log.push_part(
            &transmux::mux_stage::TrackPart {
                track: track.spec.track_id,
                start: 0,
                duration,
                timescale: track.spec.timescale.get(),
                bytes,
                samples: u32::try_from(samples).unwrap_or(u32::MAX),
            },
            track
                .segments
                .first()
                .and_then(|s| s.first())
                .is_some_and(|s| s.flags.is_sync),
        )?;
    }
    let dash = DashPackager {
        addressing: Addressing::Number,
        segments: vec![
            transmux::TrackSegments {
                track_id: spliced_video.spec.track_id,
                durations: video_seg_ticks,
            },
            transmux::TrackSegments {
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
    let specs: Vec<transmux::TrackSpec> = spliced.iter().map(|t| t.spec.clone()).collect();
    let mpd = dash.render(&specs, &log)?;

    // ------------------------------------------------------------------
    // 10. Write everything to disk.
    // ------------------------------------------------------------------
    write_outputs(&OutputBatch {
        video_init: &video_init,
        audio_init: &audio_init,
        video_segments: &video_segments,
        audio_segments: &audio_segments,
        m3u8: &m3u8,
        mpd: &mpd,
        raw_cue: &raw_cue,
        emsg_bytes: &emsg_bytes,
    })
    .map(|out_dir| Demo {
        m3u8,
        mpd,
        emsg_bytes,
        raw_cue,
        daterange,
        discontinuity_points,
        out_dir,
    })
}

/// Build one track's CMAF media segments, one per splice range, embedding
/// `emsg` at the start of its middle (inserted) segment when given.
fn build_segments(
    track: &SplicedTrack,
    emsg: Option<&EmsgBox>,
) -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
    let mut segments = Vec::with_capacity(track.segments.len());
    let mut elapsed = 0u64;
    for (i, samples) in track.segments.iter().enumerate() {
        let base_media_decode_time = track.start + elapsed;
        let frag = FragmentTrackData::new(track.spec.track_id, base_media_decode_time, samples);
        let bytes = match emsg {
            Some(emsg) if i == 1 => build_media_segment_with_events(
                (i + 1) as u32,
                &[frag],
                std::slice::from_ref(emsg),
            )?,
            _ => build_media_segment((i + 1) as u32, &[frag])?,
        };
        segments.push(bytes);
        elapsed += range_duration(samples, 0, samples.len());
    }
    Ok(segments)
}

/// The bytes [`write_outputs`] writes.
struct OutputBatch<'a> {
    video_init: &'a [u8],
    audio_init: &'a [u8],
    video_segments: &'a [Vec<u8>],
    audio_segments: &'a [Vec<u8>],
    m3u8: &'a str,
    mpd: &'a str,
    raw_cue: &'a [u8],
    emsg_bytes: &'a [u8],
}

/// Write every manifest and segment to a fresh per-call temp directory.
fn write_outputs(batch: &OutputBatch<'_>) -> Result<PathBuf, Box<dyn Error>> {
    static RUN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let out_dir = std::env::temp_dir().join(format!(
        "transmux-ssai-ad-stitch-demo-{}-{}",
        std::process::id(),
        RUN_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir_all(&out_dir)?;
    fs::write(out_dir.join("init-1.mp4"), batch.video_init)?;
    fs::write(out_dir.join("init-2.mp4"), batch.audio_init)?;
    for (i, seg) in batch.video_segments.iter().enumerate() {
        fs::write(out_dir.join(format!("seg-1-{}.m4s", i + 1)), seg)?;
    }
    for (i, seg) in batch.audio_segments.iter().enumerate() {
        fs::write(out_dir.join(format!("seg-2-{}.m4s", i + 1)), seg)?;
    }
    fs::write(out_dir.join("playlist.m3u8"), batch.m3u8)?;
    fs::write(out_dir.join("manifest.mpd"), batch.mpd)?;
    fs::write(out_dir.join("cue.bin"), batch.raw_cue)?;
    fs::write(out_dir.join("emsg.bin"), batch.emsg_bytes)?;
    Ok(out_dir)
}
