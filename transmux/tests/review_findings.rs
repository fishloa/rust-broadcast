//! Adversarial-review findings for transmux (review/transmux). Each test here
//! FAILS against the code it was written against; it pins a defect, not a
//! feature. See the PR description for the finding list.

use broadcast_common::Unpackage;
use transmux::ac3::split_eac3_syncframes;
use transmux::pipeline::CodecConfig;
use transmux::{DemuxEvent, StreamingTsDemux, TsDemux};

const TS_PACKET: usize = 188;

fn private(rel: &str) -> Option<Vec<u8>> {
    test_fixtures::require_bytes(rel, "transmux/tests/review_findings.rs")
}

fn pid_of(p: &[u8]) -> u16 {
    (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])
}

fn pusi(p: &[u8]) -> bool {
    p[1] & 0x40 != 0
}

/// E-AC-3 dependent-substream hold-back (#1186) loses a COMPLETE access unit
/// on a continuity-counter gap in the FOLLOWING PES.
///
/// `gw_ffmpeg.ts` carries one 7.1 access unit (AC-3 core + dependent E-AC-3
/// frame) per PES. Drop one non-PUSI packet out of PES 80: PES 80 is
/// (correctly) discarded as damaged. PES 79 was received whole and intact, but
/// its access unit is still sitting in `anchor.carry` (held back for a possible
/// dependent frame in PES 80), and the CC gap's `clear_audio_carry` throws it
/// away. Before the hold-back, AU 79 was emitted as soon as PES 79 completed.
#[test]
fn eac3_cc_gap_in_next_pes_drops_the_held_intact_access_unit() {
    let Some(ts) = private("eac3-dep/gw_ffmpeg.ts") else {
        return;
    };
    let Some(es) = private("eac3-dep/the_great_wall_7.1.eac3") else {
        return;
    };
    let aus: Vec<Vec<u8>> = split_eac3_syncframes(&es)
        .into_iter()
        .map(|u| u.data)
        .collect();
    assert_eq!(aus.len(), 157);

    // Index (packet number) of each audio PUSI packet.
    let packets: Vec<&[u8]> = ts.chunks(TS_PACKET).collect();
    let starts: Vec<usize> = packets
        .iter()
        .enumerate()
        .filter(|(_, p)| pid_of(p) == 0x100 && pusi(p))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(starts.len(), 157, "one PES per access unit");
    const DAMAGED: usize = 80;
    // The second audio packet of PES 80 (not a PUSI packet).
    let drop_at = (starts[DAMAGED] + 1..starts[DAMAGED + 1])
        .find(|&i| pid_of(packets[i]) == 0x100)
        .unwrap();
    assert!(!pusi(packets[drop_at]));
    let mut damaged = Vec::with_capacity(ts.len());
    for (i, p) in packets.iter().enumerate() {
        if i != drop_at {
            damaged.extend_from_slice(p);
        }
    }

    let media = TsDemux::new().unpackage(&damaged).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Eac3 { .. }))
        .expect("E-AC-3 track");
    let got: Vec<&[u8]> = track.samples.iter().map(|s| &s.data[..]).collect();
    assert!(
        got.contains(&&aus[DAMAGED - 1][..]),
        "AU {} was received whole in an intact PES but is missing from the output \
         ({} samples out)",
        DAMAGED - 1,
        got.len()
    );
    assert_eq!(got.len(), 156, "only the damaged PES's access unit is lost");

    // Same through the streaming demuxer.
    let mut d = StreamingTsDemux::new();
    d.feed(&damaged);
    d.finish();
    let mut n = 0;
    while let Some(e) = d.poll_event() {
        if let DemuxEvent::Sample { .. } = e {
            n += 1;
        }
    }
    assert_eq!(n, 156, "streaming: only the damaged PES's access unit is lost");
}
