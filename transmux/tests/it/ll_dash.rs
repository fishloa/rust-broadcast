//! Low-latency DASH gate (issue #461): chunked CMAF + LL-DASH MPD.
//!
//! Input IR: `TsDemux`-ing the deterministic 2-track `fixtures/ts/h264-aac.ts`
//! (H.264 video + AAC audio — 75 video + 131 audio samples).
//!
//! Every test bites: chunk bytes are parsed with a std-only top-level box walker
//! and re-demuxed via `Mp4Demux` (no hardcoded offsets); the MPD is parsed with
//! `quick-xml` and asserted against real elements/attributes.
#![cfg(feature = "std")]

use std::path::PathBuf;
use transmux::TrackId;

use broadcast_common::Stage;
use broadcast_core::timescale::Timescale;
use std::time::Duration;
use test_harness::collect::Collected as Media;
use transmux::Sample;
use transmux::ll_dash::LlSegmenter;
use transmux::pipeline::{FragmentTrackData, TrackSpec};
use transmux::segmenter::MAX_PENDING_SAMPLES_PER_TRACK;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    test_fixtures::root()
}

fn demux_media() -> Media {
    let ts = std::fs::read(fixtures_dir().join("ts/h264-aac.ts"))
        .expect("h264-aac.ts fixture must exist");

    test_harness::collect::media_from_ts(&ts[..]).expect("demux h264-aac.ts")
}

fn video_track(media: &Media) -> &test_harness::collect::CollectedTrack {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::pipeline::CodecConfig::Avc { .. }))
        .expect("video track")
}

fn specs(media: &Media) -> Vec<TrackSpec> {
    media.tracks.iter().map(|t| t.spec.clone()).collect()
}

// ---------------------------------------------------------------------------
// Minimal top-level box walker — no external dependency, no hardcoded offsets.
// ---------------------------------------------------------------------------

/// (four-cc, byte range) of every top-level box in `buf`.
fn top_boxes(buf: &[u8]) -> Vec<([u8; 4], std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= buf.len() {
        let size =
            u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
        let ty = [buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]];
        let box_len = if size == 0 { buf.len() - off } else { size };
        assert!(
            box_len >= 8 && off + box_len <= buf.len(),
            "malformed box {ty:?}"
        );
        out.push((ty, off..off + box_len));
        off += box_len;
    }
    out
}

/// The `tfdt` base_media_decode_time for each track fragment in a moof, in track
/// order. Walks moof→traf→tfdt by four-cc (version 1 = 64-bit).
fn moof_tfdt_bases(moof: &[u8]) -> Vec<u64> {
    let mut bases = Vec::new();
    // moof body starts after the 8-byte header.
    let trafs = child_boxes(&moof[8..], b"traf");
    for traf in trafs {
        // find tfdt within this traf body (after the 8-byte box header)
        if let Some(tfdt) = child_boxes(&traf[8..], b"tfdt").into_iter().next() {
            // tfdt: 8-byte box header, then FullBox(version(1)+flags(3)), then time
            let version = tfdt[8];
            let payload = &tfdt[12..];
            let base = if version == 1 {
                u64::from_be_bytes(payload[..8].try_into().unwrap())
            } else {
                u32::from_be_bytes(payload[..4].try_into().unwrap()) as u64
            };
            bases.push(base);
        }
    }
    bases
}

/// All direct child boxes of `four_cc` type inside a box *body* slice.
fn child_boxes<'a>(body: &'a [u8], four_cc: &[u8; 4]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= body.len() {
        let size =
            u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]) as usize;
        let ty = &body[off + 4..off + 8];
        let box_len = if size == 0 { body.len() - off } else { size };
        if box_len < 8 || off + box_len > body.len() {
            break;
        }
        if ty == four_cc {
            out.push(&body[off..off + box_len]);
        }
        off += box_len;
    }
    out
}

// ---------------------------------------------------------------------------
// Helpers to run the packagers and reconstruct samples.
// ---------------------------------------------------------------------------

/// One emitted chunk, reconstructed from its bytes: the `styp` that opens a
/// segment names its number, and the `moof`'s `mfhd` carries the sequence.
struct ChunkView {
    data: Vec<u8>,
    segment_number: u64,
    is_segment_start: bool,
    sequence_number: u32,
}

fn chunks_for(media: &Media, chunk_samples: usize) -> Vec<ChunkView> {
    let mut seg = LlSegmenter::new(
        specs(media),
        media.movie_timescale,
        Duration::from_millis(500),
        chunk_samples,
    )
    .expect("LlSegmenter");
    let outputs = crate::packagers::drive_samples(&mut seg, media).expect("drive chunks");
    view_chunks(outputs)
}

/// Group a segmenter's [`MuxOutput`](transmux::mux_stage::MuxOutput)s the way
/// the chunk metadata `Chunk` used to carry them: a chunk that opens with
/// `styp` starts a new segment, and `mfhd.sequence_number` reads back out of
/// its `moof`.
fn view_chunks(outputs: Vec<transmux::mux_stage::MuxOutput>) -> Vec<ChunkView> {
    let mut segment_number = 0u64;
    let mut out = Vec::new();
    for output in outputs {
        let transmux::mux_stage::MuxOutput::Bytes(bytes) = output else {
            continue;
        };
        let data = bytes.to_vec();
        let is_segment_start = data.get(4..8) == Some(b"styp");
        if is_segment_start {
            segment_number += 1;
        }
        out.push(ChunkView {
            sequence_number: mfhd_sequence(&data),
            data,
            segment_number,
            is_segment_start,
        });
    }
    out
}

/// Drain a segmenter's finished chunks.
fn drain(seg: &mut LlSegmenter) -> Vec<transmux::mux_stage::MuxOutput> {
    use broadcast_common::Stage;
    let mut out = Vec::new();
    while let Some(item) = seg.poll() {
        out.push(item);
    }
    out
}

/// `mfhd.sequence_number` of the first `moof` in `data`.
fn mfhd_sequence(data: &[u8]) -> u32 {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size = u32::from_be_bytes(data[off..off + 4].try_into().expect("4 bytes")) as usize;
        let ty = &data[off + 4..off + 8];
        if ty == b"moof" && off + 24 <= data.len() {
            // moof header (8) + mfhd header (8) + fullbox version/flags (4) +
            // sequence_number (4).
            return u32::from_be_bytes(
                data[off + 20..off + 24]
                    .try_into()
                    .expect("a mfhd sequence number"),
            );
        }
        if size < 8 {
            break;
        }
        off += size;
    }
    0
}

/// Demux (init + concatenated fragment bytes) back into a Media.
fn demux_bytes(init: &[u8], body: &[u8]) -> Media {
    let mut buf = Vec::with_capacity(init.len() + body.len());
    buf.extend_from_slice(init);
    buf.extend_from_slice(body);
    test_harness::collect::media_from_mp4(&buf[..], test_harness::collect::CollectMode::Seekable)
        .expect("re-demux fMP4")
}

/// The whole-segment media segment bytes produced by the batch path, for the
/// samples spanning [start,end) of each track. Mirrors what `Segmenter` cuts.
fn whole_segment(
    media: &Media,
    ranges: &[(TrackId, u64, std::ops::Range<usize>)],
    seq: u32,
) -> Vec<u8> {
    let frags: Vec<FragmentTrackData<'_>> = ranges
        .iter()
        .map(|(track_id, base, r)| {
            let t = media
                .tracks
                .iter()
                .find(|t| t.spec.track_id == *track_id)
                .unwrap();
            FragmentTrackData::new(*track_id, *base, &t.samples[r.clone()])
        })
        .collect();
    transmux::pipeline::build_media_segment(seq, &frags).expect("whole segment")
}

// ===========================================================================
// Test 1 — chunked structure
// ===========================================================================

#[test]
fn chunked_structure_multiple_chunks_contiguous_tfdt_and_seq() {
    let media = demux_media();
    // per-frame chunks: with a 0.5 s target a segment holds many frames.
    let chunks = chunks_for(&media, 1);
    assert!(
        chunks.len() > 1,
        "expected many chunks, got {}",
        chunks.len()
    );

    // Group by segment and assert N>1 chunks in the first full segment.
    let seg1: Vec<&ChunkView> = chunks.iter().filter(|c| c.segment_number == 1).collect();
    assert!(
        seg1.len() > 1,
        "segment 1 must split into >1 chunks, got {}",
        seg1.len()
    );

    // Each chunk parses as exactly one [styp?] + moof + mdat.
    for c in &chunks {
        let boxes = top_boxes(&c.data);
        let tys: Vec<[u8; 4]> = boxes.iter().map(|(t, _)| *t).collect();
        if c.is_segment_start {
            assert_eq!(
                tys,
                vec![*b"styp", *b"moof", *b"mdat"],
                "segment-start chunk = styp+moof+mdat"
            );
        } else {
            assert_eq!(
                tys,
                vec![*b"moof", *b"mdat"],
                "continuation chunk = moof+mdat"
            );
        }
    }

    // Sequence numbers strictly increase and are contiguous from 1.
    for (i, c) in chunks.iter().enumerate() {
        assert_eq!(c.sequence_number as usize, i + 1, "contiguous seq");
    }

    // tfdt base times per track are strictly increasing across the video track's
    // chunks (per-frame chunks → each carries the anchor's next frame).
    let vid_id = video_track(&media).spec.track_id;
    let mut last: Option<u64> = None;
    for c in &chunks {
        let moof = {
            let boxes = top_boxes(&c.data);
            let (_, r) = boxes.iter().find(|(t, _)| t == b"moof").unwrap();
            &c.data[r.clone()]
        };
        // Match tfdt bases to track order by reading traf track_ids.
        let trafs = child_boxes(&moof[8..], b"traf");
        let bases = moof_tfdt_bases(moof);
        for (traf, base) in trafs.iter().zip(&bases) {
            let tfhd = child_boxes(&traf[8..], b"tfhd").into_iter().next().unwrap();
            // tfhd: 8 header + 4 fullbox + 4 track_id
            let tid = u32::from_be_bytes(tfhd[12..16].try_into().unwrap());
            if tid == vid_id.value() {
                if let Some(prev) = last {
                    assert!(*base > prev, "video tfdt must increase: {prev} -> {base}");
                }
                last = Some(*base);
            }
        }
    }
    assert!(last.is_some(), "saw at least one video tfdt");
}

// ===========================================================================
// Test 2 — chunk concat == whole segment (lossless)
// ===========================================================================

#[test]
fn chunk_concat_equals_whole_segment_samples() {
    let media = demux_media();
    let chunks = chunks_for(&media, 3); // several frames per chunk

    let init = {
        let seg = LlSegmenter::new(
            specs(&media),
            media.movie_timescale,
            Duration::from_millis(500),
            3,
        )
        .unwrap();
        seg.init_segment().unwrap()
    };

    // Take segment 1's chunks and concatenate their bytes.
    let seg1: Vec<&ChunkView> = chunks.iter().filter(|c| c.segment_number == 1).collect();
    assert!(seg1.len() > 1);
    let mut concat = Vec::new();
    for c in &seg1 {
        concat.extend_from_slice(&c.data);
    }
    let chunked_media = demux_bytes(&init, &concat);

    // Determine the sample ranges segment 1 covers per track (count from the
    // re-demuxed chunk stream), then build the whole-segment equivalent.
    let mut ranges = Vec::new();
    for t in &chunked_media.tracks {
        let n = t.samples.len();
        ranges.push((t.spec.track_id, 0u64, 0..n));
    }
    let whole = whole_segment(&media, &ranges, 1);
    let whole_media = demux_bytes(&init, &whole);

    // Compare per track: coded sample bytes, in order, byte-identical.
    assert_eq!(chunked_media.tracks.len(), whole_media.tracks.len());
    for (a, b) in chunked_media.tracks.iter().zip(&whole_media.tracks) {
        assert_eq!(a.spec.track_id, b.spec.track_id);
        assert_eq!(
            a.samples.len(),
            b.samples.len(),
            "track {} sample count",
            a.spec.track_id
        );
        for (i, (sa, sb)) in a.samples.iter().zip(&b.samples).enumerate() {
            assert_eq!(
                sa.data, sb.data,
                "track {} sample {i} coded bytes differ",
                a.spec.track_id
            );
        }
    }
}

// ===========================================================================
// Test 3 — full-stream fidelity (all chunks, all segments)
// ===========================================================================

#[test]
fn full_stream_video_nal_sequence_preserved() {
    let media = demux_media();
    let chunks = chunks_for(&media, 2);

    let init = {
        let seg = LlSegmenter::new(
            specs(&media),
            media.movie_timescale,
            Duration::from_millis(500),
            2,
        )
        .unwrap();
        seg.init_segment().unwrap()
    };

    let mut body = Vec::new();
    for c in &chunks {
        body.extend_from_slice(&c.data);
    }
    let reconstructed = demux_bytes(&init, &body);

    let orig_vid = video_track(&media);
    let recon_vid = video_track(&reconstructed);

    assert_eq!(
        orig_vid.samples.len(),
        75,
        "fixture is expected to hold 75 video samples"
    );
    assert_eq!(
        recon_vid.samples.len(),
        orig_vid.samples.len(),
        "reconstructed video sample count"
    );
    for (i, (o, r)) in orig_vid.samples.iter().zip(&recon_vid.samples).enumerate() {
        assert_eq!(o.data, r.data, "video sample {i} coded NAL bytes differ");
    }
}

// ===========================================================================
// Test 4 — LL MPD attributes
// ===========================================================================

#[test]
fn ll_mpd_carries_availability_and_service_description() {
    let media = demux_media();
    let pkg = crate::ll_packager::ll_packager(
        Duration::from_secs(2),
        Duration::from_millis(500),
        Duration::from_secs(3),
    )
    .with_playback_rate(0.9, 1.1);
    let xml = crate::packagers::ll_dash(&pkg, &media).expect("LL MPD");

    let root = xml::parse(&xml);
    assert_eq!(root.name, "MPD");
    assert_eq!(root.attr("type"), Some("dynamic"), "MPD@type");
    assert_eq!(
        root.attr("availabilityStartTime"),
        Some("2026-01-01T00:00:00Z"),
        "MPD@availabilityStartTime"
    );

    // <ServiceDescription><Latency target="3000"/></ServiceDescription>
    let sd = root.find("ServiceDescription").expect("ServiceDescription");
    let latency = sd.find("Latency").expect("Latency");
    assert_eq!(latency.attr("target"), Some("3000"), "Latency@target ms");
    let pr = sd.find("PlaybackRate").expect("PlaybackRate");
    assert!(
        pr.has_attr("min") && pr.has_attr("max"),
        "PlaybackRate bounds"
    );

    // Every SegmentTemplate carries the LL availability attributes.
    let templates = descendants(&root, "SegmentTemplate");
    assert!(!templates.is_empty(), "at least one SegmentTemplate");
    for st in &templates {
        assert_eq!(
            st.attr("availabilityTimeComplete"),
            Some("false"),
            "availabilityTimeComplete"
        );
        let ato: f64 = st
            .attr("availabilityTimeOffset")
            .expect("availabilityTimeOffset")
            .parse()
            .expect("ATO numeric");
        assert!(ato > 0.0, "ATO must be positive, got {ato}");
        // ATO = segment - chunk = 2.0 - 0.5 = 1.5
        assert!((ato - 1.5).abs() < 1e-6, "ATO = seg - chunk");
    }
}

/// issue #994: `inject_ll` used to match only the self-closing
/// `<SegmentTemplate .../>` form (`Addressing::Number`). Under
/// `Addressing::Timeline`, the base [`DashPackager`] instead renders a
/// non-self-closing `<SegmentTemplate>` open tag with a `<SegmentTimeline>`
/// child and a separate `</SegmentTemplate>` close tag — so the LL attributes
/// were silently never injected for any Timeline-addressed MPD.
#[test]
fn ll_mpd_carries_availability_for_timeline_addressing() {
    let media = demux_media();
    // Any non-empty per-track duration list satisfies `Addressing::Timeline`
    // (see `dash_mpd.rs`'s `timeline_addressing_without_segments_is_rejected`
    // for what it rejects) — the durations' real values don't matter to this
    // string-injection fix, only the resulting element shape does.
    let log = crate::packagers::log(&media);
    let segments: Vec<transmux::TrackSegments> = media
        .tracks
        .iter()
        .map(|t| transmux::TrackSegments {
            track_id: t.spec.track_id,
            durations: vec![90_000, 90_000],
        })
        .collect();

    let mut pkg = crate::ll_packager::ll_packager(
        Duration::from_secs(2),
        Duration::from_millis(500),
        Duration::from_secs(3),
    );
    pkg.base.addressing = transmux::Addressing::Timeline;
    pkg.base.segments = segments;
    let xml =
        crate::packagers::ll_dash_with(&pkg, &media, &log).expect("LL MPD (Timeline addressing)");

    let root = xml::parse(&xml);
    let templates = descendants(&root, "SegmentTemplate");
    assert!(!templates.is_empty(), "at least one SegmentTemplate");
    for st in &templates {
        // Confirm this really is the non-self-closing shape the fix targets.
        assert!(
            st.find("SegmentTimeline").is_some(),
            "Timeline addressing must render a SegmentTimeline child"
        );
        assert_eq!(
            st.attr("availabilityTimeComplete"),
            Some("false"),
            "availabilityTimeComplete must be injected on the Timeline SegmentTemplate open tag"
        );
        let ato: f64 = st
            .attr("availabilityTimeOffset")
            .expect(
                "availabilityTimeOffset must be injected on the Timeline SegmentTemplate open tag",
            )
            .parse()
            .expect("ATO numeric");
        assert!((ato - 1.5).abs() < 1e-6, "ATO = seg - chunk = 2.0 - 0.5");
    }
}

// ===========================================================================
// Test 5 — keyframe-aligned segment starts preserved
// ===========================================================================

#[test]
fn segment_first_chunk_starts_on_keyframe() {
    let media = demux_media();
    let chunks = chunks_for(&media, 1);
    let vid_id = video_track(&media).spec.track_id;

    // Reconstruct the whole stream once to know sync flags in decode order,
    // then check each segment's first chunk's first video sample is a keyframe.
    // We do this directly on chunk moofs: the first chunk of each segment must,
    // for the video track, carry a sample whose flags mark it sync.
    let mut seen_segments = std::collections::BTreeSet::new();
    for c in &chunks {
        if !c.is_segment_start {
            continue;
        }
        seen_segments.insert(c.segment_number);
        let boxes = top_boxes(&c.data);
        let (_, r) = boxes.iter().find(|(t, _)| t == b"moof").unwrap();
        let moof = &c.data[r.clone()];
        let trafs = child_boxes(&moof[8..], b"traf");
        let mut checked_video = false;
        for traf in &trafs {
            let tfhd = child_boxes(&traf[8..], b"tfhd").into_iter().next().unwrap();
            let tid = u32::from_be_bytes(tfhd[12..16].try_into().unwrap());
            if tid != vid_id.value() {
                continue;
            }
            checked_video = true;
            let first_sync = first_sample_is_sync(traf);
            assert!(
                first_sync,
                "segment {} first chunk: first video sample must be a keyframe",
                c.segment_number
            );
        }
        assert!(
            checked_video,
            "segment {} first chunk must contain the video track",
            c.segment_number
        );
    }
    assert!(
        seen_segments.len() >= 2,
        "fixture should produce >=2 segments at 0.5s target, got {}",
        seen_segments.len()
    );
}

/// True if the first sample in a traf's trun is a sync sample (per §8.8.3.1
/// sample_is_non_sync_sample == 0).
fn first_sample_is_sync(traf: &[u8]) -> bool {
    const SAMPLE_FLAG_IS_NON_SYNC: u32 = 0x0001_0000;
    let trun = child_boxes(&traf[8..], b"trun").into_iter().next().unwrap();
    // trun: 8 header, then version(1)+flags(3), sample_count(4), then optional
    // data_offset(4) + first_sample_flags(4) per the tr_flags.
    let flags = u32::from_be_bytes([0, trun[9], trun[10], trun[11]]);
    let data_offset_present = flags & 0x000001 != 0;
    let first_sample_flags_present = flags & 0x000004 != 0;
    let sample_duration_present = flags & 0x000100 != 0;
    let sample_size_present = flags & 0x000200 != 0;
    let sample_flags_present = flags & 0x000400 != 0;
    let cto_present = flags & 0x000800 != 0;

    let mut off = 16usize; // after header(8)+fullbox(4)+sample_count(4)
    if data_offset_present {
        off += 4;
    }
    if first_sample_flags_present {
        // First sample uses first_sample_flags directly.
        let fsf = u32::from_be_bytes(trun[off..off + 4].try_into().unwrap());
        return fsf & SAMPLE_FLAG_IS_NON_SYNC == 0;
    }
    // Otherwise read the first per-sample record's flags field.
    let mut field = off;
    if sample_duration_present {
        field += 4;
    }
    if sample_size_present {
        field += 4;
    }
    if sample_flags_present {
        let sf = u32::from_be_bytes(trun[field..field + 4].try_into().unwrap());
        let _ = cto_present;
        return sf & SAMPLE_FLAG_IS_NON_SYNC == 0;
    }
    // No per-sample flags → defaults; the builder always sets them, so we should
    // not reach here for our output.
    true
}

// ---------------------------------------------------------------------------
// Element tree over `quick-xml` for the MPD.
// ---------------------------------------------------------------------------

fn descendants<'a>(e: &'a xml::Element, name: &str) -> Vec<&'a xml::Element> {
    let mut out = Vec::new();
    fn rec<'a>(e: &'a xml::Element, name: &str, out: &mut Vec<&'a xml::Element>) {
        for c in &e.children {
            if c.name == name {
                out.push(c);
            }
            rec(c, name, out);
        }
    }
    if e.name == name {
        out.push(e);
    }
    rec(e, name, &mut out);
    out
}

mod xml {
    #[derive(Debug, Clone)]
    pub struct Element {
        pub name: String,
        pub attrs: Vec<(String, String)>,
        pub children: Vec<Element>,
    }

    impl Element {
        pub fn attr(&self, key: &str) -> Option<&str> {
            self.attrs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        }
        pub fn has_attr(&self, key: &str) -> bool {
            self.attrs.iter().any(|(k, _)| k == key)
        }
        pub fn find<'a>(&'a self, name: &str) -> Option<&'a Element> {
            self.children.iter().find(|c| c.name == name)
        }
    }

    /// Parse `s` with `quick-xml` into an [`Element`] tree, asserting the
    /// document is well-formed (matched tags, one root, nothing but whitespace
    /// outside it). Attribute values come back unescaped.
    pub fn parse(s: &str) -> Element {
        use quick_xml::Reader;
        use quick_xml::XmlVersion;
        use quick_xml::events::{BytesStart, Event};

        fn open(e: &BytesStart<'_>) -> Element {
            let attrs = e
                .attributes()
                .map(|a| {
                    let a = a.expect("well-formed attribute");
                    let v = a
                        .normalized_value(XmlVersion::Implicit1_0)
                        .expect("resolvable attribute value");
                    (a.key.as_ref().to_string(), v.into_owned())
                })
                .collect();
            Element {
                name: e.name().as_ref().to_string(),
                attrs,
                children: Vec::new(),
            }
        }

        let mut reader = Reader::from_str(s);
        let mut stack = vec![Element {
            name: String::new(),
            attrs: Vec::new(),
            children: Vec::new(),
        }];
        loop {
            match reader.read_event().expect("well-formed XML") {
                Event::Start(e) => stack.push(open(&e)),
                Event::Empty(e) => {
                    let el = open(&e);
                    stack.last_mut().expect("document node").children.push(el);
                }
                Event::End(_) => {
                    let el = stack.pop().expect("open element");
                    stack.last_mut().expect("document node").children.push(el);
                }
                Event::Text(t) if stack.len() == 1 => {
                    assert!(
                        t.trim_ascii().is_empty(),
                        "content outside the root element (not well-formed)"
                    );
                }
                Event::Eof => break,
                _ => {}
            }
        }
        assert_eq!(stack.len(), 1, "unclosed element at end of document");
        let mut doc = stack.pop().expect("document node");
        assert_eq!(doc.children.len(), 1, "exactly one root element");
        doc.children.pop().expect("root element")
    }
}

/// `MPD/UTCTiming` (ISO/IEC 23009-1 §5.8.4.11) is a direct child of `MPD` and
/// carries the caller-supplied scheme + value, escaped for XML.
#[test]
fn ll_mpd_carries_utc_timing() {
    let media = demux_media();
    let pkg = crate::ll_packager::ll_packager(
        Duration::from_secs(2),
        Duration::from_millis(500),
        Duration::from_secs(3),
    )
    .with_utc_timing(
        transmux::ll_dash::UTCTIMING_HTTP_HEAD_2014,
        "https://time.example/x?a=1&b=2",
    );
    let xml = crate::packagers::ll_dash(&pkg, &media).expect("LL MPD");

    let root = xml::parse(&xml);
    let utc = root.find("UTCTiming").expect("UTCTiming element");
    assert_eq!(
        utc.attr("schemeIdUri"),
        Some("urn:mpeg:dash:utc:http-head:2014"),
        "registered 2014 scheme"
    );
    // quick-xml decodes the attribute, so it reads back as the caller's value.
    assert_eq!(
        utc.attr("value"),
        Some("https://time.example/x?a=1&b=2"),
        "the escaped value must round-trip to the caller's string"
    );
    // ... and on the wire the raw `&` must be escaped to `&amp;` (it would
    // otherwise not be well-formed XML).
    assert!(
        xml.contains(r#"value="https://time.example/x?a=1&amp;b=2""#),
        "a raw `&` in the value must be escaped to `&amp;` on the wire"
    );
}

/// Without `with_utc_timing` the element is absent (opt-in, so an existing
/// caller's bytes are unchanged).
#[test]
fn ll_mpd_omits_utc_timing_by_default() {
    let media = demux_media();
    let pkg = crate::ll_packager::ll_packager(
        Duration::from_secs(2),
        Duration::from_millis(500),
        Duration::from_secs(3),
    );
    let xml = crate::packagers::ll_dash(&pkg, &media).expect("LL MPD");
    assert!(
        !xml.contains("<UTCTiming"),
        "no UTCTiming unless requested: {xml}"
    );
}

/// Every non-final chunk of a muxed A/V segment carries both tracks, not only
/// the segment's last chunk (audit r05-W27).
#[test]
fn every_ll_dash_chunk_carries_both_tracks() {
    let media = demux_media();
    // 8-video-AU chunks over a real A/V fixture: several chunks per segment.
    let chunks = chunks_for(&media, 8);
    assert!(
        chunks.len() >= 3,
        "expect several chunks, got {}",
        chunks.len()
    );

    for (i, c) in chunks.iter().enumerate() {
        let moof = top_boxes(&c.data)
            .into_iter()
            .find(|(t, _)| t == b"moof")
            .map(|(_, r)| c.data[r.start + 8..r.end].to_vec())
            .expect("moof in chunk");
        let trafs = moof.windows(4).filter(|w| *w == b"traf").count();
        assert_eq!(
            trafs, 2,
            "chunk {i} (segment_start={}) must carry video + audio",
            c.is_segment_start
        );
    }
}

/// Every registered `urn:mpeg:dash:utc:*:2014` scheme constant is emitted
/// verbatim as `UTCTiming@schemeIdUri` with its `@value`.
#[test]
fn ll_mpd_emits_each_registered_utc_timing_scheme() {
    use transmux::ll_dash::{
        UTCTIMING_DIRECT_2014, UTCTIMING_HTTP_ISO_2014, UTCTIMING_HTTP_NTP_2014,
        UTCTIMING_HTTP_XSDATE_2014, UTCTIMING_NTP_2014,
    };
    let media = demux_media();
    let schemes = [
        (
            UTCTIMING_HTTP_XSDATE_2014,
            "urn:mpeg:dash:utc:http-xsdate:2014",
        ),
        (UTCTIMING_HTTP_ISO_2014, "urn:mpeg:dash:utc:http-iso:2014"),
        (UTCTIMING_NTP_2014, "urn:mpeg:dash:utc:ntp:2014"),
        (UTCTIMING_HTTP_NTP_2014, "urn:mpeg:dash:utc:http-ntp:2014"),
        (UTCTIMING_DIRECT_2014, "urn:mpeg:dash:utc:direct:2014"),
    ];
    for (constant, uri) in schemes {
        assert_eq!(constant, uri);
        let pkg = crate::ll_packager::ll_packager(
            Duration::from_secs(2),
            Duration::from_millis(500),
            Duration::from_secs(3),
        )
        .with_utc_timing(constant, "time.example");
        let xml = crate::packagers::ll_dash(&pkg, &media).expect("LL MPD");
        let root = xml::parse(&xml);
        let utc = root.find("UTCTiming").expect("UTCTiming element");
        assert_eq!(utc.attr("schemeIdUri"), Some(uri));
        assert_eq!(utc.attr("value"), Some("time.example"));
    }
}

// ===========================================================================
// take_span: the sample that lands exactly on the chunk target is included
// ===========================================================================

fn ninety_k(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn two_video_specs() -> Vec<TrackSpec> {
    let mut second = crate::hub_fixtures::video_track();
    second.track_id = TrackId::new(3);
    vec![crate::hub_fixtures::video_track(), second]
}

fn av_sample(byte: u8, duration: u32) -> Sample {
    Sample::new(vec![byte; 16], None, None, Some(duration), true).expect("valid timing")
}

/// `(track_id, sample_count)` for every `traf` in a chunk's `moof`.
fn chunk_traf_counts(chunk: &[u8]) -> Vec<(u32, usize)> {
    let moof = {
        let boxes = top_boxes(chunk);
        let (_, r) = boxes.iter().find(|(t, _)| t == b"moof").expect("moof");
        &chunk[r.clone()]
    };
    let mut out = Vec::new();
    for traf in child_boxes(&moof[8..], b"traf") {
        let mut track_id = None;
        if let Some(tfhd) = child_boxes(&traf[8..], b"tfhd").into_iter().next() {
            track_id = Some(u32::from_be_bytes(
                tfhd[12..16].try_into().expect("tfhd track id"),
            ));
        }
        let mut samples = 0usize;
        for trun in child_boxes(&traf[8..], b"trun") {
            samples +=
                u32::from_be_bytes(trun[12..16].try_into().expect("trun sample count")) as usize;
        }
        if let Some(id) = track_id {
            out.push((id, samples));
        }
    }
    out
}

fn samples_for(chunk: &[u8], track: u32) -> usize {
    chunk_traf_counts(chunk)
        .into_iter()
        .find(|(id, _)| *id == track)
        .map_or(0, |(_, n)| n)
}

#[test]
fn chunk_takes_the_sample_that_lands_exactly_on_its_target() {
    // 25 fps (3600 ticks), two anchor samples per chunk: the chunk span is
    // exactly 7200 ticks. The second track's sample is 3600 ticks, so a chunk
    // that takes 2 anchor samples must take 2 of its samples, not 1.
    let mut seg = LlSegmenter::new(
        two_video_specs(),
        ninety_k(1000),
        Duration::from_secs(10),
        2,
    )
    .expect("LlSegmenter constructor");

    let video = TrackId::new(1);
    let other = TrackId::new(3);
    seg.push(video, av_sample(0, 3600)).unwrap();
    seg.push(other, av_sample(10, 3600)).unwrap();
    seg.push(other, av_sample(11, 3600)).unwrap();
    seg.push(video, av_sample(1, 3600)).unwrap();
    let first = view_chunks(drain(&mut seg));
    assert_eq!(first.len(), 1, "one chunk after the second anchor sample");
    assert_eq!(
        samples_for(&first[0].data, 3),
        2,
        "exact-fit chunk must carry both 3600-tick samples"
    );

    // Three pending samples, only two fit: the crossing one stays behind.
    seg.push(video, av_sample(2, 3600)).unwrap();
    seg.push(other, av_sample(12, 3600)).unwrap();
    seg.push(other, av_sample(13, 3600)).unwrap();
    seg.push(other, av_sample(14, 3600)).unwrap();
    seg.push(video, av_sample(3, 3600)).unwrap();
    let second = view_chunks(drain(&mut seg));
    assert_eq!(
        second.len(),
        1,
        "one chunk after the next second anchor sample"
    );
    assert_eq!(
        samples_for(&second[0].data, 3),
        2,
        "a chunk over three pending samples must keep the crossing sample for the next chunk"
    );
}

#[test]
fn chunk_target_one_tick_past_the_fit_still_takes_two_samples() {
    let mut seg = LlSegmenter::new(
        two_video_specs(),
        ninety_k(1000),
        Duration::from_secs(10),
        2,
    )
    .expect("LlSegmenter constructor");
    let video = TrackId::new(1);
    let other = TrackId::new(3);
    // Anchor span 3600 + 3601 = 7201 ticks; the 3600-tick sample still fits.
    seg.push(video, av_sample(0, 3600)).unwrap();
    seg.push(other, av_sample(10, 3600)).unwrap();
    seg.push(other, av_sample(11, 3600)).unwrap();
    seg.push(video, av_sample(1, 3601)).unwrap();
    let chunks = view_chunks(drain(&mut seg));
    assert_eq!(samples_for(&chunks[0].data, 3), 2);
}

#[test]
fn chunk_target_one_tick_short_of_the_fit_takes_one_sample() {
    let mut seg = LlSegmenter::new(
        two_video_specs(),
        ninety_k(1000),
        Duration::from_secs(10),
        2,
    )
    .expect("LlSegmenter constructor");
    let video = TrackId::new(1);
    let other = TrackId::new(3);
    // Anchor span 3600 + 3599 = 7199 ticks; the 3600-tick sample crosses it.
    seg.push(video, av_sample(0, 3600)).unwrap();
    seg.push(other, av_sample(10, 3600)).unwrap();
    seg.push(other, av_sample(11, 3600)).unwrap();
    seg.push(video, av_sample(1, 3599)).unwrap();
    let chunks = view_chunks(drain(&mut seg));
    assert_eq!(samples_for(&chunks[0].data, 3), 1);
}

// ===========================================================================
// LlSegmenter::demand — saturated exactly at the pending bound
// ===========================================================================

#[test]
fn ll_dash_demand_is_saturated_exactly_at_the_pending_bound() {
    let mut seg = LlSegmenter::new(
        two_video_specs(),
        ninety_k(90_000),
        Duration::from_secs(3600),
        usize::MAX,
    )
    .expect("LlSegmenter constructor");
    let video = TrackId::new(1);
    for i in 0..MAX_PENDING_SAMPLES_PER_TRACK - 1 {
        seg.push(
            video,
            Sample::new(
                vec![0u8; 16],
                Some(i as i64 * 3600),
                None,
                Some(3600),
                i == 0,
            )
            .expect("valid timing"),
        )
        .unwrap();
    }
    assert!(
        !Stage::demand(&seg).saturated,
        "one sample below the bound is not saturated"
    );
    seg.push(
        video,
        Sample::new(
            vec![0u8; 16],
            Some((MAX_PENDING_SAMPLES_PER_TRACK - 1) as i64 * 3600),
            None,
            Some(3600),
            false,
        )
        .expect("valid timing"),
    )
    .unwrap();
    assert!(
        Stage::demand(&seg).saturated,
        "saturated exactly at MAX_PENDING_SAMPLES_PER_TRACK"
    );
    let err = seg
        .push(
            video,
            Sample::new(vec![0u8; 16], Some(0), None, Some(3600), false).expect("valid timing"),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("MAX_PENDING_SAMPLES_PER_TRACK"),
        "the push past the bound is the named stall error: {err}"
    );
}

// ===========================================================================
// Whole-`Media` packaging — equal decode time merges the non-anchor track first
// ===========================================================================

// ── Issue #4154: a break is handed to the caller as a period boundary ──────

/// A chunk stream cannot state a break itself; the MPD does, as a new Period.
/// The segmenter therefore emits `MuxOutput::Discontinuity` in place, so a
/// caller records it in the `SegmentSummaryLog` instead of losing it.
#[test]
fn a_break_is_handed_to_the_caller_as_a_period_boundary() {
    use transmux::mux_stage::MuxOutput;

    let two = crate::discontinuity::forward();
    let specs: Vec<_> = two.media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut segmenter = LlSegmenter::new(
        specs,
        broadcast_core::timescale::Timescale::try_new(1000).expect("non-zero timescale"),
        core::time::Duration::from_secs(2),
        1,
    )
    .expect("LL-DASH segmenter");
    let outputs = crate::discontinuity::drive_fixed(&mut segmenter, &two, None)
        .expect("a forward break is expressible");
    let marks: Vec<Option<transmux::TrackId>> = outputs
        .iter()
        .filter_map(|output| match output {
            MuxOutput::Discontinuity { track } => Some(*track),
            _ => None,
        })
        .collect();
    assert_eq!(marks, vec![None], "the break reaches the caller once");
    let index = outputs
        .iter()
        .position(|output| matches!(output, MuxOutput::Discontinuity { .. }))
        .expect("the break is in the output");
    assert!(
        outputs[..index]
            .iter()
            .any(|output| matches!(output, MuxOutput::Bytes(_))),
        "chunks precede the break"
    );
    assert!(
        outputs[index + 1..]
            .iter()
            .any(|output| matches!(output, MuxOutput::Bytes(_))),
        "chunks follow it"
    );
}

/// The break reaches the viewer, not just the caller's output queue: the
/// segment log the packager renders from opens a new `Period`, the previous
/// one closes with a duration, the next maps its continuing media clock with
/// `presentationTimeOffset`, and the period timeline is continuous.
#[test]
fn a_break_renders_a_new_period_from_the_segment_log() {
    let two = crate::discontinuity::forward();
    let specs: Vec<_> = two.media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut segmenter = LlSegmenter::new(
        specs.clone(),
        broadcast_core::timescale::Timescale::try_new(1000).expect("non-zero timescale"),
        core::time::Duration::from_secs(2),
        1,
    )
    .expect("LL-DASH segmenter");
    crate::discontinuity::drive_fixed(&mut segmenter, &two, None)
        .expect("a forward break is expressible");

    let mut packager = transmux::ll_dash::LlDashPackager::new(
        core::time::Duration::from_secs(2),
        core::time::Duration::from_secs(1),
        core::time::Duration::from_secs(3),
        "2026-01-01T00:00:00Z",
    )
    .expect("LL-DASH packager");
    packager.base.publish_time = Some("2026-01-01T00:00:00Z".to_string());
    let xml = packager
        .render(&specs, segmenter.segment_log())
        .expect("render the LL-DASH MPD");
    if let Some(schema) = test_fixtures::process::xsd::dash_mpd_schema("ll_dash_period_break") {
        test_fixtures::process::xsd::validate(&schema, "ll_dash_period_break", &xml)
            .unwrap_or_else(|error| panic!("xmllint rejected the LL-DASH MPD:\n{error}"));
    }
    let mpd = broadcast_dash::Mpd::parse(xml.as_bytes()).expect("the MPD parses");
    assert!(
        mpd.periods.len() >= 2,
        "the break must open a new Period, got {}:\n{xml}",
        mpd.periods.len()
    );
    let mut expected_start = core::time::Duration::ZERO;
    let mut offsets = Vec::new();
    for (index, period) in mpd.periods.iter().enumerate() {
        assert_eq!(
            period.start,
            Some(expected_start),
            "period {index} starts where the previous one closed"
        );
        let duration = period
            .duration
            .unwrap_or_else(|| panic!("period {index} states a duration"));
        let mut period_offsets = Vec::new();
        for set in &period.adaptation_sets {
            for representation in &set.representations {
                let template = representation
                    .segment
                    .template
                    .as_ref()
                    .expect("every Representation has a SegmentTemplate");
                period_offsets.push(template.multi.base.presentation_time_offset.unwrap_or(0));
            }
        }
        if index == 0 {
            assert!(
                period_offsets.iter().all(|offset| *offset == 0),
                "the first Period starts at media zero: {period_offsets:?}"
            );
        } else {
            assert!(
                period_offsets.iter().all(|offset| *offset > 0),
                "period {index}'s media clock continues past the break: {period_offsets:?}"
            );
        }
        offsets.push(period_offsets);
        expected_start += duration;
    }
    assert!(offsets.len() >= 2);
}
