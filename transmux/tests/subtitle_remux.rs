//! Issue #1186 (ex-`TODO(#753)`): `CodecConfig::Subtitle` carries the full
//! ISOBMFF sample entry (`stpp` / `wvtt`, ISO/IEC 14496-30) so a demuxed
//! subtitle track re-muxes to CMAF instead of being rejected.
//!
//! Real artifact: `fixtures/mp4/cmaf/av_subtitle_frag.mp4` (ffmpeg CMAF,
//! AVC + `stpp`/TTML; see PROVENANCE.md there). `wvtt` has no real producer
//! available (ffmpeg cannot write it), so it is a spec-vector round trip.

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::{CodecConfig, SubtitleFormat, SubtitleSampleEntry};
use transmux::{CmafMux, Fmp4Demux, Media, Sample, Track, TrackId, TrackSpec, WvttSampleEntry};

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mp4/cmaf/av_subtitle_frag.mp4"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn subtitle_track(m: &Media) -> &Track {
    m.tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Subtitle { .. }))
        .expect("subtitle track present")
}

#[test]
fn stpp_track_carries_entry_and_remuxes_through_cmaf() {
    let original = Fmp4Demux::new().unpackage(&fixture()).unwrap();
    let orig_sub = subtitle_track(&original);
    let CodecConfig::Subtitle {
        format,
        sample_entry: Some(SubtitleSampleEntry::Ttml(entry)),
    } = &orig_sub.spec.config
    else {
        panic!(
            "stpp must demux with its sample entry: {:?}",
            orig_sub.spec.config
        );
    };
    assert_eq!(*format, SubtitleFormat::Ttml);
    assert!(
        entry.namespace.contains("http://www.w3.org/ns/ttml"),
        "entry namespace comes from the real fixture's stpp box: {:?}",
        entry.namespace
    );

    // UNFILTERED mux (previously UnmuxableSubtitleTrack).
    let out = CmafMux::default()
        .package(&original)
        .expect("CmafMux carries stpp now");
    // sthd must precede stbl inside the subtitle trak (§6.2.3).
    let sthd = find(&out, b"sthd").expect("sthd emitted");
    let stpp = find(&out, b"stpp").expect("stpp sample entry emitted");
    assert!(sthd < stpp, "media header before the sample table");
    assert!(find(&out, b"subt").is_some(), "subt handler");

    let round = Fmp4Demux::new().unpackage(&out).unwrap();
    let new_sub = subtitle_track(&round);
    assert_eq!(
        new_sub.spec.config_subtitle_entry(),
        orig_sub.spec.config_subtitle_entry(),
        "sample entry survives demux -> mux -> demux unchanged"
    );
    let a: Vec<&[u8]> = orig_sub.samples.iter().map(|s| &s.data[..]).collect();
    let b: Vec<&[u8]> = new_sub.samples.iter().map(|s| &s.data[..]).collect();
    assert_eq!(a, b, "real TTML sample bytes survive");
    assert!(!a.is_empty());
}

trait EntryOf {
    fn config_subtitle_entry(&self) -> Option<&SubtitleSampleEntry>;
}
impl EntryOf for TrackSpec {
    fn config_subtitle_entry(&self) -> Option<&SubtitleSampleEntry> {
        match &self.config {
            CodecConfig::Subtitle { sample_entry, .. } => sample_entry.as_ref(),
            _ => None,
        }
    }
}

#[test]
fn wvtt_spec_vector_round_trips_with_nmhd() {
    let entry = WvttSampleEntry::new("WEBVTT\n\nNOTE spec vector\n");
    let spec = TrackSpec::new(
        TrackId::new(1),
        1000,
        CodecConfig::Subtitle {
            format: SubtitleFormat::WebVtt,
            sample_entry: Some(SubtitleSampleEntry::WebVtt(entry.clone())),
        },
    );
    // ISO/IEC 14496-30 §9: a cue sample is a `vttc` box holding `payl`.
    let cue: Vec<u8> = {
        let mut payl = Vec::new();
        payl.extend_from_slice(&(8u32 + 2).to_be_bytes());
        payl.extend_from_slice(b"payl");
        payl.extend_from_slice(b"Hi");
        let mut vttc = Vec::new();
        vttc.extend_from_slice(&(8u32 + payl.len() as u32).to_be_bytes());
        vttc.extend_from_slice(b"vttc");
        vttc.extend_from_slice(&payl);
        vttc
    };
    let sample = Sample::new(cue.clone(), Some(0), Some(0), Some(1000), true);
    let media = Media::new(vec![Track::new(spec, vec![sample])], 1000);
    let out = CmafMux::default().package(&media).expect("wvtt muxes");
    assert!(find(&out, b"nmhd").is_some() && find(&out, b"text").is_some());
    let round = Fmp4Demux::new().unpackage(&out).unwrap();
    let t = subtitle_track(&round);
    assert_eq!(
        t.spec.config_subtitle_entry(),
        Some(&SubtitleSampleEntry::WebVtt(entry))
    );
    assert_eq!(&t.samples[0].data[..], &cue[..]);
}

#[test]
fn entryless_subtitle_is_still_rejected_by_name() {
    let spec = TrackSpec::new(
        TrackId::new(7),
        1000,
        CodecConfig::Subtitle {
            format: SubtitleFormat::DvbBitmap,
            sample_entry: None,
        },
    );
    let media = Media::new(vec![Track::new(spec, vec![])], 1000);
    let err = CmafMux::default().package(&media).unwrap_err();
    assert!(
        matches!(err, transmux::Error::UnmuxableSubtitleTrack { .. }),
        "{err:?}"
    );
}
