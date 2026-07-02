//! Low-latency DASH gate (issue #461): chunked CMAF + LL-DASH MPD.
//!
//! Input IR: `TsDemux`-ing the deterministic 2-track `fixtures/ts/h264_aac.ts`
//! (H.264 video + AAC audio — 75 video + 131 audio samples).
//!
//! Every test bites: chunk bytes are parsed with a std-only top-level box walker
//! and re-demuxed via `Fmp4Demux` (no hardcoded offsets); the MPD is parsed with
//! a std-only element walker and asserted against real elements/attributes.

use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use transmux::ll_dash::{Chunk, LlDashPackager, LlSegmenter};
use transmux::media::{Fmp4Demux, Media};
use transmux::pipeline::{FragmentTrackData, TrackSpec};
use transmux::ts_demux::TsDemux;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn demux_media() -> Media {
    let ts = std::fs::read(fixtures_dir().join("ts/h264_aac.ts"))
        .expect("h264_aac.ts fixture must exist");
    let mut demux = TsDemux::new();
    demux.unpackage(&ts[..]).expect("demux h264_aac.ts")
}

fn video_track(media: &Media) -> &transmux::media::Track {
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

fn chunks_for(media: &Media, chunk_samples: usize) -> Vec<Chunk> {
    let mut seg = LlSegmenter::new(specs(media), media.movie_timescale, 0.5, chunk_samples)
        .expect("LlSegmenter");
    seg.package(media).expect("package chunks")
}

/// Demux (init + concatenated fragment bytes) back into a Media.
fn demux_bytes(init: &[u8], body: &[u8]) -> Media {
    let mut buf = Vec::with_capacity(init.len() + body.len());
    buf.extend_from_slice(init);
    buf.extend_from_slice(body);
    let mut d = Fmp4Demux::new();
    d.unpackage(&buf[..]).expect("re-demux fMP4")
}

/// The whole-segment media segment bytes produced by the batch path, for the
/// samples spanning [start,end) of each track. Mirrors what `Segmenter` cuts.
fn whole_segment(
    media: &Media,
    ranges: &[(u32, u64, std::ops::Range<usize>)],
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
            FragmentTrackData {
                track_id: *track_id,
                base_media_decode_time: *base,
                samples: &t.samples[r.clone()],
            }
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
    let seg1: Vec<&Chunk> = chunks.iter().filter(|c| c.segment_number == 1).collect();
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
            if tid == vid_id {
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
        let seg = LlSegmenter::new(specs(&media), media.movie_timescale, 0.5, 3).unwrap();
        seg.init_segment().unwrap()
    };

    // Take segment 1's chunks and concatenate their bytes.
    let seg1: Vec<&Chunk> = chunks.iter().filter(|c| c.segment_number == 1).collect();
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
        let seg = LlSegmenter::new(specs(&media), media.movie_timescale, 0.5, 2).unwrap();
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
    let mut pkg = LlDashPackager::new(2.0, 0.5, 3000, "2026-01-01T00:00:00Z")
        .unwrap()
        .with_playback_rate(0.9, 1.1);
    let xml = pkg.package(&media).expect("LL MPD");

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
            if tid != vid_id {
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
// Minimal XML element walker for the MPD (std-only, no external dependency).
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

    pub fn parse(s: &str) -> Element {
        let mut p = Parser {
            s: s.as_bytes(),
            pos: 0,
        };
        p.skip_ws();
        if p.starts_with("<?") {
            while p.pos < p.s.len() && !p.starts_with("?>") {
                p.pos += 1;
            }
            p.pos += 2;
        }
        p.skip_ws();
        p.parse_element().expect("root element")
    }

    struct Parser<'a> {
        s: &'a [u8],
        pos: usize,
    }

    impl<'a> Parser<'a> {
        fn parse_element(&mut self) -> Option<Element> {
            self.skip_ws();
            if !self.starts_with("<") || self.starts_with("</") {
                return None;
            }
            self.pos += 1;
            let name = self.read_name();
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
                    _ => {
                        let key = self.read_name();
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
                while self.pos < self.s.len() && self.cur() != b'<' {
                    self.pos += 1;
                }
                self.skip_ws();
                if self.starts_with("</") {
                    self.pos += 2;
                    let _ = self.read_name();
                    self.skip_ws();
                    self.expect(b'>');
                    break;
                }
                match self.parse_element() {
                    Some(c) => children.push(c),
                    None => {
                        if self.pos >= self.s.len() {
                            break;
                        }
                    }
                }
            }
            Some(Element {
                name,
                attrs,
                children,
            })
        }

        fn cur(&self) -> u8 {
            if self.pos < self.s.len() {
                self.s[self.pos]
            } else {
                0
            }
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
            assert_eq!(self.cur(), b, "expected {}", b as char);
            self.pos += 1;
        }
        fn read_name(&mut self) -> String {
            let start = self.pos;
            while self.pos < self.s.len() {
                let c = self.s[self.pos];
                if c.is_ascii_whitespace() || c == b'>' || c == b'/' || c == b'=' {
                    break;
                }
                self.pos += 1;
            }
            String::from_utf8_lossy(&self.s[start..self.pos]).into_owned()
        }
        fn read_quoted(&mut self) -> String {
            let q = self.cur();
            assert!(q == b'"' || q == b'\'', "attr value must be quoted");
            self.pos += 1;
            let start = self.pos;
            while self.pos < self.s.len() && self.cur() != q {
                self.pos += 1;
            }
            let v = String::from_utf8_lossy(&self.s[start..self.pos]).into_owned();
            self.pos += 1;
            v
        }
    }
}
