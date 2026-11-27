//! Every parse failure a `broadcast-bitstream` parser tolerates reaches the
//! demuxer's caller as one `DemuxEvent::Damage` (issue #4196, best-effort
//! reading): the bytes are real committed fixtures, mutated only enough to
//! trigger the skip, and the media after it is still delivered.

use broadcast_common::stage::{Stage, Timestamp};
use broadcast_core::damage::{Damage, DamageKind, DamagePolicy, Disposition, Severity};
use transmux::StreamingPsDemux;
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};

/// One MPEG-2 TS packet (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;

fn fixture(name: &str) -> Vec<u8> {
    test_fixtures::read(name)
}

fn ts_events(
    bytes: &[u8],
    policy: Option<DamagePolicy>,
) -> Result<Vec<DemuxEvent>, transmux::Error> {
    let mut demux = match policy {
        Some(policy) => StreamingTsDemux::new().with_damage_policy(policy),
        None => StreamingTsDemux::new(),
    };
    let mut events = Vec::new();
    for chunk in bytes.chunks(TS_PACKET_LEN) {
        demux.feed(chunk)?;
        while let Some(event) = demux.poll_event() {
            events.push(event);
        }
    }
    demux.finish()?;
    while let Some(event) = demux.poll_event() {
        events.push(event);
    }
    Ok(events)
}

fn ps_events(
    bytes: &[u8],
    policy: Option<DamagePolicy>,
) -> Result<Vec<DemuxEvent>, transmux::Error> {
    let mut demux = match policy {
        Some(policy) => StreamingPsDemux::new().with_damage_policy(policy),
        None => StreamingPsDemux::new(),
    };
    let mut events = Vec::new();
    for chunk in bytes.chunks(TS_PACKET_LEN) {
        demux.feed(chunk, Timestamp::ZERO)?;
        while let Some(event) = demux.poll() {
            events.push(event);
        }
    }
    demux.finish()?;
    while let Some(event) = demux.poll() {
        events.push(event);
    }
    Ok(events)
}

/// The first damage report of `kind` naming `what`.
fn reported<'a>(events: &'a [DemuxEvent], kind: DamageKind, what: &str) -> Option<&'a Damage> {
    events.iter().find_map(|event| match event {
        DemuxEvent::Damage(damage) if damage.kind() == kind && damage.what() == what => {
            Some(damage)
        }
        _ => None,
    })
}

fn sample_count(events: &[DemuxEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, DemuxEvent::Sample { .. }))
        .count()
}

/// A policy that stops on exactly `kind`, whatever its severity.
fn fail_on(kind: DamageKind) -> DamagePolicy {
    DamagePolicy::new().set_kind(kind, Disposition::Fail)
}

fn stopped_at(error: transmux::Error, kind: DamageKind, what: &str) {
    let transmux::Error::Damage(error) = error else {
        panic!("expected a damage stop, got {error:?}");
    };
    let damage = error.damage();
    assert_eq!(damage.kind(), kind, "the stop carries the same report");
    assert_eq!(damage.what(), what);
}

#[test]
fn a_rejected_parameter_set_is_reported() {
    let mut bytes = fixture("ts/h264-aac.ts");
    // The first SPS NAL (ITU-T H.264 §7.3.2.1.1) sits at byte 602; corrupt its
    // body so the decoder rejects it but the NAL header still claims an SPS.
    bytes[610] = 0xFF;

    let events = ts_events(&bytes, None).expect("best effort");
    let damage = reported(&events, DamageKind::MetadataSkipped, "parameter set")
        .expect("a rejected parameter set is reported");
    assert_eq!(damage.severity(), Severity::Informational);
    assert!(sample_count(&events) > 0, "the media after it is delivered");

    let error = ts_events(&bytes, Some(fail_on(DamageKind::MetadataSkipped)))
        .expect_err("a failing policy stops at the report");
    stopped_at(error, DamageKind::MetadataSkipped, "parameter set");
}

#[test]
fn a_malformed_sei_is_reported() {
    let mut bytes = fixture("ts/h264-aac.ts");
    // The capture's only SEI NAL (type 6) sits at byte 638. Its `payloadSize`
    // varint ends at 645; making that byte an escape makes the size claim run
    // past the NAL, which the SEI walk cannot read.
    bytes[645] = 0xFF;

    let events = ts_events(&bytes, None).expect("best effort");
    let damage =
        reported(&events, DamageKind::MetadataSkipped, "SEI").expect("a malformed SEI is reported");
    assert_eq!(damage.severity(), Severity::Informational);
    assert!(sample_count(&events) > 0, "the media after it is delivered");

    let error = ts_events(&bytes, Some(fail_on(DamageKind::MetadataSkipped)))
        .expect_err("a failing policy stops at the report");
    stopped_at(error, DamageKind::MetadataSkipped, "SEI");
}

#[test]
fn unparsable_hdr_metadata_is_reported() {
    let mut bytes = fixture("ts/hevc/main.ts");
    // The prefix SEI NAL at byte 687 carries a user-data message; turn its
    // `payloadType` into the `mastering_display_colour_volume` type (137,
    // ITU-T H.265 D.2.28), whose body is a fixed 24 bytes and cannot parse
    // from this message's much longer payload.
    bytes[692] = 137;

    let events = ts_events(&bytes, None).expect("best effort");
    let damage = reported(&events, DamageKind::MetadataSkipped, "HDR metadata")
        .expect("unparsable HDR metadata is reported");
    assert_eq!(damage.severity(), Severity::Informational);
    assert!(sample_count(&events) > 0, "the media after it is delivered");

    let error = ts_events(&bytes, Some(fail_on(DamageKind::MetadataSkipped)))
        .expect_err("a failing policy stops at the report");
    stopped_at(error, DamageKind::MetadataSkipped, "HDR metadata");
}

#[test]
fn an_access_unit_the_splitter_cannot_delimit_is_reported() {
    let mut bytes = fixture("ps/h264-ac3.ps");
    let middle = bytes.len() / 2;
    for byte in &mut bytes[middle..middle + 8] {
        *byte = 0;
    }

    let events = ps_events(&bytes, None).expect("best effort");
    let damage = reported(&events, DamageKind::Resynchronised, "access unit")
        .expect("the bytes the H.264 splitter dropped are reported");
    assert_eq!(damage.severity(), Severity::Degraded);
    assert!(sample_count(&events) > 0, "the media after it is delivered");

    // The splitter only starts a fresh unit after the Program Stream scanner
    // resynchronises, so that scan loss is the first report of this kind; the
    // access-unit report follows it.
    let error = ps_events(&bytes, Some(fail_on(DamageKind::Resynchronised)))
        .expect_err("a failing policy stops at the report");
    let transmux::Error::Damage(error) = error else {
        panic!("expected a damage stop, got {error:?}");
    };
    assert_eq!(error.damage().kind(), DamageKind::Resynchronised);
}

#[test]
fn an_audio_frame_header_failure_is_reported() {
    let mut bytes = fixture("ts/h264-aac.ts");
    // Break one mid-stream ADTS syncword (ISO/IEC 13818-7 §5.2): the frame it
    // heads is skipped while the splitter resynchronises on the next one.
    bytes[18_257] = 0x00;

    let events = ts_events(&bytes, None).expect("best effort");
    let damage = reported(&events, DamageKind::MediaUnitDropped, "audio frame")
        .expect("a skipped audio frame is reported");
    assert_eq!(damage.severity(), Severity::Degraded);
    assert!(sample_count(&events) > 0, "the media after it is delivered");

    let error = ts_events(&bytes, Some(fail_on(DamageKind::MediaUnitDropped)))
        .expect_err("a failing policy stops at the report");
    stopped_at(error, DamageKind::MediaUnitDropped, "audio frame");
}

#[test]
fn a_clean_fixture_reports_no_bitstream_skip() {
    for name in ["ts/h264-aac.ts", "ts/hevc/main.ts"] {
        let events = ts_events(&fixture(name), None).expect("a clean fixture plays");
        for what in [
            "parameter set",
            "SEI",
            "HDR metadata",
            "audio frame",
            "LAME tag",
        ] {
            assert!(
                reported(&events, DamageKind::MetadataSkipped, what).is_none(),
                "{name} reports no skipped {what}"
            );
        }
        assert!(
            reported(&events, DamageKind::MediaUnitDropped, "audio frame").is_none(),
            "{name} skips no audio frame"
        );
    }
}
