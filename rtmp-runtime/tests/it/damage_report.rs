//! The damage an RTMP ingest session reports, over real FLV bytes: the
//! player is told what an interrupted message cost, playback continues, and
//! a `Fail` policy stops the session with the same report.
//!
//! See `rtmp_runtime::damage` for the list of what a session reports and
//! what stays an error.

use broadcast_core::Parse;
use rtmp_runtime::RtmpError;
use rtmp_runtime::amf0::{Amf0Value, Command};
use rtmp_runtime::chunk::{ChunkWriter, Message};
use rtmp_runtime::flv::{FlvHeader, FlvTag, TagType, tags};
use rtmp_runtime::handshake::{HANDSHAKE_PACKET_LEN, RTMP_VERSION};
use rtmp_runtime::ids::{ChunkStreamId, MessageStreamId, Timestamp};
use rtmp_runtime::message::MessageTypeId;
use rtmp_runtime::server::{RtmpServerConfig, ServerEvent, ServerSession};

const CLIENT_CSID: u32 = 3;
const MEDIA_CSID: u32 = 4;
const STREAM_ID: u32 = 1;

fn handshake() -> (Vec<u8>, Vec<u8>) {
    let mut c0_c1 = vec![0u8; 1 + HANDSHAKE_PACKET_LEN];
    c0_c1[0] = RTMP_VERSION;
    (c0_c1, vec![0u8; HANDSHAKE_PACKET_LEN])
}

fn command(name: &str, stream_id: u32, txn: f64, args: Vec<Amf0Value>) -> Vec<u8> {
    let body = Command {
        name: name.to_string(),
        transaction_id: txn,
        arguments: args,
    }
    .to_body()
    .expect("command body");
    ChunkWriter::new()
        .write(&Message {
            chunk_stream_id: ChunkStreamId::new(CLIENT_CSID),
            timestamp: Timestamp::ZERO,
            message_type_id: MessageTypeId::CommandAmf0,
            message_stream_id: MessageStreamId::new(stream_id),
            payload: body,
        })
        .expect("command message")
}

/// Drive the handshake, `connect`, `createStream` and `publish`, exactly as a
/// publisher does.
fn publishing(policy: broadcast_core::damage::DamagePolicy) -> ServerSession {
    let mut session = ServerSession::new(RtmpServerConfig::default().with_damage_policy(policy));
    let (c0_c1, c2) = handshake();
    session.handle_data(&c0_c1).expect("C0+C1");
    session.handle_data(&c2).expect("C2");
    let connect = command(
        "connect",
        0,
        1.0,
        vec![Amf0Value::Object(vec![(
            "app".to_string(),
            Amf0Value::String("live".to_string()),
        )])],
    );
    session.handle_data(&connect).expect("connect");
    session
        .handle_data(&command("createStream", 0, 2.0, vec![Amf0Value::Null]))
        .expect("createStream");
    let (_out, _events) = session
        .handle_data(&command(
            "publish",
            STREAM_ID,
            3.0,
            vec![
                Amf0Value::Null,
                Amf0Value::String("key".to_string()),
                Amf0Value::String("live".to_string()),
            ],
        ))
        .expect("publish");
    session
}

/// Every Audio/Video/Data tag of the committed FLV fixture, as RTMP media
/// messages on [`MEDIA_CSID`].
fn fixture_media() -> Vec<(MessageTypeId, u32, Vec<u8>)> {
    let bytes = test_fixtures::read("flv/av.flv");
    let header = FlvHeader::parse(&bytes).expect("FLV header");
    let mut out = Vec::new();
    for tag in tags(&bytes, &header).take(4) {
        let tag: FlvTag<'_> = tag.expect("FLV tag");
        let message_type_id = match tag.tag_type {
            TagType::Audio => MessageTypeId::Audio,
            TagType::Video => MessageTypeId::Video,
            TagType::Script => MessageTypeId::DataAmf0,
            _ => continue,
        };
        out.push((message_type_id, tag.timestamp, tag.body.to_vec()));
    }
    assert!(!out.is_empty(), "the fixture must carry media tags");
    out
}

fn media(tag: &(MessageTypeId, u32, Vec<u8>)) -> Vec<u8> {
    media_on_stream(STREAM_ID, tag)
}

fn media_on_stream(stream_id: u32, tag: &(MessageTypeId, u32, Vec<u8>)) -> Vec<u8> {
    ChunkWriter::new()
        .write(&Message {
            chunk_stream_id: ChunkStreamId::new(MEDIA_CSID),
            timestamp: Timestamp::from_millis(tag.1),
            message_type_id: tag.0,
            message_stream_id: MessageStreamId::new(stream_id),
            payload: tag.2.clone(),
        })
        .expect("media message")
}

/// A chunk stream whose message is interrupted mid-payload: the first chunk of
/// a 300-byte audio message, then a complete one-byte Type 0 header for a new
/// message on the same chunk stream. The interrupted message can never
/// complete, which is the damage.
fn interrupted_message_then(tag: &(MessageTypeId, u32, Vec<u8>)) -> Vec<u8> {
    let mut writer = ChunkWriter::new();
    let partial = writer
        .write(&Message {
            chunk_stream_id: ChunkStreamId::new(MEDIA_CSID),
            timestamp: Timestamp::from_millis(tag.1),
            message_type_id: MessageTypeId::Audio,
            message_stream_id: MessageStreamId::new(STREAM_ID),
            payload: vec![0xAA; 300],
        })
        .expect("partial message");
    let first_chunk = 1 + 11 + 128;
    let mut wire = partial.get(..first_chunk).expect("first chunk").to_vec();

    // A new message header on the same csid, mid-message: written by this
    // crate's own chunk writer so the framing is exactly the wire's.
    wire.extend_from_slice(
        &ChunkWriter::new()
            .write(&Message {
                chunk_stream_id: ChunkStreamId::new(MEDIA_CSID),
                timestamp: Timestamp::from_millis(tag.1 + 1),
                message_type_id: MessageTypeId::Audio,
                message_stream_id: MessageStreamId::new(STREAM_ID),
                payload: vec![0x55],
            })
            .expect("interrupting message"),
    );
    wire
}

#[test]
fn a_damaged_message_is_reported_and_the_session_continues() {
    let mut session = publishing(broadcast_core::damage::DamagePolicy::new());
    let fixture = fixture_media();

    let (_out, events) = session
        .handle_data(&interrupted_message_then(&fixture[0]))
        .expect("an interrupted message must not end the session");
    let reports: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            ServerEvent::Damage(damage) => Some(*damage.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reports.len(), 1, "one report per interrupted message");
    assert_eq!(
        reports[0].kind(),
        broadcast_core::damage::DamageKind::MediaUnitDropped
    );
    assert_eq!(
        reports[0].severity(),
        broadcast_core::damage::Severity::Degraded
    );
    assert_eq!(reports[0].what(), "RTMP message");

    // Playback continues: a complete real fixture tag still arrives.
    for tag in &fixture {
        let (_out, events) = session.handle_data(&media(tag)).expect("next tag");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ServerEvent::Media { .. })),
            "media after the damage must still be delivered"
        );
    }
}

#[test]
fn the_fail_policy_ends_the_same_session_with_the_same_report() {
    let mut session = publishing(
        broadcast_core::damage::DamagePolicy::new()
            .fail_on(broadcast_core::damage::Severity::Degraded),
    );
    let fixture = fixture_media();
    let error = session
        .handle_data(&interrupted_message_then(&fixture[0]))
        .expect_err("a failing policy stops the session");
    let RtmpError::Damage(ref damage) = error else {
        panic!("expected RtmpError::Damage, got {error:?}");
    };
    assert_eq!(
        damage.damage().kind(),
        broadcast_core::damage::DamageKind::MediaUnitDropped
    );
    assert_eq!(damage.damage().what(), "RTMP message");
}

#[test]
fn a_clean_publish_reports_nothing() {
    let mut session = publishing(broadcast_core::damage::DamagePolicy::new());
    let mut media_events = 0usize;
    for tag in &fixture_media() {
        let (_out, events) = session.handle_data(&media(tag)).expect("clean tag");
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ServerEvent::Damage(_))),
            "a clean publish reports nothing: {events:?}"
        );
        media_events += events
            .iter()
            .filter(|event| matches!(event, ServerEvent::Media { .. }))
            .count();
    }
    assert!(media_events > 0, "the clean run must deliver media");
}

#[test]
fn media_on_a_stream_id_the_session_never_opened_is_reported() {
    let mut session = publishing(broadcast_core::damage::DamagePolicy::new());
    let fixture = fixture_media();
    let (_out, events) = session
        .handle_data(&media_on_stream(STREAM_ID + 1, &fixture[0]))
        .expect("an unknown stream id must not end the session");
    assert_eq!(events.len(), 1, "{events:?}");
    let ServerEvent::Damage(damage) = &events[0] else {
        panic!("expected a damage report, got {events:?}");
    };
    assert_eq!(
        damage.kind(),
        broadcast_core::damage::DamageKind::MediaUnitDropped
    );
    assert_eq!(
        damage.severity(),
        broadcast_core::damage::Severity::Informational
    );
}

#[test]
fn an_oversized_message_is_skipped_by_the_chunk_readers_own_report() {
    // The chunk layer's reports reach the session's event channel: a message
    // larger than the C0/C1 handshake path can buffer is consumed without
    // being held, reported once, and the session keeps reading.
    let mut session = publishing(broadcast_core::damage::DamagePolicy::new());
    let mut wire = vec![0x04u8];
    wire.extend_from_slice(&[0, 0, 0]); // timestamp
    wire.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // ~16 MiB declared
    wire.push(MessageTypeId::Video.to_u8());
    wire.extend_from_slice(&[0, 0, 0, STREAM_ID as u8]);
    wire.extend(core::iter::repeat_n(0x11u8, 128));
    let (_out, events) = session.handle_data(&wire).expect("skipped, not fatal");
    assert!(
        events.iter().any(|event| matches!(
            event,
            ServerEvent::Damage(damage)
                if damage.kind() == broadcast_core::damage::DamageKind::LimitReached
        )),
        "the over-limit message must be reported: {events:?}"
    );

    let fixture = fixture_media();
    let (_out, events) = session.handle_data(&media(&fixture[0])).expect("next");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ServerEvent::Media { .. }))
    );
}

#[test]
fn the_flv_readers_tolerant_walk_reports_what_it_skipped() {
    // `tags_resync` is this crate's tolerant FLV reader: a fixture truncated
    // mid-tag must report the tail it lost, not just expose a tuple.
    let bytes = test_fixtures::read("flv/av.flv");
    let header = FlvHeader::parse(&bytes).expect("FLV header");
    let intact = rtmp_runtime::flv::tags_resync(&bytes, &header).expect("walk");
    assert!(
        intact.damage().is_empty(),
        "an intact fixture reports nothing: {:?}",
        intact.damage()
    );

    // Cut the last tag one byte into its body: the walk keeps every complete
    // tag before it and reports the body it could not read.
    let (last_offset, last_tag) = intact.tags.last().expect("a last tag");
    let cut_at = last_offset + rtmp_runtime::flv::FLV_TAG_HEADER_LEN + 1;
    let cut = &bytes[..cut_at];
    let truncated = rtmp_runtime::flv::tags_resync(cut, &header).expect("walk");
    let reports = truncated.damage();
    assert_eq!(reports.len(), 1, "the short tail is reported: {reports:?}");
    assert!(last_tag.body.len() > 1, "the last tag must have a body");
    assert_eq!(
        reports[0].kind(),
        broadcast_core::damage::DamageKind::InputTruncated
    );
    assert!(reports[0].location().byte_range().is_some());
}

#[test]
fn a_flv_tag_stream_with_damaged_framing_reports_the_span_it_jumped() {
    let bytes = test_fixtures::read("flv/av.flv");
    let header = FlvHeader::parse(&bytes).expect("FLV header");
    let mut damaged = bytes.clone();
    let first_tag = header.first_tag_offset();
    // Shrink the first tag's `DataSize` so its body no longer ends where the
    // next tag starts: the chain stops holding from that offset on.
    damaged[first_tag + 1] = 0;
    damaged[first_tag + 2] = 0;
    damaged[first_tag + 3] = 1;
    let walk = rtmp_runtime::flv::tags_resync(&damaged, &header).expect("walk");
    let reports = walk.damage();
    assert!(
        reports
            .iter()
            .any(|r| r.kind() == broadcast_core::damage::DamageKind::Resynchronised),
        "a jumped span must be reported: {reports:?}"
    );
    assert!(
        reports
            .iter()
            .filter_map(|r| r.location().byte_range())
            .any(|range| range.start() == first_tag as u64 && !range.is_empty()),
        "the report names the bytes jumped"
    );
    assert!(!walk.tags.is_empty(), "the tags after the damage survive");
}
