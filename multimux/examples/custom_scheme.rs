//! Register a custom input scheme with **zero multimux edits** (issue #663
//! external scheme plugin registry).
//!
//! A third-party crate that wants to feed multimux from a transport this
//! crate doesn't know about (WebRTC, a proprietary camera SDK, ...) writes
//! its own [`media_plane::ingress::Dialer`]/[`media_plane::ingress::IngestSession`]
//! pair, registers a factory for it under a `type_tag` in a
//! [`multimux::SchemeRegistry`], and names that tag in a config's
//! `InputSpec::Custom` — no fork, no PR against this crate.
//!
//! # The plugin shape (issue #805 task 5)
//!
//! Every built-in source (`multimux::source::rtsp`, `ts_udp`, `rtmp`, ...) is
//! driven by [`multimux::supervise_driver`] over a `Dialer`/`Listener` +
//! `IngestSession` pair from `media_plane::ingress` — a `Custom` scheme's own
//! factory drives its own such pair the identical way, so this example is not
//! a simplified toy: it is the same shape every in-tree source uses, just
//! with a trivial (synthetic, single-track) session in place of a real
//! transport. [`DemoDialer`]/[`DemoSession`] below are that pair;
//! [`run_demo`] is the one dial-through-disconnect `attempt` closure
//! [`multimux::supervise_driver`] retries with backoff, exactly mirroring
//! what `multimux::source::ts_udp::run_ts_udp` (etc.) does internally.
//!
//! **One call** inside [`run_demo`] is the whole extension contract:
//! [`multimux::source::advance_route`] publishes each newly announced
//! program's driver-minted `Trunk` into the route's registry (the *only* way
//! an external factory can make its ingest resolvable — the registry itself
//! is crate-private) *and* turns the raw samples that land into real,
//! LL-HLS-servable init/segment/part bytes, over one opaque
//! [`multimux::source::DriverProgress`] state value the caller declares once
//! per connection attempt and threads through every call. Earlier revisions
//! of this crate exposed the two steps (`report_driver_progress` +
//! `segment::drive_program_segmenters`) separately, which meant a plugin
//! author had to call both, in the right order, every iteration; skipping
//! either (or getting the order wrong) meant the route either never resolved
//! (a permanent `NotYetAnnounced`/`503`) or ingested silently with nothing
//! ever servable. `advance_route` makes that impossible: there is exactly one
//! call to make.
//!
//! This example actually drives the registered factory end to end (not just
//! registry lookup + config parsing): it builds a bare [`multimux::RouteHandle`]
//! (standing in for the one `multimux::origin::serve_with_registry` would
//! build per configured route), invokes the factory with an [`InputCtx`]
//! exactly like that function does, and waits for real fMP4 init bytes to
//! land — proving the whole chain actually produces servable media, not just
//! that the pieces compile. It never calls
//! [`multimux::serve`]/[`multimux::serve_with_registry`] (which block forever
//! serving HTTP over a real socket) — a real embedding application wires the
//! same registry into one of those instead.
//!
//! `examples/custom-scheme.json` is the on-disk counterpart of the inline
//! JSON built below: the same single-route config (a `"demo"`-tagged
//! `InputSpec::Custom`) as a standalone file, for a `multimux-cli --config`
//! invocation naming this exact scheme.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example custom_scheme
//! ```

use std::collections::VecDeque;
use std::convert::Infallible;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{
    Dialer, HandshakePolicy, IngestDriver, IngestSession, ProgramId, SessionEvent,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};
use multimux::config::{Config, InputSpec};
use multimux::registry::InputCtx;
use multimux::source::{DriverProgress, advance_route};
use multimux::{Backoff, RouteHandle, SchemeRegistry, supervise_driver};
use transmux::avc_config_from_sprop;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

/// A real-ish `sprop-parameter-sets` pair (SPS+PPS) — the same one used
/// throughout this crate's own tests — decoded into a genuine `avcC`
/// configuration record, so the media this example produces is structurally
/// real, not a fabricated placeholder.
const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
/// 90 kHz video timescale — 1/30 s per access unit at 30 fps.
const VIDEO_TIMESCALE: u32 = 90_000;
const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
/// Enough synthetic frames (2 s @ 30 fps) that a real scheme's samples would
/// already have landed in the `Trunk`.
const FRAME_COUNT: u32 = 60;
/// One sync (IDR) frame per second.
const SYNC_INTERVAL_FRAMES: u32 = 30;

fn track_spec() -> TrackSpec {
    let config = avc_config_from_sprop(SPROP).expect("valid sprop");
    TrackSpec::new(
        1,
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config,
            width: 0,
            height: 0,
        },
    )
}

/// A trivial [`IngestSession`] standing in for a real external ingest
/// transport.
///
/// Its **first** [`Stage::feed`] call announces one program carrying a
/// single synthetic AVC track *and* queues every one of its (already-built)
/// samples, in that same call — the ordinary shape a real transport
/// produces (e.g. a single MPEG-TS feed batch commonly carries the PMT and
/// the first PES samples together), and exactly the shape
/// [`multimux::source::segment::ProgramSegmenter`]'s
/// `subscribe_from_backlog` cursor (issue #808) exists to handle: it
/// replays whatever backlog is already resident in the ring by the time
/// `drive_program_segmenters` builds the segmenter, so samples published in
/// the same `feed` call that announced the program are not lost.
struct DemoSession {
    pending: VecDeque<SessionEvent>,
    sent: bool,
}

impl DemoSession {
    fn new() -> Self {
        let mut pending = VecDeque::new();
        pending.push_back(SessionEvent::Established);
        DemoSession {
            pending,
            sent: false,
        }
    }
}

impl Stage for DemoSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = Infallible;

    fn demand(&self) -> Demand {
        Demand::new(1)
    }

    fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
        if !self.sent {
            self.sent = true;
            self.pending.push_back(SessionEvent::NewProgram {
                program: ProgramId(0),
                tracks: vec![track_spec()],
            });
            for i in 0..FRAME_COUNT {
                let is_sync = i % SYNC_INTERVAL_FRAMES == 0;
                let data = vec![0xAAu8.wrapping_add((i % 251) as u8); 32];
                let sample = Sample::new(
                    data,
                    Some(i64::from(i) * i64::from(FRAME_DUR)),
                    Some(i64::from(i) * i64::from(FRAME_DUR)),
                    Some(FRAME_DUR),
                    is_sync,
                );
                self.pending.push_back(SessionEvent::Sample {
                    program: ProgramId(0),
                    track_id: 1,
                    retention: RetentionClass::Timed,
                    sample,
                });
            }
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl IngestSession for DemoSession {
    /// Uninhabited: this session never has anything of its own to send
    /// (`poll_transmit`'s `None` default is exactly right) — a byte-stream
    /// scheme would set this to `bytes::Bytes` instead (see
    /// `multimux::source::rtsp::RtspIngestSession` for that shape).
    type Request = Infallible;
}

/// Constructs a [`DemoSession`] — performs no I/O, exactly like every other
/// [`Dialer::dial`] in this crate (a real scheme's own connect/bind happens
/// inside its `run_*`-equivalent attempt, not here — see
/// `multimux::source::ts_udp`'s module doc, "zero executor bridge").
#[derive(Clone, Copy, Debug, Default)]
struct DemoDialer;

impl Dialer for DemoDialer {
    type Session = DemoSession;
    type Error = Infallible;

    fn dial(&mut self) -> Result<DemoSession, Infallible> {
        Ok(DemoSession::new())
    }
}

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero capacity")
}

/// One dial-through-disconnect attempt for the `"demo"` scheme — the closure
/// [`multimux::supervise_driver`] retries with backoff. Mirrors every
/// in-tree `run_*` (e.g. `multimux::source::ts_udp::run_ts_udp`) exactly:
/// dial, wrap in an [`IngestDriver`], feed, and after every feed call
/// [`advance_route`] (the one facade call: publish the registry + flip
/// health, then turn samples into servable segments/parts). `DemoSession`
/// announces its program and queues all of its samples in one `feed` call
/// (see its own doc), so this attempt drives exactly two feeds
/// (announce+sample, then one after `finish()` to flush the trailing
/// partial segment) and returns `Ok(())` — a real transport's attempt would
/// instead loop forever, awaiting the next read, and only return on a
/// transport error (see `multimux::supervise_driver`'s own doc for why
/// returning is always safe: it just means "try again").
async fn run_demo(route_handle: Arc<RouteHandle>) -> multimux::Result<()> {
    let mut dialer = DemoDialer;
    let session = dialer.dial().unwrap_or_else(|never| match never {});
    // Ring capacities are this factory's own choice, not something multimux
    // hands you (unlike a built-in source's own tuned constants) — sized
    // generously for this trivial demo.
    let trunk_config = TrunkConfig::new(nz(64), nz(16), nz(8), nz(64), nz(64));
    // A generous handshake bound; a real scheme would derive this from its
    // own configured connect timeout.
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let mut driver: IngestDriver<DemoSession> = IngestDriver::new(
        session,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );

    let mut progress = DriverProgress::new();

    // One feed: announces NewProgram AND queues every sample — mints the
    // driver-side Trunk and publishes its samples in the same batch.
    // `advance_route`'s own segmenting step subscribes via
    // `subscribe_from_backlog` (issue #808), which replays whatever backlog
    // is already resident in the ring rather than starting from "now" — so
    // this single feed's samples are not lost.
    driver.feed(&[], Timestamp::from_nanos(0));
    advance_route(&driver, &route_handle, &mut progress);

    // Nothing more to send: end the session cleanly and flush the trailing
    // buffered partial segment, exactly as a real scheme does on a clean
    // disconnect.
    driver.finish();
    advance_route(&driver, &route_handle, &mut progress);

    Ok(())
}

/// Builds a [`SchemeRegistry`] with one custom input scheme (`"demo"`)
/// registered — exactly what an external crate would do in its own code to
/// add a new ingest transport without editing multimux. The factory spawns
/// [`multimux::supervise_driver`] over [`run_demo`], mirroring exactly how
/// `multimux::origin::serve_with_registry`'s own dispatch wires up every
/// built-in `InputSpec` variant.
fn build_registry() -> SchemeRegistry {
    let mut registry = SchemeRegistry::new();
    registry.register_input(
        "demo",
        Arc::new(|ctx: InputCtx| {
            Ok(tokio::spawn(supervise_driver(
                run_demo,
                ctx.store,
                Backoff::production_default(),
                ctx.name,
                ctx.shutdown_rx,
            )))
        }),
    );
    registry
}

#[tokio::main]
async fn main() {
    let registry = build_registry();

    // The registered tag resolves; anything else doesn't — the same lookup
    // `serve_with_registry` performs for an `InputSpec::Custom` route.
    assert!(registry.input("demo").is_some());
    assert!(registry.input("nope").is_none());

    // A config naming the registered scheme parses like any real config
    // would; `Config::validate` always accepts a `Custom` input structurally
    // (the registry — not `validate` — is what would reject a bad `params`,
    // at route-build time inside `serve_with_registry`).
    let json = r#"{
        "routes": [
            {
                "name": "cam1",
                "input": { "type": "custom", "type_tag": "demo", "params": {} }
            }
        ]
    }"#;
    let config: Config = serde_json::from_str(json).expect("valid JSON");
    config
        .validate()
        .expect("a Custom input is always structurally valid");
    match &config.routes[0].input {
        InputSpec::Custom { type_tag, .. } => assert_eq!(type_tag, "demo"),
        other => panic!("expected InputSpec::Custom, got {other:?}"),
    }

    // Actually invoke the registered factory the way `serve_with_registry`
    // would for this route, and wait for real, servable media to land — this
    // is the proof the wiring above is not just type-checked but genuinely
    // moves samples end to end.
    let store = Arc::new(RouteHandle::new(1.0, 500, 8));
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let factory = registry.input("demo").expect("registered above");
    let handle = factory(InputCtx {
        name: "cam1".to_string(),
        params: serde_json::Value::Null,
        store: store.clone(),
        target_duration_secs: 1.0,
        part_target_ms: 500,
        shutdown_rx,
    })
    .expect("factory must succeed");

    // Checks both init bytes *and* a real closed segment: init bytes alone
    // only prove the track spec resolved (`ProgramSegmenter::try_new` builds
    // them from `Trunk::tracks()`, before a single sample has to land), so
    // `window_segments` non-empty is what actually proves the samples
    // `DemoSession`'s second `feed` call queued were observed by the
    // segmenter's cursor and turned into a real, servable segment.
    let landed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if store.init_bytes(ProgramId(0)).is_some()
                && !store.window_segments(ProgramId(0)).is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .is_ok();
    assert!(
        landed,
        "the \"demo\" scheme's synthetic media must land real init bytes AND at least one \
         closed segment in the store"
    );

    handle.abort();
    println!(
        "custom_scheme: registered a \"demo\" input scheme with zero multimux edits; \
         its factory drove a real Dialer/IngestSession through supervise_driver and \
         landed real, servable LL-HLS media in the route store."
    );
}
