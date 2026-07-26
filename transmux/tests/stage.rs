//! `broadcast_common::Stage` adoption (media plane step 2e).
//!
//! Two things this file is trying to prove, honestly:
//!
//! 1. **The `Stage` shape genuinely unifies the byte-stream demux family.**
//!    [`generic_drive_helper_unifies_ts_flv_and_progressive_demuxers`] drives
//!    three demuxers that, before this step, had three different native APIs
//!    (`StreamingTsDemux::feed` returns `()`; `StreamingFlvDemux::feed`
//!    returns `Result<(), FlvError>`; `ProgressiveDemux` has no incremental
//!    `feed` at all, just a whole-buffer `Unpackage::unpackage`) through the
//!    *exact same* generic `drive::<S: Stage>` function, and checks the
//!    result against each type's own trusted batch/inherent API — not just
//!    "it compiles".
//! 2. **Where `Stage` does NOT fit, this crate does not force it.** The four
//!    segmenters (`Segmenter`, `LlHlsSegmenter`, `LlSegmenter`,
//!    `StreamingTsHlsSegmenter`) are deliberately not given a `Stage` impl:
//!    their real per-call input is a typed `Sample` (dts/pts/duration/flags/
//!    data), not bytes, and `Stage::feed`'s signature is hardcoded to
//!    `&[u8]` — there is no encoding of `Sample` into bytes anyone downstream
//!    wants, so a `Stage` impl for them would either silently discard real
//!    input or exist purely to satisfy the trait shape. See the media-plane
//!    architecture spec §7 ("`Stage` may under-earn") and this step's report.

use std::path::PathBuf;

use broadcast_common::{Stage, Timestamp};
use transmux::media::Media;
use transmux::{DemuxEvent, ProgressiveDemux, StreamingFlvDemux, StreamingTsDemux, TsDemux};

use broadcast_common::Unpackage;

const FLV: &[u8] = include_bytes!("../../fixtures/flv/av.flv");
const PROGRESSIVE_MP4: &[u8] = include_bytes!("../../fixtures/transmux/h264_aac_prog.mp4");

fn fixtures_ts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts")
}

fn read(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The generic drive loop the brief asks for: feed each chunk (draining
/// `poll()` after every call, since a single `feed` may unlock more than one
/// output), then `finish()` and drain the rest. Works against *any* `Stage`
/// implementor, regardless of what its native `feed`/`finish` signatures
/// looked like before this step.
fn drive<S>(stage: &mut S, chunks: &[&[u8]]) -> Vec<S::Out>
where
    S: Stage,
    S::Error: core::fmt::Debug,
{
    let mut out = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        stage
            .feed(chunk, Timestamp::from_nanos(i as u64))
            .expect("feed");
        while let Some(ev) = stage.poll() {
            out.push(ev);
        }
    }
    stage.finish().expect("finish");
    while let Some(ev) = stage.poll() {
        out.push(ev);
    }
    out
}

/// Drive `StreamingTsDemux` via the generic `drive()` helper, chunked into
/// small (317-byte, deliberately not TS-packet-aligned) pieces, and check the
/// resulting `Sample`/`TrackAdded` counts against the trusted batch `TsDemux`
/// — the same oracle `transmux/tests/streaming_demux.rs` uses.
#[test]
fn generic_drive_helper_unifies_ts_flv_and_progressive_demuxers() {
    // --- StreamingTsDemux ---------------------------------------------------
    let ts_bytes = read(&fixtures_ts_dir().join("h264_aac.ts"));
    let oracle_ts: Media = TsDemux::new().unpackage(&ts_bytes).expect("batch TS demux");
    let oracle_ts_samples: usize = oracle_ts.tracks.iter().map(|t| t.samples.len()).sum();

    let ts_chunks: Vec<&[u8]> = ts_bytes.chunks(317).collect();
    let mut ts_stage = StreamingTsDemux::new();
    let ts_events = drive(&mut ts_stage, &ts_chunks);
    let ts_added = ts_events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::TrackAdded(_)))
        .count();
    let ts_samples = ts_events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::Sample { .. }))
        .count();
    assert_eq!(
        ts_added,
        oracle_ts.tracks.len(),
        "Stage-driven StreamingTsDemux must add the same tracks as the batch oracle"
    );
    assert_eq!(
        ts_samples, oracle_ts_samples,
        "Stage-driven StreamingTsDemux must yield the same sample count as the batch oracle"
    );

    // --- StreamingFlvDemux ---------------------------------------------------
    let flv_bytes: &[u8] = FLV;
    let oracle_flv: Media = transmux::FlvDemux::new()
        .unpackage(flv_bytes)
        .expect("batch FLV demux");
    let oracle_flv_samples: usize = oracle_flv.tracks.iter().map(|t| t.samples.len()).sum();

    let flv_chunks: Vec<&[u8]> = flv_bytes.chunks(257).collect();
    let mut flv_stage = StreamingFlvDemux::new();
    let flv_events = drive(&mut flv_stage, &flv_chunks);
    let flv_added = flv_events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::TrackAdded(_)))
        .count();
    let flv_samples = flv_events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::Sample { .. }))
        .count();
    assert_eq!(
        flv_added,
        oracle_flv.tracks.len(),
        "Stage-driven StreamingFlvDemux must add the same tracks as the batch oracle"
    );
    assert_eq!(
        flv_samples, oracle_flv_samples,
        "Stage-driven StreamingFlvDemux must yield the same sample count as the batch oracle"
    );

    // --- ProgressiveDemux -----------------------------------------------------
    // Whole-file parse (see the type's own docs): `Out = Media`, one value
    // popped from `poll()` once, after `finish()` — proving `drive()` handles
    // an `Out` type that isn't `DemuxEvent` at all just as well.
    let oracle_prog: Media = ProgressiveDemux::new()
        .unpackage(PROGRESSIVE_MP4)
        .expect("batch progressive demux");

    let prog_chunks: Vec<&[u8]> = PROGRESSIVE_MP4.chunks(4096).collect();
    let mut prog_stage = ProgressiveDemux::new();
    let mut prog_media = drive(&mut prog_stage, &prog_chunks);
    assert_eq!(
        prog_media.len(),
        1,
        "ProgressiveDemux emits exactly one Media"
    );
    let media = prog_media.pop().unwrap();
    assert_eq!(
        media.tracks.len(),
        oracle_prog.tracks.len(),
        "Stage-driven ProgressiveDemux must yield the same track count as Unpackage::unpackage"
    );
    let prog_samples: usize = media.tracks.iter().map(|t| t.samples.len()).sum();
    let oracle_prog_samples: usize = oracle_prog.tracks.iter().map(|t| t.samples.len()).sum();
    assert_eq!(
        prog_samples, oracle_prog_samples,
        "Stage-driven ProgressiveDemux must yield the same sample count as Unpackage::unpackage"
    );
}

/// [`StreamingTsDemux::demand`]'s `saturated` flag must be honest: it tracks
/// the one bound this demuxer actually enforces end-to-end, the
/// never-claimed-PID `unattributed` replay buffer. Flood a PID that never
/// appears in any PAT/PMT (so every packet lands in `unattributed`) until the
/// buffer is packed to its cap, and confirm `demand().saturated` flips to
/// `true` — a real `Limits`-bound transition, not a fabricated one.
#[test]
fn demand_saturated_flips_true_at_the_unattributed_bytes_bound() {
    const NEVER_CLAIMED_PID: u16 = 0x0234;

    fn payload_only_packet(pid: u16, cc: u8) -> [u8; 188] {
        let mut pkt = [0xFFu8; 188];
        pkt[0] = 0x47;
        pkt[1] = ((pid >> 8) as u8) & 0x1F; // payload_unit_start_indicator = 0
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = 0x10 | (cc & 0x0F); // adaptation_field_control = payload only
        pkt
    }

    let mut demux = StreamingTsDemux::new();
    let mut cc: u8 = 0;
    let mut saw_saturated = false;
    // 4 MiB / ~184 payload bytes per packet ~= 22808 packets to cross the
    // cap; 30_000 comfortably clears it (matches the scale the crate's own
    // MAX_PES_BUFFER_BYTES flood test already uses without being slow).
    for _ in 0..30_000u32 {
        let pkt = payload_only_packet(NEVER_CLAIMED_PID, cc);
        cc = cc.wrapping_add(1);
        Stage::feed(&mut demux, &pkt, Timestamp::ZERO).expect("feed never errors");
        while Stage::poll(&mut demux).is_some() {} // drain — no PMT ever resolves this PID
        if Stage::demand(&demux).saturated {
            saw_saturated = true;
            break;
        }
    }
    assert!(
        saw_saturated,
        "demand().saturated must flip true once the unattributed replay buffer hits its cap"
    );
}
