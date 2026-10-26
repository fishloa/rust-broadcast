//! A muxed Representation (video and audio in one init) and an encrypted track, played from real
//! fragmented MP4 files that the committed fixtures provide, split into an init and segments and
//! described by a minimal `SegmentList` MPD. The oracle is `Fmp4Demux` over the whole file.

#[path = "support/dash_common.rs"]
mod dash_common;
#[path = "support/dash_sim.rs"]
mod dash_sim;
#[allow(dead_code)]
#[path = "support/fmp4_split.rs"]
mod fmp4_split;

use broadcast_client::{Config, ErrorKind, Event};
use broadcast_common::Unpackage;
use dash_common::loopback::{Origin, Sig, sigs};
use dash_common::world::World;
use dash_sim::{DashSim, SEC, StaticBackend};
use transmux::Fmp4Demux;

const HOST: &str = "http://dash.test";
const GROUP: usize = 4;

fn build(fixture: &str) -> (DashSim, Vec<Vec<Sig>>) {
    let whole = test_fixtures::read(fixture);
    let split = fmp4_split::split(&whole).expect("a fragmented file");
    let segments = fmp4_split::group(&split.segments, GROUP);
    let mut entries = String::new();
    let mut origin = Origin::new().doc(&format!("{HOST}/m/init.mp4"), split.init.clone());
    for (i, seg) in segments.iter().enumerate() {
        entries.push_str(&format!("<SegmentURL media=\"s{i}.m4s\"/>"));
        origin = origin.doc(&format!("{HOST}/m/s{i}.m4s"), seg.clone());
    }
    let mpd = format!(
        "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT{}S\" minBufferTime=\"PT1S\"><Period><AdaptationSet mimeType=\"video/mp4\"><Representation id=\"m\" bandwidth=\"500000\" codecs=\"avc1.64001e,mp4a.40.2\"><SegmentList timescale=\"1000\" duration=\"1000\"><Initialization sourceURL=\"init.mp4\"/>{entries}</SegmentList></Representation></AdaptationSet></Period></MPD>",
        segments.len()
    );
    let url = format!("{HOST}/m/manifest.mpd");
    let origin = origin.doc(&url, mpd.into_bytes());
    let sim = DashSim::new(
        &url,
        Config::default(),
        World::new(1_700_000_000 * 1_000_000),
        Box::new(StaticBackend(origin)),
    );
    let media = Fmp4Demux::new()
        .unpackage(whole.as_slice())
        .expect("oracle");
    let oracle = media.tracks.iter().map(|t| sigs(&t.samples)).collect();
    (sim, oracle)
}

#[test]
fn a_muxed_representation_plays_every_track_of_its_init_with_consecutive_ids() {
    let (mut sim, oracle) = build("transmux/h264_aac_frag.mp4");
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    assert!(sim.errors().is_empty(), "{:?}", sim.errors());
    let tracks: Vec<Vec<u32>> = sim
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t.iter().map(|s| s.track_id.value()).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(tracks, vec![vec![1, 2]]);
    dash_sim::assert_samples_equal_oracle(&sim, &oracle);
    let manifest = sim
        .events
        .iter()
        .find_map(|e| match e {
            Event::Variants(m) => Some(m),
            _ => None,
        })
        .unwrap();
    assert!(manifest.renditions.is_empty(), "no separate audio set");
}

#[test]
fn an_encrypted_track_is_reported_and_the_clear_track_still_plays() {
    let (mut sim, oracle) = build("transmux/h264_cenc.mp4");
    sim.run_to_terminal(120 * SEC);
    assert!(sim.ended(), "{:?}", sim.errors());
    assert_eq!(
        sim.errors(),
        vec![(
            ErrorKind::UnsupportedEncryption {
                method: "encv".to_owned()
            },
            false
        )]
    );
    let got = sim.samples();
    assert!(got.values().any(|v| !v.is_empty()), "the clear audio plays");
    let audio_oracle = oracle.iter().find(|t| !t.is_empty()).map(Vec::len);
    assert!(audio_oracle.is_some());
    assert_eq!(
        got.len(),
        1,
        "one track plays: {:?}",
        got.keys().collect::<Vec<_>>()
    );
}
