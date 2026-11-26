//! Incremental writers that grow a TS or FLV file to a target size in bounded
//! memory, for tests of the file-playout path (issue #4057).
//!
//! A 256 MiB capture cannot be committed as a fixture, so these writers
//! generate one at test time: demux a small committed fixture once, then append
//! its samples pass after pass, each pass's timestamps advanced by that track's
//! own span, flushing every pass to disk. Live bytes stay at one pass plus the
//! muxer's own state, which the module's tests assert with `test_alloc`'s
//! `ProcessPeak` allocator (a dev-dependency, so not linkable from here).

use std::fmt;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

use crate::collect::{Collected, drive};
use broadcast_common::{Parse, Serialize, Stage as _};
use rtmp_runtime::flv::{FlvHeader, FlvTag, TagType};
use transmux::mux_stage::MuxOutput;
use transmux::{
    CodecConfig, DEFAULT_PSI_INTERVAL, FlvPayloadKind, Sample, TsMuxStage, flv_frame_payload,
    flv_sequence_header_payloads, on_metadata_body,
};

/// Nanoseconds in one second: turns a timescale-tick span into a [`Duration`]
/// without a floating-point round trip that could drift off the written
/// timestamps.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Bytes in one MPEG-2 TS packet: a segment boundary that is not on one splits
/// a packet, which a demuxer fed the segments in order would still resync on but
/// a fixture need not rely on.
const TS_PACKET: usize = 188;

/// What an incremental write produced.
#[derive(Debug, Clone)]
pub struct BigMedia {
    /// Exact number of bytes written to the file.
    pub bytes: u64,
    /// `(track id, samples written)` in source track order.
    pub samples_per_track: Vec<(u32, u64)>,
    /// Collected timeline the written samples span.
    pub duration: Duration,
}

/// Write an MPEG-2 TS file of at least `target_bytes` by repeating
/// `source_fixture`'s samples through a single [`TsMuxStage`].
pub fn write_big_ts(path: &Path, source_fixture: &Path, target_bytes: u64) -> BigMedia {
    let source = read_source(source_fixture);
    let media = expect(
        "demux TS source",
        crate::collect::media_from_ts(source.as_slice()),
    );
    let step = pass_step(&media);
    let mut mux = TsMuxStage::new(Some(DEFAULT_PSI_INTERVAL));
    let mut out = BufWriter::new(expect("create output", File::create(path)));
    let mut written = 0u64;
    let mut passes = 0u64;
    while written < target_bytes {
        let shifted = media_at_pass(&media, &step, passes);
        let chunk = mux_pass(&mut mux, &shifted);
        expect("write TS pass", out.write_all(&chunk));
        written += expect("sum chunk length", u64::try_from(chunk.len()));
        passes += 1;
    }
    expect("flush TS output", out.flush());
    BigMedia {
        bytes: written,
        samples_per_track: counts(&media, passes),
        duration: step.duration_for(passes),
    }
}

/// Write an FLV file of at least `target_bytes` by repeating `source_fixture`'s
/// samples through the FLV tag builders, framing each tag as it is produced.
pub fn write_big_flv(path: &Path, source_fixture: &Path, target_bytes: u64) -> BigMedia {
    let source = read_source(source_fixture);
    let media = expect(
        "demux FLV source",
        crate::collect::media_from_flv(source.as_slice()),
    );
    let step = pass_step(&media);
    let mut out = BufWriter::new(expect("create output", File::create(path)));
    let mut written = write_flv_preamble(&mut out, &media);
    let mut passes = 0u64;
    while written < target_bytes {
        let shifted = media_at_pass(&media, &step, passes);
        for track in &shifted.tracks {
            for sample in &track.samples {
                let payload = expect(
                    "build FLV frame payload",
                    flv_frame_payload(&track.spec, sample),
                );
                if let Some(payload) = payload {
                    written += write_flv_tag(
                        &mut out,
                        payload_tag_type(payload.kind),
                        payload.timestamp_ms,
                        &payload.body,
                    );
                }
            }
        }
        passes += 1;
    }
    expect("flush FLV output", out.flush());
    BigMedia {
        bytes: written,
        samples_per_track: counts(&media, passes),
        duration: step.duration_for(passes),
    }
}

/// A long, continuous TS timeline cut into fixed-size segments, generated one
/// pass at a time: a test server answers thousands of segment requests without
/// holding the presentation, which is what a bounded-memory test needs.
///
/// Every segment is a whole number of TS packets, and consecutive segments
/// continue one another, so a demuxer fed them in order sees exactly the stream
/// [`write_big_ts`] would have written, only cut up.
pub struct TsSegmentStream {
    media: Collected,
    step: PassStep,
    mux: TsMuxStage,
    segments: u64,
    segment_bytes: usize,
    emitted: u64,
    next_pass: u64,
    pending: Vec<u8>,
}

impl TsSegmentStream {
    /// A stream of exactly `segments` segments over `source_fixture`, pass after
    /// pass, each pass's timestamps advanced by that fixture's span, in segments
    /// of at most `segment_bytes` rounded down to a whole number of TS packets.
    pub fn new(source_fixture: &Path, segments: u64, segment_bytes: usize) -> Self {
        let source = read_source(source_fixture);
        let media = expect(
            "demux TS source",
            crate::collect::media_from_ts(source.as_slice()),
        );
        let step = pass_step(&media);
        Self {
            media,
            step,
            mux: TsMuxStage::new(Some(DEFAULT_PSI_INTERVAL)),
            segments,
            segment_bytes: (segment_bytes / TS_PACKET * TS_PACKET).max(TS_PACKET),
            emitted: 0,
            next_pass: 0,
            pending: Vec::new(),
        }
    }

    /// Bytes in one segment, as [`Self::new`] rounded it.
    pub fn segment_bytes(&self) -> usize {
        self.segment_bytes
    }

    /// Segments the stream produces in total.
    pub fn segments(&self) -> u64 {
        self.segments
    }

    /// The next segment, `None` once all of them have been produced.
    pub fn next_segment(&mut self) -> Option<Vec<u8>> {
        if self.emitted >= self.segments {
            return None;
        }
        while self.pending.len() < self.segment_bytes {
            let shifted = media_at_pass(&self.media, &self.step, self.next_pass);
            let chunk = mux_pass(&mut self.mux, &shifted);
            self.pending.extend_from_slice(&chunk);
            self.next_pass += 1;
        }
        let len = self.segment_bytes.min(self.pending.len());
        self.emitted += 1;
        Some(self.pending.drain(..len).collect())
    }
}

/// Mux one pass of a generated TS through `stage`, collecting the bytes it
/// emits.
fn mux_pass(stage: &mut TsMuxStage, media: &Collected) -> Vec<u8> {
    let outputs = expect(
        "mux TS pass",
        drive(stage, media, broadcast_common::stage::Timestamp::ZERO),
    );
    let mut bytes = Vec::new();
    for output in outputs {
        if let MuxOutput::Bytes(chunk) = output {
            bytes.extend_from_slice(&chunk);
        }
    }
    bytes
}

/// FLV header, `onMetaData` and the AVC/AAC sequence headers, written once.
///
/// `onMetaData`'s `duration` is omitted: the file's final length is not known
/// until the target is reached, and FLV makes that field optional (see
/// [`on_metadata_body`]).
fn write_flv_preamble(out: &mut impl Write, media: &Collected) -> u64 {
    let has_video = media
        .tracks
        .iter()
        .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. }));
    let has_audio = media
        .tracks
        .iter()
        .any(|t| matches!(t.spec.config, CodecConfig::Aac { .. }));
    let header = expect(
        "serialize FLV header",
        FlvHeader::new(has_audio, has_video).try_to_bytes(),
    );
    expect("write FLV header", out.write_all(&header));
    let mut written = expect("sum FLV header length", u64::try_from(header.len()));
    let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let meta = expect("build onMetaData", on_metadata_body(&specs, None));
    written += write_flv_tag(out, TagType::Script, 0, &meta);
    for payload in expect(
        "build FLV sequence headers",
        flv_sequence_header_payloads(&specs),
    ) {
        written += write_flv_tag(
            out,
            payload_tag_type(payload.kind),
            payload.timestamp_ms,
            &payload.body,
        );
    }
    written
}

/// Frame one FLV tag body and write it.
fn write_flv_tag(out: &mut impl Write, tag_type: TagType, timestamp_ms: u32, body: &[u8]) -> u64 {
    let bytes = expect(
        "serialize FLV tag",
        FlvTag::new(tag_type, timestamp_ms, body).try_to_bytes(),
    );
    expect("write FLV tag", out.write_all(&bytes));
    expect("sum FLV tag length", u64::try_from(bytes.len()))
}

/// [`FlvPayloadKind`] is `#[non_exhaustive]` but its only two variants are the
/// tag types a media payload can carry, so anything else falls to audio rather
/// than inventing a third tag type.
fn payload_tag_type(kind: FlvPayloadKind) -> TagType {
    if kind == FlvPayloadKind::Video {
        TagType::Video
    } else {
        TagType::Audio
    }
}

fn read_source(path: &Path) -> Vec<u8> {
    expect("read source fixture", std::fs::read(path))
}

/// The one media-time step every pass advances by: the longest track's span,
/// kept as an exact ticks/timescale rational.
///
/// Per-track spans differ (audio runs a frame past video, a 90 kHz and a 44.1
/// kHz track round differently), so advancing each track by its *own* span
/// makes the tracks drift apart by that difference on every pass — seconds of
/// A/V skew across a few hundred passes. One file-wide step in seconds, applied
/// to every track, keeps the source's A/V offset.
#[derive(Debug, Clone, Copy)]
struct PassStep {
    ticks: u64,
    timescale: u32,
}

impl PassStep {
    /// `pass` steps expressed in `timescale` ticks: the step's exact rational
    /// multiplied by the pass count first, then divided once, so the only
    /// rounding is a single floor rather than an accumulated per-pass error.
    fn offset_for(&self, pass: u64, timescale: u32) -> i64 {
        let ticks = u128::from(self.ticks)
            .checked_mul(u128::from(pass))
            .and_then(|product| product.checked_mul(u128::from(timescale)))
            .map(|product| product / u128::from(self.timescale))
            .and_then(|ticks| i64::try_from(ticks).ok());
        expect(
            "pass offset fits i64",
            ticks.ok_or("pass offset overflows i64"),
        )
    }

    /// The media timeline `passes` passes span: `passes * step`, in the same
    /// exact-rational form.
    fn duration_for(&self, passes: u64) -> Duration {
        let nanos = u128::from(self.ticks)
            .checked_mul(u128::from(passes))
            .and_then(|product| product.checked_mul(u128::from(NANOS_PER_SECOND)))
            .map(|product| product / u128::from(self.timescale))
            .and_then(|nanos| u64::try_from(nanos).ok());
        Duration::from_nanos(expect(
            "pass duration fits u64 nanoseconds",
            nanos.ok_or("pass duration overflows u64 nanoseconds"),
        ))
    }
}

/// Each track's own span in ticks (`last end - first dts`).
fn track_spans(media: &Collected) -> Vec<u64> {
    media
        .tracks
        .iter()
        .map(|track| {
            let mut first: Option<i64> = None;
            let mut last_end: Option<i64> = None;
            for sample in &track.samples {
                let Some(timing) = sample.timing.as_ref() else {
                    continue;
                };
                let Some(dts) = timing.dts() else {
                    continue;
                };
                first = Some(first.map_or(dts, |f| f.min(dts)));
                let end = dts.saturating_add(i64::from(timing.duration().unwrap_or(0)));
                last_end = Some(last_end.map_or(end, |e| e.max(end)));
            }
            match (first, last_end) {
                (Some(first), Some(end)) => u64::try_from(end.saturating_sub(first)).unwrap_or(0),
                _ => 0,
            }
        })
        .collect()
}

/// The longest track span as a [`PassStep`], comparing spans as seconds
/// cross-multiplied so tracks with different timescales compare exactly.
fn pass_step(media: &Collected) -> PassStep {
    let spans = track_spans(media);
    let timescales: Vec<u32> = media
        .tracks
        .iter()
        .map(|track| track.spec.timescale.get())
        .collect();
    let mut longest: Option<(u64, u32)> = None;
    for (&span, &timescale) in spans.iter().zip(&timescales) {
        longest = match longest {
            Some((best_span, best_timescale))
                if u128::from(span) * u128::from(best_timescale)
                    <= u128::from(best_span) * u128::from(timescale) =>
            {
                Some((best_span, best_timescale))
            }
            _ => Some((span, timescale)),
        };
    }
    let (ticks, timescale) = expect(
        "source has a timed track",
        longest.ok_or("source has no tracks"),
    );
    PassStep { ticks, timescale }
}

/// `media` shifted so pass `pass` continues the timeline `step` starts.
fn media_at_pass(media: &Collected, step: &PassStep, pass: u64) -> Collected {
    let mut shifted = media.clone();
    for track in &mut shifted.tracks {
        let offset = step.offset_for(pass, track.spec.timescale.get());
        if offset == 0 {
            continue;
        }
        for sample in &mut track.samples {
            if let Some(timing) = sample.timing.as_mut() {
                timing.shift(offset);
            }
        }
    }
    shifted
}

fn counts(media: &Collected, passes: u64) -> Vec<(u32, u64)> {
    media
        .tracks
        .iter()
        .map(|track| {
            let per_pass = expect("sum sample count", u64::try_from(track.samples.len()));
            (track.spec.track_id.value(), per_pass * passes)
        })
        .collect()
}

/// Unwrap one harness step, naming it in the failure message. The writers
/// return [`BigMedia`] rather than `Result` because a missing fixture, an
/// unwritable temp dir or a demux failure is a broken test, not a case a test
/// should branch on.
fn expect<T, E: fmt::Display>(what: &str, result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("big_media: {what}: {error}"),
    }
}

/// Bytes in one kibibyte, for the default large-file target.
const KIB: u64 = 1024;
/// Bytes in one mebibyte.
const MIB: u64 = KIB * KIB;
/// Bytes in one gibibyte.
const GIB: u64 = MIB * KIB;

/// Samples per chunk in a generated MP4: enough chunks that `co64` carries
/// more than one entry, small enough that its table stays a few KB.
const BIG_MP4_CHUNK_SAMPLES: u32 = 10;
/// Track id of the generated video track.
const BIG_MP4_VIDEO_TRACK: u32 = 1;
/// Track id of the generated audio track.
const BIG_MP4_AUDIO_TRACK: u32 = 2;
/// Movie timescale of a generated MP4.
const BIG_MP4_MOVIE_TIMESCALE: u32 = 1000;
/// `mvhd.rate` unity (16.16 fixed point).
const BIG_MP4_RATE_UNITY: u32 = 0x0001_0000;
/// `mvhd.volume` unity (8.8 fixed point).
const BIG_MP4_VOLUME_UNITY: u16 = 0x0100;
/// `tkhd.volume` unity (8.8 fixed point).
const BIG_MP4_TRACK_VOLUME_UNITY: i16 = 0x0100;
/// `mvhd.next_track_id` after the two generated tracks.
const BIG_MP4_NEXT_TRACK_ID: u32 = BIG_MP4_AUDIO_TRACK + 1;
/// Identity display matrix (§8.2.2.2).
const BIG_MP4_IDENTITY_MATRIX: [i32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
/// Width of a 32-bit box field.
const BIG_MP4_WORD: usize = 4;
/// Width of two 32-bit box fields.
const BIG_MP4_DOUBLE_WORD: usize = 8;
/// Bytes in the forced 64-bit `mdat` header, as a slice length.
const BIG_MP4_MDAT_HEADER_BYTES: usize = 16;
/// `mdhd.language` undetermined (ISO 639-2 `und` packed per §8.4.2.3).
const BIG_MP4_LANGUAGE_UNDETERMINED: u16 = 0x55c4;
/// Generated video width, 16.16 fixed point.
const BIG_MP4_VIDEO_WIDTH: u32 = 1920 << 16;
/// Generated video height, 16.16 fixed point.
const BIG_MP4_VIDEO_HEIGHT: u32 = 1080 << 16;
/// `mdat` header written in the forced 64-bit `largesize` form: 4-byte size
/// marker, 4-byte type, 8-byte largesize.
const BIG_MP4_MDAT_HEADER_LEN: u64 = 16;
/// `size` value that announces a 64-bit `largesize` (§4.2).
const BIG_MP4_SIZE_MARKER: u32 = 1;
/// Minor version of the generated `ftyp`.
const BIG_MP4_FTYP_MINOR: u32 = 0;
/// `hdlr.handler_type` of the generated video track.
const BIG_MP4_VIDEO_HANDLER: &[u8; 4] = b"vide";
/// `hdlr.handler_type` of the generated audio track.
const BIG_MP4_AUDIO_HANDLER: &[u8; 4] = b"soun";
/// The public fixture whose sample entries the generated tracks copy.
const BIG_MP4_SOURCE_ENTRY: &str = "mp4/progressive/av-prog.mp4";

/// Knobs for [`write_big_mp4`].
#[derive(Debug, Clone)]
pub struct BigMp4Options {
    /// Minimum declared file size; a sparse `mdat` fills the difference.
    pub target_bytes: u64,
    /// Samples in each generated track.
    pub sample_count: u32,
    /// Write `moov` before `mdat` (fast start) or after it.
    pub moov_first: bool,
    /// Video track timescale.
    pub video_timescale: u32,
    /// Audio track timescale.
    pub audio_timescale: u32,
    /// Video sample duration, in the video timescale.
    pub video_delta: u32,
    /// Audio sample duration, in the audio timescale.
    pub audio_delta: u32,
    /// Nominal bytes in one video sample.
    pub video_sample_bytes: u32,
    /// Nominal bytes in one audio sample.
    pub audio_sample_bytes: u32,
}

impl Default for BigMp4Options {
    fn default() -> Self {
        Self {
            target_bytes: 4 * GIB,
            sample_count: 200_000,
            moov_first: true,
            video_timescale: 90_000,
            audio_timescale: 48_000,
            video_delta: 3_000,
            audio_delta: 1_024,
            video_sample_bytes: 1_000,
            audio_sample_bytes: 200,
        }
    }
}

/// What [`write_big_mp4`] produced.
#[derive(Debug, Clone)]
pub struct BigMp4 {
    /// Exact file length in bytes.
    pub bytes: u64,
    /// Serialized `moov` length in bytes.
    pub moov_bytes: u64,
    /// Whether `moov` was written before `mdat`.
    pub moov_first: bool,
    /// `(track id, samples)` in track order.
    pub samples_per_track: Vec<(u32, u64)>,
}

/// Write a real progressive MP4 of at least [`BigMp4Options::target_bytes`]:
/// a real `ftyp`, a real `moov` with two tracks and full sample tables, and a
/// sparse `mdat` whose body is never written.
pub fn write_big_mp4(path: &Path, opts: &BigMp4Options) -> BigMp4 {
    use broadcast_common::timescale::Timescale;
    use isobmff::TrackId;
    use isobmff::init_segment::{MovieBox, MovieHeaderBox};
    use isobmff::sample_table::{build_stsc, build_stts};
    use isobmff::segments::FileTypeBox;

    let (video_entry, audio_entry) = big_mp4_source_entries();
    let chunk_count = opts.sample_count.div_ceil(BIG_MP4_CHUNK_SAMPLES);
    let chunk_samples = vec![BIG_MP4_CHUNK_SAMPLES; chunk_count as usize];
    let video_deltas = vec![opts.video_delta; opts.sample_count as usize];
    let audio_deltas = vec![opts.audio_delta; opts.sample_count as usize];
    let video_stts = build_stts(&video_deltas);
    let audio_stts = build_stts(&audio_deltas);
    let video_stsc = expect("video stsc", build_stsc(&chunk_samples));
    let audio_stsc = expect("audio stsc", build_stsc(&chunk_samples));
    let video_duration = u64::from(opts.sample_count) * u64::from(opts.video_delta);
    let audio_duration = u64::from(opts.sample_count) * u64::from(opts.audio_delta);
    let video_chunk_bytes = u64::from(BIG_MP4_CHUNK_SAMPLES) * u64::from(opts.video_sample_bytes);
    let audio_chunk_bytes = u64::from(BIG_MP4_CHUNK_SAMPLES) * u64::from(opts.audio_sample_bytes);
    let video_total = u64::from(chunk_count) * video_chunk_bytes;
    let audio_total = u64::from(chunk_count) * audio_chunk_bytes;
    let required = video_total.saturating_add(audio_total);
    let movie_timescale = expect(
        "movie timescale",
        Timescale::try_new(BIG_MP4_MOVIE_TIMESCALE),
    );
    let mvhd_duration = movie_duration(
        video_duration,
        opts.video_timescale,
        audio_duration,
        opts.audio_timescale,
        BIG_MP4_MOVIE_TIMESCALE,
    );
    let mvhd = MovieHeaderBox {
        version: 0,
        flags: 0,
        creation_time: 0,
        modification_time: 0,
        timescale: movie_timescale,
        duration: mvhd_duration,
        rate: BIG_MP4_RATE_UNITY,
        volume: BIG_MP4_VOLUME_UNITY,
        matrix: BIG_MP4_IDENTITY_MATRIX,
        next_track_id: TrackId::new(BIG_MP4_NEXT_TRACK_ID),
    };
    let ftyp = FileTypeBox {
        major_brand: *b"iso5",
        minor_version: BIG_MP4_FTYP_MINOR,
        compatible_brands: vec![*b"iso5", *b"iso6", *b"mp41"],
    };
    let ftyp_bytes = expect("serialize ftyp", ftyp.try_to_bytes());
    let zero_video = vec![0u64; chunk_count as usize];
    let zero_audio = vec![0u64; chunk_count as usize];
    let build = |video_offsets: &[u64], audio_offsets: &[u64]| {
        let video = big_mp4_track(BigMp4Track {
            track_id: BIG_MP4_VIDEO_TRACK,
            handler: *BIG_MP4_VIDEO_HANDLER,
            timescale: opts.video_timescale,
            delta: video_duration,
            sample_bytes: opts.video_sample_bytes,
            sample_count: opts.sample_count,
            entry: &video_entry,
            stts: &video_stts,
            stsc: &video_stsc,
            chunk_offsets: video_offsets,
            audio: false,
        });
        let audio = big_mp4_track(BigMp4Track {
            track_id: BIG_MP4_AUDIO_TRACK,
            handler: *BIG_MP4_AUDIO_HANDLER,
            timescale: opts.audio_timescale,
            delta: audio_duration,
            sample_bytes: opts.audio_sample_bytes,
            sample_count: opts.sample_count,
            entry: &audio_entry,
            stts: &audio_stts,
            stsc: &audio_stsc,
            chunk_offsets: audio_offsets,
            audio: true,
        });
        MovieBox::new(mvhd.clone(), vec![video, audio], None, vec![])
    };
    let provisional = build(&zero_video, &zero_audio);
    let moov_len = provisional.serialized_len() as u64;
    let payload_start = u64::try_from(ftyp_bytes.len()).unwrap_or(0)
        + if opts.moov_first { moov_len } else { 0 }
        + BIG_MP4_MDAT_HEADER_LEN;
    let video_offsets: Vec<u64> = (0..chunk_count)
        .map(|index| payload_start + u64::from(index) * video_chunk_bytes)
        .collect();
    let audio_offsets: Vec<u64> = (0..chunk_count)
        .map(|index| payload_start + video_total + u64::from(index) * audio_chunk_bytes)
        .collect();
    let movie = build(&video_offsets, &audio_offsets);
    let moov_bytes = expect("serialize moov", movie.try_to_bytes());
    let overhead = u64::try_from(ftyp_bytes.len()).unwrap_or(0)
        + u64::try_from(moov_bytes.len()).unwrap_or(0)
        + BIG_MP4_MDAT_HEADER_LEN;
    let body_len = opts.target_bytes.saturating_sub(overhead).max(required);
    let mut file = expect("create big mp4", std::fs::File::create(path));
    expect("write ftyp", file.write_all(&ftyp_bytes));
    if opts.moov_first {
        expect("write moov", file.write_all(&moov_bytes));
        expect(
            "write mdat header",
            write_big_mp4_mdat_header(&mut file, body_len),
        );
        expect("extend sparse mdat", file.set_len(payload_start + body_len));
    } else {
        expect(
            "write mdat header",
            write_big_mp4_mdat_header(&mut file, body_len),
        );
        let mdat_end =
            u64::try_from(ftyp_bytes.len()).unwrap_or(0) + BIG_MP4_MDAT_HEADER_LEN + body_len;
        expect("extend sparse mdat", file.set_len(mdat_end));
        expect("seek past mdat", file.seek(SeekFrom::Start(mdat_end)));
        expect("write trailing moov", file.write_all(&moov_bytes));
    }
    let bytes = expect("file length", file.metadata()).len();
    BigMp4 {
        bytes,
        moov_bytes: u64::try_from(moov_bytes.len()).unwrap_or(0),
        moov_first: opts.moov_first,
        samples_per_track: vec![
            (BIG_MP4_VIDEO_TRACK, u64::from(opts.sample_count)),
            (BIG_MP4_AUDIO_TRACK, u64::from(opts.sample_count)),
        ],
    }
}

struct BigMp4Track<'a> {
    track_id: u32,
    handler: [u8; 4],
    timescale: u32,
    delta: u64,
    sample_bytes: u32,
    sample_count: u32,
    entry: &'a isobmff::SampleEntryVariant,
    stts: &'a isobmff::timing::TimeToSampleBox,
    stsc: &'a isobmff::init_segment::SampleToChunkBox,
    chunk_offsets: &'a [u64],
    audio: bool,
}

fn big_mp4_track(spec: BigMp4Track<'_>) -> isobmff::init_segment::TrackBox {
    use broadcast_common::timescale::Timescale;
    use isobmff::init_segment::{
        ChunkLargeOffsetBox, DataEntry, DataEntryUrlBox, DataInformationBox, DataReferenceBox,
        HandlerBox, MediaBox, MediaHeaderBox, MediaInformationBox, SampleSizeBox, SampleTableBox,
        SoundMediaHeaderBox, StblChild, TrackBox, TrackHeaderBox, VideoMediaHeaderBox,
    };
    use isobmff::{SampleDescriptionBox, TrackId};

    let stbl = SampleTableBox {
        children: vec![
            StblChild::Stsd(SampleDescriptionBox {
                version: 0,
                flags: 0,
                entries: vec![spec.entry.clone()],
            }),
            StblChild::Stts(spec.stts.clone()),
            StblChild::Stsc(spec.stsc.clone()),
            StblChild::Stsz(SampleSizeBox {
                version: 0,
                flags: 0,
                sample_size: spec.sample_bytes,
                sample_count: spec.sample_count,
                entries: vec![],
            }),
            StblChild::Co64(ChunkLargeOffsetBox {
                version: 0,
                flags: 0,
                entries: spec.chunk_offsets.to_vec(),
            }),
        ],
    };
    let dinf = DataInformationBox::new(
        Some(DataReferenceBox {
            version: 0,
            flags: 0,
            entries: vec![DataEntry::Url(DataEntryUrlBox {
                version: 0,
                flags: 1,
                location: vec![],
            })],
        }),
        vec![],
    );
    let minf = if spec.audio {
        MediaInformationBox::new(
            None,
            Some(SoundMediaHeaderBox {
                version: 0,
                flags: 0,
                balance: 0,
            }),
            Some(dinf),
            Some(stbl),
            vec![],
        )
    } else {
        MediaInformationBox::new(
            Some(VideoMediaHeaderBox {
                version: 0,
                flags: 1,
                graphicsmode: 0,
                opcolor: [0, 0, 0],
            }),
            None,
            Some(dinf),
            Some(stbl),
            vec![],
        )
    };
    let mdia = MediaBox::new(
        Some(MediaHeaderBox {
            version: 0,
            flags: 0,
            creation_time: 0,
            modification_time: 0,
            timescale: expect("track timescale", Timescale::try_new(spec.timescale)),
            duration: spec.delta,
            language: BIG_MP4_LANGUAGE_UNDETERMINED,
        }),
        Some(HandlerBox {
            version: 0,
            flags: 0,
            pre_defined: 0,
            reserved: [0u8; 12],
            handler_type: spec.handler,
            name: vec![],
        }),
        Some(minf),
        vec![],
    );
    TrackBox::new(
        TrackHeaderBox {
            version: 0,
            flags: 3,
            creation_time: 0,
            modification_time: 0,
            track_id: TrackId::new(spec.track_id),
            duration: spec.delta,
            layer: 0,
            alternate_group: 0,
            volume: if spec.audio {
                BIG_MP4_TRACK_VOLUME_UNITY
            } else {
                0
            },
            matrix: BIG_MP4_IDENTITY_MATRIX,
            width: if spec.audio { 0 } else { BIG_MP4_VIDEO_WIDTH },
            height: if spec.audio { 0 } else { BIG_MP4_VIDEO_HEIGHT },
        },
        None,
        Some(mdia),
        vec![],
    )
}

fn movie_duration(
    video_duration: u64,
    video_timescale: u32,
    audio_duration: u64,
    audio_timescale: u32,
    movie_timescale: u32,
) -> u64 {
    let video = rescale_ceil(video_duration, video_timescale, movie_timescale);
    let audio = rescale_ceil(audio_duration, audio_timescale, movie_timescale);
    video.max(audio)
}

fn rescale_ceil(value: u64, from: u32, to: u32) -> u64 {
    if from == to {
        return value;
    }
    let numerator = u128::from(value) * u128::from(to);
    let scaled = numerator.div_ceil(u128::from(from));
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

fn write_big_mp4_mdat_header(out: &mut impl Write, body_len: u64) -> std::io::Result<()> {
    let mut header = [0u8; BIG_MP4_MDAT_HEADER_BYTES];
    header[..BIG_MP4_WORD].copy_from_slice(&BIG_MP4_SIZE_MARKER.to_be_bytes());
    header[BIG_MP4_WORD..BIG_MP4_DOUBLE_WORD].copy_from_slice(&isobmff::segments::MDAT);
    header[BIG_MP4_DOUBLE_WORD..]
        .copy_from_slice(&(BIG_MP4_MDAT_HEADER_LEN + body_len).to_be_bytes());
    out.write_all(&header)
}

fn big_mp4_source_entries() -> (isobmff::SampleEntryVariant, isobmff::SampleEntryVariant) {
    let bytes = read_source(&test_fixtures::path(BIG_MP4_SOURCE_ENTRY));
    let moov =
        isobmff::box_types::find_box(&bytes, &isobmff::init_segment::MOOV).expect("source moov");
    let movie = expect("parse source moov", isobmff::MovieBox::parse(moov));
    let entry = |handler: &[u8; 4]| -> isobmff::SampleEntryVariant {
        movie
            .tracks
            .iter()
            .find(|trak| {
                trak.mdia
                    .as_ref()
                    .and_then(|mdia| mdia.hdlr.as_ref())
                    .is_some_and(|hdlr| &hdlr.handler_type == handler)
            })
            .and_then(first_sample_entry)
            .cloned()
            .expect("source sample entry")
    };
    (entry(BIG_MP4_VIDEO_HANDLER), entry(BIG_MP4_AUDIO_HANDLER))
}

fn first_sample_entry(
    trak: &isobmff::init_segment::TrackBox,
) -> Option<&isobmff::SampleEntryVariant> {
    trak.mdia
        .as_ref()?
        .minf
        .as_ref()?
        .stbl
        .as_ref()?
        .children
        .iter()
        .find_map(|child| match child {
            isobmff::init_segment::StblChild::Stsd(stsd) => stsd.entries.first(),
            _ => None,
        })
}

/// Number of fragments in the default generated fragmented MP4.
const BIG_FMP4_FRAGMENTS: u32 = 1_024;
/// Nominal bytes in the generated fragmented MP4's one sample per fragment.
const BIG_FMP4_SAMPLE_BYTES: u32 = 4 * 1024 * 1024;
/// The generated fragment's sample duration, in the video timescale.
const BIG_FMP4_SAMPLE_DELTA: u32 = 3_000;
/// Bytes of a 32-bit `mdat` header.
const BIG_FMP4_MDAT_HEADER_BYTES: u64 = 8;
/// Entries written into the generated `tfra`: one per evenly spaced fragment.
const BIG_FMP4_MFRA_ENTRIES: u32 = 16;

/// Knobs for [`write_big_fmp4`].
#[derive(Debug, Clone)]
pub struct BigFmp4Options {
    /// Minimum file size; the sample count is raised until it is reached.
    pub target_bytes: u64,
    /// Fragments to write (at least).
    pub fragment_count: u32,
    /// Bytes in each fragment's one sample (sparse, never written).
    pub sample_bytes: u32,
    /// Write a trailing `mfra` with `tfra` entries for evenly spaced fragments.
    pub with_mfra: bool,
    /// Write a CENC-protected file (`cenc`, AES-CTR) through the crypto and
    /// CMAF muxer stages, one sample at a time, instead of the sparse
    /// cleartext fragments (see [`write_big_fmp4`]). Nothing is sparse then,
    /// so `with_mfra` is ignored and the result's `moof_offsets` is empty.
    pub protect: bool,
}

impl Default for BigFmp4Options {
    fn default() -> Self {
        Self {
            target_bytes: 4 * GIB,
            fragment_count: BIG_FMP4_FRAGMENTS,
            sample_bytes: BIG_FMP4_SAMPLE_BYTES,
            with_mfra: true,
            protect: false,
        }
    }
}

/// What [`write_big_fmp4`] produced.
#[derive(Debug, Clone)]
pub struct BigFmp4 {
    /// Exact file length in bytes.
    pub bytes: u64,
    /// Serialized `moov` length in bytes.
    pub moov_bytes: u64,
    /// Fragments written.
    pub fragment_count: u32,
    /// Bytes in each sample (the sparse `mdat` payload).
    pub sample_bytes: u32,
    /// Whether a trailing `mfra` was written.
    pub with_mfra: bool,
    /// Bytes of each fragment's `moof`.
    pub moof_bytes: u64,
    /// Each fragment's `moof` offset, in file order.
    pub moof_offsets: Vec<u64>,
    /// `(track id, samples written)` in track order.
    pub samples_per_track: Vec<(u32, u64)>,
    /// The CENC report, when [`BigFmp4Options::protect`] asked for a protected
    /// file.
    pub cenc: Option<BigFmp4Cenc>,
}

/// What a CENC-protected [`write_big_fmp4`] run reports about its cleartext
/// samples, for a caller checking that a decrypt reproduced them.
#[derive(Debug, Clone)]
pub struct BigFmp4Cenc {
    /// Track the protected samples belong to.
    pub track_id: u32,
    /// The Key ID every fragment's `tenc` carries.
    pub key_id: [u8; BIG_FMP4_KEY_LEN],
    /// The content key that protects the fragments.
    pub content_key: [u8; BIG_FMP4_KEY_LEN],
    /// CRC-32/MPEG-2 of each of the first [`BigFmp4Cenc::hashes`] cleartext
    /// samples, in decode order.
    pub head_hashes: Vec<u32>,
    /// CRC-32/MPEG-2 of each of the last [`BigFmp4Cenc::hashes`] cleartext
    /// samples, in decode order.
    pub tail_hashes: Vec<u32>,
    /// How many leading and trailing samples each hash list covers.
    pub hashes: usize,
}

/// Write a fragmented MP4 of at least [`BigFmp4Options::target_bytes`].
///
/// With [`BigFmp4Options::protect`] the file is real: `ftyp` + a protected
/// `moov`, then one `moof`+`mdat` per sample whose **ciphertext** is written,
/// produced by [`transmux::cenc_encrypt::CencEncryptStage`] feeding
/// [`transmux::CmafMuxStage`] one sample at a time (`cenc`, AES-CTR, subsample
/// encryption for the video track), so the generator itself never holds the
/// file. Without it the fragments' payloads are left sparse and never written.
pub fn write_big_fmp4(path: &Path, opts: &BigFmp4Options) -> BigFmp4 {
    if opts.protect {
        write_big_fmp4_cenc(path, opts)
    } else {
        write_big_fmp4_sparse(path, opts)
    }
}

fn write_big_fmp4_sparse(path: &Path, opts: &BigFmp4Options) -> BigFmp4 {
    use broadcast_common::timescale::Timescale;
    use isobmff::init_segment::{MovieExtendsBox, TrackExtendsBox};
    use isobmff::movie_fragment::{
        MovieFragmentBox, MovieFragmentHeaderBox, SAMPLE_FLAGS_SYNC, TFHD_DEFAULT_BASE_IS_MOOF,
        TRUN_DATA_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT, TRUN_SAMPLE_FLAGS_PRESENT,
        TRUN_SAMPLE_SIZE_PRESENT, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox,
        TrackFragmentHeaderBox, TrackFragmentRunBox, TrunSample,
    };
    use isobmff::sample_table::{build_stsc, build_stts};
    use isobmff::segments::FileTypeBox;
    use isobmff::timing::{
        MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox,
        TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry,
    };
    use isobmff::{MovieBox, MovieHeaderBox, TrackId};

    let (video_entry, audio_entry) = big_mp4_source_entries();
    let _ = &audio_entry;
    let sample_bytes = opts.sample_bytes;
    let per_fragment = BIG_FMP4_MDAT_HEADER_BYTES + u64::from(sample_bytes);
    let needed = opts.target_bytes / per_fragment.max(1);
    let count = opts
        .fragment_count
        .max(u32::try_from(needed).unwrap_or(u32::MAX));
    let movie_timescale = expect(
        "movie timescale",
        Timescale::try_new(BIG_MP4_MOVIE_TIMESCALE),
    );
    let total_duration = u64::from(count) * u64::from(BIG_FMP4_SAMPLE_DELTA);
    let mvhd = MovieHeaderBox {
        version: 0,
        flags: 0,
        creation_time: 0,
        modification_time: 0,
        timescale: movie_timescale,
        duration: rescale_ceil(total_duration, 90_000, BIG_MP4_MOVIE_TIMESCALE),
        rate: BIG_MP4_RATE_UNITY,
        volume: BIG_MP4_VOLUME_UNITY,
        matrix: BIG_MP4_IDENTITY_MATRIX,
        next_track_id: TrackId::new(BIG_MP4_VIDEO_TRACK + 1),
    };
    let empty_stts = build_stts(&[]);
    let empty_stsc = expect("empty stsc", build_stsc(&[]));
    let trak = big_mp4_track(BigMp4Track {
        track_id: BIG_MP4_VIDEO_TRACK,
        handler: *BIG_MP4_VIDEO_HANDLER,
        timescale: 90_000,
        delta: 0,
        sample_bytes: 0,
        sample_count: 0,
        entry: &video_entry,
        stts: &empty_stts,
        stsc: &empty_stsc,
        chunk_offsets: &[],
        audio: false,
    });
    let mvex = MovieExtendsBox::new(
        vec![TrackExtendsBox {
            version: 0,
            flags: 0,
            track_id: TrackId::new(BIG_MP4_VIDEO_TRACK),
            default_sample_description_index: 1,
            default_sample_duration: BIG_FMP4_SAMPLE_DELTA,
            default_sample_size: sample_bytes,
            default_sample_flags: SAMPLE_FLAGS_SYNC,
        }],
        vec![],
    );
    let movie = MovieBox::new(mvhd, vec![trak], Some(mvex), vec![]);
    let moov_bytes = expect("serialize fragmented moov", movie.try_to_bytes());
    let ftyp = FileTypeBox {
        major_brand: *b"iso5",
        minor_version: BIG_MP4_FTYP_MINOR,
        compatible_brands: vec![*b"iso5", *b"iso6", *b"mp41"],
    };
    let ftyp_bytes = expect("serialize ftyp", ftyp.try_to_bytes());

    let mut moof = MovieFragmentBox::new(
        MovieFragmentHeaderBox::new(1),
        vec![TrackFragmentBox::new(
            TrackFragmentHeaderBox {
                flags: TFHD_DEFAULT_BASE_IS_MOOF,
                track_id: TrackId::new(BIG_MP4_VIDEO_TRACK),
                base_data_offset: None,
                sample_description_index: None,
                default_sample_duration: None,
                default_sample_size: None,
                default_sample_flags: None,
            },
            Some(TrackFragmentBaseMediaDecodeTimeBox::new_v1(0)),
            vec![TrackFragmentRunBox {
                version: 1,
                tr_flags: TRUN_DATA_OFFSET_PRESENT
                    | TRUN_SAMPLE_DURATION_PRESENT
                    | TRUN_SAMPLE_SIZE_PRESENT
                    | TRUN_SAMPLE_FLAGS_PRESENT,
                data_offset: Some(0),
                first_sample_flags: None,
                samples: vec![TrunSample {
                    sample_duration: Some(BIG_FMP4_SAMPLE_DELTA),
                    sample_size: Some(sample_bytes),
                    sample_flags: Some(SAMPLE_FLAGS_SYNC),
                    sample_composition_time_offset: None,
                }],
            }],
        )],
    );
    let moof_len = moof.serialized_len();
    let data_offset = u32::try_from(moof_len)
        .ok()
        .and_then(|len| len.checked_add(BIG_FMP4_MDAT_HEADER_BYTES as u32))
        .and_then(|offset| i32::try_from(offset).ok())
        .expect("the generated moof fits an i32 data offset");
    for traf in &mut moof.traf {
        for run in &mut traf.trun {
            run.data_offset = Some(data_offset);
        }
    }

    let mut file = expect("create big fmp4", std::fs::File::create(path));
    expect("write ftyp", file.write_all(&ftyp_bytes));
    expect("write moov", file.write_all(&moov_bytes));
    let mut offset = u64::try_from(ftyp_bytes.len() + moov_bytes.len()).expect("header lengths");
    let mut moof_offsets = Vec::with_capacity(count as usize);
    let mut mdat_header = [0u8; BIG_FMP4_MDAT_HEADER_BYTES as usize];
    mdat_header[..BIG_MP4_WORD].copy_from_slice(
        &u32::try_from(BIG_FMP4_MDAT_HEADER_BYTES + u64::from(sample_bytes))
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    mdat_header[BIG_MP4_WORD..BIG_MP4_DOUBLE_WORD].copy_from_slice(&isobmff::segments::MDAT);
    for index in 0..count {
        if let Some(slot) = moof.traf.first_mut().and_then(|traf| traf.tfdt.as_mut()) {
            *slot = TrackFragmentBaseMediaDecodeTimeBox::new_v1(
                u64::from(index) * u64::from(BIG_FMP4_SAMPLE_DELTA),
            );
        }
        let bytes = expect("serialize moof", moof.try_to_bytes());
        moof_offsets.push(offset);
        expect("write moof", file.write_all(&bytes));
        expect("write mdat header", file.write_all(&mdat_header));
        let next = offset
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(0))
            .saturating_add(per_fragment);
        expect("extend sparse mdat", file.seek(SeekFrom::Start(next)));
        offset = next;
    }
    if opts.with_mfra {
        let stride = (count / BIG_FMP4_MFRA_ENTRIES).max(1);
        let entries: Vec<TrackFragmentRandomAccessEntry> = (0..count)
            .step_by(stride as usize)
            .take(BIG_FMP4_MFRA_ENTRIES as usize)
            .filter_map(|index| {
                Some(TrackFragmentRandomAccessEntry {
                    time: u64::from(index) * u64::from(BIG_FMP4_SAMPLE_DELTA),
                    moof_offset: *moof_offsets.get(index as usize)?,
                    traf_number: 1,
                    trun_number: 1,
                    sample_number: 1,
                })
            })
            .collect();
        let tfra = TrackFragmentRandomAccessBox {
            version: 1,
            flags: 0,
            track_id: TrackId::new(BIG_MP4_VIDEO_TRACK),
            length_size_of_traf_num: 0,
            length_size_of_trun_num: 0,
            length_size_of_sample_num: 0,
            entries,
        };
        let mfra_len = 8
            + tfra.serialized_len()
            + MovieFragmentRandomAccessOffsetBox {
                version: 0,
                flags: 0,
                parent_size: 0,
            }
            .serialized_len();
        let mfra = MovieFragmentRandomAccessBox {
            tfra: vec![tfra],
            mfro: MovieFragmentRandomAccessOffsetBox {
                version: 0,
                flags: 0,
                parent_size: u32::try_from(mfra_len).expect("mfra length fits u32"),
            },
        };
        let bytes = expect("serialize mfra", mfra.try_to_bytes());
        expect("write mfra", file.write_all(&bytes));
        offset = offset.saturating_add(u64::try_from(bytes.len()).unwrap_or(0));
    }
    expect("set final length", file.set_len(offset));
    expect("flush big fmp4", file.flush());
    let bytes = expect("file length", file.metadata()).len();
    BigFmp4 {
        bytes,
        moov_bytes: u64::try_from(moov_bytes.len()).unwrap_or(0),
        fragment_count: count,
        sample_bytes,
        with_mfra: opts.with_mfra,
        moof_bytes: moof_len as u64,
        moof_offsets,
        samples_per_track: vec![(BIG_MP4_VIDEO_TRACK, u64::from(count))],
        cenc: None,
    }
}

// ── CENC-protected fragmented MP4 ───────────────────────────────────────────

/// Bytes in one CENC Key ID / content key.
const BIG_FMP4_KEY_LEN: usize = 16;
/// The Key ID the generated CENC fragments announce.
const BIG_FMP4_CENC_KEY_ID: [u8; BIG_FMP4_KEY_LEN] = [
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];
/// The content key that protects the generated CENC fragments.
const BIG_FMP4_CENC_CONTENT_KEY: [u8; BIG_FMP4_KEY_LEN] = [
    0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf,
];
/// How many leading and trailing cleartext samples the generated CENC report
/// hashes.
const BIG_FMP4_CENC_HASHES: usize = 8;
/// First byte of the filler SEI payload's repeating pattern: keeps every
/// payload byte non-zero, so an Annex B round-trip of the cleartext cannot
/// mistake a run for a start-code prefix.
const BIG_FMP4_FILLER_PATTERN_MIN: u8 = 0x40;
/// One past the filler SEI payload's last pattern byte.
const BIG_FMP4_FILLER_PATTERN_END: u8 = 0xc0;
/// Bytes of the per-sample variation written at the start of the filler SEI
/// payload (the protected region), so no two samples are identical.
const BIG_FMP4_CENC_VARY_BYTES: usize = 8;
/// Width of a length-prefixed NAL unit's length field.
const BIG_FMP4_NAL_LENGTH_SIZE: usize = 4;
/// AVC SEI NAL unit type (`nal_unit_type` 6, ITU-T H.264 Table 7-1).
const BIG_FMP4_AVC_SEI: u8 = 6;
/// Lowest AVC VCL `nal_unit_type` (coded slice, ITU-T H.264 Table 7-1).
const BIG_FMP4_AVC_VCL_MIN: u8 = 1;
/// Highest AVC VCL `nal_unit_type` (IDR slice, ITU-T H.264 Table 7-1).
const BIG_FMP4_AVC_VCL_MAX: u8 = 5;
/// Mask selecting an AVC NAL unit's `nal_unit_type` from its first byte
/// (ITU-T H.264 §7.3.1: `forbidden_zero_bit` + 2-bit `nal_ref_idc` above it).
const BIG_FMP4_AVC_NAL_TYPE_MASK: u8 = 0x1f;
/// The generated track's media timescale.
const BIG_FMP4_VIDEO_TIMESCALE: u32 = 90_000;
/// The generated track's id.
const BIG_FMP4_CENC_TRACK: u32 = 1;
/// Byte value XORed into each variation byte; `0x00` varies to `0xff`, so a
/// varied byte is never zero either.
const BIG_FMP4_CENC_VARY_XOR: u8 = 0xff;
/// Sequence number of the first generated `moof`.
const BIG_FMP4_CENC_FIRST_SEQUENCE: u32 = 1;
/// Bits per byte, for extracting the per-index variation bytes.
const BIG_FMP4_BITS_PER_BYTE: u32 = 8;

/// Write a CENC-protected fragmented MP4 of at least
/// [`BigFmp4Options::target_bytes`], one sample at a time.
///
/// Every sample is the committed source fixture's keyframe access unit with a
/// filler SEI NAL inserted before its first slice (so the picture, its SPS/PPS
/// and its sync flag stay real, and the muxer never has to add parameter sets
/// itself) and its payload padded to [`BigFmp4Options::sample_bytes`]. Each
/// sample's variation bytes make it distinct, so a decrypt that applied the
/// wrong IV cannot pass a hash comparison.
fn write_big_fmp4_cenc(path: &Path, opts: &BigFmp4Options) -> BigFmp4 {
    use broadcast_common::stage::{Stage, Timestamp};
    use broadcast_common::timescale::Timescale;
    use isobmff::cenc::CencScheme;
    use transmux::cenc_encrypt::{CencEncryptStage, EncryptConfig, SubsamplePolicy};
    use transmux::mux_stage::{MuxInput, MuxOutput};
    use transmux::{CmafMuxStage, FragmentCut, TrackId};

    let timescale = expect(
        "generated video timescale",
        Timescale::try_new(BIG_FMP4_VIDEO_TIMESCALE),
    );
    let source = read_source(&test_fixtures::path(BIG_MP4_SOURCE_ENTRY));
    let media = expect(
        "demux the source fixture",
        crate::collect::media_from_mp4(&source, crate::collect::CollectMode::Seekable),
    );
    let video = expect(
        "the source fixture carries an AVC track",
        media
            .tracks
            .iter()
            .find(|track| matches!(track.spec.config, CodecConfig::Avc { .. }))
            .ok_or("no AVC track"),
    );
    let (template, variation_at) = cenc_sample_template(&video.samples, opts.sample_bytes);
    let mut spec = video.spec.clone();
    spec.track_id = TrackId::new(BIG_FMP4_CENC_TRACK);
    spec.timescale = timescale;

    let needed = opts
        .target_bytes
        .div_ceil(u64::from(opts.sample_bytes).max(1));
    let count = opts
        .fragment_count
        .max(u32::try_from(needed).unwrap_or(u32::MAX));

    let mut encrypt = expect(
        "build the CENC encrypt stage",
        CencEncryptStage::new(
            BIG_FMP4_CENC_CONTENT_KEY,
            EncryptConfig::new(
                CencScheme::Cenc,
                BIG_FMP4_CENC_KEY_ID,
                SubsamplePolicy::Video,
            ),
        ),
    );
    let mut mux = CmafMuxStage::new(
        expect(
            "movie timescale",
            Timescale::try_new(BIG_MP4_MOVIE_TIMESCALE),
        ),
        FragmentCut::OnFlush,
        BIG_FMP4_CENC_FIRST_SEQUENCE,
    );
    expect(
        "announce the source track to the encrypt stage",
        encrypt.feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO),
    );
    let protected = expect(
        "the encrypt stage emits the announced track",
        encrypt
            .poll()
            .and_then(|input| match input {
                MuxInput::Track(spec) => Some(*spec),
                _ => None,
            })
            .ok_or("the encrypt stage dropped the track announcement"),
    );
    let protection = expect(
        "the encrypt stage protects the announced track",
        protected
            .protection
            .clone()
            .ok_or("the track arrived unprotected"),
    );
    expect(
        "announce the protected track to the CMAF muxer",
        mux.feed(MuxInput::Track(Box::new(protected)), Timestamp::ZERO),
    );

    let mut file = BufWriter::new(expect("create big CENC fmp4", File::create(path)));
    let mut written = 0u64;
    let mut moof_bytes = 0u64;
    let mut head_hashes = Vec::with_capacity(BIG_FMP4_CENC_HASHES);
    let mut tail_hashes: std::collections::VecDeque<u32> =
        std::collections::VecDeque::with_capacity(BIG_FMP4_CENC_HASHES);
    for index in 0..count {
        let sample = cenc_sample(&template, variation_at, index);
        let hash = broadcast_common::crc32_mpeg2::compute(&sample.data);
        if head_hashes.len() < BIG_FMP4_CENC_HASHES {
            head_hashes.push(hash);
        } else {
            if tail_hashes.len() == BIG_FMP4_CENC_HASHES {
                tail_hashes.pop_front();
            }
            tail_hashes.push_back(hash);
        }
        expect(
            "encrypt one sample",
            encrypt.feed(
                MuxInput::Sample {
                    track: TrackId::new(BIG_FMP4_CENC_TRACK),
                    sample,
                },
                Timestamp::ZERO,
            ),
        );
        while let Some(input) = encrypt.poll() {
            if let MuxInput::Sample { track, sample } = input {
                expect(
                    "mux one protected sample",
                    mux.feed(MuxInput::Sample { track, sample }, Timestamp::ZERO),
                );
            }
        }
        expect(
            "close the fragment",
            mux.feed(MuxInput::Flush, Timestamp::ZERO),
        );
        while let Some(output) = mux.poll() {
            match output {
                MuxOutput::Init(init) => {
                    let protected_init = expect(
                        "protect the init segment",
                        transmux::cenc_segment::protect_init_segment(
                            &init,
                            TrackId::new(BIG_FMP4_CENC_TRACK),
                            &protection,
                        ),
                    );
                    written += write_bytes(&mut file, &protected_init);
                }
                MuxOutput::Segment(segment) => {
                    if moof_bytes == 0 {
                        moof_bytes = moof_length(&segment.bytes);
                    }
                    written += write_bytes(&mut file, &segment.bytes);
                }
                _ => {}
            }
        }
    }
    expect("flush big CENC fmp4", file.flush());
    let bytes = expect("file length", file.get_ref().metadata()).len();
    assert_eq!(bytes, written, "the CENC fragment writer lost bytes");
    BigFmp4 {
        bytes,
        moov_bytes: 0,
        fragment_count: count,
        sample_bytes: opts.sample_bytes,
        with_mfra: false,
        moof_bytes,
        moof_offsets: Vec::new(),
        samples_per_track: vec![(BIG_FMP4_CENC_TRACK, u64::from(count))],
        cenc: Some(BigFmp4Cenc {
            track_id: BIG_FMP4_CENC_TRACK,
            key_id: BIG_FMP4_CENC_KEY_ID,
            content_key: BIG_FMP4_CENC_CONTENT_KEY,
            head_hashes,
            tail_hashes: tail_hashes.into_iter().collect(),
            hashes: BIG_FMP4_CENC_HASHES,
        }),
    }
}

/// Byte length of the first `moof` box in a CMAF segment.
fn moof_length(segment: &[u8]) -> u64 {
    isobmff::box_types::find_box(segment, &isobmff::segments::MOOF)
        .map(|moof| u64::try_from(moof.len()).unwrap_or(0))
        .unwrap_or(0)
}

/// The cleartext sample every generated fragment carries, before its per-index
/// variation is applied: the source fixture's keyframe access unit with a
/// filler SEI NAL inserted before its first VCL NAL and sized so the whole
/// sample is `sample_bytes`. Returns the template and the byte offset of the
/// filler payload's first variation byte.
fn cenc_sample_template(samples: &[Sample], sample_bytes: u32) -> (Vec<u8>, usize) {
    let keyframe = expect(
        "the source fixture has a keyframe",
        samples
            .iter()
            .find(|sample| sample.flags.is_sync)
            .ok_or("no sync sample"),
    );
    let nals = expect(
        "split the keyframe into NAL units",
        transmux::iter_length_prefixed_nals(&keyframe.data),
    );
    let mut before_vcl = Vec::new();
    let mut from_vcl = Vec::new();
    let mut seen_vcl = false;
    for nal in &nals {
        let first_type = nal.first().map(|byte| byte & BIG_FMP4_AVC_NAL_TYPE_MASK);
        if !seen_vcl
            && first_type
                .is_some_and(|kind| (BIG_FMP4_AVC_VCL_MIN..=BIG_FMP4_AVC_VCL_MAX).contains(&kind))
        {
            seen_vcl = true;
            push_nal(&mut from_vcl, nal);
        } else if seen_vcl {
            push_nal(&mut from_vcl, nal);
        } else {
            push_nal(&mut before_vcl, nal);
        }
    }
    expect(
        "the source fixture's keyframe has a VCL NAL to fill around",
        if seen_vcl { Ok(()) } else { Err("no VCL NAL") },
    );
    let filler_overhead = BIG_FMP4_NAL_LENGTH_SIZE + 1;
    let fixed = before_vcl.len() + filler_overhead + from_vcl.len();
    let filler_payload = expect("sample size", usize::try_from(sample_bytes)).saturating_sub(fixed);
    let mut template = Vec::with_capacity(fixed + filler_payload);
    template.extend_from_slice(&before_vcl);
    template.extend_from_slice(
        &expect(
            "filler NAL length",
            u32::try_from(filler_payload.saturating_add(1)),
        )
        .to_be_bytes(),
    );
    template.push(BIG_FMP4_AVC_SEI);
    let variation_at = template.len();
    let span =
        usize::from(BIG_FMP4_FILLER_PATTERN_END.saturating_sub(BIG_FMP4_FILLER_PATTERN_MIN)).max(1);
    for index in 0..filler_payload {
        let offset = u8::try_from(index % span).unwrap_or(0);
        template.push(BIG_FMP4_FILLER_PATTERN_MIN + offset);
    }
    template.extend_from_slice(&from_vcl);
    (template, variation_at)
}

/// Append one NAL unit to a length-prefixed sample.
fn push_nal(out: &mut Vec<u8>, nal: &[u8]) {
    out.extend_from_slice(&u32::try_from(nal.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(nal);
}

/// One generated cleartext sample: the [`cenc_sample_template`] with this
/// sample's protected variation bytes written at `variation_at`.
fn cenc_sample(template: &[u8], variation_at: usize, index: u32) -> Sample {
    let mut data = template.to_vec();
    for slot in 0..BIG_FMP4_CENC_VARY_BYTES {
        let shift = u32::try_from(slot).unwrap_or(0) * BIG_FMP4_BITS_PER_BYTE;
        let byte = u8::try_from((u64::from(index) >> shift) & u64::from(u8::MAX)).unwrap_or(0)
            ^ BIG_FMP4_CENC_VARY_XOR;
        if let Some(target) = data.get_mut(variation_at.saturating_add(slot)) {
            *target = byte;
        }
    }
    let dts = i64::from(index) * i64::from(BIG_FMP4_SAMPLE_DELTA);
    expect(
        "build a generated sample",
        Sample::new(
            data,
            Some(dts),
            Some(dts),
            Some(BIG_FMP4_SAMPLE_DELTA),
            true,
        ),
    )
}

fn write_bytes(file: &mut impl Write, bytes: &[u8]) -> u64 {
    expect("write big CENC bytes", file.write_all(bytes));
    expect("sum bytes", u64::try_from(bytes.len()))
}

/// Write an MPEG-1/2 Program Stream file of at least `target_bytes` by
/// repeating `source_fixture`'s packs, advancing every pack's SCR and every
/// PES PTS/DTS by one pass span, streaming each pack to disk.
///
/// Live memory is the fixture's own pack list plus one pack's byte image, never
/// the generated file.
pub fn write_big_ps(path: &Path, source_fixture: &Path, target_bytes: u64) -> BigMedia {
    let source = read_source(source_fixture);
    let scan = mpeg_ps::program_stream::scan_packs(&source);
    assert!(
        scan.skipped.is_empty() && scan.remaining.is_empty(),
        "the PS source fixture must walk cleanly"
    );
    assert!(!scan.packs.is_empty(), "the PS source fixture has no pack");
    let (first_stamp_90k, span_90k) = ps_span_90k(&scan.packs);
    let pass_ticks_27mhz = span_90k.saturating_mul(300);
    let source_media = expect(
        "demux PS source",
        crate::collect::media_from_ps(&source, 64 << 10),
    );

    let mut out = BufWriter::new(expect("create output", File::create(path)));
    let mut written = 0u64;
    let mut passes = 0u64;
    while written < target_bytes {
        let pts_shift = pass_ticks_27mhz / 300;
        let scr_shift = pass_ticks_27mhz;
        for pack in &scan.packs {
            let bytes = ps_pack_at_pass(pack, scr_shift, pts_shift);
            expect("write PS pass", out.write_all(&bytes));
            written += expect("sum pack length", u64::try_from(bytes.len()));
        }
        passes += 1;
    }
    expect("flush PS output", out.flush());
    let _ = first_stamp_90k;
    BigMedia {
        bytes: written,
        samples_per_track: source_media
            .tracks
            .iter()
            .map(|track| {
                (
                    track.spec.track_id.value(),
                    track.samples.len() as u64 * passes,
                )
            })
            .collect(),
        duration: Duration::from_secs_f64(span_90k as f64 / 90_000.0 * passes as f64),
    }
}

/// The first PES stamp and the span of every PES stamp in the fixtures' packs,
/// in 90 kHz ticks (at least one 25 Hz frame).
fn ps_span_90k(packs: &[mpeg_ps::program_stream::Pack<'_>]) -> (u64, u64) {
    let mut min: Option<u64> = None;
    let mut max: Option<u64> = None;
    for pack in packs {
        for pes in &pack.pes_packets {
            let Some(header) = &pes.header else {
                continue;
            };
            for stamp in [
                header.pts.map(|pts| pts.ticks()),
                header.dts.map(|dts| dts.ticks()),
            ]
            .into_iter()
            .flatten()
            {
                min = Some(min.map_or(stamp, |value| value.min(stamp)));
                max = Some(max.map_or(stamp, |value| value.max(stamp)));
            }
        }
    }
    let first = min.unwrap_or(0);
    let span = max.map_or(90_000, |value| value.saturating_sub(first)) + 3_600;
    (first, span.max(3_600))
}

/// One pack's byte image with its SCR and PES timestamps advanced.
fn ps_pack_at_pass(
    pack: &mpeg_ps::program_stream::Pack<'_>,
    scr_shift: u64,
    pts_shift: u64,
) -> Vec<u8> {
    use broadcast_common::Serialize;
    use broadcast_common::clock33::{PCR_WRAP_27MHZ, pack_marked, pcr_split, unpack_marked};

    let header_len = pack.pack_header.header_len();
    let mut header = pack.pack_header.clone();
    let ticks = (header.scr.ticks() + scr_shift) % PCR_WRAP_27MHZ;
    let (base, extension) = pcr_split(ticks);
    header.scr = mpeg_ps::Scr { base, extension };
    let mut bytes = vec![0u8; pack.serialized_len()];
    expect("serialize pack", pack.serialize_into(&mut bytes));
    expect(
        "serialize shifted pack header",
        header.serialize_into(&mut bytes[..header_len]),
    );

    let mut at = header_len;
    if pack.system_header.is_some() {
        at += 6 + usize::from(u16::from_be_bytes([bytes[at + 4], bytes[at + 5]]));
    }
    if pack.psm.is_some() {
        at += 6 + usize::from(u16::from_be_bytes([bytes[at + 4], bytes[at + 5]]));
    }
    while at + 6 <= bytes.len() {
        let length = usize::from(u16::from_be_bytes([bytes[at + 4], bytes[at + 5]]));
        let flags = bytes[at + 7] >> 6;
        if flags == 2 || flags == 3 {
            let field: [u8; 5] = bytes[at + 9..at + 14].try_into().expect("5-byte PTS");
            let marked = unpack_marked(&field);
            let shifted = pack_marked(marked.timestamp.wrapping_add(pts_shift), marked.prefix);
            bytes[at + 9..at + 14].copy_from_slice(&shifted);
        }
        if flags == 3 {
            let field: [u8; 5] = bytes[at + 14..at + 19].try_into().expect("5-byte DTS");
            let marked = unpack_marked(&field);
            let shifted = pack_marked(marked.timestamp.wrapping_add(pts_shift), marked.prefix);
            bytes[at + 14..at + 19].copy_from_slice(&shifted);
        }
        at += 6 + length;
        if length == 0 {
            break;
        }
    }
    bytes
}

/// Write a Matroska (`.mkv`) file of at least `target_bytes` by repeating
/// `source_fixture`'s samples through one `transmux::MkvMuxStage`, pass after pass with
/// each pass's timestamps advanced by the source's span.
///
/// Appends are streamed to disk as the writer emits them; the writer's
/// `Info.Duration` patch is applied by seeking at the end. Live memory is the
/// source fixture's own parsed media plus the writer's one pending Block per
/// track and one open Cluster, never the generated file.
pub fn write_big_mkv(path: &Path, source_fixture: &Path, target_bytes: u64) -> BigMedia {
    let source = read_source(source_fixture);
    let media = expect(
        "demux Matroska source",
        crate::collect::media_from_matroska(&source, crate::collect::CollectMode::Seekable),
    );
    let step = pass_step(&media);
    let mut stage = transmux::MkvMuxStage::new(Duration::from_millis(MKV_CLUSTER_SPAN_MS));
    for track in &media.tracks {
        expect(
            "announce Matroska track",
            stage.feed(
                transmux::mux_stage::MuxInput::Track(Box::new(track.spec.clone())),
                broadcast_common::Timestamp::ZERO,
            ),
        );
    }
    let mut out = BufWriter::new(expect("create output", File::create(path)));
    let mut patches: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut written = 0u64;
    let mut passes = 0u64;
    let mut kept_sync = false;
    while written < target_bytes {
        let mut shifted = media_at_pass(&media, &step, passes);
        // A synthetic soak input keeps only the first video sync sample: the
        // coded frames are untouched, but the seek index stays one entry so a
        // multi-hour file does not build a Cues table the size of its keyframe
        // count (a real capture has one keyframe every few seconds, not one per
        // repeated fixture pass).
        for track in &mut shifted.tracks {
            if !track.spec.config.is_video() {
                continue;
            }
            for sample in &mut track.samples {
                if !sample.flags.is_sync {
                    continue;
                }
                if kept_sync {
                    sample.flags.is_sync = false;
                } else {
                    kept_sync = true;
                }
            }
        }
        for track in &shifted.tracks {
            for sample in &track.samples {
                expect(
                    "feed Matroska sample",
                    stage.feed(
                        transmux::mux_stage::MuxInput::Sample {
                            track: track.spec.track_id,
                            sample: sample.clone(),
                        },
                        broadcast_common::Timestamp::ZERO,
                    ),
                );
                while let Some(output) = stage.poll() {
                    written += write_mkv_output(&mut out, output, &mut patches);
                }
            }
        }
        passes += 1;
    }
    expect("finish Matroska muxer", stage.finish());
    while let Some(output) = stage.poll() {
        written += write_mkv_output(&mut out, output, &mut patches);
    }
    expect("flush Matroska output", out.flush());
    for (offset, bytes) in patches {
        expect("seek Matroska patch", out.seek(SeekFrom::Start(offset)));
        expect("write Matroska patch", out.write_all(&bytes));
    }
    expect("flush Matroska patches", out.flush());
    BigMedia {
        bytes: written,
        samples_per_track: counts(&media, passes),
        duration: step.duration_for(passes),
    }
}

/// The `Cluster` span a generated Matroska file uses.
const MKV_CLUSTER_SPAN_MS: u64 = 5_000;

/// Write one Matroska mux output; returns the bytes appended.
fn write_mkv_output<W: Write>(
    out: &mut W,
    output: MuxOutput,
    patches: &mut Vec<(u64, Vec<u8>)>,
) -> u64 {
    match output {
        MuxOutput::Bytes(chunk) => {
            expect("write Matroska bytes", out.write_all(&chunk));
            expect("sum Matroska length", u64::try_from(chunk.len()))
        }
        MuxOutput::BytesAt { offset, bytes } => {
            patches.push((offset, bytes.to_vec()));
            0
        }
        _ => 0,
    }
}

#[cfg(test)]
mod big_mp4_tests {
    use super::*;
    use isobmff::sample_table::SampleRef;
    use isobmff::{Reader, ReaderConfig, SampleOrder, Step};

    const SMALL_TARGET: u64 = MIB;
    const SMALL_SAMPLE_COUNT: u32 = 20;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "test-harness-big-mp4-{}-{tag}.mp4",
            std::process::id()
        ))
    }

    fn drive(bytes: &[u8]) -> Vec<SampleRef> {
        let config = ReaderConfig::default()
            .with_total_len(Some(bytes.len() as u64))
            .with_order(SampleOrder::File);
        let mut reader = Reader::new(config);
        let mut samples = Vec::new();
        loop {
            match reader.step().expect("reader step") {
                Step::Need(range) => {
                    let start = usize::try_from(range.offset).expect("offset");
                    let end = usize::try_from(range.end()).expect("end");
                    reader
                        .supply(range.offset, bytes.get(start..end).unwrap_or(&[]))
                        .expect("supply");
                }
                Step::Sample(sample) => samples.push(sample),
                Step::Tracks(_) => {}
                Step::Metadata(_) => {}
                Step::Done => break,
                _ => break,
            }
        }
        samples
    }

    #[test]
    fn the_generated_mp4_reads_in_both_layouts() {
        for moov_first in [true, false] {
            let path = temp_path(if moov_first { "first" } else { "last" });
            let opts = BigMp4Options {
                target_bytes: SMALL_TARGET,
                sample_count: SMALL_SAMPLE_COUNT,
                moov_first,
                ..BigMp4Options::default()
            };
            let written = write_big_mp4(&path, &opts);
            let bytes = std::fs::read(&path).expect("read generated file");
            std::fs::remove_file(&path).expect("remove generated file");
            assert_eq!(written.bytes, bytes.len() as u64);
            assert!(written.moov_bytes > 0);
            let moov = isobmff::box_types::find_box(&bytes, b"moov").expect("moov");
            let movie = expect("parse moov", isobmff::MovieBox::parse(moov));
            assert_eq!(movie.tracks.len(), 2);
            let samples = drive(&bytes);
            assert_eq!(samples.len(), 40);
            for track in [BIG_MP4_VIDEO_TRACK, BIG_MP4_AUDIO_TRACK] {
                let count = samples
                    .iter()
                    .filter(|sample| sample.track.value() == track)
                    .count();
                assert_eq!(count, SMALL_SAMPLE_COUNT as usize);
            }
            assert!(samples.iter().all(|sample| sample.offset < written.bytes));
        }
    }
}
