//! Microsoft Smooth Streaming ([MS-SSTR]) output gate (issue #473).
//!
//! Input IR: `TsDemux`-of-`fixtures/ts/h264_aac.ts` (75 video + 131 audio
//! samples). Each test bites by parsing the produced outputs (manifest XML +
//! fragment boxes) and asserting against what the crate itself demuxes — never
//! a bare substring or a hardcoded offset.

use std::path::PathBuf;

use broadcast_common::{Package, Parse, Unpackage};
use transmux::aac_asc::AudioSpecificConfig;
use transmux::media::{Fmp4Demux, Media};
use transmux::pipeline::CodecConfig;
use transmux::smooth::{SmoothOutput, SmoothPackager, FOURCC_AACL, FOURCC_H264, TFXD_UUID};
use transmux::ts_demux::TsDemux;

// ---------------------------------------------------------------------------
// Fixtures + packaging
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

fn build_smooth() -> (Media, SmoothOutput) {
    let media = demux_media();
    let mut pkg = SmoothPackager::default();
    let out = pkg.package(&media).expect("package Smooth");
    (media, out)
}

// ---------------------------------------------------------------------------
// Minimal XML walker (hand-rolled; no external dependency).
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

    fn parse_document(&mut self) -> Element {
        self.skip_ws();
        if self.starts_with("<?") {
            while self.pos < self.s.len() && !self.starts_with("?>") {
                self.pos += 1;
            }
            self.pos += 2;
        }
        self.skip_ws();
        self.parse_element().expect("root element")
    }

    fn parse_element(&mut self) -> Option<Element> {
        self.skip_ws();
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
        assert_eq!(self.cur(), b, "expected '{}' at {}", b as char, self.pos);
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
        raw
    }
}

fn parse_xml(s: &str) -> Element {
    XmlParser::new(s).parse_document()
}

/// Decode an uppercase-hex `CodecPrivateData` string to bytes.
fn hex_decode(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "hex must be even length");
    let val = |c: u8| -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'A'..=b'F' => c - b'A' + 10,
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("bad hex digit {}", c as char),
        }
    };
    s.as_bytes()
        .chunks(2)
        .map(|p| (val(p[0]) << 4) | val(p[1]))
        .collect()
}

// ---------------------------------------------------------------------------
// Oracle helpers — pull expected values from the demuxed IR.
// ---------------------------------------------------------------------------

fn video_track(media: &Media) -> &transmux::media::Track {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track")
}

fn audio_track(media: &Media) -> &transmux::media::Track {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
}

/// Demuxed SPS + ASC bytes for the oracle checks.
fn demuxed_sps(media: &Media) -> Vec<u8> {
    match &video_track(media).spec.config {
        CodecConfig::Avc { config, .. } => config.config.sps.first().expect("SPS").0.clone(),
        _ => unreachable!(),
    }
}

fn demuxed_asc(media: &Media) -> Vec<u8> {
    match &audio_track(media).spec.config {
        CodecConfig::Aac { esds, .. } => esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|dc| dc.decoder_specific_info.as_ref())
            .expect("ASC")
            .data
            .clone(),
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Test 1 — Manifest shape.
// ---------------------------------------------------------------------------

#[test]
fn manifest_shape() {
    let (_media, out) = build_smooth();
    let root = parse_xml(&out.manifest);

    assert_eq!(root.name, "SmoothStreamingMedia", "root element");
    assert_eq!(
        root.attr("MajorVersion"),
        Some("2"),
        "MajorVersion must be 2"
    );
    assert!(root.has_attr("TimeScale"), "must carry a TimeScale");
    assert!(root.has_attr("Duration"), "must carry a Duration");

    let sis = root.find_all("StreamIndex");
    assert_eq!(sis.len(), 2, "exactly two StreamIndex (video + audio)");

    let types: std::collections::BTreeSet<&str> =
        sis.iter().filter_map(|s| s.attr("Type")).collect();
    assert!(types.contains("video"), "a video StreamIndex");
    assert!(types.contains("audio"), "an audio StreamIndex");

    for si in &sis {
        assert_eq!(
            si.find_all("QualityLevel").len(),
            1,
            "each StreamIndex has exactly one QualityLevel"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2 — Codec signalling correct (against demuxed config).
// ---------------------------------------------------------------------------

#[test]
fn codec_signalling_correct() {
    let (media, out) = build_smooth();
    let root = parse_xml(&out.manifest);
    let sis = root.find_all("StreamIndex");

    let video_si = sis
        .iter()
        .find(|s| s.attr("Type") == Some("video"))
        .expect("video StreamIndex");
    let audio_si = sis
        .iter()
        .find(|s| s.attr("Type") == Some("audio"))
        .expect("audio StreamIndex");

    let video_ql = video_si.find("QualityLevel").unwrap();
    let audio_ql = audio_si.find("QualityLevel").unwrap();

    // Video: FourCC H264 + CodecPrivateData = start-code SPS+PPS whose SPS matches.
    assert_eq!(video_ql.attr("FourCC"), Some(FOURCC_H264));
    let cpd = hex_decode(
        video_ql
            .attr("CodecPrivateData")
            .expect("video CodecPrivateData"),
    );
    // Must begin with a start code, then the demuxed SPS bytes.
    assert_eq!(
        &cpd[0..4],
        &[0x00, 0x00, 0x00, 0x01],
        "SPS start code prefix"
    );
    let sps = demuxed_sps(&media);
    assert_eq!(
        &cpd[4..4 + sps.len()],
        &sps[..],
        "CodecPrivateData SPS must equal the demuxed SPS bytes"
    );
    // A PPS start code must appear after the SPS.
    let rest = &cpd[4 + sps.len()..];
    assert_eq!(
        &rest[0..4],
        &[0x00, 0x00, 0x00, 0x01],
        "a PPS start code must follow the SPS"
    );

    // Audio: FourCC AACL + CodecPrivateData = ASC == demuxed ASC.
    assert_eq!(audio_ql.attr("FourCC"), Some(FOURCC_AACL));
    let asc_bytes = hex_decode(
        audio_ql
            .attr("CodecPrivateData")
            .expect("audio CodecPrivateData"),
    );
    let want_asc = demuxed_asc(&media);
    assert_eq!(
        asc_bytes, want_asc,
        "audio CodecPrivateData must equal the demuxed ASC bytes"
    );

    // SamplingRate/Channels must match the decoded ASC.
    let asc = AudioSpecificConfig::parse(&want_asc).expect("parse ASC");
    let want_rate = asc.sampling_frequency.unwrap_or_else(|| {
        const RATES: [u32; 13] = [
            96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
        ];
        RATES[asc.sampling_frequency_index.raw() as usize]
    });
    let manifest_rate: u32 = audio_ql
        .attr("SamplingRate")
        .expect("SamplingRate")
        .parse()
        .unwrap();
    assert_eq!(manifest_rate, want_rate, "SamplingRate must match ASC");
    let manifest_ch: u16 = audio_ql
        .attr("Channels")
        .expect("Channels")
        .parse()
        .unwrap();
    assert_eq!(
        manifest_ch,
        asc.channel_configuration.raw() as u16,
        "Channels must match ASC channel configuration"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — Fragment `c` timeline.
// ---------------------------------------------------------------------------

#[test]
fn fragment_c_timeline() {
    let (media, out) = build_smooth();
    let root = parse_xml(&out.manifest);

    for si in root.find_all("StreamIndex") {
        let ty = si.attr("Type").unwrap();
        let track = match ty {
            "video" => video_track(&media),
            "audio" => audio_track(&media),
            _ => panic!("unexpected type"),
        };

        // Number of `c` == number of emitted fragments for this track.
        let cs = si.find_all("c");
        let emitted = out
            .fragments
            .iter()
            .filter(|f| f.track_id == track.spec.track_id)
            .count();
        assert_eq!(
            cs.len(),
            emitted,
            "`c` count must equal emitted fragments for {ty}"
        );
        assert!(emitted > 0, "at least one fragment for {ty}");

        // Sum of c@d == the track total duration in TimeScale ticks.
        let sum_d: u64 = cs
            .iter()
            .map(|c| c.attr("d").expect("c@d").parse::<u64>().unwrap())
            .sum();
        let media_ticks: u64 = track.samples.iter().map(|s| s.duration as u64).sum();
        // Expected total in smooth ticks (same round-trip the packager does).
        let ts = track.spec.timescale.max(1) as u64;
        let expected = (media_ticks * 10_000_000 + ts / 2) / ts;
        assert_eq!(
            sum_d, expected,
            "sum of c@d must equal the track total duration ({ty})"
        );

        // Only the first `c` carries @t; all carry @d.
        assert!(cs[0].has_attr("t"), "first c must carry @t");
        for c in &cs {
            assert!(c.has_attr("d"), "every c must carry @d");
        }
    }
}

// ---------------------------------------------------------------------------
// Test 4 — Fragment box structure + tfxd.
// ---------------------------------------------------------------------------

/// Find the tfxd uuid box inside a moof, returning (AbsoluteTime, Duration).
fn find_tfxd(fragment: &[u8]) -> (u64, u64) {
    // Walk top-level boxes to the moof.
    let mut off = 0usize;
    while off + 8 <= fragment.len() {
        let size = u32::from_be_bytes([
            fragment[off],
            fragment[off + 1],
            fragment[off + 2],
            fragment[off + 3],
        ]) as usize;
        let ty = &fragment[off + 4..off + 8];
        if ty == b"moof" {
            // Search for a uuid box with the tfxd usertype within the moof.
            let moof = &fragment[off..off + size];
            let mut i = 8usize;
            // Descend: mfhd, traf(...). We scan for "uuid" fourcc + TFXD usertype.
            while i + 8 <= moof.len() {
                let bsz =
                    u32::from_be_bytes([moof[i], moof[i + 1], moof[i + 2], moof[i + 3]]) as usize;
                let bty = &moof[i + 4..i + 8];
                if bty == b"uuid" && i + 8 + 16 <= moof.len() && moof[i + 8..i + 24] == TFXD_UUID {
                    // body after box header(8) + usertype(16): version/flags(4) then two u64s.
                    let p = &moof[i + 24 + 4..];
                    let at = u64::from_be_bytes([p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]]);
                    let du =
                        u64::from_be_bytes([p[8], p[9], p[10], p[11], p[12], p[13], p[14], p[15]]);
                    return (at, du);
                }
                // Descend into traf container to keep scanning children.
                if bty == b"traf" {
                    i += 8;
                    continue;
                }
                if bsz == 0 {
                    break;
                }
                i += bsz;
            }
            panic!("tfxd uuid not found in moof");
        }
        if size == 0 {
            break;
        }
        off += size;
    }
    panic!("moof not found");
}

#[test]
fn fragment_box_structure_and_tfxd() {
    let (media, out) = build_smooth();
    let vid_id = video_track(&media).spec.track_id;
    let video_frags: Vec<_> = out
        .fragments
        .iter()
        .filter(|f| f.track_id == vid_id)
        .collect();
    assert!(!video_frags.is_empty(), "video fragments present");

    let mut last_seq = 0u32;
    for (idx, frag) in video_frags.iter().enumerate() {
        // Parses as moof + mdat.
        use transmux::box_types::parse_box;
        let (bx0, c0) = parse_box(&frag.data).expect("parse first box");
        // First box is styp; then moof, then mdat.
        assert_eq!(&bx0.header.box_type.0, b"styp");
        let (bx1, c1) = parse_box(&frag.data[c0..]).expect("parse second box");
        assert_eq!(&bx1.header.box_type.0, b"moof", "second box is moof");
        let (bx2, _c2) = parse_box(&frag.data[c0 + c1..]).expect("parse third box");
        assert_eq!(&bx2.header.box_type.0, b"mdat", "third box is mdat");

        // tfxd present with the right UUID + AbsoluteTime == fragment start.
        let (at, du) = find_tfxd(&frag.data);
        assert_eq!(
            at, frag.start_time,
            "tfxd AbsoluteTime must equal the fragment start (frag {idx})"
        );
        assert_eq!(
            du, frag.duration,
            "tfxd Duration must equal the fragment duration"
        );

        // Sequence numbers increase.
        assert!(
            frag.sequence_number > last_seq,
            "sequence numbers must strictly increase"
        );
        last_seq = frag.sequence_number;
    }
}

// ---------------------------------------------------------------------------
// Test 5 — Lossless round-trip (Smooth fragmentation is lossless).
// ---------------------------------------------------------------------------

#[test]
fn lossless_round_trip_video() {
    let (media, out) = build_smooth();
    let vid = video_track(&media);
    let vid_id = vid.spec.track_id;

    // Build a fragmented-MP4 file: the CMAF init segment + every video
    // fragment's moof+mdat concatenated (drop the per-fragment styp so
    // Fmp4Demux sees a clean moov + moof/mdat stream).
    use transmux::box_types::parse_box;
    let specs = vec![vid.spec.clone()];
    let mut file = transmux::pipeline::build_init_segment(&specs, media.movie_timescale)
        .expect("init segment");

    for frag in out.fragments.iter().filter(|f| f.track_id == vid_id) {
        // Strip the leading styp; append moof + mdat.
        let (styp, sc) = parse_box(&frag.data).unwrap();
        assert_eq!(&styp.header.box_type.0, b"styp");
        file.extend_from_slice(&frag.data[sc..]);
    }

    let media2 = Fmp4Demux::new().unpackage(&file[..]).expect("re-demux");
    let vid2 = media2
        .tracks
        .iter()
        .find(|t| t.spec.track_id == vid_id)
        .expect("video track in re-demux");

    assert_eq!(
        vid2.samples.len(),
        vid.samples.len(),
        "sample count preserved (75 video samples)"
    );
    assert_eq!(vid.samples.len(), 75, "fixture has 75 video samples");

    for (i, (a, b)) in vid.samples.iter().zip(vid2.samples.iter()).enumerate() {
        assert_eq!(
            a.data, b.data,
            "coded NAL payload byte-identical at sample {i}"
        );
        assert_eq!(a.duration, b.duration, "duration preserved at sample {i}");
        assert_eq!(a.is_sync, b.is_sync, "sync flag preserved at sample {i}");
    }
}

// ---------------------------------------------------------------------------
// Empty media is rejected.
// ---------------------------------------------------------------------------

#[test]
fn empty_media_rejected() {
    let media = Media::new(vec![], 90_000);
    let mut pkg = SmoothPackager::default();
    assert!(pkg.package(&media).is_err(), "empty Media must not package");
}
