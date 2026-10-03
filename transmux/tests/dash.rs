//! DASH `.mpd` manifest generation gate (issue #464).
//!
//! Oracle: `fixtures/dash/manifest.mpd` is a real ffmpeg-generated DASH MPD for
//! a 2-track (video + audio) CMAF. The input IR is built by `TsDemux`-ing the
//! deterministic 2-track `fixtures/ts/h264_aac.ts` (H.264 video + AAC audio)
//! and fed to [`DashPackager`].
//!
//! Every test bites: the produced XML is parsed with `quick-xml` into an element
//! tree and asserted against real structure — never a bare
//! substring `contains`, and codec/geometry values are asserted against what
//! the crate itself computes, not hardcoded literals.
#![cfg(feature = "std")]

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
// Element tree over `quick-xml`.
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

/// Parse `s` with `quick-xml` into an [`Element`] tree, asserting the document
/// is well-formed: matched tags (checked by the reader), a single root element,
/// and nothing but whitespace outside it.
fn parse_xml(s: &str) -> Element {
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
    // Bottom of the stack is a synthetic document node holding the root.
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

// ---------------------------------------------------------------------------
// AdaptationSet grouping (ISO/IEC 23009-1 §5.3.3, audit r05-W28)
// ---------------------------------------------------------------------------

/// Build an `ISO_639_language_descriptor`-carrying ES_info loop for `lang`.
fn lang_es_info(lang: &[u8; 3]) -> Vec<u8> {
    // tag 0x0A, length 4, ISO_639_language_code(3) + audio_type(1).
    vec![0x0A, 0x04, lang[0], lang[1], lang[2], 0x00]
}

/// Two audio tracks in different languages — and a video track — must produce
/// **three** AdaptationSets: the ABR client must never switch between languages.
#[test]
fn adaptation_sets_split_by_language_and_codec() {
    let base = demux_media();
    let video = base
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track")
        .clone();
    let audio = base
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
        .clone();

    let mut eng = audio.clone();
    eng.spec.track_id = 2;
    eng.spec.es_info_descriptors = lang_es_info(b"eng");
    let mut fra = audio.clone();
    fra.spec.track_id = 3;
    fra.spec.es_info_descriptors = lang_es_info(b"fra");

    let media = transmux::media::Media::new(vec![video, eng, fra], 90_000);
    let mut pkg = DashPackager::default();
    let xml = pkg.package(&media).expect("MPD");

    let root = parse_xml(&xml);
    let period = root.find("Period").expect("Period");
    let sets: Vec<&Element> = period.find_all("AdaptationSet");
    assert_eq!(
        sets.len(),
        3,
        "one AdaptationSet per (kind, lang): video + eng + fra, got {}",
        sets.len()
    );

    let audio_langs: Vec<Option<&str>> = sets
        .iter()
        .filter(|s| s.attr("contentType") == Some("audio"))
        .map(|s| s.attr("lang"))
        .collect();
    assert_eq!(audio_langs.len(), 2, "two audio AdaptationSets");
    assert!(
        audio_langs.contains(&Some("eng")),
        "eng set: {audio_langs:?}"
    );
    assert!(
        audio_langs.contains(&Some("fra")),
        "fra set: {audio_langs:?}"
    );

    // Each audio set contains exactly one Representation.
    for s in sets
        .iter()
        .filter(|s| s.attr("contentType") == Some("audio"))
    {
        assert_eq!(
            s.find_all("Representation").len(),
            1,
            "each language audio set carries only its own Representation"
        );
    }
}

/// Same language, same codec family → one AdaptationSet (the switchable encodings
/// case), and the ordering/structure of a single-audio stream is unchanged.
#[test]
fn adaptation_sets_merge_same_language_and_codec() {
    let base = demux_media();
    let video = base
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track")
        .clone();
    let audio = base
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
        .clone();

    let mut a1 = audio.clone();
    a1.spec.track_id = 2;
    a1.spec.es_info_descriptors = lang_es_info(b"eng");
    let mut a2 = audio.clone();
    a2.spec.track_id = 3;
    a2.spec.es_info_descriptors = lang_es_info(b"eng");

    let media = transmux::media::Media::new(vec![video, a1, a2], 90_000);
    let mut pkg = DashPackager::default();
    let xml = pkg.package(&media).expect("MPD");

    let root = parse_xml(&xml);
    let period = root.find("Period").expect("Period");
    let sets: Vec<&Element> = period.find_all("AdaptationSet");
    assert_eq!(sets.len(), 2, "video + one merged eng audio set");

    let audio_set = sets
        .iter()
        .find(|s| s.attr("contentType") == Some("audio"))
        .expect("audio set");
    assert_eq!(audio_set.attr("lang"), Some("eng"), "@lang on the set");
    assert_eq!(
        audio_set.find_all("Representation").len(),
        2,
        "both same-language, same-codec Representations stay in one set"
    );
}

// ---------------------------------------------------------------------------
// AdaptationSet identity + codec-family splitting (ISO/IEC 23009-1 §5.3.3)
// ---------------------------------------------------------------------------

/// Every `AdaptationSet` carries a unique `@id`, and two audio tracks in the
/// **same** language but different codec families land in separate sets, each
/// with its own `@codecs` — they are not switchable encodings of one another.
#[test]
fn adaptation_sets_have_unique_ids_and_split_by_codec_family() {
    // A real AAC track (from the TS fixture) and a real AC-3 track (from the
    // Dolby fixture), both with `eng` audio.
    let base = demux_media();
    let video = base
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track")
        .clone();
    let mut aac = base
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("aac track")
        .clone();
    aac.spec.track_id = 2;
    aac.spec.es_info_descriptors = lang_es_info(b"eng");

    let ac3_ts = std::fs::read(fixtures_dir().join("ts/dolby/ac3.ts")).expect("ac3 fixture");
    let ac3_ir = TsDemux::new().unpackage(&ac3_ts[..]).expect("demux ac3");
    let mut ac3 = ac3_ir.tracks[0].clone();
    ac3.spec.track_id = 3;
    ac3.spec.es_info_descriptors = lang_es_info(b"eng");

    let media = transmux::media::Media::new(vec![video, aac, ac3], 90_000);
    let mut pkg = DashPackager::default();
    let xml = pkg.package(&media).expect("MPD");

    let root = parse_xml(&xml);
    let period = root.find("Period").expect("Period");
    let sets: Vec<&Element> = period.find_all("AdaptationSet");
    assert_eq!(
        sets.len(),
        3,
        "video + one set per audio codec family, got {}:\n{xml}",
        sets.len()
    );

    // Unique @id across every set.
    let mut ids: Vec<&str> = sets
        .iter()
        .map(|s| s.attr("id").expect("AdaptationSet@id is required"))
        .collect();
    let unique = {
        let mut v = ids.clone();
        v.sort_unstable();
        v.dedup();
        v.len()
    };
    assert_eq!(
        unique,
        ids.len(),
        "AdaptationSet@id must be unique: {ids:?}"
    );
    ids.sort_unstable();
    assert_eq!(ids, vec!["t0", "t1", "t2"], "deterministic ids");

    // Two audio sets, both `eng`, with different codec families.
    let audio: Vec<&&Element> = sets
        .iter()
        .filter(|s| s.attr("contentType") == Some("audio"))
        .collect();
    assert_eq!(audio.len(), 2, "AAC and AC-3 are separate sets:\n{xml}");
    for s in &audio {
        assert_eq!(s.attr("lang"), Some("eng"), "same language, same @lang");
        assert_eq!(s.find_all("Representation").len(), 1);
    }
    let codecs: Vec<&str> = audio
        .iter()
        .flat_map(|s| s.find_all("Representation"))
        .map(|r| r.attr("codecs").expect("Representation@codecs"))
        .collect();
    assert!(
        codecs.iter().any(|c| c.starts_with("mp4a")),
        "the AAC Representation carries mp4a.*: {codecs:?}"
    );
    assert!(
        codecs
            .iter()
            .any(|c| c.starts_with("ac-3") || c.starts_with("ec-3")),
        "the AC-3 Representation carries ac-3: {codecs:?}"
    );

    // And the video set is first, with `video` contentType + mimeType.
    assert_eq!(sets[0].attr("contentType"), Some("video"));
    assert_eq!(sets[0].attr("mimeType"), Some("video/mp4"));
    for s in &audio {
        assert_eq!(s.attr("mimeType"), Some("audio/mp4"));
    }
}
