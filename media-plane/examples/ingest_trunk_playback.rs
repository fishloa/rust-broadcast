//! Real end-to-end ingress -> `Trunk` -> egress-cursor pipeline (media-plane
//! step 3f acceptance): demux a genuine broadcast capture with
//! [`transmux::StreamingTsDemux`], adapt its [`transmux::DemuxEvent`] stream
//! into the [`IngestSession`] shape [`IngestDriver`] drives, and subscribe a
//! [`media_plane::SampleCursor`] to the resulting [`media_plane::Trunk`] to
//! show real decoded samples flowing out the far end -- the four-layer
//! architecture from the crate-root docs, exercised with genuine samples
//! rather than the one-byte synthetic samples the unit tests use.
//!
//! Reads `fixtures/ts/h264_aac.ts` (a real broadcast capture shared across
//! this workspace's crates -- see `fixtures/ts/GENERATE.md`) via
//! [`std::fs::read`].
//!
//! ```text
//! cargo run -p media-plane --example ingest_trunk_playback --features std
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::num::NonZeroUsize;

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::{
    DEFAULT_MAX_PROGRAMS, HandshakePolicy, IngestDriver, IngestSession, ProgramId, RetentionClass,
    SampleCursorItem, SessionEvent, TrunkConfig,
};
use transmux::{DemuxEvent, StreamingTsDemux};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
const TS_PACKET_SIZE: usize = 188;

/// Adapts [`StreamingTsDemux`]'s [`DemuxEvent`] stream into the
/// [`IngestSession`] shape: everything demuxed from this real single-program
/// capture lands on one program ([`ProgramId(0)`]), matching how a real SPTS
/// ingest session would report it -- see the module doc for why this exists
/// (no concrete `IngestSession` ships in `media-plane` itself; that is
/// sibling crates' job, e.g. `rtmp-runtime`).
struct TsIngestSession {
    demux: StreamingTsDemux,
    announced: bool,
    pending: VecDeque<SessionEvent>,
}

impl Stage for TsIngestSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = Infallible;

    fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
        self.demux.feed(input);
        self.drain_demux();
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        self.demux.finish();
        self.drain_demux();
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(188 * 256)
    }
}

impl TsIngestSession {
    /// Drain whatever `StreamingTsDemux` has ready, announcing the program
    /// (with whatever tracks have been discovered so far) the first time a
    /// `Sample` arrives -- exactly the "gate on the first sample" contract
    /// [`SessionEvent::Established`]'s own doc describes for a container
    /// with no up-front track declaration, applied here to `NewProgram`.
    fn drain_demux(&mut self) {
        let mut new_tracks = Vec::new();
        while let Some(event) = self.demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) => new_tracks.push(spec),
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    if !self.announced {
                        self.pending.push_back(SessionEvent::NewProgram {
                            program: ProgramId(0),
                            tracks: std::mem::take(&mut new_tracks),
                        });
                        self.announced = true;
                    }
                    self.pending.push_back(SessionEvent::Sample {
                        program: ProgramId(0),
                        track_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
                _ => {}
            }
        }
    }
}

/// Takes the default `poll_transmit` (nothing to send -- this is a replayed
/// capture, not a live handshake).
impl IngestSession for TsIngestSession {}

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("literal capacity is non-zero")
}

fn main() {
    let ts_bytes = std::fs::read(FIXTURE).expect("read the committed real-capture TS fixture");
    println!(
        "ingest_trunk_playback: read {} bytes from {FIXTURE}",
        ts_bytes.len()
    );

    let session = TsIngestSession {
        demux: StreamingTsDemux::new(),
        announced: false,
        pending: VecDeque::new(),
    };
    let trunk_config = TrunkConfig::new(nz(1024), nz(64), nz(32), nz(32), nz(16));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    // `max_programs` bounds how many `Trunk`s one session may mint (the fifth
    // unbounded-allocation vector this workspace has had). A single capture
    // announces one program, so the default ceiling is ample here.
    let mut driver = IngestDriver::new(session, trunk_config, handshake, DEFAULT_MAX_PROGRAMS);

    // Feed the capture in bounded chunks, subscribing a `SampleCursor` the
    // moment the program is announced (right after the first chunk that
    // carries a sample) -- not after the whole file has already been fed.
    // `SampleCursor::subscribe` starts a fresh reader at "nothing published
    // yet" (a live-tail cursor, not a DVR replay from the top of the ring),
    // exactly like a real subscriber that connects mid-stream. Chunking also
    // matches how bytes actually arrive off a real connection, unlike
    // handing the whole file to one `feed` call.
    const CHUNK_PACKETS: usize = 64;
    const CHUNK_SIZE: usize = TS_PACKET_SIZE * CHUNK_PACKETS;
    let mut cursor = None;
    let mut per_track: BTreeMap<u32, usize> = BTreeMap::new();
    let mut lagged = 0u64;

    let drain_cursor = |cursor: &mut Option<media_plane::SampleCursor>,
                        per_track: &mut BTreeMap<u32, usize>,
                        lagged: &mut u64| {
        if let Some(c) = cursor {
            while let Some(item) = c.poll() {
                match item {
                    SampleCursorItem::Timed { track_id, .. } => {
                        *per_track.entry(track_id).or_insert(0) += 1;
                    }
                    SampleCursorItem::Lagged { skipped } => *lagged += skipped,
                    other => println!("ingest_trunk_playback: other cursor item: {other:?}"),
                }
            }
        }
    };

    for (i, chunk) in ts_bytes.chunks(CHUNK_SIZE).enumerate() {
        driver.feed(chunk, Timestamp::from_nanos(i as u64));
        if cursor.is_none() {
            if let Some(trunk) = driver.trunk(ProgramId(0)) {
                cursor = Some(trunk.subscribe());
            }
        }
        drain_cursor(&mut cursor, &mut per_track, &mut lagged);
    }
    driver.finish();
    drain_cursor(&mut cursor, &mut per_track, &mut lagged);
    cursor.expect("a real TS capture with elementary streams announces a program and a Trunk");

    println!("ingest_trunk_playback: real samples landed in the Trunk, by track_id:");
    for (track_id, count) in &per_track {
        println!("  track {track_id}: {count} sample(s)");
    }
    assert!(
        !per_track.is_empty(),
        "the real capture must have produced at least one sample end to end"
    );
    if lagged > 0 {
        println!(
            "ingest_trunk_playback: {lagged} sample(s) evicted before the cursor read them (ring bound hit)"
        );
    }
}
