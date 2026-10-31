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
//! `LOCAL 0` is output time `0` ([`MPEGTS_AT_ORIGIN`]).
//!
//! # Hold-back
//!
//! Segment N is published when every cue overlapping it has closed, or once
//! the leader's media time has passed the end of N by one target duration,
//! whichever comes first. A cue still open at that deadline is written into N
//! with N's end as its end; the next segment writes it again with its
//! original start, and so on until it closes. Every cue is written with its
//! own start and end, whatever part of it falls outside the segment (RFC 8216
//! §3.5). "Every cue overlapping N has closed" is only known once the
//! teletext PES clock has reached the end of N, so a PID that sends nothing
//! holds each segment until its deadline.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use broadcast_common::Timestamp;
use broadcast_common::clock33;
use caption_convert::{Cue, MediaTime, TeletextToWebVtt, cue_block};
use dvb_si::descriptors::teletext::{TeletextEntry, TeletextType};
use dvb_si::descriptors::{AnyDescriptor, parse_loop};
use hls_runtime::server::{SubtitleRenditionDecl, bcp47_language};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk};
use transmux::SegmentMeta;
use transmux::TrackId;
use transmux::ll_hls::PartCutReady;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

use super::segment::{PendingPublish, drain_publish_queue};
use super::split_audio::LeaderDurations;
use crate::route::{MAX_SUBTITLE_RENDITIONS, RouteHandle, SubtitleServing, subtitle_playlist_uri};

const PTS_HZ: i128 = 90_000;
const NANOS_PER_SEC_F64: f64 = 1_000_000_000.0;

/// `MPEGTS` of every segment's `X-TIMESTAMP-MAP`: the 90 kHz output time the
/// leader's fMP4 timeline has at the timeline origin, which is its `tfdt` 0.
const MPEGTS_AT_ORIGIN: u64 = 0;

/// Suffix of the `NAME` of a hearing-impaired page.
const HEARING_IMPAIRED_SUFFIX: &str = " (SDH)";

/// Where `magazine_number` 0 points (EN 300 468 §6.2.43: magazine 8).
const MAGAZINE_ZERO_IS: u8 = 8;

/// One teletext subtitle page a program offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TeletextPage {
    pub(crate) source_track: TrackId,
    pub(crate) language: String,
    pub(crate) hearing_impaired: bool,
    pub(crate) magazine: u8,
    pub(crate) page: u8,
}

impl TeletextPage {
    fn number(&self) -> String {
        format!("{}{:02X}", self.magazine, self.page)
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
/// a warning, as is every page past [`MAX_SUBTITLE_RENDITIONS`].
pub(crate) fn teletext_pages(route: &str, tracks: &[TrackSpec]) -> Vec<TeletextPage> {
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
                if pages.len() == MAX_SUBTITLE_RENDITIONS {
                    tracing::warn!(
                        route,
                        max = MAX_SUBTITLE_RENDITIONS,
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

/// The rendition declarations for `pages`: `NAME` is the language (with
/// ` (SDH)` for a hearing-impaired page), plus the page number where two
/// pages would otherwise share it.
pub(crate) fn declarations(pages: &[TeletextPage]) -> Vec<SubtitleRenditionDecl> {
    let base: Vec<String> = pages
        .iter()
        .map(|p| {
            let suffix = if p.hearing_impaired {
                HEARING_IMPAIRED_SUFFIX
            } else {
                ""
            };
            format!("{}{suffix}", p.language)
        })
        .collect();
    pages
        .iter()
        .enumerate()
        .map(|(slot, page)| {
            let clash = base.iter().filter(|b| **b == base[slot]).count() > 1;
            let name = if clash {
                format!("{} {}", base[slot], page.number())
            } else {
                base[slot].clone()
            };
            let mut decl = SubtitleRenditionDecl::new(
                name,
                page.language.clone(),
                subtitle_playlist_uri(slot),
            );
            decl.hearing_impaired = page.hearing_impaired;
            decl
        })
        .collect()
}

/// Declares the subtitle renditions of `trunk`'s program when the route wants
/// them and the track set shows teletext subtitle pages, and returns the
/// pages with their servings.
pub(crate) fn resolve(
    route: &RouteHandle,
    trunk: &Trunk,
    tracks: &[TrackSpec],
) -> Vec<(TeletextPage, SubtitleServing)> {
    if !route.subtitles_enabled() || route.container() != hls_runtime::server::Container::Fmp4 {
        return Vec::new();
    }
    let pages = teletext_pages(route.name(), tracks);
    if pages.is_empty() {
        return Vec::new();
    }
    let decls = declarations(&pages);
    let Some(servings) = route.declare_subtitles_for_trunk(trunk, &decls) else {
        return Vec::new();
    };
    pages.into_iter().zip(servings).collect()
}

/// Builds one follower per declared page.
pub(super) fn build_followers(
    route: &RouteHandle,
    trunk: &Trunk,
    tracks: &[TrackSpec],
    target_duration_secs: f64,
) -> Vec<SubtitleFollower> {
    resolve(route, trunk, tracks)
        .into_iter()
        .filter_map(|(page, serving)| {
            let spec = tracks.iter().find(|t| t.track_id == page.source_track)?;
            SubtitleFollower::try_new(&page, spec.timescale, &serving, target_duration_secs)
        })
        .collect()
}

/// What turns teletext PES data fields into cues. The real decoder is
/// [`TeletextToWebVtt`]; the seam lets the hold-back policy be driven by
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
    msn: u32,
    duration_secs: f64,
    body: String,
}

/// Cue bookkeeping and the hold-back policy, in 90 kHz ticks relative to the
/// timeline origin ("local" ticks).
struct Core<D: CueDecoder> {
    decoder: D,
    origin_raw: Option<u64>,
    origin_dts: Option<(i64, u32)>,
    spans: BTreeMap<u32, Span>,
    next_msn: Option<u32>,
    closed: Vec<Cue>,
    newest_local: Option<i64>,
    newest_raw: Option<u64>,
    edge: i64,
    target_ticks: i64,
}

fn rescale_90k(value: i64, timescale: u32) -> i64 {
    let ticks = i128::from(value) * PTS_HZ / i128::from(timescale.max(1));
    i64::try_from(ticks).unwrap_or(if ticks < 0 { i64::MIN } else { i64::MAX })
}

fn to_media_time(local: i64) -> MediaTime {
    MediaTime(u64::try_from(local.max(0)).unwrap_or(0))
}

impl<D: CueDecoder> Core<D> {
    fn new(decoder: D, target_duration_secs: f64) -> Self {
        Core {
            decoder,
            origin_raw: None,
            origin_dts: None,
            spans: BTreeMap::new(),
            next_msn: None,
            closed: Vec::new(),
            newest_local: None,
            newest_raw: None,
            edge: 0,
            target_ticks: (target_duration_secs * PTS_HZ as f64) as i64,
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

    fn push_sample(
        &mut self,
        pts_33: u64,
        data: &[u8],
        leaders: &LeaderDurations,
    ) -> Vec<ReadySegment> {
        self.decoder.push(pts_33, data);
        self.closed.extend(self.decoder.take_closed());
        self.newest_raw = Some(pts_33);
        self.newest_local = self.origin_raw.map(|_| self.local_of(pts_33));
        self.publish(false, leaders)
    }

    fn on_cuts(&mut self, cuts: &[PartCutReady], leaders: &LeaderDurations) -> Vec<ReadySegment> {
        for cut in cuts {
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
                && cut.part_index == 0
            {
                span.end = Some(local);
            }
            self.next_msn.get_or_insert(cut.msn);
        }
        if self.newest_local.is_none() {
            self.newest_local = self.newest_raw.map(|raw| self.local_of(raw));
        }
        self.publish(false, leaders)
    }

    fn close_spans(&mut self, leaders: &LeaderDurations) {
        for (msn, span) in &mut self.spans {
            if span.end.is_none()
                && let Some(duration) = leaders.get(msn)
            {
                let ticks = (duration * PTS_HZ as f64).round() as i64;
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

    fn complete(&self, end: i64) -> bool {
        self.newest_local.is_some_and(|n| n >= end)
            && self.open_cue().is_none_or(|(start, _)| start >= end)
    }

    fn publish(&mut self, finishing: bool, leaders: &LeaderDurations) -> Vec<ReadySegment> {
        self.close_spans(leaders);
        let mut ready = Vec::new();
        while let Some(msn) = self.next_msn
            && let Some(span) = self.spans.get(&msn).copied()
            && let Some(end) = span.end
        {
            let released = finishing
                || self.complete(end)
                || self.edge >= end.saturating_add(self.target_ticks);
            if !released {
                break;
            }
            let duration_secs = leaders
                .get(&msn)
                .copied()
                .unwrap_or((end - span.start) as f64 / PTS_HZ as f64);
            ready.push(ReadySegment {
                msn,
                duration_secs,
                body: self.render(span.start, end),
            });
            self.spans.remove(&msn);
            self.next_msn = Some(msn.saturating_add(1));
            let boundary = end;
            let retained: Vec<Cue> = std::mem::take(&mut self.closed)
                .into_iter()
                .filter(|c| self.local_of(c.end.0) > boundary)
                .collect();
            self.closed = retained;
        }
        ready
    }

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
        let mut body =
            format!("WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:{MPEGTS_AT_ORIGIN}\n\n");
        for cue in &cues {
            body.push_str(&cue_block(cue));
            body.push('\n');
        }
        body
    }

    fn finish(&mut self, leaders: &LeaderDurations) -> Vec<ReadySegment> {
        self.close_spans(leaders);
        let end_raw = self.raw_of_local(self.edge);
        self.decoder.finalize(end_raw);
        self.closed.extend(self.decoder.take_closed());
        self.publish(true, leaders)
    }
}

/// One teletext page being published as a WebVTT rendition.
pub(super) struct SubtitleFollower {
    pub(super) source_track_id: TrackId,
    timescale: u32,
    core: Core<TeletextToWebVtt>,
    writer: SegmentWriter,
    pending: VecDeque<PendingPublish>,
    pending_since: Option<Instant>,
    next_timeline_ns: u64,
}

impl SubtitleFollower {
    fn try_new(
        page: &TeletextPage,
        timescale: u32,
        serving: &SubtitleServing,
        target_duration_secs: f64,
    ) -> Option<Self> {
        let writer = serving.trunk.segment_writer()?;
        Some(SubtitleFollower {
            source_track_id: page.source_track,
            timescale,
            core: Core::new(
                TeletextToWebVtt::new(page.magazine, page.page),
                target_duration_secs,
            ),
            writer,
            pending: VecDeque::new(),
            pending_since: None,
            next_timeline_ns: 0,
        })
    }

    pub(super) fn push(&mut self, sample: &Sample, leaders: &LeaderDurations) {
        let Some(pts) = sample.pts.or(sample.dts) else {
            return;
        };
        let wrap = i64::try_from(clock33::WRAP_33BIT).unwrap_or(i64::MAX);
        let pts_33 = u64::try_from(rescale_90k(pts, self.timescale).rem_euclid(wrap)).unwrap_or(0);
        let ready = self.core.push_sample(pts_33, &sample.data, leaders);
        self.queue(ready);
    }

    pub(super) fn on_part_cuts(&mut self, cuts: &[PartCutReady], leaders: &LeaderDurations) {
        let ready = self.core.on_cuts(cuts, leaders);
        self.queue(ready);
    }

    pub(super) fn finish(&mut self, leaders: &LeaderDurations) {
        let ready = self.core.finish(leaders);
        self.queue(ready);
    }

    fn queue(&mut self, ready: Vec<ReadySegment>) {
        for segment in ready {
            let start_ns = self.next_timeline_ns;
            self.next_timeline_ns = self
                .next_timeline_ns
                .saturating_add((segment.duration_secs * NANOS_PER_SEC_F64) as u64);
            self.pending
                .push_back(PendingPublish::Segment(SegmentEntry::new(
                    segment.body.into_bytes(),
                    SegmentSeq::new(segment.msn),
                    Duration::from_secs_f64(segment.duration_secs),
                    Timestamp::from_nanos(start_ns),
                    SegmentMeta {
                        discontinuous: false,
                    },
                )));
        }
        drain_publish_queue(&mut self.writer, &mut self.pending, &mut self.pending_since);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET_SECS: f64 = 2.0;
    const SEG_TICKS: i64 = 180_000;

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

    fn cut(msn: u32, start_dts: i64) -> PartCutReady {
        PartCutReady {
            msn,
            part_index: 0,
            start_dts,
            timescale: 90_000,
        }
    }

    fn core() -> Core<Scripted> {
        Core::new(Scripted::default(), TARGET_SECS)
    }

    /// The leader closes segment `msn` and cuts the first part of the next
    /// one at `next_start`.
    fn next_segment(
        core: &mut Core<Scripted>,
        leaders: &mut LeaderDurations,
        msn: u32,
        next_start: i64,
    ) -> Vec<ReadySegment> {
        leaders.insert(msn, TARGET_SECS);
        core.on_cuts(&[cut(msn + 1, next_start)], leaders)
    }

    fn msns(ready: &[ReadySegment]) -> Vec<u32> {
        ready.iter().map(|s| s.msn).collect()
    }

    fn cue(start: u64, end: u64, text: &str) -> Cue {
        Cue {
            start: MediaTime(start),
            end: MediaTime(end),
            text: text.to_string(),
        }
    }

    #[test]
    fn held_segment_is_released_after_one_target_duration() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        assert!(core.on_cuts(&[cut(1, 0)], &leaders).is_empty());
        core.decoder.open = Some((100_000, "hello".into()));
        assert!(core.push_sample(190_000, &[], &leaders).is_empty());

        let ready = next_segment(&mut core, &mut leaders, 1, SEG_TICKS);
        assert!(
            ready.is_empty(),
            "segment 1 is held while its cue is open: {:?}",
            msns(&ready)
        );
        assert!(core.push_sample(370_000, &[], &leaders).is_empty());

        let ready = next_segment(&mut core, &mut leaders, 2, 2 * SEG_TICKS);
        assert_eq!(msns(&ready), [1], "published at the deadline");
        assert!(
            ready[0]
                .body
                .contains("00:00:01.111 --> 00:00:02.000\nhello"),
            "cue end clamped to the segment end:\n{}",
            ready[0].body
        );
        assert!(
            ready[0]
                .body
                .starts_with("WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:0\n\n")
        );

        let ready = next_segment(&mut core, &mut leaders, 3, 3 * SEG_TICKS);
        assert_eq!(msns(&ready), [2]);
        assert!(
            ready[0]
                .body
                .contains("00:00:01.111 --> 00:00:04.000\nhello"),
            "the next segment repeats the cue with its original start:\n{}",
            ready[0].body
        );
    }

    #[test]
    fn held_segment_is_released_when_the_open_cue_closes() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        assert!(core.on_cuts(&[cut(1, 0)], &leaders).is_empty());
        core.decoder.open = Some((100_000, "hello".into()));
        assert!(core.push_sample(190_000, &[], &leaders).is_empty());
        assert!(next_segment(&mut core, &mut leaders, 1, SEG_TICKS).is_empty());

        core.decoder.open = None;
        core.decoder.closed.push(cue(100_000, 250_000, "hello"));
        let ready = core.push_sample(250_000, &[], &leaders);
        assert_eq!(
            msns(&ready),
            [1],
            "published at the close, before the deadline"
        );
        assert!(
            ready[0]
                .body
                .contains("00:00:01.111 --> 00:00:02.777\nhello"),
            "the real end time is written:\n{}",
            ready[0].body
        );
    }

    #[test]
    fn a_segment_without_cues_is_still_published() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        assert!(core.on_cuts(&[cut(1, 0)], &leaders).is_empty());
        assert!(core.push_sample(100_000, &[], &leaders).is_empty());
        let ready = next_segment(&mut core, &mut leaders, 1, SEG_TICKS);
        assert!(ready.is_empty(), "nothing proves the span is complete yet");
        let ready = core.push_sample(190_000, &[], &leaders);
        assert_eq!(msns(&ready), [1]);
        assert_eq!(
            ready[0].body,
            "WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:0\n\n"
        );
        assert!((ready[0].duration_secs - TARGET_SECS).abs() < 1e-9);
    }

    #[test]
    fn a_closed_cue_crossing_the_boundary_keeps_its_full_display_time() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        assert!(core.on_cuts(&[cut(1, 0)], &leaders).is_empty());
        core.decoder.closed.push(cue(150_000, 250_000, "across"));
        assert!(core.push_sample(260_000, &[], &leaders).is_empty());
        let ready = next_segment(&mut core, &mut leaders, 1, SEG_TICKS);
        assert_eq!(msns(&ready), [1]);
        assert!(
            ready[0]
                .body
                .contains("00:00:01.666 --> 00:00:02.777\nacross")
        );
        assert!(next_segment(&mut core, &mut leaders, 2, 2 * SEG_TICKS).is_empty());
        let ready = core.push_sample(370_000, &[], &leaders);
        assert_eq!(msns(&ready), [2]);
        assert!(
            ready[0]
                .body
                .contains("00:00:01.666 --> 00:00:02.777\nacross")
        );
    }

    #[test]
    fn a_cue_before_the_origin_starts_at_zero() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        let origin: i64 = 1_000_000;
        assert!(core.on_cuts(&[cut(1, origin)], &leaders).is_empty());
        core.decoder
            .closed
            .push(cue(origin as u64 - 90_000, origin as u64 + 45_000, "early"));
        assert!(
            core.push_sample(origin as u64 + 190_000, &[], &leaders)
                .is_empty()
        );
        let ready = next_segment(&mut core, &mut leaders, 1, origin + SEG_TICKS);
        assert_eq!(msns(&ready), [1]);
        assert!(
            ready[0]
                .body
                .contains("00:00:00.000 --> 00:00:00.500\nearly")
        );
    }

    #[test]
    fn cue_text_is_escaped() {
        let mut core = core();
        let mut leaders = LeaderDurations::new();
        assert!(core.on_cuts(&[cut(1, 0)], &leaders).is_empty());
        core.decoder
            .closed
            .push(cue(1_000, 90_000, "a < b & c > d"));
        assert!(core.push_sample(190_000, &[], &leaders).is_empty());
        let ready = next_segment(&mut core, &mut leaders, 1, SEG_TICKS);
        assert!(ready[0].body.contains("a &lt; b &amp; c &gt; d"));
    }

    #[test]
    fn page_names_are_unique_and_flag_hearing_impaired() {
        let page = |lang: &str, sdh: bool, page: u8| TeletextPage {
            source_track: TrackId::new(1),
            language: lang.into(),
            hearing_impaired: sdh,
            magazine: 8,
            page,
        };
        let decls = declarations(&[page("fr", true, 0x88), page("fr", false, 0x89)]);
        assert_eq!(decls[0].name, "fr (SDH)");
        assert!(decls[0].hearing_impaired);
        assert_eq!(decls[1].name, "fr");
        assert!(!decls[1].hearing_impaired);
        assert_eq!(decls[1].uri, "subs-1.m3u8");

        let clash = declarations(&[page("fr", false, 0x89), page("fr", false, 0x90)]);
        assert_eq!(clash[0].name, "fr 889");
        assert_eq!(clash[1].name, "fr 890");
    }
}
