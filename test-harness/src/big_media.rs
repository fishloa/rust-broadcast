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

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use rtmp_runtime::flv::{FlvHeader, FlvTag, TagType};
use transmux::{
    CodecConfig, FlvDemux, FlvPayloadKind, Media, StreamingTsMux, TsDemux, flv_frame_payloads,
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
    /// Media timeline the written samples span.
    pub duration: Duration,
}

/// Write an MPEG-2 TS file of at least `target_bytes` by repeating
/// `source_fixture`'s samples through a single [`StreamingTsMux`].
pub fn write_big_ts(path: &Path, source_fixture: &Path, target_bytes: u64) -> BigMedia {
    let source = read_source(source_fixture);
    let media = expect(
        "demux TS source",
        TsDemux::new().unpackage(source.as_slice()),
    );
    let step = pass_step(&media);
    let mut mux = StreamingTsMux::new();
    let mut out = BufWriter::new(expect("create output", File::create(path)));
    let mut written = 0u64;
    let mut passes = 0u64;
    while written < target_bytes {
        let shifted = media_at_pass(&media, &step, passes);
        let chunk = expect("mux TS pass", mux.package(&shifted));
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
        FlvDemux::new().unpackage(source.as_slice()),
    );
    let step = pass_step(&media);
    let mut out = BufWriter::new(expect("create output", File::create(path)));
    let mut written = write_flv_preamble(&mut out, &media);
    let mut passes = 0u64;
    while written < target_bytes {
        let shifted = media_at_pass(&media, &step, passes);
        for payload in expect("build FLV frame payloads", flv_frame_payloads(&shifted)) {
            written += write_flv_tag(
                &mut out,
                payload_tag_type(payload.kind),
                payload.timestamp_ms,
                &payload.body,
            );
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
    media: Media,
    step: PassStep,
    mux: StreamingTsMux,
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
            TsDemux::new().unpackage(source.as_slice()),
        );
        let step = pass_step(&media);
        Self {
            media,
            step,
            mux: StreamingTsMux::new(),
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
            let chunk = expect("mux TS pass", self.mux.package(&shifted));
            self.pending.extend_from_slice(&chunk);
            self.next_pass += 1;
        }
        let len = self.segment_bytes.min(self.pending.len());
        self.emitted += 1;
        Some(self.pending.drain(..len).collect())
    }
}

/// FLV header, `onMetaData` and the AVC/AAC sequence headers, written once.
///
/// `onMetaData`'s `duration` is omitted: the file's final length is not known
/// until the target is reached, and FLV makes that field optional (see
/// [`on_metadata_body`]).
fn write_flv_preamble(out: &mut impl Write, media: &Media) -> u64 {
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
    let meta = expect("build onMetaData", on_metadata_body(media, None));
    written += write_flv_tag(out, TagType::Script, 0, &meta);
    for payload in expect(
        "build FLV sequence headers",
        flv_sequence_header_payloads(media),
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
fn track_spans(media: &Media) -> Vec<u64> {
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
fn pass_step(media: &Media) -> PassStep {
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
fn media_at_pass(media: &Media, step: &PassStep, pass: u64) -> Media {
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

fn counts(media: &Media, passes: u64) -> Vec<(u32, u64)> {
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
