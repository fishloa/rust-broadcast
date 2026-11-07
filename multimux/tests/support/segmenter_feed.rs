use multimux::route::RouteHandle;
use transmux::TrackId;
use transmux::avc_config_from_sprop;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

pub const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
pub const VIDEO_TIMESCALE: u32 = 90_000;
pub const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
pub const TARGET_DURATION_SECS: f64 = 1.0;
pub const PART_TARGET_MS: u32 = 500;

pub fn video_track_spec_sized(width: u16, height: u16) -> TrackSpec {
    let config = avc_config_from_sprop(SPROP).expect("valid sprop");
    TrackSpec::new(
        TrackId::new(1),
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config,
            width,
            height,
        },
    )
}

pub fn video_track_spec() -> TrackSpec {
    video_track_spec_sized(0, 0)
}

pub fn video_batches_n(n: u32) -> Vec<Vec<(u32, Sample)>> {
    (0..n)
        .map(|i| {
            let ts = i64::from(i) * i64::from(FRAME_DUR);
            let data = vec![0xAAu8.wrapping_add((i % 251) as u8); 64];
            vec![(
                1u32,
                Sample::new(data, Some(ts), Some(ts), Some(FRAME_DUR), i % 30 == 0),
            )]
        })
        .collect()
}

/// Publishes program 0 on `store`, sets the init segment and pushes every
/// batch through a real `LlHlsSegmenter`, leaving the in-progress segment open
/// (a preload hint is present).
pub fn feed_open_segment(
    store: &RouteHandle,
    specs: Vec<TrackSpec>,
    batches: Vec<Vec<(u32, Sample)>>,
) -> LlHlsSegmenter {
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);

    let mut seg = LlHlsSegmenter::with_part_target(
        specs,
        transmux::VIDEO_CLOCK_RATE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(program, seg.init_segment().expect("init segment builds"));

    for batch in batches {
        for (track_id, sample) in batch {
            seg.push(TrackId::new(track_id), sample)
                .expect("push succeeds");
        }
        drain_ready(store, &mut seg);
    }
    seg
}

/// [`feed_open_segment`], then flushes the final segment.
///
/// MUTATION VERIFIED: commenting out the `store.set_init(...)` call in
/// `feed_open_segment` makes `end_to_end_pipeline_serves_valid_llhls` fail on
/// the init-segment `GET` (404 instead of 200): with no init bytes set the
/// route has nothing to serve.
pub fn feed_via_segmenter(
    store: &RouteHandle,
    specs: Vec<TrackSpec>,
    batches: Vec<Vec<(u32, Sample)>>,
) {
    let mut seg = feed_open_segment(store, specs, batches);
    seg.flush().expect("flush succeeds");
    drain_ready(store, &mut seg);
}

fn drain_ready(store: &RouteHandle, seg: &mut LlHlsSegmenter) {
    let program = media_plane::ProgramId(0);
    for part in seg.take_ready_parts() {
        store.add_part(program, part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(program, segment).expect("add_segment");
    }
}
