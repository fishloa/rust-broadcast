//! Real `wvtt` (WebVTT) and `stpp` (TTML) files from real producers - MP4Box
//! and gpac - through transmux, checked against those tools' own parse.
//!
//! Artifacts: `fixtures/subtitles/` (inputs authored for this repository,
//! tool outputs produced by `gen.sh`, versions and commands in its
//! `PROVENANCE.md`). The independent oracles are gpac's `inspect` of every
//! file (per-sample dts / cts / duration / size / CRC32 - `oracle/*.gpac_inspect.txt`)
//! and MP4Box's `-raw` WebVTT export of each wvtt track
//! (`oracle/*.mp4box_raw.vtt`), both produced by the tools, never by transmux.
//!
//! What must hold for every file:
//! 1. demux finds the subtitle track with the right sample entry (`wvtt` /
//!    `stpp`) and its samples match gpac's: count, dts, cts, duration, size
//!    and CRC32 of the payload;
//! 2. re-muxing through CMAF and demuxing again changes no sample byte, time
//!    or sample entry;
//! 3. the DASH MPD and Smooth manifest carry the track.

use std::collections::BTreeSet;

use broadcast_common::{Package, Serialize, Unpackage};
use transmux::pipeline::{CodecConfig, SubtitleSampleEntry};
use transmux::{
    CmafMux, DashPackager, Fmp4Demux, Media, ProgressiveDemux, SmoothPackager, Track,
    WvttSampleBox, parse_wvtt_sample,
};

const DIR: &str = "subtitles";

/// How a fixture is stored on disk.
enum Layout {
    /// One progressive MP4 (moov before mdat).
    Progressive(&'static str),
    /// One fragmented MP4.
    Fragmented(&'static str),
    /// A DASH init segment followed by media segments.
    Dash {
        init: &'static str,
        segments: &'static [&'static str],
    },
}

struct Case {
    label: &'static str,
    layout: Layout,
    /// Oracle file stem under `subtitles/oracle/`.
    oracle: &'static str,
    /// `wvtt` (true) or `stpp` (false).
    webvtt: bool,
    /// Where MP4Box's `-raw` WebVTT export lives (wvtt files only).
    raw_vtt: Option<&'static str>,
}

fn cases() -> Vec<Case> {
    const MP4BOX_DASH: &[&str] = &[
        "mp4box/dash/wvtt_1.m4s",
        "mp4box/dash/wvtt_2.m4s",
        "mp4box/dash/wvtt_3.m4s",
        "mp4box/dash/wvtt_4.m4s",
        "mp4box/dash/wvtt_5.m4s",
        "mp4box/dash/wvtt_6.m4s",
    ];
    const GPAC_DASH: &[&str] = &[
        "gpac/dash/gp_1.m4s",
        "gpac/dash/gp_2.m4s",
        "gpac/dash/gp_3.m4s",
        "gpac/dash/gp_4.m4s",
        "gpac/dash/gp_5.m4s",
        "gpac/dash/gp_6.m4s",
    ];
    vec![
        Case {
            label: "MP4Box wvtt progressive",
            layout: Layout::Progressive("mp4box/wvtt_progressive.mp4"),
            oracle: "mp4box_wvtt_progressive.mp4",
            webvtt: true,
            raw_vtt: Some("mp4box_wvtt_progressive.mp4.mp4box_raw.vtt"),
        },
        Case {
            label: "MP4Box wvtt fragmented",
            layout: Layout::Fragmented("mp4box/wvtt_fragmented.mp4"),
            oracle: "mp4box_wvtt_fragmented.mp4",
            webvtt: true,
            raw_vtt: Some("mp4box_wvtt_fragmented.mp4.mp4box_raw.vtt"),
        },
        Case {
            label: "MP4Box wvtt DASH",
            layout: Layout::Dash {
                init: "mp4box/dash/wvtt_init.mp4",
                segments: MP4BOX_DASH,
            },
            oracle: "mp4box_dash_manifest.mpd",
            webvtt: true,
            raw_vtt: None,
        },
        Case {
            label: "gpac wvtt progressive",
            layout: Layout::Progressive("gpac/wvtt_progressive.mp4"),
            oracle: "gpac_wvtt_progressive.mp4",
            webvtt: true,
            raw_vtt: Some("gpac_wvtt_progressive.mp4.mp4box_raw.vtt"),
        },
        Case {
            label: "gpac wvtt fragmented",
            layout: Layout::Fragmented("gpac/wvtt_fragmented.mp4"),
            oracle: "gpac_wvtt_fragmented.mp4",
            webvtt: true,
            raw_vtt: Some("gpac_wvtt_fragmented.mp4.mp4box_raw.vtt"),
        },
        Case {
            label: "gpac wvtt DASH",
            layout: Layout::Dash {
                init: "gpac/dash/gp_init.mp4",
                segments: GPAC_DASH,
            },
            oracle: "gpac_dash_manifest.mpd",
            webvtt: true,
            raw_vtt: None,
        },
        Case {
            label: "MP4Box stpp progressive",
            layout: Layout::Progressive("mp4box/stpp_progressive.mp4"),
            oracle: "mp4box_stpp_progressive.mp4",
            webvtt: false,
            raw_vtt: None,
        },
        Case {
            label: "MP4Box stpp fragmented",
            layout: Layout::Fragmented("mp4box/stpp_fragmented.mp4"),
            oracle: "mp4box_stpp_fragmented.mp4",
            webvtt: false,
            raw_vtt: None,
        },
        Case {
            label: "gpac stpp progressive",
            layout: Layout::Progressive("gpac/stpp_progressive.mp4"),
            oracle: "gpac_stpp_progressive.mp4",
            webvtt: false,
            raw_vtt: None,
        },
    ]
}

fn read(rel: &str) -> Vec<u8> {
    test_fixtures::read(&format!("{DIR}/{rel}"))
}

fn demux(layout: &Layout) -> Media {
    match layout {
        Layout::Progressive(f) => ProgressiveDemux::new(usize::MAX)
            .expect("demuxer")
            .unpackage(&read(f))
            .unwrap_or_else(|e| panic!("{f}: {e:?}")),
        Layout::Fragmented(f) => Fmp4Demux::new()
            .unpackage(&read(f))
            .unwrap_or_else(|e| panic!("{f}: {e:?}")),
        Layout::Dash { init, segments } => {
            let mut bytes = read(init);
            for s in *segments {
                bytes.extend_from_slice(&read(s));
            }
            Fmp4Demux::new()
                .unpackage(&bytes)
                .unwrap_or_else(|e| panic!("{init}: {e:?}"))
        }
    }
}

fn entry_of(t: &Track) -> Option<&SubtitleSampleEntry> {
    match &t.spec.config {
        CodecConfig::Subtitle { sample_entry, .. } => sample_entry.as_ref(),
        _ => None,
    }
}

fn subtitle_track(m: &Media) -> &Track {
    let mut it = m
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Subtitle { .. }));
    let t = it.next().expect("a subtitle track");
    assert!(it.next().is_none(), "exactly one subtitle track");
    t
}

/// gpac `inspect`'s per-packet record.
#[derive(Debug, PartialEq, Eq)]
struct Pkt {
    dts: i64,
    cts: i64,
    dur: u64,
    size: usize,
    crc32: u32,
}

fn oracle(stem: &str) -> Vec<Pkt> {
    let text = test_fixtures::read_to_string(&format!("{DIR}/oracle/{stem}.gpac_inspect.txt"));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let field = |k: &str| -> &str {
                let key = format!("{k}=");
                let i = l.find(&key).unwrap_or_else(|| panic!("{k} in {l}")) + key.len();
                l[i..].split_whitespace().next().unwrap()
            };
            Pkt {
                dts: field("dts").parse().unwrap(),
                cts: field("cts").parse().unwrap(),
                dur: field("dur").parse().unwrap(),
                size: field("size").parse().unwrap(),
                crc32: u32::from_str_radix(field("crc32").trim_start_matches("0x"), 16).unwrap(),
            }
        })
        .collect()
}

/// The checksum gpac prints for each packet is its `gf_crc_32`, the MPEG-2
/// CRC-32 (poly 0x04C11DB7, init all-ones, no reflection) - the workspace's
/// one implementation.
fn crc32_gpac(data: &[u8]) -> u32 {
    broadcast_common::crc32_mpeg2::compute(data)
}

fn ours(track: &Track) -> Vec<Pkt> {
    track
        .samples
        .iter()
        .map(|s| Pkt {
            dts: s.dts.expect("dts"),
            cts: s.pts.expect("pts"),
            dur: u64::from(s.duration.expect("duration")),
            size: s.data.len(),
            crc32: crc32_gpac(&s.data),
        })
        .collect()
}

#[test]
fn every_real_file_demuxes_to_gpacs_own_samples() {
    for c in cases() {
        let media = demux(&c.layout);
        let track = subtitle_track(&media);
        let CodecConfig::Subtitle { sample_entry, .. } = &track.spec.config else {
            unreachable!()
        };
        match (c.webvtt, sample_entry) {
            (true, Some(SubtitleSampleEntry::WebVtt(_))) => {}
            (false, Some(SubtitleSampleEntry::Ttml(_))) => {}
            (_, other) => panic!("{}: unexpected sample entry {other:?}", c.label),
        }
        assert_eq!(track.spec.timescale, 1000, "{}", c.label);
        assert_eq!(ours(track), oracle(c.oracle), "{}", c.label);
    }
}

#[test]
fn every_real_file_remuxes_through_cmaf_unchanged() {
    for c in cases() {
        let media = demux(&c.layout);
        let before = subtitle_track(&media);
        let out = CmafMux::new(1)
            .package(&media)
            .unwrap_or_else(|e| panic!("{}: {e:?}", c.label));
        let round = Fmp4Demux::new().unpackage(&out).expect("re-demux");
        let after = subtitle_track(&round);
        assert_eq!(
            entry_of(after),
            entry_of(before),
            "{}: sample entry changed",
            c.label
        );
        assert_eq!(ours(after), ours(before), "{}: samples changed", c.label);
    }
}

/// A cue as the WebVTT text MP4Box exports describes it.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RawCue {
    id: Option<String>,
    settings: Option<String>,
    payload: String,
}

/// The cues and `NOTE` blocks of the WebVTT text MP4Box exports, as sets.
fn raw_vtt(text: &str) -> (BTreeSet<RawCue>, BTreeSet<String>) {
    let mut cues = BTreeSet::new();
    let mut notes = BTreeSet::new();
    for block in text.split("\n\n").skip(1) {
        let block = block.trim_end_matches('\n');
        if block.is_empty() {
            continue;
        }
        let lines: Vec<&str> = block.lines().collect();
        if lines[0] == "NOTE" || lines[0].starts_with("NOTE ") {
            notes.insert(block.to_string());
            continue;
        }
        let Some(t) = lines.iter().position(|l| l.contains(" --> ")) else {
            continue; // STYLE / REGION blocks
        };
        let settings: Vec<&str> = lines[t].split_whitespace().skip(3).collect();
        cues.insert(RawCue {
            id: (t > 0).then(|| lines[..t].join("\n")),
            settings: (!settings.is_empty()).then(|| settings.join(" ")),
            payload: lines[t + 1..].join("\n"),
        });
    }
    (cues, notes)
}

#[test]
fn wvtt_samples_parse_typed_and_serialize_to_the_same_bytes() {
    for c in cases().into_iter().filter(|c| c.webvtt) {
        let media = demux(&c.layout);
        let track = subtitle_track(&media);
        let (mut cues, mut notes, mut empties) = (0usize, 0usize, 0usize);
        for (i, sample) in track.samples.iter().enumerate() {
            let boxes = parse_wvtt_sample(&sample.data)
                .unwrap_or_else(|e| panic!("{} sample {i}: {e:?}", c.label));
            let mut again = Vec::new();
            for b in &boxes {
                again.extend_from_slice(&b.try_to_bytes().expect("serialize"));
                match b {
                    WvttSampleBox::Cue(_) => cues += 1,
                    WvttSampleBox::AdditionalText(_) => notes += 1,
                    WvttSampleBox::Empty(_) => empties += 1,
                    other => panic!("{} sample {i}: unexpected box {other:?}", c.label),
                }
            }
            assert_eq!(
                &again[..],
                &sample.data[..],
                "{} sample {i} round trip",
                c.label
            );
        }
        // 7 authored cue blocks, 2 NOTE blocks, and the gaps between them;
        // overlapping cues repeat across samples.
        assert!(
            cues >= 8 && notes == 2 && empties >= 5,
            "{}: {cues}/{notes}/{empties}",
            c.label
        );
    }
}

#[test]
fn wvtt_cue_ids_settings_text_and_notes_match_mp4boxs_own_webvtt_export() {
    for c in cases().into_iter().filter(|c| c.raw_vtt.is_some()) {
        let media = demux(&c.layout);
        let track = subtitle_track(&media);
        let mut cues = BTreeSet::new();
        let mut notes = BTreeSet::new();
        for sample in &track.samples {
            for b in parse_wvtt_sample(&sample.data).unwrap() {
                match b {
                    WvttSampleBox::Cue(cue) => {
                        cues.insert(RawCue {
                            id: cue.cue_id.map(|i| i.cue_id),
                            settings: cue.settings.map(|s| s.settings),
                            payload: cue.payload.cue_text,
                        });
                    }
                    WvttSampleBox::AdditionalText(t) => {
                        notes.insert(t.text);
                    }
                    _ => {}
                }
            }
        }
        let (want_cues, want_notes) = raw_vtt(&test_fixtures::read_to_string(&format!(
            "{DIR}/oracle/{}",
            c.raw_vtt.unwrap()
        )));
        assert!(
            want_cues.len() >= 7 && want_notes.len() == 2,
            "{}: oracle",
            c.label
        );
        assert_eq!(cues, want_cues, "{}: cues", c.label);
        assert_eq!(notes, want_notes, "{}: NOTE blocks", c.label);
    }
}

#[test]
fn dash_mpd_and_smooth_manifest_carry_the_real_tracks() {
    for c in cases() {
        let media = demux(&c.layout);
        let codec = if c.webvtt { "wvtt" } else { "stpp" };
        let mpd = DashPackager::default()
            .package(&media)
            .unwrap_or_else(|e| panic!("{}: {e:?}", c.label));
        assert!(mpd.contains("contentType=\"text\""), "{}: {mpd}", c.label);
        assert!(
            mpd.contains(&format!("codecs=\"{codec}\"")),
            "{}: {mpd}",
            c.label
        );
        let smooth = SmoothPackager::default()
            .package(&media)
            .unwrap_or_else(|e| panic!("{}: {e:?}", c.label));
        assert!(smooth.manifest.contains("Type=\"text\""), "{}", c.label);
        assert!(
            smooth.manifest.contains(&format!("FourCC=\"{codec}\"")),
            "{}",
            c.label
        );
        let track = subtitle_track(&media);
        let frag_bytes: usize = smooth.fragments.iter().map(|f| f.data.len()).sum();
        let sample_bytes: usize = track.samples.iter().map(|s| s.data.len()).sum();
        assert!(
            frag_bytes > sample_bytes,
            "{}: fragments carry the samples",
            c.label
        );
    }
}
