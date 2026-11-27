//! Damage reporting through the RTSP client: junk between interleaved frames is
//! resynchronised over and reported, the frames after it still decode, and the
//! caller's policy decides whether the report ends the session.

use broadcast_core::damage::{DamageKind, DamagePolicy, Severity};
use rtsp_runtime::{ClientEvent, ClientSession, InterleavedFrame, RtspClientConfig};

fn wire(text: &str) -> Vec<u8> {
    text.replace('\n', "\r\n").into_bytes()
}

fn frames() -> (Vec<u8>, Vec<u8>) {
    let first = InterleavedFrame::new(0, vec![1u8, 2, 3])
        .to_bytes()
        .expect("frame");
    let second = InterleavedFrame::new(0, vec![4u8, 5, 6])
        .to_bytes()
        .expect("frame");
    (first, second)
}

fn media(events: &[ClientEvent]) -> Vec<u8> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::MediaData { data, .. } => Some(data.to_vec()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn damages(events: &[ClientEvent]) -> Vec<broadcast_core::damage::Damage> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::Damage(damage) => Some((**damage).clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn junk_between_frames_is_one_report_and_the_frames_still_decode() {
    let mut client = ClientSession::default();
    let (first, second) = frames();
    let mut inbound = first.clone();
    inbound.extend_from_slice(b"garbage!");
    inbound.extend_from_slice(&second);

    let outcome = client.handle_data(&inbound);
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    let reports = damages(&outcome.events);
    assert_eq!(reports.len(), 1, "one report for the skipped run");
    assert_eq!(reports[0].kind(), DamageKind::Resynchronised);
    assert_eq!(reports[0].severity(), Severity::Degraded);
    let range = reports[0]
        .location()
        .byte_range()
        .expect("the junk's byte range");
    assert_eq!(range.start(), first.len() as u64);
    assert_eq!(range.len(), 8);
    assert_eq!(
        media(&outcome.events),
        vec![1, 2, 3, 4, 5, 6],
        "both frames decode past the resync"
    );
}

#[test]
fn a_clean_interleaved_stream_reports_nothing() {
    let mut client = ClientSession::default();
    let (first, second) = frames();
    let mut inbound = first;
    inbound.extend_from_slice(&second);
    let outcome = client.handle_data(&inbound);
    assert!(outcome.error.is_none());
    assert!(damages(&outcome.events).is_empty());
    assert_eq!(media(&outcome.events), vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn the_fail_policy_ends_the_session_with_the_same_report() {
    let mut client = ClientSession::new(
        RtspClientConfig::default()
            .with_damage_policy(DamagePolicy::new().fail_on(Severity::Degraded)),
    )
    .expect("client");
    let (first, second) = frames();
    let mut inbound = first;
    inbound.extend_from_slice(b"junk");
    inbound.extend_from_slice(&second);
    let outcome = client.handle_data(&inbound);
    let error = outcome.error.expect("the policy stops the session");
    let rtsp_runtime::Error::Damage(damage_error) = error else {
        panic!("expected the policy stop, got {error}");
    };
    assert_eq!(damage_error.damage().kind(), DamageKind::Resynchronised);
    assert_eq!(damage_error.damage().severity(), Severity::Degraded);
    assert_eq!(
        damage_error
            .damage()
            .location()
            .byte_range()
            .map(|r| r.len()),
        Some(4)
    );
}

#[test]
fn a_response_without_a_cseq_is_reported_as_skipped_metadata() {
    let mut client = ClientSession::default();
    let outcome = client.handle_data(&wire("RTSP/1.0 200 OK\n\n"));
    assert!(outcome.error.is_none());
    let reports = damages(&outcome.events);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].kind(), DamageKind::MetadataSkipped);
    assert_eq!(reports[0].severity(), Severity::Informational);
    assert_eq!(reports[0].what(), "RTSP response (no CSeq)");
}

#[test]
fn a_media_section_dropped_from_the_plan_converts_to_one_report() {
    use rtsp_runtime::play::SkippedMedia;
    let media = SkippedMedia::new(2, "rtsp://other.example/x");
    let damage: broadcast_core::damage::Damage = (&media).into();
    assert_eq!(damage.kind(), DamageKind::TrackDropped);
    assert_eq!(damage.severity(), Severity::Degraded);
    assert_eq!(damage.location().ordinal(), Some(2));
    assert_eq!(damage, media.damage());
}
