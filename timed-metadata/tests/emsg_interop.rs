//! Real SCTE-35-carrying emsg fixtures (incl. DASH-IF livesim2): extract the
//! splice and re-wrap it, asserting byte-identical round-trip.
use dvb_common::traits::Parse;
use mp4_emsg::EmsgBox;
use timed_metadata::convert::{emsg_to_scte35, scte35_to_emsg, EmsgConfig};

fn rewrap_matches(emsg_bytes: &[u8]) {
    let splice = emsg_to_scte35(emsg_bytes).expect("extract splice");
    // Rebuild cfg from the parsed box so the re-wrap is faithful.
    let b = EmsgBox::parse(emsg_bytes).unwrap();
    let cfg = EmsgConfig {
        timescale: b.timescale,
        presentation: b.presentation_time,
        event_duration: b.event_duration,
        value: b.value.to_string(),
        id: b.id,
    };
    let rebuilt = scte35_to_emsg(&splice, &cfg).expect("re-wrap");
    assert_eq!(
        rebuilt, emsg_bytes,
        "emsg round-trip must be byte-identical"
    );
}

#[test]
fn v0_scte35_emsg_round_trips() {
    rewrap_matches(include_bytes!("fixtures/scte35_emsg_v0.bin"));
}

#[test]
fn v1_livesim_scte35_emsg_round_trips() {
    rewrap_matches(include_bytes!("fixtures/emsg_v1_scte35_livesim.bin"));
}
