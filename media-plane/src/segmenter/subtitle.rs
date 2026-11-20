//! WebVTT subtitle renditions from DVB teletext (issue #1428).
//!
//! A teletext PID arrives as a `CodecConfig::Data` track whose PMT ES-info
//! loop carries the teletext descriptor (ETSI EN 300 468 §6.2.43, tag 0x56).
//! Every page it lists as a subtitle page (`teletext_type` 0x02) or a subtitle
//! page for hearing impaired people (0x05) becomes one `EXT-X-MEDIA
//! TYPE=SUBTITLES` rendition, declared as soon as the track set shows the
//! track, and served from its own sibling `Trunk`/`HlsOrigin` with
//! `Container::WebVtt`.
//!
//! # Timing
//!
//! Subtitle segment N covers exactly the span of leader segment N and carries
//! its media sequence number, including when it has no cue (RFC 8216 §3.5).
//! The follower learns the span from the leader's part cuts and segment
//! durations, never from the wall clock.
//!
//! All segments share one `X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:0`: the
//! leader's fMP4 timeline starts at zero at the first decode time it cut on
//! (the follower's timeline origin, the same one the split-audio followers
//! use), so cue time `t` is the cue's PTS minus that origin and
//! `LOCAL 0` is output time `0` (`MPEGTS_AT_ORIGIN`).
//!
//! # Partial segments and open cues
//!
//! On a low-latency route every subtitle rendition publishes Partial Segments
//! cut exactly where the leader cut its own, and the parent segment when the
//! leader closes its segment. Nothing is held back: a part or segment is
//! published at the leader's cut with the cues the decoder has by then.
//!
//! Each part is a complete WebVTT document with the same header and
//! `X-TIMESTAMP-MAP` as the segments, holding the cues displayed during its
//! time range (RFC 8216bis §3.1.4). A cue that has closed is written with its
//! own start and end whatever part of it falls outside the range. A cue still
//! on screen has no end yet, so it is written with its end clamped to the end
//! of the range, and again, with its original start, in every following part
//! and segment until it closes, when its real end is used.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use broadcast_common::Timestamp;
use broadcast_common::bcp47_language;
use broadcast_common::clock33;
use broadcast_common::timescale::{Rounding, Timescale, rescale_i64};
use caption_convert::{Cue, MediaTime, TeletextToWebVtt, cue_block};
use dvb_si::descriptors::teletext::{TeletextEntry, TeletextType};
use dvb_si::descriptors::{AnyDescriptor, parse_loop};
use transmux::ll_hls::PartCutReady;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::{SegmentMeta, TrackId};

use crate::ids::{PartIndex, SegmentSeq};
use crate::publish_queue::{PendingPublish, drain_publish_queue};
use crate::segmenter::audio::LeaderDurations;
use crate::trunk::{PartEntry, SegmentEntry, SegmentWriter, Trunk};
use broadcast_hls::{MediaSequence, PartIndex as HlsPartIndex};

const PTS_HZ_TICKS: i128 = clock33::PTS_HZ as i128;

/// Wall-clock nanoseconds to 90 kHz PTS ticks (the subtitle timeline unit).
fn duration_ticks(duration: Duration) -> i64 {
    let nanos = i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX);
    i64::try_from(nanos.saturating_mul(PTS_HZ_TICKS) / 1_000_000_000).unwrap_or(i64::MAX)
}

/// 90 kHz PTS ticks to a wall-clock [`Duration`].
fn ticks_duration(ticks: i64) -> Duration {
    let nanos = i128::from(ticks).saturating_mul(1_000_000_000) / PTS_HZ_TICKS;
    let nanos = u64::try_from(nanos.max(0)).unwrap_or(u64::MAX);
    Duration::from_nanos(nanos)
}
const NANOS_PER_SEC: u64 = 1_000_000_000;

/// `MPEGTS` of every segment's `X-TIMESTAMP-MAP`: the 90 kHz output time the
/// leader's fMP4 timeline has at the timeline origin, which is its `tfdt` 0.
const MPEGTS_AT_ORIGIN: u64 = 0;

/// Where `magazine_number` 0 points (EN 300 468 §6.2.43: magazine 8).
const MAGAZINE_ZERO_IS: u8 = 8;

/// One teletext subtitle page a program offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeletextPage {
    pub source_track: TrackId,
    pub language: String,
    pub hearing_impaired: bool,
    pub magazine: u8,
    pub page: u8,
}

impl TeletextPage {
    /// The page as teletext writes it: magazine digit then two hex digits.
    pub fn number(&self) -> String {
        format!("{}{:02X}", self.magazine, self.page)
    }

    /// A key for matching a page to its rendition across track-set changes.
    pub fn identity(&self) -> String {
        format!(
            "{}:{}:{}",
            self.number(),
            self.language,
            if self.hearing_impaired { "sdh" } else { "std" }
        )
    }
}

fn is_subtitle(entry: &TeletextEntry) -> Option<bool> {
    match entry.teletext_type {
        TeletextType::SubtitlePage => Some(false),
        TeletextType::HearingImpairedSubtitlePage => Some(true),
        _ => None,
    }
}

/// The subtitle pages the teletext descriptors of `tracks` list, in
/// descriptor order. A page whose language has no BCP 47 form is skipped with
/// a warning, as is every page past `max_pages`.
pub fn teletext_pages(route: &str, max_pages: usize, tracks: &[TrackSpec]) -> Vec<TeletextPage> {
    let mut pages: Vec<TeletextPage> = Vec::new();
    for track in tracks {
        if !matches!(track.config, CodecConfig::Data { .. }) {
            continue;
        }
        for descriptor in parse_loop(&track.es_info_descriptors).flatten() {
            let AnyDescriptor::Teletext(teletext) = descriptor else {
                continue;
            };
            for entry in &teletext.entries {
                let Some(hearing_impaired) = is_subtitle(entry) else {
                    continue;
                };
                let magazine = match entry.magazine_number {
                    0 => MAGAZINE_ZERO_IS,
                    m => m,
                };
                let Some(language) = bcp47_language(&entry.language_code.as_str()) else {
                    tracing::warn!(
                        route,
                        language = %entry.language_code,
                        magazine,
                        page = entry.page_number,
                        "teletext subtitle page has no BCP 47 language; not declared"
                    );
                    continue;
                };
                let page = TeletextPage {
                    source_track: track.track_id,
                    language,
                    hearing_impaired,
                    magazine,
                    page: entry.page_number,
                };
                if pages.iter().any(|p| {
                    (p.source_track, p.magazine, p.page)
                        == (page.source_track, page.magazine, page.page)
                }) {
                    continue;
                }
                if pages.len() == max_pages {
                    tracing::warn!(
                        route,
                        max = max_pages,
                        "more teletext subtitle pages than subtitle slots; ignoring the rest"
                    );
                    return pages;
                }
                pages.push(page);
            }
        }
    }
    pages
}

/// What turns teletext PES data fields into cues. The real decoder is
/// [`TeletextToWebVtt`]; the seam lets the cue bookkeeping be driven by
/// scripted cues.
trait CueDecoder {
    fn push(&mut self, pts_33: u64, data: &[u8]);
    fn take_closed(&mut self) -> Vec<Cue>;
    fn open(&self) -> Option<(u64, String)>;
    fn finalize(&mut self, end_pts_33: u64);
}

impl CueDecoder for TeletextToWebVtt {
    fn push(&mut self, pts_33: u64, data: &[u8]) {
        if let Err(error) = self.push_pes_data_field(pts_33, data) {
            tracing::debug!(%error, "teletext PES data field did not parse; skipped");
        }
    }

    fn take_closed(&mut self) -> Vec<Cue> {
        self.take_closed_cues()
    }

    fn open(&self) -> Option<(u64, String)> {
        self.open_cue().map(|c| (c.start_pts_90k, c.text))
    }

    fn finalize(&mut self, end_pts_33: u64) {
        TeletextToWebVtt::finalize(self, end_pts_33);
    }
}

#[derive(Clone, Copy)]
struct Span {
    start: i64,
    end: Option<i64>,
}

struct ReadySegment {
    msn: MediaSequence,
    duration: Duration,
    body: String,
}

struct ReadyPart {
    msn: MediaSequence,
    part_index: HlsPartIndex,
    duration: Duration,
    body: String,
}

enum Ready {
    Part(ReadyPart),
    Segment(ReadySegment),
}

/// A part the leader has cut, with the duration its own playlist lists.
pub struct LeaderPart {
    pub cut: PartCutReady,
    pub duration: Duration,
}

/// Cue bookkeeping and the part and segment ranges, in 90 kHz ticks relative
/// to the timeline origin ("local" ticks).
struct Core<D: CueDecoder> {
    decoder: D,
    origin_raw: Option<u64>,
    origin_dts: Option<(i64, Timescale)>,
    spans: BTreeMap<MediaSequence, Span>,
    next_msn: Option<MediaSequence>,
    closed: Vec<Cue>,
    edge: i64,
    mpegts_at_origin: u64,
}

fn rescale_90k(value: i64, timescale: Timescale) -> i64 {
    rescale_i64(
        value,
        timescale,
        clock33::PTS_TIMESCALE,
        Rounding::TowardZero,
    )
}

fn to_media_time(local: i64) -> MediaTime {
    MediaTime(u64::try_from(local.max(0)).unwrap_or(0))
}

impl<D: CueDecoder> Core<D> {
    fn new(decoder: D) -> Self {
        Core {
            decoder,
            origin_raw: None,
            origin_dts: None,
            spans: BTreeMap::new(),
            next_msn: None,
            closed: Vec::new(),
            edge: 0,
            mpegts_at_origin: MPEGTS_AT_ORIGIN,
        }
    }

    /// `raw` (a 90 kHz time modulo 2^33) as local ticks, placed within half a
    /// wrap of the leader's current media time so a long-running stream
    /// stays exact.
    fn local_of(&self, raw: u64) -> i64 {
        let Some(origin_raw) = self.origin_raw else {
            return 0;
        };
        let edge_raw = clock33::add_signed(origin_raw, self.edge);
        self.edge + clock33::signed_distance(edge_raw, raw % clock33::WRAP_33BIT)
    }

    fn raw_of_local(&self, local: i64) -> u64 {
        clock33::add_signed(self.origin_raw.unwrap_or(0), local)
    }

    fn push_sample(&mut self, pts_33: u64, data: &[u8]) {
        self.decoder.push(pts_33, data);
        self.closed.extend(self.decoder.take_closed());
    }

    fn note_cut(&mut self, cut: &PartCutReady) -> i64 {
        let origin_dts = *self.origin_dts.get_or_insert_with(|| {
            let origin = (cut.start_dts, cut.timescale);
            self.origin_raw = Some(
                u64::try_from(
                    rescale_90k(cut.start_dts, cut.timescale)
                        .rem_euclid(i64::try_from(clock33::WRAP_33BIT).unwrap_or(i64::MAX)),
                )
                .unwrap_or(0),
            );
            origin
        });
        let local = rescale_90k(cut.start_dts - origin_dts.0, cut.timescale);
        self.edge = self.edge.max(local);
        self.spans.entry(cut.msn).or_insert(Span {
            start: local,
            end: None,
        });
        if let Some(previous) = cut.msn.checked_sub(1)
            && let Some(span) = self.spans.get_mut(&previous)
            && span.end.is_none()
            && cut.part_index == HlsPartIndex::ZERO
        {
            span.end = Some(local);
        }
        self.next_msn.get_or_insert(cut.msn);
        local
    }

    /// Publishes the part of every cut in `parts`, and each segment as soon
    /// as the leader has closed it, in the order the leader produced them:
    /// a segment never follows a part of a later segment.
    fn on_parts(&mut self, parts: &[LeaderPart], leaders: &LeaderDurations) -> Vec<Ready> {
        let mut ready = Vec::new();
        for part in parts {
            let start = self.note_cut(&part.cut);
            self.close_spans(leaders);
            self.publish_segments(Some(part.cut.msn), leaders, &mut ready);
            let ticks = duration_ticks(part.duration).max(1);
            let end = start.saturating_add(ticks);
            self.edge = self.edge.max(end);
            ready.push(Ready::Part(ReadyPart {
                msn: part.cut.msn,
                part_index: part.cut.part_index,
                duration: part.duration,
                body: self.render(start, end),
            }));
        }
        self.close_spans(leaders);
        self.publish_segments(None, leaders, &mut ready);
        ready
    }

    fn close_spans(&mut self, leaders: &LeaderDurations) {
        for (msn, span) in &mut self.spans {
            if span.end.is_none()
                && let Some(duration) = leaders.get(&u32::try_from(msn.value()).unwrap_or(u32::MAX))
            {
                let ticks = duration_ticks(*duration);
                span.end = Some(span.start + ticks);
            }
            if let Some(end) = span.end {
                self.edge = self.edge.max(end);
            }
        }
    }

    fn open_cue(&self) -> Option<(i64, String)> {
        self.decoder
            .open()
            .map(|(start, text)| (self.local_of(start), text))
    }

    fn publish_segments(
        &mut self,
        before: Option<MediaSequence>,
        leaders: &LeaderDurations,
        ready: &mut Vec<Ready>,
    ) {
        while let Some(msn) = self.next_msn
            && before.is_none_or(|b| msn < b)
            && let Some(span) = self.spans.get(&msn).copied()
            && let Some(end) = span.end
        {
            ready.push(Ready::Segment(ReadySegment {
                msn,
                duration: leaders
                    .get(&u32::try_from(msn.value()).unwrap_or(u32::MAX))
                    .copied()
                    .unwrap_or_else(|| ticks_duration(end - span.start)),
                body: self.render(span.start, end),
            }));
            self.spans.remove(&msn);
            self.next_msn = msn.checked_next();
            let boundary = end;
            let retained: Vec<Cue> = std::mem::take(&mut self.closed)
                .into_iter()
                .filter(|c| self.local_of(c.end.0) > boundary)
                .collect();
            self.closed = retained;
        }
    }

    /// The WebVTT document for the range `start..end`.
    ///
    /// RFC 8216bis §3.1.4: a cue that has closed keeps its full start and end
    /// even where it extends outside the range. A cue still on screen has no
    /// known end, which is the live case the section allows ("the cue time
    /// range in each Segment MAY be limited to the WebVTT time range covered
    /// by the Segment"), so its end is clamped to `end`; it is written again
    /// with the same start in each later range until it closes.
    fn render(&self, start: i64, end: i64) -> String {
        let mut cues: Vec<Cue> = Vec::new();
        for cue in &self.closed {
            let (cue_start, cue_end) = (self.local_of(cue.start.0), self.local_of(cue.end.0));
            if cue_end > start && cue_start < end && cue_end > cue_start.max(0) {
                cues.push(Cue {
                    start: to_media_time(cue_start),
                    end: to_media_time(cue_end),
                    text: cue.text.clone(),
                });
            }
        }
        if let Some((cue_start, text)) = self.open_cue()
            && cue_start < end
            && end > cue_start.max(0)
        {
            cues.push(Cue {
                start: to_media_time(cue_start),
                end: to_media_time(end),
                text,
            });
        }
        cues.sort_by_key(|c| c.start.0);
        let mut body = format!(
            "WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:{}\n\n",
            self.mpegts_at_origin % clock33::WRAP_33BIT
        );
        for cue in &cues {
            body.push_str(&cue_block(cue));
            body.push('\n');
        }
        body
    }

    fn finish(&mut self, leaders: &LeaderDurations) -> Vec<Ready> {
        self.close_spans(leaders);
        let end_raw = self.raw_of_local(self.edge);
        self.decoder.finalize(end_raw);
        self.closed.extend(self.decoder.take_closed());
        let mut ready = Vec::new();
        self.publish_segments(None, leaders, &mut ready);
        ready
    }
}

/// One teletext page being published as a WebVTT rendition.
pub struct SubtitleFollower {
    pub source_track_id: TrackId,
    timescale: Timescale,
    core: Core<TeletextToWebVtt>,
    writer: SegmentWriter,
    pending: VecDeque<PendingPublish>,
    pending_since: Option<Instant>,
    next_timeline_ns: u64,
    first_discontinuous: bool,
}

impl SubtitleFollower {
    /// Build a follower for `page` that publishes into the sibling `trunk`; `None` when its segment writer is already claimed.
    pub fn try_new(page: &TeletextPage, timescale: Timescale, trunk: &Arc<Trunk>) -> Option<Self> {
        let writer = trunk.segment_writer()?;
        Some(SubtitleFollower {
            source_track_id: page.source_track,
            timescale,
            core: Core::new(TeletextToWebVtt::new(page.magazine, page.page)),
            writer,
            pending: VecDeque::new(),
            pending_since: None,
            next_timeline_ns: 0,
            first_discontinuous: false,
        })
    }

    pub fn set_start_time(&mut self, start: Duration) {
        self.next_timeline_ns = u64::try_from(start.as_nanos()).unwrap_or(u64::MAX);
        self.first_discontinuous = true;
        self.core.mpegts_at_origin = u64::try_from(
            i128::try_from(start.as_nanos()).unwrap_or(i128::MAX) * PTS_HZ_TICKS
                / i128::from(NANOS_PER_SEC),
        )
        .unwrap_or(u64::MAX);
    }

    pub fn push(&mut self, sample: &Sample) {
        let Some(pts) = sample
            .timing
            .and_then(|t| t.pts())
            .or(sample.timing.and_then(|t| t.dts()))
        else {
            return;
        };
        let wrap = i64::try_from(clock33::WRAP_33BIT).unwrap_or(i64::MAX);
        let pts_33 = u64::try_from(rescale_90k(pts, self.timescale).rem_euclid(wrap)).unwrap_or(0);
        self.core.push_sample(pts_33, &sample.data);
    }

    pub fn on_part_cuts(&mut self, parts: &[LeaderPart], leaders: &LeaderDurations) {
        let ready = self.core.on_parts(parts, leaders);
        self.queue(ready);
    }

    pub fn finish(&mut self, leaders: &LeaderDurations) {
        let ready = self.core.finish(leaders);
        self.queue(ready);
    }

    fn queue(&mut self, ready: Vec<Ready>) {
        for item in ready {
            match item {
                Ready::Part(part) => {
                    self.pending.push_back(PendingPublish::Part(PartEntry::new(
                        part.body.into_bytes(),
                        SegmentSeq::new(u32::try_from(part.msn.value()).unwrap_or(u32::MAX)),
                        PartIndex::new(part.part_index.value()),
                        part.duration,
                        true,
                    )));
                }
                Ready::Segment(segment) => {
                    let start_ns = self.next_timeline_ns;
                    self.next_timeline_ns = self.next_timeline_ns.saturating_add(
                        u64::try_from(segment.duration.as_nanos()).unwrap_or(u64::MAX),
                    );
                    self.pending
                        .push_back(PendingPublish::Segment(SegmentEntry::new(
                            segment.body.into_bytes(),
                            SegmentSeq::new(u32::try_from(segment.msn.value()).unwrap_or(u32::MAX)),
                            segment.duration,
                            Timestamp::from_nanos(start_ns),
                            SegmentMeta {
                                discontinuous: std::mem::take(&mut self.first_discontinuous),
                            },
                        )));
                }
            }
        }
        drain_publish_queue(&mut self.writer, &mut self.pending, &mut self.pending_since);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET_SECS: Duration = Duration::from_secs(2);
    const PART_SECS: Duration = Duration::from_millis(500);
    const SEG_TICKS: i64 = 180_000;
    const PART_TICKS: i64 = 45_000;

    #[derive(Default)]
    struct Scripted {
        closed: Vec<Cue>,
        open: Option<(u64, String)>,
    }

    impl CueDecoder for Scripted {
        fn push(&mut self, _pts_33: u64, _data: &[u8]) {}
        fn take_closed(&mut self) -> Vec<Cue> {
            std::mem::take(&mut self.closed)
        }
        fn open(&self) -> Option<(u64, String)> {
            self.open.clone()
        }
        fn finalize(&mut self, _end_pts_33: u64) {}
    }

    fn part(msn: MediaSequence, part_index: u32, start_dts: i64) -> LeaderPart {
        LeaderPart {
            cut: PartCutReady {
                msn,
                part_index: HlsPartIndex::new(part_index),
                start_dts,
                timescale: Timescale::try_new(90_000).expect("non-zero timescale"),
            },
            duration: PART_SECS,
        }
    }

    fn teletext_descriptor(entries: &[(&str, TeletextType, u8, u8)]) -> Vec<u8> {
        let mut out = vec![
            dvb_si::descriptors::teletext::TAG,
            (entries.len() * 5) as u8,
        ];
        for (language, teletext_type, magazine, page) in entries {
            out.extend_from_slice(language.as_bytes());
            out.push((teletext_type.to_u8() << 3) | (magazine & 0x07));
            out.push(*page);
        }
        out
    }

    fn data_track(track_id: TrackId, descriptors: Vec<u8>) -> TrackSpec {
        let mut spec = TrackSpec::new(
            track_id,
            Timescale::try_new(90_000).expect("non-zero timescale"),
            CodecConfig::Data {
                stream_type: 0x06,
                descriptors: Vec::new(),
                carriage: transmux::pipeline::DataCarriage::Sections,
            },
        );
        spec.es_info_descriptors = descriptors;
        spec
    }

    fn video_track(track_id: TrackId) -> TrackSpec {
        let mut spec = TrackSpec::new(
            track_id,
            Timescale::try_new(90_000).expect("non-zero timescale"),
            CodecConfig::Avc {
                config: transmux::avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==")
                    .expect("valid sprop"),
                width: 0,
                height: 0,
            },
        );
        spec.es_info_descriptors =
            teletext_descriptor(&[("eng", TeletextType::SubtitlePage, 8, 0x88)]);
        spec
    }

    #[test]
    fn is_subtitle_classifies_only_the_two_subtitle_page_types() {
        assert_eq!(
            is_subtitle(&TeletextEntry {
                language_code: dvb_si::text::LangCode(*b"eng"),
                teletext_type: TeletextType::SubtitlePage,
                magazine_number: 8,
                page_number: 0x88,
            }),
            Some(false)
        );
        assert_eq!(
            is_subtitle(&TeletextEntry {
                language_code: dvb_si::text::LangCode(*b"eng"),
                teletext_type: TeletextType::HearingImpairedSubtitlePage,
                magazine_number: 8,
                page_number: 0x89,
            }),
            Some(true)
        );
        for non_subtitle in [
            TeletextType::InitialPage,
            TeletextType::AdditionalInformationPage,
            TeletextType::ProgrammeSchedulePage,
            TeletextType::Reserved(0x1F),
        ] {
            assert_eq!(
                is_subtitle(&TeletextEntry {
                    language_code: dvb_si::text::LangCode(*b"eng"),
                    teletext_type: non_subtitle,
                    magazine_number: 8,
                    page_number: 0x88,
                }),
                None,
                "{}",
                non_subtitle.name()
            );
        }
    }

    #[test]
    fn teletext_page_number_and_identity_are_the_wire_page() {
        let page = TeletextPage {
            source_track: TrackId::new(1),
            language: "en".to_string(),
            hearing_impaired: false,
            magazine: 8,
            page: 0x88,
        };
        assert_eq!(page.number(), "888");
        assert_eq!(page.identity(), "888:en:std");
        let sdh = TeletextPage {
            hearing_impaired: true,
            page: 0x89,
            ..page
        };
        assert_eq!(sdh.number(), "889");
        assert_eq!(sdh.identity(), "889:en:sdh");
        assert_ne!(sdh.identity(), "889:en:std");
    }

    #[test]
    fn teletext_pages_keeps_the_subtitle_pages_in_descriptor_order() {
        let descriptors = teletext_descriptor(&[
            ("eng", TeletextType::SubtitlePage, 8, 0x88),
            ("fra", TeletextType::HearingImpairedSubtitlePage, 0, 0x89),
            ("eng", TeletextType::InitialPage, 8, 0x90),
            ("zzz", TeletextType::SubtitlePage, 8, 0x91),
            ("eng", TeletextType::SubtitlePage, 8, 0x88),
        ]);
        let pages = teletext_pages("route", 8, &[data_track(TrackId::new(1), descriptors)]);
        assert_eq!(pages.len(), 2, "only the two real subtitle pages");
        assert_eq!(pages[0].number(), "888");
        assert_eq!(pages[0].language, "en");
        assert!(!pages[0].hearing_impaired);
        assert_eq!(pages[1].number(), "889");
        assert_eq!(pages[1].language, "fr");
        assert!(pages[1].hearing_impaired);
        assert_ne!(pages[0].identity(), pages[1].identity());
    }

    #[test]
    fn teletext_pages_caps_at_max_pages_and_ignores_non_data_tracks() {
        let descriptors = teletext_descriptor(&[
            ("eng", TeletextType::SubtitlePage, 8, 0x88),
            ("eng", TeletextType::SubtitlePage, 8, 0x89),
            ("eng", TeletextType::SubtitlePage, 8, 0x8A),
        ]);
        let pages = teletext_pages("route", 2, &[data_track(TrackId::new(1), descriptors)]);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].number(), "888");
        assert_eq!(pages[1].number(), "889");

        let video = video_track(TrackId::new(2));
        assert!(
            teletext_pages("route", 8, &[video]).is_empty(),
            "a teletext descriptor on a video track declares no subtitle rendition"
        );
    }

    fn teletext_fixture() -> Option<Vec<(u64, Vec<u8>)>> {
        use broadcast_common::mux::Unpackage;
        let path = test_fixtures::tsduck("input/test-012.ts", "media-plane")?;
        let bytes = std::fs::read(path).ok()?;
        let media = transmux::TsDemux::new().unpackage(&bytes).ok()?;
        let track = media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Data { .. }))?;
        Some(
            track
                .samples
                .iter()
                .filter_map(|s| {
                    let pts = u64::try_from(s.timing.and_then(|t| t.pts())?.max(0)).ok()?;
                    Some((pts, s.data.to_vec()))
                })
                .collect(),
        )
    }

    #[test]
    fn ticks_duration_scales_at_ninety_kilohertz() {
        assert_eq!(ticks_duration(90_000), Duration::from_secs(1));
        assert_eq!(ticks_duration(45_000), Duration::from_millis(500));
        assert_eq!(ticks_duration(1), Duration::from_nanos(11_111));
    }

    #[test]
    fn teletext_adapter_collects_the_fixtures_cues() {
        let Some(samples) = teletext_fixture() else {
            return;
        };
        let mut core = Core::new(TeletextToWebVtt::new(8, 0x89));
        for (pts, data) in &samples {
            core.push_sample(*pts, data);
        }
        let end = samples.last().map_or(0, |(pts, _)| *pts + 1);
        core.decoder.finalize(end);
        core.closed.extend(core.decoder.take_closed());
        assert_eq!(
            core.closed.len(),
            9,
            "the fixture carries nine cues on page 889"
        );
        assert_eq!(
            core.closed[0],
            cue(
                3_856_831_433,
                3_857_281_433,
                "Un train met dix secondes\npour dépasser un point donné."
            )
        );
    }

    #[test]
    fn teletext_adapter_reports_an_open_cue_only_while_it_is_on_screen() {
        let Some(samples) = teletext_fixture() else {
            return;
        };
        let mut core = Core::new(TeletextToWebVtt::new(8, 0x89));
        core.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        let mut seen_open = None;
        for (pts, data) in &samples {
            core.push_sample(*pts, data);
            if seen_open.is_none()
                && let Some(open) = core.open_cue()
            {
                seen_open = Some(open);
            }
        }
        assert_eq!(
            seen_open,
            Some((
                3_856_831_433,
                "Un train met dix secondes\npour dépasser un point donné.".to_string()
            )),
            "the open cue carries the fixture's start and text"
        );
    }

    #[test]
    fn render_excludes_cues_touching_either_range_edge() {
        let mut core = core();
        core.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        core.closed.push(cue(0, 180_000, "ends_at_start"));
        core.closed.push(cue(180_000, 270_000, "starts_at_end"));
        core.closed.push(cue(90_000, 90_000, "zero_length"));

        let first = core.render(0, 180_000);
        assert!(first.contains("ends_at_start"), "{first}");
        assert!(!first.contains("starts_at_end"), "{first}");
        assert!(!first.contains("zero_length"), "{first}");

        let second = core.render(180_000, 360_000);
        assert!(!second.contains("ends_at_start"), "{second}");
        assert!(second.contains("starts_at_end"), "{second}");
        assert!(!second.contains("zero_length"), "{second}");
    }

    #[test]
    fn local_and_raw_round_trip_across_the_wrap() {
        let mut core = core();
        core.origin_raw = Some(clock33::WRAP_33BIT - 45_000);
        core.edge = 0;
        assert_eq!(core.local_of(15_000), 60_000);
        assert_eq!(core.raw_of_local(60_000), 15_000);

        core.edge = 180_000;
        assert_eq!(core.local_of(45_000), 90_000);
        let local = core.local_of(9_000);
        assert_eq!(core.local_of(core.raw_of_local(local)), local);
    }

    #[test]
    fn publish_segments_emits_each_closed_span_once_in_order() {
        let mut core = core();
        core.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        core.note_cut(&part(MediaSequence::new(2), 0, 180_000).cut);
        core.note_cut(&part(MediaSequence::new(3), 0, 360_000).cut);
        let leaders: LeaderDurations = [(3u32, TARGET_SECS)].into_iter().collect();
        core.close_spans(&leaders);
        let mut ready = Vec::new();
        core.publish_segments(None, &leaders, &mut ready);
        let segments = segments(&ready);
        assert_eq!(
            segments.iter().map(|s| s.msn).collect::<Vec<_>>(),
            [
                MediaSequence::new(1),
                MediaSequence::new(2),
                MediaSequence::new(3)
            ]
        );
        for segment in segments {
            assert_eq!(
                segment.duration, TARGET_SECS,
                "each 2 s span keeps its exact duration"
            );
        }

        core.note_cut(&part(MediaSequence::new(4), 0, 540_000).cut);
        core.note_cut(&part(MediaSequence::new(5), 0, 720_000).cut);
        let mut ready = Vec::new();
        core.publish_segments(Some(MediaSequence::new(5)), &leaders, &mut ready);
        assert_eq!(
            msns(&ready),
            [MediaSequence::new(4)],
            "a segment strictly before the boundary is published"
        );
        let mut ready = Vec::new();
        core.publish_segments(Some(MediaSequence::new(5)), &leaders, &mut ready);
        assert!(ready.is_empty(), "nothing is left at the boundary");
    }

    #[test]
    fn a_segment_at_the_boundary_is_not_published_early() {
        let mut core = core();
        core.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        core.note_cut(&part(MediaSequence::new(2), 0, 180_000).cut);
        core.note_cut(&part(MediaSequence::new(3), 0, 360_000).cut);
        let mut ready = Vec::new();
        core.publish_segments(
            Some(MediaSequence::new(2)),
            &LeaderDurations::new(),
            &mut ready,
        );
        assert_eq!(msns(&ready), [MediaSequence::new(1)]);
        let mut ready = Vec::new();
        core.publish_segments(
            Some(MediaSequence::new(2)),
            &LeaderDurations::new(),
            &mut ready,
        );
        assert!(
            ready.is_empty(),
            "the segment at the boundary belongs with the next part"
        );
    }

    #[test]
    fn finish_publishes_the_final_partial_segment() {
        let mut core = core();
        core.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        core.note_cut(&part(MediaSequence::new(2), 0, 180_000).cut);
        let leaders: LeaderDurations = [(2u32, TARGET_SECS)].into_iter().collect();
        let ready = core.finish(&leaders);
        assert_eq!(
            msns(&ready),
            [MediaSequence::new(1), MediaSequence::new(2)],
            "finish closes and publishes the trailing partial segment"
        );
    }

    #[test]
    fn close_spans_never_moves_an_already_closed_end() {
        let mut core = core();
        core.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        let two: LeaderDurations = [(1u32, TARGET_SECS)].into_iter().collect();
        core.close_spans(&two);
        let one: LeaderDurations = [(1u32, Duration::from_secs(1))].into_iter().collect();
        core.close_spans(&one);
        let mut ready = Vec::new();
        core.publish_segments(None, &LeaderDurations::new(), &mut ready);
        assert_eq!(segments(&ready)[0].duration, TARGET_SECS);
    }

    #[test]
    fn note_cut_closes_the_previous_span_only_for_part_zero() {
        let mut nonzero = core();
        nonzero.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        nonzero.note_cut(&part(MediaSequence::new(2), 1, 180_000).cut);
        let mut ready = Vec::new();
        nonzero.publish_segments(None, &LeaderDurations::new(), &mut ready);
        assert!(
            ready.is_empty(),
            "a non-zero part does not close the previous segment"
        );

        let mut z = core();
        z.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        z.note_cut(&part(MediaSequence::new(2), 0, 180_000).cut);
        let mut ready = Vec::new();
        z.publish_segments(None, &LeaderDurations::new(), &mut ready);
        assert_eq!(msns(&ready), [MediaSequence::new(1)]);

        let mut closed = core();
        closed.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        let two: LeaderDurations = [(1u32, TARGET_SECS)].into_iter().collect();
        closed.close_spans(&two);
        closed.note_cut(&part(MediaSequence::new(2), 0, 270_000).cut);
        let mut ready = Vec::new();
        closed.publish_segments(None, &LeaderDurations::new(), &mut ready);
        assert_eq!(
            segments(&ready)[0].duration,
            TARGET_SECS,
            "a later part must not overwrite a closed span's end"
        );
    }

    #[test]
    fn a_closed_cue_ending_at_the_boundary_is_not_retained() {
        let mut core = core();
        core.note_cut(&part(MediaSequence::new(1), 0, 0).cut);
        core.note_cut(&part(MediaSequence::new(2), 0, 180_000).cut);
        core.closed.push(cue(90_000, 180_000, "ends_at_boundary"));
        let mut ready = Vec::new();
        core.publish_segments(
            Some(MediaSequence::new(2)),
            &LeaderDurations::new(),
            &mut ready,
        );
        assert_eq!(msns(&ready), [MediaSequence::new(1)]);
        assert!(
            core.closed.is_empty(),
            "a cue that ended at the segment boundary belongs to that segment only"
        );
    }

    #[test]
    fn render_rejects_an_open_cue_that_ends_at_the_range_end() {
        let mut core = core();
        core.origin_raw = Some(90_000);
        core.decoder.open = Some((0, "early".into()));
        assert!(
            !core.render(-90_000, 0).contains("early"),
            "a cue whose clamped end is the range end is not on screen in it"
        );
    }

    fn page() -> TeletextPage {
        TeletextPage {
            source_track: TrackId::new(2),
            language: "en".to_string(),
            hearing_impaired: false,
            magazine: 8,
            page: 0x89,
        }
    }

    fn subtitle_trunk() -> Arc<Trunk> {
        Trunk::new(
            crate::trunk::TrunkConfig::default()
                .with_timed_capacity(std::num::NonZeroUsize::MIN.saturating_add(63))
                .with_sparse_capacity(std::num::NonZeroUsize::MIN.saturating_add(15))
                .with_segment_capacity(std::num::NonZeroUsize::MIN.saturating_add(15))
                .with_event_capacity(std::num::NonZeroUsize::MIN.saturating_add(7))
                .with_part_capacity(std::num::NonZeroUsize::MIN.saturating_add(63)),
        )
    }

    #[test]
    fn subtitle_follower_try_new_needs_a_free_segment_writer() {
        let trunk = subtitle_trunk();
        let follower = SubtitleFollower::try_new(
            &page(),
            Timescale::try_new(90_000).expect("non-zero timescale"),
            &trunk,
        );
        assert!(follower.is_some());
        assert!(
            SubtitleFollower::try_new(
                &page(),
                Timescale::try_new(90_000).expect("non-zero timescale"),
                &trunk,
            )
            .is_none(),
            "a second follower cannot share the trunk's segment writer"
        );
        let video = video_track(TrackId::new(3));
        assert!(
            teletext_pages("route", 4, &[video]).is_empty(),
            "a track with no subtitle page yields no follower to build"
        );
    }

    #[test]
    fn subtitle_follower_maps_a_cue_to_one_local_second_and_follows_leader_cuts() {
        let Some(samples) = teletext_fixture() else {
            return;
        };
        let trunk = subtitle_trunk();
        let mut follower = SubtitleFollower::try_new(
            &page(),
            Timescale::try_new(90_000).expect("non-zero timescale"),
            &trunk,
        )
        .expect("follower");
        let mut cursor = trunk.subscribe_segments();

        follower.set_start_time(Duration::from_secs(2));
        for (pts, data) in &samples {
            let pts = i64::try_from(*pts).expect("pts fits");
            let sample =
                Sample::new(data.clone(), Some(pts), Some(pts), None, false).expect("valid timing");
            follower.push(&sample);
        }
        const CUT_START: i64 = 3_856_741_433;
        let leaders: LeaderDurations = [(1u32, TARGET_SECS), (2, TARGET_SECS)]
            .into_iter()
            .collect();
        follower.on_part_cuts(&[part(MediaSequence::new(1), 0, CUT_START)], &leaders);
        follower.on_part_cuts(
            &[part(MediaSequence::new(2), 0, CUT_START + SEG_TICKS)],
            &leaders,
        );
        follower.finish(&leaders);

        let mut entries = Vec::new();
        while let Some(item) = cursor.poll() {
            if let crate::trunk::SegmentCursorItem::Segment(entry) = item {
                entries.push(entry);
            }
        }
        assert_eq!(
            entries
                .iter()
                .map(|e| e.sequence_number.value())
                .collect::<Vec<_>>(),
            vec![1, 2],
            "two leader segments produce two subtitle segments"
        );
        let first = String::from_utf8_lossy(&entries[0].bytes);
        assert!(
            first.contains("MPEGTS:180000"),
            "set_start_time(2 s) moved X-TIMESTAMP-MAP: {first}"
        );
        assert!(
            first.contains("00:00:01.000"),
            "the cue at CUT_START + 1 s is local t=1 s: {first}"
        );
        assert!(first.contains("Un train met dix secondes"), "{first}");
        assert_eq!(entries[0].timeline_position.as_nanos(), 2_000_000_000);
        assert!(entries[0].meta.discontinuous);
        assert_eq!(
            entries[1].timeline_position.as_nanos(),
            4_000_000_000,
            "segment 1 is 2 s long, so segment 2 starts at 2 s + 2 s"
        );
    }

    fn core() -> Core<Scripted> {
        Core::new(Scripted::default())
    }

    fn segments(ready: &[Ready]) -> Vec<&ReadySegment> {
        ready
            .iter()
            .filter_map(|r| match r {
                Ready::Segment(s) => Some(s),
                Ready::Part(_) => None,
            })
            .collect()
    }

    fn parts(ready: &[Ready]) -> Vec<&ReadyPart> {
        ready
            .iter()
            .filter_map(|r| match r {
                Ready::Part(p) => Some(p),
                Ready::Segment(_) => None,
            })
            .collect()
    }

    fn msns(ready: &[Ready]) -> Vec<MediaSequence> {
        segments(ready).iter().map(|s| s.msn).collect()
    }

    fn four_parts(msn: MediaSequence, first_start: i64) -> Vec<LeaderPart> {
        (0..4)
            .map(|i| part(msn, i, first_start + i64::from(i) * PART_TICKS))
            .collect()
    }

    /// The leader closes segment `msn`; the next segment's first part follows.
    fn close_segment(
        core: &mut Core<Scripted>,
        leaders: &mut LeaderDurations,
        msn: MediaSequence,
    ) -> Vec<Ready> {
        leaders.insert(u32::try_from(msn.value()).unwrap_or(u32::MAX), TARGET_SECS);
        core.on_parts(&[], leaders)
    }

    fn cue(start: u64, end: u64, text: &str) -> Cue {
        Cue {
            start: MediaTime(start),
            end: MediaTime(end),
            text: text.to_string(),
        }
    }

    const HEADER: &str = "WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:0\n\n";

    #[test]
    fn open_cue_is_clamped_and_repeated_with_its_original_start() {
        let mut core = core();
        let leaders = LeaderDurations::new();
        core.decoder.open = Some((10_000, "hello".into()));

        let ready = core.on_parts(&[part(MediaSequence::new(1), 0, 0)], &leaders);
        let first = parts(&ready);
        assert_eq!(first.len(), 1);
        assert!(first[0].body.starts_with(HEADER), "{}", first[0].body);
        assert!(
            first[0]
                .body
                .contains("00:00:00.111 --> 00:00:00.500\nhello"),
            "first part ends the open cue at the part end:\n{}",
            first[0].body
        );

        let ready = core.on_parts(&[part(MediaSequence::new(1), 1, PART_TICKS)], &leaders);
        assert!(
            parts(&ready)[0]
                .body
                .contains("00:00:00.111 --> 00:00:01.000\nhello"),
            "second part repeats it with the same start, ending at its own end:\n{}",
            parts(&ready)[0].body
        );

        core.decoder.open = None;
        core.decoder.closed.push(cue(10_000, 120_000, "hello"));
        core.push_sample(0, &[]);
        let ready = core.on_parts(&[part(MediaSequence::new(1), 2, 2 * PART_TICKS)], &leaders);
        assert!(
            parts(&ready)[0]
                .body
                .contains("00:00:00.111 --> 00:00:01.333\nhello"),
            "third part carries the real end:\n{}",
            parts(&ready)[0].body
        );
    }

    #[test]
    fn a_reconnected_run_maps_local_zero_to_its_start_on_the_media_timeline() {
        let mut core = core();
        core.mpegts_at_origin = 8 * 90_000;
        let ready = core.on_parts(
            &[part(MediaSequence::new(9), 0, 0)],
            &LeaderDurations::new(),
        );
        assert_eq!(
            parts(&ready)[0].body,
            "WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:720000\n\n"
        );
    }

    #[test]
    fn a_part_without_cues_is_a_complete_document() {
        let mut core = core();
        let ready = core.on_parts(
            &[part(MediaSequence::new(1), 0, 0)],
            &LeaderDurations::new(),
        );
        assert_eq!(parts(&ready)[0].body, HEADER);
        assert_eq!(parts(&ready)[0].duration, PART_SECS);
    }

    #[test]
    fn a_segment_is_published_when_the_leader_closes_it() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        let ready = core.on_parts(&four_parts(MediaSequence::new(1), 0), &leaders);
        assert!(segments(&ready).is_empty(), "the leader has not closed it");
        assert_eq!(parts(&ready).len(), 4);
        let ready = close_segment(&mut core, &mut leaders, MediaSequence::new(1));
        assert_eq!(msns(&ready), [MediaSequence::new(1)]);
        assert_eq!(segments(&ready)[0].body, HEADER);
        assert_eq!(segments(&ready)[0].duration, TARGET_SECS);
    }

    #[test]
    fn parts_of_a_segment_precede_the_segment_and_follow_the_previous_one() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        core.on_parts(&four_parts(MediaSequence::new(1), 0), &leaders);
        leaders.insert(1, TARGET_SECS);
        let ready = core.on_parts(&[part(MediaSequence::new(2), 0, SEG_TICKS)], &leaders);
        let order: Vec<(char, MediaSequence)> = ready
            .iter()
            .map(|r| match r {
                Ready::Segment(s) => ('s', s.msn),
                Ready::Part(p) => ('p', p.msn),
            })
            .collect();
        assert_eq!(
            order,
            [('s', MediaSequence::new(1)), ('p', MediaSequence::new(2))]
        );
    }

    #[test]
    fn segment_with_an_open_cue_is_clamped_and_the_next_segment_repeats_it() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        core.decoder.open = Some((100_000, "hello".into()));
        core.on_parts(&four_parts(MediaSequence::new(1), 0), &leaders);
        let ready = close_segment(&mut core, &mut leaders, MediaSequence::new(1));
        assert!(
            segments(&ready)[0]
                .body
                .contains("00:00:01.111 --> 00:00:02.000\nhello"),
            "{}",
            segments(&ready)[0].body
        );
        core.on_parts(&four_parts(MediaSequence::new(2), SEG_TICKS), &leaders);
        let ready = close_segment(&mut core, &mut leaders, MediaSequence::new(2));
        assert!(
            segments(&ready)[0]
                .body
                .contains("00:00:01.111 --> 00:00:04.000\nhello"),
            "{}",
            segments(&ready)[0].body
        );
    }

    #[test]
    fn a_segment_carries_cues_that_closed_during_its_earlier_parts() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        core.on_parts(
            &[
                part(MediaSequence::new(1), 0, 0),
                part(MediaSequence::new(1), 1, PART_TICKS),
            ],
            &leaders,
        );
        core.decoder.closed.push(cue(1_000, 20_000, "early"));
        core.push_sample(30_000, &[]);
        core.on_parts(
            &[
                part(MediaSequence::new(1), 2, 2 * PART_TICKS),
                part(MediaSequence::new(1), 3, 3 * PART_TICKS),
            ],
            &leaders,
        );
        let ready = close_segment(&mut core, &mut leaders, MediaSequence::new(1));
        assert!(
            segments(&ready)[0]
                .body
                .contains("00:00:00.011 --> 00:00:00.222\nearly"),
            "{}",
            segments(&ready)[0].body
        );
    }

    #[test]
    fn a_closed_cue_crossing_the_boundary_keeps_its_full_display_time() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        core.decoder.closed.push(cue(150_000, 250_000, "across"));
        core.push_sample(0, &[]);
        core.on_parts(&four_parts(MediaSequence::new(1), 0), &leaders);
        let ready = close_segment(&mut core, &mut leaders, MediaSequence::new(1));
        assert_eq!(msns(&ready), [MediaSequence::new(1)]);
        assert!(
            segments(&ready)[0]
                .body
                .contains("00:00:01.666 --> 00:00:02.777\nacross")
        );
        let ready = core.on_parts(&four_parts(MediaSequence::new(2), SEG_TICKS), &leaders);
        assert!(
            parts(&ready)[0]
                .body
                .contains("00:00:01.666 --> 00:00:02.777\nacross")
        );
        let ready = close_segment(&mut core, &mut leaders, MediaSequence::new(2));
        assert!(
            segments(&ready)[0]
                .body
                .contains("00:00:01.666 --> 00:00:02.777\nacross")
        );
    }

    #[test]
    fn a_cue_before_the_origin_starts_at_zero() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        let origin: i64 = 1_000_000;
        core.decoder
            .closed
            .push(cue(origin as u64 - 90_000, origin as u64 + 45_000, "early"));
        core.push_sample(0, &[]);
        core.on_parts(&four_parts(MediaSequence::new(1), origin), &leaders);
        let ready = close_segment(&mut core, &mut leaders, MediaSequence::new(1));
        assert_eq!(msns(&ready), [MediaSequence::new(1)]);
        assert!(
            segments(&ready)[0]
                .body
                .contains("00:00:00.000 --> 00:00:00.500\nearly")
        );
    }

    #[test]
    fn cue_text_is_escaped() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        core.decoder
            .closed
            .push(cue(1_000, 90_000, "a < b & c > d"));
        core.push_sample(0, &[]);
        core.on_parts(&four_parts(MediaSequence::new(1), 0), &leaders);
        let ready = close_segment(&mut core, &mut leaders, MediaSequence::new(1));
        assert!(segments(&ready)[0].body.contains("a &lt; b &amp; c &gt; d"));
    }
}
