//! Issue #1186 (ex-`TODO(#753)`): `CodecConfig::Subtitle` carries the full
//! ISOBMFF sample entry (`stpp` / `wvtt`, ISO/IEC 14496-30) so a demuxed
//! subtitle track re-muxes to CMAF instead of being rejected.
//!
//! Real artifact: `fixtures/mp4/cmaf-av-subtitle/av-subtitle-frag.mp4` (ffmpeg CMAF,
//! AVC + `stpp`/TTML; see PROVENANCE.md there). `wvtt` has no real producer
//! available (ffmpeg cannot write it), so it is a spec-vector round trip.

use broadcast_common::Serialize;
use broadcast_common::timescale::Timescale;
use isobmff::{VttCueBox, WvttSampleEntry};
use transmux::pipeline::{CodecConfig, SubtitleFormat, SubtitleSampleEntry};
use transmux::{Media, Sample, Track, TrackId, TrackSpec};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn fixture() -> Vec<u8> {
    let path = test_fixtures::path("mp4/cmaf-av-subtitle/av-subtitle-frag.mp4");
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
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
    let original = test_harness::collect::media_from_mp4(
        &fixture(),
        test_harness::collect::CollectMode::Seekable,
    )
    .unwrap();
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
    let out = crate::cmaf::package(&original).expect("CmafMuxStage carries stpp now");
    // sthd must precede stbl inside the subtitle trak (§7.3.4).
    let sthd = find(&out, b"sthd").expect("sthd emitted");
    let stpp = find(&out, b"stpp").expect("stpp sample entry emitted");
    assert!(sthd < stpp, "media header before the sample table");
    assert!(find(&out, b"subt").is_some(), "subt handler");

    let round =
        test_harness::collect::media_from_mp4(&out, test_harness::collect::CollectMode::Seekable)
            .unwrap();
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
        timescale(1000),
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
    let sample =
        Sample::new(cue.clone(), Some(0), Some(0), Some(1000), true).expect("valid timing");
    let media = Media::new(vec![Track::new(spec, vec![sample])], timescale(1000));
    let out = crate::cmaf::package(&media).expect("wvtt muxes");
    assert!(find(&out, b"nmhd").is_some() && find(&out, b"text").is_some());
    let round =
        test_harness::collect::media_from_mp4(&out, test_harness::collect::CollectMode::Seekable)
            .unwrap();
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
        timescale(1000),
        CodecConfig::Subtitle {
            format: SubtitleFormat::DvbBitmap,
            sample_entry: None,
        },
    );
    let media = Media::new(vec![Track::new(spec, vec![])], timescale(1000));
    let err = crate::cmaf::package(&media).unwrap_err();
    assert!(
        matches!(err, transmux::Error::UnmuxableSubtitleTrack { .. }),
        "{err:?}"
    );
}

fn child(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = ((8 + body.len()) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(ty);
    v.extend_from_slice(body);
    v
}

fn vttc(children: &[Vec<u8>]) -> Vec<u8> {
    child(b"vttc", &children.concat())
}

#[test]
fn vttc_later_children_and_eight_byte_empty_box_round_trip() {
    let iden = child(b"iden", b"id1");
    let sttg = child(b"sttg", b"align:end");
    let payl = child(b"payl", b"text");
    let vsid = child(b"vsid", &[0, 0, 0, 7]);
    let ctim = child(b"ctim", b"00:00:01.000");
    let empty = child(b"empt", &[]);
    assert_eq!(empty.len(), 8, "an empty child is header-only");

    let mut children = vec![iden, sttg, payl, vsid, ctim];
    let without_empty = vttc(&children);
    let cue = VttCueBox::bare_parse(&without_empty).expect("parse vttc");
    assert_eq!(cue.cue_id.as_ref().expect("iden").cue_id, "id1");
    assert_eq!(cue.settings.as_ref().expect("sttg").settings, "align:end");
    assert_eq!(cue.payload.cue_text, "text");
    assert_eq!(cue.extra_children.len(), 2);
    assert_eq!(&cue.extra_children[0].box_type, b"vsid");
    assert_eq!(&cue.extra_children[1].box_type, b"ctim");
    assert_eq!(cue.try_to_bytes().unwrap(), without_empty);

    // The empty 8-byte child sits exactly at the end of the body: a `<=`
    // tightened to `<` would leave the loop before reading it.
    children.push(empty);
    let with_empty = vttc(&children);
    let cue = VttCueBox::bare_parse(&with_empty).expect("parse vttc with an empty child");
    assert_eq!(cue.extra_children.len(), 3);
    assert_eq!(&cue.extra_children[2].box_type, b"empt");
    assert!(cue.extra_children[2].data.is_empty());
    assert_eq!(cue.try_to_bytes().unwrap(), with_empty);
}

#[test]
fn vttc_trailing_short_box_stops_the_walk_without_being_written() {
    let iden = child(b"iden", b"id1");
    let sttg = child(b"sttg", b"align:end");
    let payl = child(b"payl", b"text");
    let vsid = child(b"vsid", &[0, 0, 0, 7]);
    let ctim = child(b"ctim", b"00:00:01.000");
    let good = vttc(&[iden, sttg, payl, vsid, ctim]);

    // A final 8-byte slot whose size field is below the header length.
    let mut garbage = 4u32.to_be_bytes().to_vec();
    garbage.extend_from_slice(b"junk");
    let mut wire = good.clone();
    wire.extend_from_slice(&garbage);
    let wire_len = wire.len() as u32;
    wire[0..4].copy_from_slice(&wire_len.to_be_bytes());

    let cue = VttCueBox::bare_parse(&wire).expect("parse vttc with trailing short box");
    assert_eq!(cue.extra_children.len(), 2);
    assert_eq!(cue.try_to_bytes().unwrap(), good);
}

#[test]
fn vttc_extra_removal_reindexes_without_a_panic() {
    let iden = child(b"iden", b"id1");
    let sttg = child(b"sttg", b"align:end");
    let payl = child(b"payl", b"text");
    let vsid = child(b"vsid", &[0, 0, 0, 7]);
    let ctim = child(b"ctim", b"00:00:01.000");
    let both = vttc(&[iden.clone(), sttg.clone(), payl.clone(), vsid.clone(), ctim]);
    let only_vsid = vttc(&[iden, sttg, payl, vsid]);

    let mut cue = VttCueBox::bare_parse(&both).expect("parse two extras");
    cue.extra_children.pop().expect("two extras");
    assert_eq!(
        cue.try_to_bytes()
            .expect("serialize after removing an extra"),
        only_vsid,
        "a removed extra must not be indexed by its stale wire slot"
    );
}

#[test]
fn wvtt_entry_extra_boxes_and_eight_byte_empty_box_round_trip() {
    let vttc = child(b"vttC", b"WEBVTT");
    let vlab = child(b"vlab", b"label");
    let btrt = child(b"btrt", &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let empty = child(b"empt", &[]);

    let entry_body = |extras: &[Vec<u8>]| {
        let mut body = vec![0u8; 6];
        body.extend_from_slice(&1u16.to_be_bytes());
        body.extend_from_slice(&vttc);
        for e in extras {
            body.extend_from_slice(e);
        }
        child(b"wvtt", &body)
    };

    let two_extras = entry_body(&[vlab.clone(), btrt.clone()]);
    let entry = WvttSampleEntry::bare_parse(&two_extras).expect("parse wvtt");
    assert_eq!(entry.data_reference_index, 1);
    assert_eq!(entry.config.config, "WEBVTT");
    assert_eq!(entry.extra_boxes.len(), 2);
    assert_eq!(&entry.extra_boxes[0].box_type, b"vlab");
    assert_eq!(&entry.extra_boxes[1].box_type, b"btrt");
    assert_eq!(entry.try_to_bytes().unwrap(), two_extras);

    let with_empty = entry_body(&[vlab, btrt, empty]);
    let entry = WvttSampleEntry::bare_parse(&with_empty).expect("parse wvtt with an empty child");
    assert_eq!(entry.extra_boxes.len(), 3);
    assert_eq!(&entry.extra_boxes[2].box_type, b"empt");
    assert!(entry.extra_boxes[2].data.is_empty());
    assert_eq!(entry.try_to_bytes().unwrap(), with_empty);

    // A final size<8 slot stops the walk instead of slicing a 4-byte child.
    let vttc = child(b"vttC", b"WEBVTT");
    let vlab = child(b"vlab", b"label");
    let mut body = vec![0u8; 6];
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&vttc);
    body.extend_from_slice(&vlab);
    let mut garbage = 4u32.to_be_bytes().to_vec();
    garbage.extend_from_slice(b"junk");
    body.extend_from_slice(&garbage);
    let wire = child(b"wvtt", &body);
    let entry = WvttSampleEntry::bare_parse(&wire).expect("parse wvtt with trailing short box");
    assert_eq!(entry.extra_boxes.len(), 1);
    let mut good_body = vec![0u8; 6];
    good_body.extend_from_slice(&1u16.to_be_bytes());
    good_body.extend_from_slice(&vttc);
    good_body.extend_from_slice(&vlab);
    assert_eq!(entry.try_to_bytes().unwrap(), child(b"wvtt", &good_body));
}

/// A child size whose first two bytes differ: `0x0100_0008`. The walk reads
/// the second size byte from `pos + 1`; `pos * 1` would read the first byte
/// again and swallow the following child.
#[test]
fn vttc_and_wvtt_read_the_second_byte_of_a_child_size() {
    const HUGE: usize = 0x0100_0008;
    let mut huge = (HUGE as u32).to_be_bytes().to_vec();
    huge.extend_from_slice(b"huge");
    huge.extend_from_slice(&vec![0u8; HUGE - 8]);
    let small = child(b"vide", b"x");

    let payl = child(b"payl", b"text");
    let body = [payl, huge.clone(), small.clone()].concat();
    let wire = child(b"vttc", &body);
    let cue = VttCueBox::bare_parse(&wire).expect("parse vttc with a large child");
    assert_eq!(cue.extra_children.len(), 2);
    assert_eq!(&cue.extra_children[0].box_type, b"huge");
    assert_eq!(&cue.extra_children[1].box_type, b"vide");

    let vttc = child(b"vttC", b"WEBVTT");
    let mut entry_body = vec![0u8; 6];
    entry_body.extend_from_slice(&1u16.to_be_bytes());
    entry_body.extend_from_slice(&vttc);
    entry_body.extend_from_slice(&huge);
    entry_body.extend_from_slice(&small);
    let entry = WvttSampleEntry::bare_parse(&child(b"wvtt", &entry_body))
        .expect("parse wvtt with a large child");
    assert_eq!(entry.extra_boxes.len(), 2);
    assert_eq!(&entry.extra_boxes[0].box_type, b"huge");
    assert_eq!(&entry.extra_boxes[1].box_type, b"vide");
}
