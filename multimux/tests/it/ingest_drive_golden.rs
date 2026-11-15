#![cfg(all(feature = "whip", feature = "test-hooks"))]
//! Issue #2373 step 1: the behaviour the WHIP and RTMP ingest session drive
//! loops must preserve when they collapse into one shared driver.
//!
//! Each case drives a REAL protocol session the way `run_whip`/`run_rtmp` do
//! today, through `multimux::ingest_golden`'s recording
//! [`Listener`](media_plane::ingress::Listener), and compares the resulting
//! ordered `SessionEvent` sequence — plus the program and track specs it
//! lands, and the session's final health — against a committed golden under
//! `fixtures/golden/ingest/`.
//!
//! Each case's input is real: the WHIP publish replays the H.264 access units
//! of the committed `ts/h264-aac.ts` capture as RFC 6184 RTP packets, and the
//! RTMP publish replays the ffmpeg `rtmp/obs-publish.bin` capture through a
//! real `rtmp_runtime::io::RtmpConnection` over an in-memory pipe.
//!
//! # Why a session-level recorder, not the live route
//!
//! The live loops batch their feeds differently (one WHIP session per
//! decrypted-datagram batch, RTMP per `next_events` batch), so a golden
//! recorded through either loop would pin that loop's batching rather than the
//! ingest contract. Recording at the session boundary pins the contract both
//! loops share and the shared driver must reproduce.
//!
//! # Regenerating
//!
//! `UPDATE_INGEST_GOLDENS=1 cargo test -p multimux --all-features --locked
//! --test it ingest_drive_golden` rewrites the goldens. A plain run never
//! writes: a mismatch is a failure carrying both documents.

use std::num::NonZeroUsize;

use broadcast_common::{Serialize, Timestamp};
use media_plane::ingress::{HandshakePolicy, ListenDriver, SessionId};
use media_plane::trunk::TrunkConfig;
use multimux::ingest_golden::{
    Recorder, RecordingListener, recording_driver, render_health, render_programs,
};
use multimux::source::rtmp::RtmpIngestSession;
use multimux::source::whip::WhipIngestSession;
use rtmp_runtime::io::{RtmpConnection, RtmpServerConfig};
use rtmp_runtime::server::{RtmpServerConfig as SessionServerConfig, ServerEvent, ServerSession};

const GOLDEN_DIR: &str = "golden/ingest";

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("test capacity must be non-zero")
}

fn trunk_config() -> TrunkConfig {
    TrunkConfig::default()
        .with_timed_capacity(nz(64))
        .with_sparse_capacity(nz(16))
        .with_segment_capacity(nz(8))
        .with_event_capacity(nz(8))
        .with_part_capacity(nz(8))
}

fn handshake() -> HandshakePolicy {
    HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
}

/// The golden document for one session: its events in order, then the
/// programs the session landed, then its final health.
fn render_document(events: &[String], programs: &[String], health: &str) -> String {
    let mut doc = String::new();
    for (i, line) in events.iter().enumerate() {
        doc.push_str(&format!("event[{i}] {line}\n"));
    }
    for line in programs {
        doc.push_str(&format!("published {line}\n"));
    }
    doc.push_str(&format!("health {health}\n"));
    doc
}

/// Compare `actual` against `fixtures/<GOLDEN_DIR>/<name>`; rewrite the file
/// when `UPDATE_INGEST_GOLDENS` is set.
fn assert_golden(name: &str, actual: &str) {
    let rel = format!("{GOLDEN_DIR}/{name}");
    if std::env::var_os("UPDATE_INGEST_GOLDENS").is_some() {
        let path = test_fixtures::path(&rel);
        std::fs::create_dir_all(path.parent().expect("golden parent")).expect("create golden dir");
        std::fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = test_fixtures::read_to_string(&rel);
    let hint = format!(
        "the {name} ingest event sequence changed; if that is intended, regenerate with \
         UPDATE_INGEST_GOLDENS=1"
    );
    assert_eq!(actual, expected, "{hint}");
}

// ---- WHIP ---------------------------------------------------------------

/// The real capture whose H.264 access units a publish replays: Main profile,
/// 320x240, 25 fps (`fixtures/ts/PROVENANCE.md`), already committed for every
/// other demux test in this crate.
const WHIP_SOURCE: &str = "ts/h264-aac.ts";

/// The publisher's RTP payload type (its SDP `m=` format); the clock comes
/// from the track's own timescale (RFC 6184 §1: 90 kHz).
const WHIP_PAYLOAD_TYPE: u8 = 96;
/// The publish's RTP sequence start (RFC 3550 §5.1: any value).
const FIRST_SEQUENCE: u16 = 1;

/// The `avcC` the publisher learned out-of-band from its encoder, and the
/// access units it will send — the first of which carries an IDR.
fn whip_source() -> (transmux::TrackSpec, Vec<transmux::Sample>) {
    let ts = test_fixtures::read(WHIP_SOURCE);
    let media = transmux::TsDemux::new()
        .demux(&ts)
        .expect("demux the capture");
    let video = media
        .tracks
        .into_iter()
        .find(|t| t.spec.config.is_video())
        .expect("the capture carries a video track");
    (video.spec, video.samples)
}

/// The publish's RTP source identifier (RFC 3550 §5.1: any value).
const RTP_SSRC: u32 = 0xCAFE_BABE;

/// One RFC 3550 §5.1 RTP packet from the publish's single SSRC, serialized
/// through `rtp-packet`, so the wire bytes are the library's, not a test's.
fn rtp(sequence: u16, timestamp: u32, payload: &[u8]) -> rtp_packet::RtpPacket<'_> {
    rtp_packet::RtpPacket {
        marker: true,
        payload_type: WHIP_PAYLOAD_TYPE,
        sequence_number: sequence,
        timestamp,
        ssrc: RTP_SSRC,
        csrc: vec![],
        extension: None,
        padding: None,
        payload,
    }
}

/// One WHIP publish: the session the drive loop admits, fed the capture's own
/// access units one RFC 6184 single-NAL RTP packet per NAL unit, in wire
/// order — the shape a browser publisher sends. The session announces its
/// program and establishes only once it has captured a real SPS/PPS pair from
/// the keyframe, and every sample before that point is buffered and replayed
/// in arrival order.
#[tokio::test]
async fn whip_publish_event_sequence_matches_the_golden() {
    let (spec, samples) = whip_source();
    let session = WhipIngestSession::from_spec(spec.clone(), WHIP_PAYLOAD_TYPE);
    let (mut driver, recorder) =
        recording_driver(vec![session], 1, trunk_config(), handshake(), nz(64));
    let media_plane::ingress::AcceptOutcome::Admitted(id) = driver.poll_accept() else {
        panic!("the recording listener must admit its one session");
    };

    let mut first_timestamp = None;
    let mut sequence = FIRST_SEQUENCE;
    for (index, sample) in samples.iter().enumerate() {
        let timestamp = u32::try_from(
            sample
                .timing
                .and_then(|t| t.pts())
                .unwrap_or(sample.timing.and_then(|t| t.dts()).unwrap_or(0)),
        )
        .expect("a 90 kHz timestamp fits 32 bits");
        let base = *first_timestamp.get_or_insert(timestamp);
        for unit in
            transmux::iter_length_prefixed_nals(sample.data.as_ref()).expect("length-prefixed NALs")
        {
            let packet = rtp(sequence, timestamp.wrapping_sub(base), unit);
            sequence = sequence.wrapping_add(1);
            let mut sized = vec![0u8; packet.serialized_len()];
            packet
                .serialize_into(&mut sized)
                .expect("the packet fits its own serialized_len");
            driver.feed(id, &sized, Timestamp::from_nanos(index as u64 + 1));
        }
    }
    if let Some(d) = driver.driver_mut(id) {
        d.finish();
    }

    let document = render_session(&driver, &recorder, id);
    assert_golden("whip-publish.txt", &document);
}

// ---- RTMP ---------------------------------------------------------------

/// The real ffmpeg RTMP publish capture (`fixtures/rtmp/obs-publish.bin`),
/// replayed through a fresh `ServerSession` in small chunks — the framing the
/// live accept loop reads it in — and closed at end of capture.
fn rtmp_events() -> Vec<ServerEvent> {
    let capture = test_fixtures::read("rtmp/obs-publish.bin");
    let mut session = ServerSession::new(SessionServerConfig::default());
    let mut events = Vec::new();
    for chunk in capture.chunks(RTMP_CHUNK_BYTES) {
        let Ok((_, emitted)) = session.handle_data(chunk) else {
            break;
        };
        events.extend(emitted);
    }
    events
}

/// Bytes per replay chunk: well under one RTMP chunk-stream size, so the
/// capture reassembles across many `handle_data` calls exactly as a live
/// socket delivers it.
const RTMP_CHUNK_BYTES: usize = 509;

/// Size of the in-memory pipe the captured publish is written through: the
/// whole capture fits, so the writer never blocks on a slow reader (it drains
/// the server's replies concurrently regardless).
const RTMP_PIPE_BYTES: usize = 1 << 20;

/// How long the RTMP replay may take before the test gives up (a hang guard,
/// not a timing claim — the capture replays in milliseconds).
const RTMP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

/// One RTMP publish: the session announces its program and establishes only
/// once the FLV demux has resolved both sequence headers, and every sample
/// that arrived while still establishing is replayed in arrival order.
///
/// The connection can only be built over a stream, so the capture is written
/// into an in-memory pipe and a real [`RtmpConnection`] reads it — the same
/// code path a live publisher's socket drives.
#[tokio::test]
async fn rtmp_publish_event_sequence_matches_the_golden() {
    let capture = test_fixtures::read("rtmp/obs-publish.bin");
    let (client, server) = tokio::io::duplex(RTMP_PIPE_BYTES);
    let writer = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let (mut read, mut write) = tokio::io::split(client);
        let drain = async {
            let mut sink = [0u8; 64 * 1024];
            while read.read(&mut sink).await.is_ok_and(|n| n > 0) {}
        };
        let send = async {
            let _ = write.write_all(&capture).await;
            let _ = write.shutdown().await;
        };
        tokio::join!(drain, send);
    });
    let conn = RtmpConnection::from_stream(server, RtmpServerConfig::default());
    let session = RtmpIngestSession::from_connection(conn);
    let (mut driver, recorder) =
        recording_driver(vec![session], 1, trunk_config(), handshake(), nz(64));
    let media_plane::ingress::AcceptOutcome::Admitted(id) = driver.poll_accept() else {
        panic!("the recording listener must admit its one session");
    };

    let expected_media = rtmp_events()
        .iter()
        .filter(|e| matches!(e, ServerEvent::Media { .. }))
        .count();
    let mut fed = 0usize;
    while fed < expected_media {
        let Some(d) = driver.driver_mut(id) else {
            break;
        };
        let outcome = tokio::time::timeout(
            RTMP_DEADLINE,
            d.session_mut().inner_mut().next_events_for_test(),
        )
        .await;
        let Ok(Ok(Some(batch))) = outcome else {
            break;
        };
        fed += batch
            .iter()
            .filter(|e| matches!(e, ServerEvent::Media { .. }))
            .count();
        d.feed(&batch[..], Timestamp::from_nanos(fed as u64));
    }
    if let Some(d) = driver.driver_mut(id) {
        d.finish();
    }
    writer.abort();

    let programs = render_programs(&driver, id);
    let health = render_health(&driver, id);
    let document = render_document(&recorder.lines(), &programs, &health);
    assert_golden("rtmp-publish.txt", &document);
}

// ---- WHIP, through the live drive loop ----------------------------------

/// The session's health tokens, in the order a golden never needs to mistrust
/// them: "live" while the driver holds it, "reaped" once it is gone.
const HEALTH_REAPED: &str = "reaped";
const HEALTH_LIVE: &str = "live";

/// What one session's driver recorded, rendered as the golden document.
fn render_session(
    driver: &ListenDriver<RecordingListener<WhipIngestSession>>,
    recorder: &Recorder,
    id: SessionId,
) -> String {
    let health = match driver.health(id) {
        None => HEALTH_REAPED,
        Some(_) => HEALTH_LIVE,
    };
    let programs = render_programs(driver, id);
    render_document(&recorder.lines(), &programs, health)
}
