//! DASH MPD generation gate — extended addressing/live/content coverage
//! (issue #566), on top of the base MPD writer gated by `tests/dash.rs`
//! (issue #464).
//!
//! Every assertion here is computed from what the crate itself produced
//! (via [`Segmenter`]/[`Fmp4Demux`]/[`DashPackager`]) — never a hardcoded
//! literal — and cross-checks two independent code paths against each other:
//! the [`Segmenter`]'s own segment-cut bookkeeping vs. an independent
//! [`Fmp4Demux`] re-parse of the produced CMAF bytes (which reads the real
//! `tfdt`/`trun` box fields), and the [`DashPackager`]'s emitted XML vs. that
//! re-parse.
//!
//! Oracle input: `fixtures/ts/h264_aac.ts` (the same deterministic 2-track
//! H.264 + AAC capture `tests/dash.rs` uses), whose video keyframes are
//! spaced exactly 1.0 s apart — so a 1.0 s target [`Segmenter`] reliably cuts
//! 3 video segments (matching the shape of the real ffmpeg oracle
//! `fixtures/dash/manifest.mpd`, which SegmentTimeline-addresses the same
//! source: 3 equal 90000-tick video segments, several unequal audio ones).

use std::collections::BTreeMap;
use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::{
    Addressing, CmafMux, ContentProtectionSystem, DashPackager, Fmp4Demux, InbandEventStream,
    Media, Sample, Segmenter, TrackSegments, TrackSpec, TsDemux,
};

// ---------------------------------------------------------------------------
// Fixture + IR helpers
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn demux_media() -> Media {
    let ts = std::fs::read(fixtures_dir().join("ts/h264_aac.ts")).expect("h264_aac.ts fixture");
    let mut demux = TsDemux::new();
    demux.unpackage(&ts[..]).expect("demux h264_aac.ts")
}

/// Cut `media` into real CMAF media segments with [`Segmenter`], pushing
/// samples interleaved by decode time (mirroring
/// [`transmux::LlSegmenter::package`]'s merge so a segment boundary sees that
/// segment's audio already buffered, matching real live-remux ordering).
/// Returns the init segment plus every media segment's raw bytes.
fn segment_via_segmenter(media: &Media, target_secs: f64) -> (Vec<u8>, Vec<Vec<u8>>) {
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut seg = Segmenter::new(specs, media.movie_timescale, target_secs).expect("segmenter");
    let init = seg.init_segment().expect("init segment");

    let anchor_id = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .map_or(media.tracks[0].spec.track_id, |t| t.spec.track_id);

    struct Cursor<'a> {
        track_id: u32,
        timescale: u64,
        samples: &'a [Sample],
        idx: usize,
        dts_ticks: u64,
        is_anchor: bool,
    }
    let mut cursors: Vec<Cursor<'_>> = media
        .tracks
        .iter()
        .map(|t| Cursor {
            track_id: t.spec.track_id,
            timescale: (t.spec.timescale as u64).max(1),
            samples: &t.samples,
            idx: 0,
            dts_ticks: 0,
            is_anchor: t.spec.track_id == anchor_id,
        })
        .collect();

    loop {
        let mut best: Option<usize> = None;
        for (i, c) in cursors.iter().enumerate() {
            if c.idx >= c.samples.len() {
                continue;
            }
            best = Some(match best {
                None => i,
                Some(b) => {
                    let lhs = c.dts_ticks as u128 * cursors[b].timescale as u128;
                    let rhs = cursors[b].dts_ticks as u128 * c.timescale as u128;
                    if lhs < rhs || (lhs == rhs && !c.is_anchor && cursors[b].is_anchor) {
                        i
                    } else {
                        b
                    }
                }
            });
        }
        let Some(i) = best else { break };
        let (track_id, sample) = {
            let c = &mut cursors[i];
            let s = c.samples[c.idx].clone();
            c.dts_ticks += s.duration as u64;
            c.idx += 1;
            (c.track_id, s)
        };
        seg.push(track_id, sample).expect("push sample");
    }
    seg.flush().expect("flush trailing segment");
    (init, seg.take_ready())
}

/// Independently recover, for every track, the per-segment sample-duration
/// sum and the segment's `tfdt.baseMediaDecodeTime` — by re-demuxing each
/// produced CMAF segment with [`Fmp4Demux`] (reading the real `trun`/`tfdt`
/// box fields), **not** by trusting [`Segmenter`]'s own bookkeeping.
fn recover_segment_timing(
    init: &[u8],
    raw_segments: &[Vec<u8>],
) -> BTreeMap<u32, (Vec<u64>, Vec<u64>)> {
    let mut out: BTreeMap<u32, (Vec<u64>, Vec<u64>)> = BTreeMap::new();
    for seg in raw_segments {
        let mut buf = Vec::with_capacity(init.len() + seg.len());
        buf.extend_from_slice(init);
        buf.extend_from_slice(seg);
        let media = Fmp4Demux::new().unpackage(&buf[..]).expect("demux segment");
        for t in &media.tracks {
            let dur: u64 = t.samples.iter().map(|s| s.duration as u64).sum();
            let entry = out.entry(t.spec.track_id).or_default();
            entry.0.push(dur);
            entry.1.push(t.start_decode_time);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Minimal XML walker — self-contained (no shared code with tests/dash.rs).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Element>,
}

impl Element {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    fn has_attr(&self, key: &str) -> bool {
        self.attrs.iter().any(|(k, _)| k == key)
    }
    fn find_all<'a>(&'a self, name: &str) -> Vec<&'a Element> {
        self.children.iter().filter(|c| c.name == name).collect()
    }
    fn find<'a>(&'a self, name: &str) -> Option<&'a Element> {
        self.children.iter().find(|c| c.name == name)
    }
}

struct XmlParser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> XmlParser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s: s.as_bytes(),
            pos: 0,
        }
    }

    /// Parse the whole document and assert nothing but whitespace trails the
    /// root element close tag — the well-formedness check this test suite
    /// relies on (unbalanced/garbage-trailing XML fails here).
    fn parse_document(&mut self) -> Element {
        self.skip_ws();
        if self.starts_with("<?") {
            while self.pos < self.s.len() && !self.starts_with("?>") {
                self.pos += 1;
            }
            self.pos += 2;
        }
        self.skip_ws();
        let root = self.parse_element().expect("root element");
        self.skip_ws();
        assert_eq!(
            self.pos,
            self.s.len(),
            "trailing content after the root element close tag (not well-formed)"
        );
        root
    }

    fn parse_element(&mut self) -> Option<Element> {
        self.skip_ws();
        if !self.starts_with("<") || self.starts_with("</") {
            return None;
        }
        self.pos += 1;
        let name = self.read_name();
        assert!(!name.is_empty(), "empty element name at pos {}", self.pos);
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            match self.cur() {
                b'/' => {
                    self.pos += 1;
                    self.expect(b'>');
                    return Some(Element {
                        name,
                        attrs,
                        children: Vec::new(),
                    });
                }
                b'>' => {
                    self.pos += 1;
                    break;
                }
                0 => panic!("unterminated start tag <{name}>"),
                _ => {
                    let key = self.read_name();
                    assert!(!key.is_empty(), "malformed attribute in <{name}>");
                    self.skip_ws();
                    self.expect(b'=');
                    self.skip_ws();
                    let value = self.read_quoted();
                    attrs.push((key, value));
                }
            }
        }
        let mut children = Vec::new();
        loop {
            self.skip_ws();
            self.skip_text();
            self.skip_ws();
            if self.starts_with("</") {
                self.pos += 2;
                let close = self.read_name();
                assert_eq!(close, name, "mismatched close tag: <{name}> ... </{close}>");
                self.skip_ws();
                self.expect(b'>');
                break;
            }
            match self.parse_element() {
                Some(c) => children.push(c),
                None => {
                    assert!(
                        self.pos < self.s.len(),
                        "unterminated element <{name}> (EOF before close tag)"
                    );
                }
            }
        }
        Some(Element {
            name,
            attrs,
            children,
        })
    }

    fn skip_text(&mut self) {
        while self.pos < self.s.len() && self.cur() != b'<' {
            self.pos += 1;
        }
    }
    fn cur(&self) -> u8 {
        self.s.get(self.pos).copied().unwrap_or(0)
    }
    fn starts_with(&self, tok: &str) -> bool {
        self.s[self.pos.min(self.s.len())..].starts_with(tok.as_bytes())
    }
    fn skip_ws(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }
    fn expect(&mut self, b: u8) {
        assert_eq!(
            self.cur(),
            b,
            "expected '{}' at pos {}",
            b as char,
            self.pos
        );
        self.pos += 1;
    }
    fn read_name(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.s.len() {
            let c = self.s[self.pos];
            if c.is_ascii_whitespace() || c == b'=' || c == b'>' || c == b'/' {
                break;
            }
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.pos]).into_owned()
    }
    fn read_quoted(&mut self) -> String {
        let q = self.cur();
        assert!(q == b'"' || q == b'\'', "attribute value must be quoted");
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.s.len() && self.cur() != q {
            self.pos += 1;
        }
        let raw = String::from_utf8_lossy(&self.s[start..self.pos]).into_owned();
        self.pos += 1;
        unescape(&raw)
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_xml(s: &str) -> Element {
    XmlParser::new(s).parse_document()
}

// ---------------------------------------------------------------------------
// Static MPD: real single-segment CMAF (CmafMux) + real multi-segment
// $Number$ nominal-duration math (Segmenter).
// ---------------------------------------------------------------------------

#[test]
fn static_mpd_structure_codecs_and_number_duration_math() {
    let media = demux_media();

    // Sanity: CmafMux can package the same IR into one real CMAF artifact
    // (evidence the IR this MPD describes is itself a valid segment set).
    let mut cmaf = CmafMux::new(1);
    let cmaf_bytes = cmaf.package(&media).expect("CmafMux package");
    assert!(!cmaf_bytes.is_empty());

    // Real multi-segment cut (1.0 s target — the fixture's keyframes are
    // exactly 1.0 s apart, see module docs).
    let (init, raw_segments) = segment_via_segmenter(&media, 1.0);
    assert!(
        raw_segments.len() >= 2,
        "expected multiple real segments, got {}",
        raw_segments.len()
    );
    let recovered = recover_segment_timing(&init, &raw_segments);

    let segments: Vec<TrackSegments> = recovered
        .iter()
        .map(|(&track_id, (durations, _tfdt))| TrackSegments {
            track_id,
            durations: durations.clone(),
        })
        .collect();

    let mut pkg = DashPackager {
        segments,
        ..DashPackager::default()
    };
    let xml = pkg.package(&media).expect("package static MPD");
    let root = parse_xml(&xml);

    assert_eq!(root.name, "MPD");
    assert_eq!(root.attr("type"), Some("static"));
    assert!(root.has_attr("mediaPresentationDuration"));
    assert!(!root.has_attr("availabilityStartTime"));

    let period = root.find("Period").expect("Period");
    let sets = period.find_all("AdaptationSet");
    assert_eq!(sets.len(), 2, "video + audio AdaptationSets");

    let mimes: std::collections::BTreeSet<&str> =
        sets.iter().filter_map(|s| s.attr("mimeType")).collect();
    assert!(mimes.contains("video/mp4"));
    assert!(mimes.contains("audio/mp4"));

    // Every AdaptationSet carries a Role (main) — ISO/IEC 23009-1 §5.8.5.5.
    for s in &sets {
        let role = s.find("Role").expect("Role element");
        assert_eq!(role.attr("schemeIdUri"), Some("urn:mpeg:dash:role:2011"));
        assert_eq!(role.attr("value"), Some("main"));
    }

    // $Number$/duration math: `@duration` is the nominal (first-segment)
    // duration, and — the invariant the task calls out — the *real* segment
    // durations for a track (independently recovered via Fmp4Demux) must sum
    // to exactly that track's total sample-duration (the segments fully
    // partition the track; nothing dropped/duplicated by segmentation).
    for (&track_id, (durations, _)) in &recovered {
        let repr = sets
            .iter()
            .flat_map(|s| s.find_all("Representation"))
            .find(|r| r.attr("id") == Some(&track_id.to_string()))
            .unwrap_or_else(|| panic!("Representation id={track_id}"));
        let st = repr.find("SegmentTemplate").expect("SegmentTemplate");
        assert!(st.has_attr("duration"), "$Number$ mode carries @duration");
        assert!(
            st.find("SegmentTimeline").is_none(),
            "$Number$ mode must not carry a SegmentTimeline"
        );
        let media_tpl = st.attr("media").unwrap();
        assert!(media_tpl.contains("$Number$"));

        let nominal: u64 = st.attr("duration").unwrap().parse().unwrap();
        assert_eq!(
            nominal, durations[0],
            "@duration is the first segment's duration"
        );

        let sum_of_real_segments: u64 = durations.iter().sum();
        let track_total: u64 = media
            .tracks
            .iter()
            .find(|t| t.spec.track_id == track_id)
            .expect("track")
            .samples
            .iter()
            .map(|s| s.duration as u64)
            .sum();
        assert_eq!(
            sum_of_real_segments, track_total,
            "sum of the real per-segment durations must equal the track's total \
             sample-duration sum (the segment partition is exact) for track {track_id}"
        );
    }

    // mediaPresentationDuration, converted back to (tenth-of-a-second) ticks,
    // must match the longest track's total duration within one rounded tenth
    // — the "within rounding" tolerance the xs:duration tenths formatting
    // introduces.
    let mpd_dur = root.attr("mediaPresentationDuration").unwrap();
    let tenths = parse_xs_duration_tenths(mpd_dur);
    let want_tenths = recovered
        .iter()
        .map(|(&track_id, (durations, _))| {
            let ts = media
                .tracks
                .iter()
                .find(|t| t.spec.track_id == track_id)
                .unwrap()
                .spec
                .timescale as u64;
            let total: u64 = durations.iter().sum();
            (total * 10 + ts / 2) / ts
        })
        .max()
        .unwrap();
    assert!(
        tenths.abs_diff(want_tenths) <= 1,
        "mediaPresentationDuration {mpd_dur} ({tenths} tenths) must match the derived \
         total ({want_tenths} tenths) within rounding"
    );
}

/// Parse `PT<seconds>.<tenth>S` back into whole tenths of a second.
fn parse_xs_duration_tenths(s: &str) -> u64 {
    let inner = s
        .strip_prefix("PT")
        .and_then(|s| s.strip_suffix('S'))
        .unwrap_or_else(|| panic!("not an xs:duration seconds value: {s}"));
    let (whole, frac) = inner.split_once('.').unwrap_or((inner, "0"));
    let whole: u64 = whole.parse().unwrap();
    let frac: u64 = frac.parse().unwrap();
    whole * 10 + frac
}

// ---------------------------------------------------------------------------
// SegmentTimeline / $Time$ addressing, cross-checked against real tfdt.
// ---------------------------------------------------------------------------

#[test]
fn segment_timeline_time_addressing_matches_tfdt() {
    let media = demux_media();
    let (init, raw_segments) = segment_via_segmenter(&media, 1.0);
    assert!(raw_segments.len() >= 2);
    let recovered = recover_segment_timing(&init, &raw_segments);

    let segments: Vec<TrackSegments> = recovered
        .iter()
        .map(|(&track_id, (durations, _tfdt))| TrackSegments {
            track_id,
            durations: durations.clone(),
        })
        .collect();

    let mut pkg = DashPackager {
        addressing: Addressing::Timeline,
        segments,
        ..DashPackager::default()
    };
    let xml = pkg.package(&media).expect("package timeline MPD");
    let root = parse_xml(&xml);
    let period = root.find("Period").unwrap();
    let sets = period.find_all("AdaptationSet");

    for (&track_id, (durations, tfdts)) in &recovered {
        let repr = sets
            .iter()
            .flat_map(|s| s.find_all("Representation"))
            .find(|r| r.attr("id") == Some(&track_id.to_string()))
            .unwrap_or_else(|| panic!("Representation id={track_id}"));
        let st = repr.find("SegmentTemplate").expect("SegmentTemplate");
        assert!(
            !st.has_attr("duration"),
            "Timeline mode must not also carry @duration (mutually exclusive, §5.3.9.2.2)"
        );
        let media_tpl = st.attr("media").unwrap();
        assert!(media_tpl.contains("$Time$"), "media template: {media_tpl}");

        let timeline = st.find("SegmentTimeline").expect("SegmentTimeline");
        let entries = timeline.find_all("S");
        assert!(!entries.is_empty(), "at least one <S> for track {track_id}");

        // Expand the run-length-encoded <S> list back into a flat per-segment
        // (t, d) sequence and compare against the independently-recovered
        // (duration, tfdt) pairs.
        let mut flat_t = Vec::new();
        let mut flat_d = Vec::new();
        let mut t = 0u64;
        let mut first = true;
        for s in &entries {
            if let Some(explicit_t) = s.attr("t") {
                t = explicit_t.parse().unwrap();
                assert!(
                    first,
                    "@t must only appear on the first <S> in this packager's output"
                );
            } else {
                assert!(!first, "the first <S> must carry @t");
            }
            first = false;
            let d: u64 = s.attr("d").expect("@d mandatory").parse().unwrap();
            let r: u64 = s.attr("r").map_or(0, |v| v.parse().unwrap());
            for _ in 0..=r {
                flat_t.push(t);
                flat_d.push(d);
                t += d;
            }
        }

        assert_eq!(
            flat_d, *durations,
            "expanded <S>@d sequence must equal the real per-segment sample-duration sums \
             for track {track_id}"
        );
        assert_eq!(
            flat_t, *tfdts,
            "expanded <S>@t sequence must equal the real tfdt.baseMediaDecodeTime values \
             (independently recovered via Fmp4Demux) for track {track_id}"
        );
        assert_eq!(flat_t[0], 0, "first segment starts at decode time zero");
    }
}

#[test]
fn timeline_addressing_without_segments_is_rejected() {
    let media = demux_media();
    let mut pkg = DashPackager {
        addressing: Addressing::Timeline,
        ..DashPackager::default()
    };
    assert!(
        pkg.package(&media).is_err(),
        "Addressing::Timeline with no DashPackager::segments must error, not silently \
         fall back to a whole-track SegmentTimeline"
    );
}

// ---------------------------------------------------------------------------
// Dynamic (live) MPD.
// ---------------------------------------------------------------------------

#[test]
fn dynamic_mpd_carries_live_attributes() {
    let media = demux_media();
    let mut pkg = DashPackager {
        dynamic: true,
        availability_start_time: Some("2024-01-01T00:00:00Z".to_string()),
        publish_time: Some("2024-01-01T00:05:00Z".to_string()),
        minimum_update_period: Some("PT2S".to_string()),
        time_shift_buffer_depth: Some("PT30S".to_string()),
        suggested_presentation_delay: Some("PT4S".to_string()),
        ..DashPackager::default()
    };
    let xml = pkg.package(&media).expect("package dynamic MPD");
    let root = parse_xml(&xml);

    assert_eq!(root.attr("type"), Some("dynamic"));
    assert_eq!(
        root.attr("availabilityStartTime"),
        Some("2024-01-01T00:00:00Z")
    );
    assert_eq!(root.attr("publishTime"), Some("2024-01-01T00:05:00Z"));
    assert_eq!(root.attr("minimumUpdatePeriod"), Some("PT2S"));
    assert_eq!(root.attr("timeShiftBufferDepth"), Some("PT30S"));
    assert_eq!(root.attr("suggestedPresentationDelay"), Some("PT4S"));
    assert!(
        !root.has_attr("mediaPresentationDuration"),
        "a dynamic MPD must not carry mediaPresentationDuration"
    );

    // Well-formed-ness of the whole document (namespace + balanced tags —
    // parse_xml already asserts no trailing garbage / mismatched tags).
    assert_eq!(root.attr("xmlns"), Some(transmux::dash::MPD_NAMESPACE));
}

#[test]
fn dynamic_mpd_omits_absent_live_attributes() {
    let media = demux_media();
    let mut pkg = DashPackager {
        dynamic: true,
        availability_start_time: Some("2024-01-01T00:00:00Z".to_string()),
        ..DashPackager::default()
    };
    let xml = pkg.package(&media).expect("package dynamic MPD");
    let root = parse_xml(&xml);
    assert!(!root.has_attr("publishTime"));
    assert!(!root.has_attr("timeShiftBufferDepth"));
    assert!(!root.has_attr("suggestedPresentationDelay"));
}

// ---------------------------------------------------------------------------
// AdaptationSet content: ContentProtection hook + InbandEventStream.
// ---------------------------------------------------------------------------

#[test]
fn content_protection_and_inband_event_stream_hooks() {
    let media = demux_media();
    let kid: [u8; 16] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd,
        0xef,
    ];
    let mut pkg = DashPackager {
        content_protection: vec![ContentProtectionSystem {
            scheme_id_uri: "urn:mpeg:dash:mp4protection:2011".to_string(),
            value: Some("cenc".to_string()),
            default_kid: Some(kid),
        }],
        inband_event_streams: vec![InbandEventStream {
            scheme_id_uri: "urn:scte:scte35:2013:bin".to_string(),
            value: None,
        }],
        ..DashPackager::default()
    };
    let xml = pkg.package(&media).expect("package MPD");
    let root = parse_xml(&xml);

    // xmlns:cenc only appears because a default_kid was supplied.
    assert_eq!(root.attr("xmlns:cenc"), Some("urn:mpeg:cenc:2013"));

    let period = root.find("Period").unwrap();
    let sets = period.find_all("AdaptationSet");
    for s in &sets {
        let cp = s
            .find("ContentProtection")
            .expect("ContentProtection element");
        assert_eq!(
            cp.attr("schemeIdUri"),
            Some("urn:mpeg:dash:mp4protection:2011")
        );
        assert_eq!(cp.attr("value"), Some("cenc"));
        assert_eq!(
            cp.attr("cenc:default_KID"),
            Some("01234567-89ab-cdef-0123-456789abcdef")
        );
    }

    let video_set = sets
        .iter()
        .find(|s| s.attr("mimeType") == Some("video/mp4"))
        .unwrap();
    let audio_set = sets
        .iter()
        .find(|s| s.attr("mimeType") == Some("audio/mp4"))
        .unwrap();
    assert!(
        video_set.find("InbandEventStream").is_some(),
        "InbandEventStream must be advertised on the video AdaptationSet"
    );
    assert!(
        audio_set.find("InbandEventStream").is_none(),
        "this packager only advertises InbandEventStream on the video AdaptationSet"
    );
    let ies = video_set.find("InbandEventStream").unwrap();
    assert_eq!(ies.attr("schemeIdUri"), Some("urn:scte:scte35:2013:bin"));
    assert!(!ies.has_attr("value"), "no @value was supplied");
}

// ---------------------------------------------------------------------------
// Optional ffprobe cross-check (skips cleanly if ffprobe is unavailable).
// ---------------------------------------------------------------------------

#[test]
fn ffprobe_reads_the_static_mpd_if_available() {
    use std::process::Command;

    if !ffprobe_has_dash_demuxer() {
        eprintln!(
            "ffprobe unavailable or built without the DASH demuxer (needs libxml2) — \
             skipping cross-check"
        );
        return;
    }

    let media = demux_media();
    let mut dash = DashPackager::default();
    let xml = dash.package(&media).expect("package MPD");

    let dir = std::env::temp_dir().join(format!("transmux-dash-mpd-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let mpd_path = dir.join("manifest.mpd");
    std::fs::write(&mpd_path, &xml).expect("write mpd");

    // The default templates reference one **single-track** init segment plus
    // one media segment per Representation (`init-stream$RepresentationID$`,
    // `chunk-stream$RepresentationID$-1` — `startNumber` defaults to 1, and
    // `DashPackager::default()` addresses the whole track as one segment).
    // Build those for real via a single-track `Segmenter` with a target
    // duration longer than the whole track, so exactly one segment is cut —
    // matching `DashPackager::default()`'s whole-track-as-one-segment model.
    for t in &media.tracks {
        let id = t.spec.track_id;
        let single = media
            .select_tracks_by(|tt| tt.spec.track_id == id)
            .expect("select single track");
        let track = &single.tracks[0];
        let mut seg = Segmenter::new(vec![track.spec.clone()], media.movie_timescale, 100.0)
            .expect("single-track segmenter");
        let init = seg.init_segment().expect("init segment");
        for s in &track.samples {
            seg.push(id, s.clone()).expect("push sample");
        }
        seg.flush().expect("flush");
        let mut segs = seg.take_ready();
        assert_eq!(
            segs.len(),
            1,
            "one segment (target duration exceeds track length)"
        );
        std::fs::write(dir.join(format!("init-stream{id}.m4s")), &init).unwrap();
        std::fs::write(
            dir.join(format!("chunk-stream{id}-1.m4s")),
            segs.pop().unwrap(),
        )
        .unwrap();
    }

    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(&mpd_path)
        .output()
        .expect("run ffprobe");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "ffprobe failed to read the generated MPD: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("video"),
        "ffprobe must identify a video stream: {stdout}"
    );
    assert!(
        stdout.contains("audio"),
        "ffprobe must identify an audio stream: {stdout}"
    );
}

/// True if `ffprobe` is present **and** its build registers a demuxer named
/// exactly `dash` with decode capability. Many distro/homebrew ffmpeg builds
/// omit it (it needs `libxml2`) and only carry `webm_dash_manifest`, which
/// cannot read a plain ISOBMFF-profile MPD — so probing for the binary alone
/// is not enough to predict whether this cross-check can run.
fn ffprobe_has_dash_demuxer() -> bool {
    let Ok(output) = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-demuxers"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let mut fields = line.split_whitespace();
        let flags = fields.next().unwrap_or("");
        let name = fields.next().unwrap_or("");
        flags.contains('D') && name == "dash"
    })
}
