//! DASH `.mpd` manifest generation gate (issue #464).
//!
//! Oracle: `fixtures/dash/manifest.mpd` is a real ffmpeg-generated DASH MPD for
//! a 2-track (video + audio) CMAF. The input IR is built by `TsDemux`-ing the
//! deterministic 2-track `fixtures/ts/h264_aac.ts` (H.264 video + AAC audio)
//! and fed to [`DashPackager`].
//!
//! Every test bites: the produced XML is parsed with a std-only element walker
//! (no XML dependency) and asserted against real structure — never a bare
//! substring `contains`, and codec/geometry values are asserted against what
//! the crate itself computes, not hardcoded literals.

use std::path::PathBuf;

use broadcast_common::{Package, Parse, Unpackage};
use transmux::aac_asc::AudioSpecificConfig;
use transmux::dash::{DashPackager, MPD_NAMESPACE};
use transmux::pipeline::CodecConfig;
use transmux::sps::rfc6381_avc1;
use transmux::ts_demux::TsDemux;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn demux_media() -> transmux::media::Media {
    let ts = std::fs::read(fixtures_dir().join("ts/h264_aac.ts"))
        .expect("h264_aac.ts fixture must exist");
    let mut demux = TsDemux::new();
    demux.unpackage(&ts[..]).expect("demux h264_aac.ts")
}

fn build_mpd() -> String {
    let media = demux_media();
    let mut pkg = DashPackager::default();
    pkg.package(&media).expect("package DASH MPD")
}

fn ref_mpd() -> String {
    std::fs::read_to_string(fixtures_dir().join("dash/manifest.mpd"))
        .expect("reference manifest.mpd must exist")
}

// ---------------------------------------------------------------------------
// Minimal XML walker — no external dependency.
// ---------------------------------------------------------------------------

/// A parsed XML element (name + attributes + children). Text content is ignored
/// (the MPD carries none we assert on).
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
    /// Direct children with the given tag name.
    fn find_all<'a>(&'a self, name: &str) -> Vec<&'a Element> {
        self.children.iter().filter(|c| c.name == name).collect()
    }
    fn find<'a>(&'a self, name: &str) -> Option<&'a Element> {
        self.children.iter().find(|c| c.name == name)
    }
    /// Recursive descendant search (first match, DFS).
    fn descendant<'a>(&'a self, name: &str) -> Option<&'a Element> {
        for c in &self.children {
            if c.name == name {
                return Some(c);
            }
            if let Some(d) = c.descendant(name) {
                return Some(d);
            }
        }
        None
    }
    /// Collect the names of every element in the tree (self + descendants).
    fn all_names(&self, out: &mut std::collections::BTreeSet<String>) {
        out.insert(self.name.clone());
        for c in &self.children {
            c.all_names(out);
        }
    }
}

/// A hand-rolled recursive-descent XML parser sufficient for the MPD structure:
/// the `<?xml?>` decl, elements, attributes (double-quoted values), self-closing
/// tags, and nesting. Not general-purpose (no entities, CDATA, comments) but the
/// MPD we produce and the ffmpeg oracle both stay within this grammar.
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

    fn parse_document(&mut self) -> Element {
        self.skip_ws();
        // Optional <?xml ...?> declaration.
        if self.starts_with("<?") {
            while self.pos < self.s.len() && !self.starts_with("?>") {
                self.pos += 1;
            }
            self.pos += 2; // consume "?>"
        }
        self.skip_ws();
        self.parse_element().expect("root element")
    }

    fn parse_element(&mut self) -> Option<Element> {
        self.skip_ws();
        // Skip comments / processing instructions between siblings.
        while self.starts_with("<!") || self.starts_with("<?") {
            while self.pos < self.s.len() && self.cur() != b'>' {
                self.pos += 1;
            }
            self.pos += 1;
            self.skip_ws();
        }
        if !self.starts_with("<") || self.starts_with("</") {
            return None;
        }
        self.pos += 1; // '<'
        let name = self.read_name();
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            match self.cur() {
                b'/' => {
                    // self-closing
                    self.pos += 1; // '/'
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
        // Children until the matching close tag.
        let mut children = Vec::new();
        loop {
            self.skip_ws();
            self.skip_text();
            self.skip_ws();
            if self.starts_with("</") {
                self.pos += 2;
                let _close = self.read_name();
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

    fn skip_text(&mut self) {
        while self.pos < self.s.len() && self.cur() != b'<' {
            self.pos += 1;
        }
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
        self.pos += 1; // closing quote
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
// Oracle helpers — compute the expected codec/geometry values from the IR.
// ---------------------------------------------------------------------------

/// The expected RFC 6381 codec string + SPS dimensions for the video track.
fn expected_video(media: &transmux::media::Media) -> (String, u32, u32) {
    let vid = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    match &vid.spec.config {
        CodecConfig::Avc { config, .. } => {
            let codecs = rfc6381_avc1(
                config.config.profile_indication,
                config.config.profile_compatibility,
                config.config.level_indication,
            );
            let sps = config.config.sps.first().expect("SPS");
            let info = sps.decode().expect("decode SPS");
            (codecs, info.width, info.height)
        }
        _ => unreachable!(),
    }
}

/// The expected audio RFC 6381 codec + sampling rate.
fn expected_audio(media: &transmux::media::Media) -> (String, u32) {
    let aud = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track");
    match &aud.spec.config {
        CodecConfig::Aac {
            esds, sample_rate, ..
        } => {
            let dsi = esds
                .es_descriptor
                .decoder_config
                .as_ref()
                .and_then(|dc| dc.decoder_specific_info.as_ref())
                .expect("ASC in esds");
            let asc = AudioSpecificConfig::parse(&dsi.data).expect("parse ASC");
            let rate = asc.sampling_frequency.unwrap_or(*sample_rate);
            (asc.rfc6381(), rate)
        }
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Test 1 — well-formed + schema shape.
// ---------------------------------------------------------------------------

#[test]
fn well_formed_and_schema_shape() {
    let xml = build_mpd();
    let root = parse_xml(&xml);

    assert_eq!(root.name, "MPD", "root element must be MPD");
    assert_eq!(
        root.attr("xmlns"),
        Some(MPD_NAMESPACE),
        "MPD must carry the DASH namespace"
    );
    assert!(
        root.has_attr("profiles"),
        "MPD must carry a profiles attribute"
    );

    let periods = root.find_all("Period");
    assert_eq!(periods.len(), 1, "exactly one Period");
    let period = periods[0];

    let sets = period.find_all("AdaptationSet");
    assert_eq!(sets.len(), 2, "exactly two AdaptationSets (video + audio)");

    // One video/mp4 and one audio/mp4 set, each with exactly one Representation.
    let mimes: std::collections::BTreeSet<&str> =
        sets.iter().filter_map(|s| s.attr("mimeType")).collect();
    assert!(mimes.contains("video/mp4"), "a video/mp4 AdaptationSet");
    assert!(mimes.contains("audio/mp4"), "an audio/mp4 AdaptationSet");

    for s in &sets {
        assert_eq!(
            s.find_all("Representation").len(),
            1,
            "each AdaptationSet has exactly one Representation"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2 — codec strings correct (against the crate's own computation).
// ---------------------------------------------------------------------------

#[test]
fn codec_strings_match_crate_computation() {
    let media = demux_media();
    let (want_video_codecs, _, _) = expected_video(&media);
    let (want_audio_codecs, _) = expected_audio(&media);
    assert_eq!(
        want_audio_codecs, "mp4a.40.2",
        "AAC-LC audio codec string sanity"
    );

    let xml = build_mpd();
    let root = parse_xml(&xml);
    let period = root.find("Period").unwrap();

    let video_set = period
        .find_all("AdaptationSet")
        .into_iter()
        .find(|s| s.attr("mimeType") == Some("video/mp4"))
        .expect("video set");
    let audio_set = period
        .find_all("AdaptationSet")
        .into_iter()
        .find(|s| s.attr("mimeType") == Some("audio/mp4"))
        .expect("audio set");

    let video_repr = video_set.find("Representation").unwrap();
    let audio_repr = audio_set.find("Representation").unwrap();

    assert_eq!(
        video_repr.attr("codecs"),
        Some(want_video_codecs.as_str()),
        "video @codecs must equal the crate's rfc6381_avc1 output"
    );
    assert_eq!(
        audio_repr.attr("codecs"),
        Some(want_audio_codecs.as_str()),
        "audio @codecs must equal the crate's ASC rfc6381 output"
    );

    // mimeType present + correct on the AdaptationSet.
    assert_eq!(video_set.attr("mimeType"), Some("video/mp4"));
    assert_eq!(audio_set.attr("mimeType"), Some("audio/mp4"));
}

// ---------------------------------------------------------------------------
// Test 3 — SegmentTemplate structure matches the real reference MPD.
// ---------------------------------------------------------------------------

#[test]
fn segment_template_structure_matches_reference() {
    let ours = parse_xml(&build_mpd());
    let reference = parse_xml(&ref_mpd());

    // The reference MPD carries these structural elements; ours must too.
    let mut ref_names = std::collections::BTreeSet::new();
    reference.all_names(&mut ref_names);
    let mut our_names = std::collections::BTreeSet::new();
    ours.all_names(&mut our_names);

    for e in [
        "MPD",
        "Period",
        "AdaptationSet",
        "Representation",
        "SegmentTemplate",
    ] {
        assert!(
            ref_names.contains(e),
            "reference MPD unexpectedly lacks <{e}>"
        );
        assert!(our_names.contains(e), "our MPD lacks <{e}>");
    }

    // SegmentTemplate attribute-presence must match the reference set.
    let ref_st = reference
        .descendant("SegmentTemplate")
        .expect("reference SegmentTemplate");
    let our_st = ours
        .descendant("SegmentTemplate")
        .expect("our SegmentTemplate");

    for attr in ["timescale", "startNumber", "initialization", "media"] {
        assert!(
            ref_st.has_attr(attr),
            "reference SegmentTemplate lacks @{attr}"
        );
        assert!(our_st.has_attr(attr), "our SegmentTemplate lacks @{attr}");
    }
    // We additionally carry @duration (number+duration addressing); require it.
    assert!(
        our_st.has_attr("duration"),
        "our SegmentTemplate must carry @duration"
    );

    // Templates must be addressing templates ($Number$ or $RepresentationID$).
    let init = our_st.attr("initialization").unwrap();
    let media = our_st.attr("media").unwrap();
    assert!(
        init.contains("$RepresentationID$"),
        "initialization template must reference $RepresentationID$: {init}"
    );
    assert!(
        media.contains("$Number$") && media.contains("$RepresentationID$"),
        "media template must reference $Number$ and $RepresentationID$: {media}"
    );

    // timescale/startNumber must be positive integers.
    let ts: u64 = our_st
        .attr("timescale")
        .unwrap()
        .parse()
        .expect("timescale int");
    let sn: u64 = our_st
        .attr("startNumber")
        .unwrap()
        .parse()
        .expect("startNumber int");
    assert!(ts > 0, "timescale must be positive");
    assert!(sn >= 1, "startNumber must be >= 1");
}

// ---------------------------------------------------------------------------
// Test 4 — video geometry + audio params against decoded values.
// ---------------------------------------------------------------------------

#[test]
fn video_geometry_and_audio_params() {
    let media = demux_media();
    let (_, sps_w, sps_h) = expected_video(&media);
    let (_, asc_rate) = expected_audio(&media);

    let root = parse_xml(&build_mpd());
    let period = root.find("Period").unwrap();

    let video_repr = period
        .find_all("AdaptationSet")
        .into_iter()
        .find(|s| s.attr("mimeType") == Some("video/mp4"))
        .unwrap()
        .find("Representation")
        .unwrap();
    let audio_set = period
        .find_all("AdaptationSet")
        .into_iter()
        .find(|s| s.attr("mimeType") == Some("audio/mp4"))
        .unwrap();
    let audio_repr = audio_set.find("Representation").unwrap();

    let w: u32 = video_repr.attr("width").expect("@width").parse().unwrap();
    let h: u32 = video_repr.attr("height").expect("@height").parse().unwrap();
    assert_eq!(w, sps_w, "video @width must match SPS-decoded width");
    assert_eq!(h, sps_h, "video @height must match SPS-decoded height");

    let rate: u32 = audio_repr
        .attr("audioSamplingRate")
        .expect("@audioSamplingRate")
        .parse()
        .unwrap();
    assert_eq!(rate, asc_rate, "audio @audioSamplingRate must match ASC");

    // AudioChannelConfiguration present with the DASH scheme + a value.
    let acc = audio_repr
        .find("AudioChannelConfiguration")
        .expect("AudioChannelConfiguration");
    assert_eq!(
        acc.attr("schemeIdUri"),
        Some("urn:mpeg:dash:23003:3:audio_channel_configuration:2011")
    );
    let ch: u32 = acc.attr("value").expect("channel value").parse().unwrap();
    assert!(ch >= 1, "channel count must be >= 1");
}

// ---------------------------------------------------------------------------
// Test 5 — bandwidth present + positive on every Representation.
// ---------------------------------------------------------------------------

#[test]
fn bandwidth_present_and_positive() {
    let root = parse_xml(&build_mpd());
    let period = root.find("Period").unwrap();

    let mut reprs = 0usize;
    for set in period.find_all("AdaptationSet") {
        for repr in set.find_all("Representation") {
            reprs += 1;
            let bw: u64 = repr
                .attr("bandwidth")
                .expect("every Representation has @bandwidth")
                .parse()
                .expect("bandwidth must be an integer");
            assert!(bw > 0, "@bandwidth must be a positive integer, got {bw}");
        }
    }
    assert_eq!(reprs, 2, "two Representations total");
}

// ---------------------------------------------------------------------------
// Empty media is rejected.
// ---------------------------------------------------------------------------

#[test]
fn empty_media_rejected() {
    let media = transmux::media::Media::new(vec![], 90_000);
    let mut pkg = DashPackager::default();
    assert!(pkg.package(&media).is_err(), "empty Media must not package");
}
