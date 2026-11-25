//! A media segment of a generated video track is never assembled into an
//! `init ++ segment` buffer: the Representation's stage is fed the init once
//! and each segment afterwards.
//!
//! The stage's own byte buffer grows once, on the first segment, to the init's
//! bytes plus that segment; every later segment reuses it. So the measured
//! window is a segment after the first: a demuxer that concatenated init and
//! segment per segment (the behaviour this lane removes) allocates exactly that
//! size again for it, while the streaming stage allocates only the segment.
//!
//! The counter is process-wide, so this binary carries exactly one `#[test]`.

use broadcast_common::Unpackage;
use dash_runtime::client::{DashClient, PullLimits};
use test_harness::dash::loopback::{Drive, Hook, Origin, drive};
use transmux::ir::{SampleFlags, SampleTiming};
use transmux::pipeline::{FragmentTrackData, build_init_segment, build_media_segment};
use transmux::ts_demux::TsDemux;

#[global_allocator]
static A: test_alloc::ProcessCounting = test_alloc::ProcessCounting::new();

/// Bytes in the generated sample: large enough that a concatenation buffer is
/// visibly larger than the segment-carrying buffers the streaming path needs.
const SAMPLE_BYTES: usize = 4 << 20;

/// The drive step whose response is the second media segment; the counter is
/// reset at it, after the first segment's one-time buffer growth.
const SECOND_SEGMENT_STEP: usize = 3;

/// Segments in the generated presentation.
const SEGMENTS: u64 = 2;

const MANIFEST: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT2S" minBufferTime="PT1S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
 <Period start="PT0S">
  <AdaptationSet mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
   <Representation id="v" bandwidth="1000000" width="320" height="240">
    <SegmentTemplate timescale="{timescale}" duration="{timescale}" startNumber="1" initialization="init.mp4" media="seg-$Number$.m4s"/>
   </Representation>
  </AdaptationSet>
 </Period>
</MPD>
"#;

fn presentation() -> (String, Vec<u8>, Vec<u8>) {
    let ts = test_fixtures::read("ts/h264-aac.ts");
    let media = TsDemux::new().unpackage(&ts[..]).expect("demux TS fixture");
    let track = &media.tracks[0];
    let timescale = track.spec.timescale.get();
    let init = build_init_segment(std::slice::from_ref(&track.spec), media.movie_timescale)
        .expect("build init");

    let mut sample = track.samples[0].clone();
    sample.data = vec![0xAAu8; SAMPLE_BYTES].into();
    sample.timing = Some(SampleTiming::timed_with_duration(0, 0, timescale.max(1)));
    sample.flags = SampleFlags::SYNC;
    let samples = [sample];
    let fragment = FragmentTrackData::new(track.spec.track_id, 0, &samples);
    let segment = build_media_segment(1, &[fragment]).expect("build segment");

    let mpd = MANIFEST
        .replace("{timescale}", &timescale.to_string())
        .replace("{timescale}", &timescale.to_string());
    (mpd, init, segment)
}

#[test]
fn a_later_segment_is_never_assembled_into_an_init_plus_segment_buffer() {
    const URL: &str = "http://dash.test/generated/manifest.mpd";
    const INIT_URL: &str = "http://dash.test/generated/init.mp4";

    A.set_armed(false);
    let (mpd, init, segment) = presentation();
    let mut origin = Origin::new()
        .doc(URL, mpd.into_bytes())
        .doc(INIT_URL, init.clone());
    for number in 1..=SEGMENTS {
        origin = origin.doc(
            &format!("http://dash.test/generated/seg-{number}.m4s"),
            segment.clone(),
        );
    }
    let budget = init.len() + segment.len();
    A.reset_largest();
    A.set_armed(true);
    let mut primed = false;
    let mut opts = Drive {
        hook: Some(Box::new(move |_: &mut DashClient, step: usize| {
            if step == SECOND_SEGMENT_STEP && !primed {
                primed = true;
                A.reset_largest();
            }
        }) as Hook),
        ..Drive::default()
    };
    let run = drive(URL, PullLimits::default(), &mut origin, &mut opts);
    let largest = A.largest();
    A.set_armed(false);

    assert!(run.ended, "{:?}", run.fatal);
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    let samples: usize = run.samples().values().map(Vec::len).sum();
    assert_eq!(
        samples,
        usize::try_from(SEGMENTS).expect("segment count fits usize"),
        "each generated segment carries one sample"
    );
    assert!(
        largest >= SAMPLE_BYTES,
        "the window did not cover the segment demux: largest allocation {largest}"
    );
    assert!(
        largest < budget,
        "largest allocation {largest} reached the {budget}-byte init+segment concatenation (init {}, segment {})",
        init.len(),
        segment.len()
    );
}
