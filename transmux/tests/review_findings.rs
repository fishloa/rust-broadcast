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

/// `SegmentTimeline::resolve` compares `S@t` with the Period end as if `S@t`
/// were Period-relative, but `S@t` is on the MEDIA timeline: the Period-
/// relative start is `S@t - @presentationTimeOffset` (ISO/IEC 23009-1 Table 17;
/// `specs/rules/dash-mpd-rules.md`: `SAST = PSwc + (t + d - o) / ts`; ETSI TS
/// 103 285 §10.x sets `@presentationTimeOffset` to the earliest presentation
/// time, i.e. the first `S@t`). `resolve` has no PTO input, so a real live/VoD
/// timeline with `presentationTimeOffset == S@t` and `@r = -1` resolves to
/// ceil((PeriodLen - S@t)/d) segments: 10 instead of 15 here (the last 10 s of
/// the Period are never planned), and ZERO - silently, `saturating_sub` - once
/// the PTO exceeds the Period length (the usual live case: t ~ epoch ticks).
///
/// MPD shape: timescale 90000, presentationTimeOffset 900000, Period 30 s,
/// `<S t="900000" d="180000" r="-1"/>` -> 15 two-second segments.
#[test]
fn segment_timeline_negative_r_ignores_presentation_time_offset() {
    use transmux::dash_parse::{S, SegmentTimeline};
    let tl = SegmentTimeline {
        segments: vec![S {
            t: Some(900_000),
            d: 180_000,
            r: -1,
        }],
    };
    // The documented contract: `period_end_ticks` = (PEwc - PSwc) in ticks.
    let segs = tl.resolve(1, Some(30 * 90_000)).expect("resolve");
    assert_eq!(segs.len(), 15, "30 s Period / 2 s segments");
}

/// A negative `@r` whose next `S@t` is not after this one's start yields ZERO
/// segments with no error: the `S` (whose `@d` the MPD declares) silently
/// vanishes from the plan. ceil((t_next - t)/d) with t_next <= t must be an
/// error (malformed timeline) or at least the one segment the `S` names.
#[test]
fn segment_timeline_negative_r_non_increasing_next_t_silently_drops_the_s() {
    use transmux::dash_parse::{S, SegmentTimeline};
    let tl = SegmentTimeline {
        segments: vec![
            S {
                t: Some(1_000),
                d: 100,
                r: -1,
            },
            S {
                t: Some(1_000),
                d: 100,
                r: 0,
            },
        ],
    };
    match tl.resolve(1, None) {
        Err(_) => {}
        Ok(segs) => assert!(
            segs.iter().filter(|s| s.time == 1_000).count() >= 1 && segs.len() >= 2,
            "the r=-1 S vanished without an error: {segs:?}"
        ),
    }
}

/// A whole box in the `size = 1` + 64-bit `largesize` form (ISO/IEC 14496-12
/// §4.2): 16-byte header, then `body`.
fn largesize_box(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = 1u32.to_be_bytes().to_vec();
    b.extend_from_slice(fourcc);
    b.extend_from_slice(&((16 + body.len()) as u64).to_be_bytes());
    b.extend_from_slice(body);
    b
}

/// `stsd` (compact header, version 0, one entry) around `entry`.
fn stsd_with(entry: &[u8]) -> Vec<u8> {
    let mut b = ((16 + entry.len()) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(b"stsd");
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    b.extend_from_slice(entry);
    b
}

fn panics<R>(f: impl FnOnce() -> R + std::panic::UnwindSafe) -> bool {
    std::panic::catch_unwind(f).is_err()
}

/// #1147 taught the sample-entry readers the `largesize` header by slicing the
/// body at `box_header_len` (16), but their minimum-length guards still assume
/// an 8-byte header, so a short `largesize` entry indexes past the body and
/// PANICS instead of returning `BufferTooShort`. Reachable from any untrusted
/// init segment: `stsd` dispatches `stpp`/`av01`/`vp09`/`avs3` entries straight
/// to these readers. Before #1147 the body was `bytes[8..]` and the guards held.
#[test]
fn short_largesize_sample_entries_panic_instead_of_erroring() {
    use broadcast_common::Parse;
    use transmux::{
        Av1SampleEntry, AvsVideoSampleEntry, SampleDescriptionBox, VisualSampleEntryFields,
        XmlSubtitleSampleEntry,
    };

    // stpp: guard is `len >= 19`; 19 bytes with a 16-byte header leave a 3-byte
    // body, and the reader takes body[6..8] (data_reference_index).
    let stpp = largesize_box(b"stpp", &[0, 0, 0]);
    assert_eq!(stpp.len(), 19);
    let mut bad = Vec::new();
    if panics(|| XmlSubtitleSampleEntry::parse(&stpp)) {
        bad.push("XmlSubtitleSampleEntry::parse(stpp)");
    }
    if panics(|| SampleDescriptionBox::parse(&stsd_with(&stpp))) {
        bad.push("SampleDescriptionBox::parse(stsd{stpp})");
    }

    // VisualSampleEntry readers: guard is `len >= 8 + 78`; 86 bytes with a
    // 16-byte header leave a 70-byte body, and the reader takes body[74..76].
    let av01 = largesize_box(b"av01", &[0u8; 70]);
    assert_eq!(av01.len(), 86);
    if panics(|| VisualSampleEntryFields::parse_body(&av01, "av01")) {
        bad.push("VisualSampleEntryFields::parse_body");
    }
    if panics(|| Av1SampleEntry::parse_entry(&av01)) {
        bad.push("Av1SampleEntry::parse_entry(av01)");
    }
    if panics(|| SampleDescriptionBox::parse(&stsd_with(&av01))) {
        bad.push("SampleDescriptionBox::parse(stsd{av01})");
    }
    let avs3 = largesize_box(b"avs3", &[0u8; 70]);
    if panics(|| AvsVideoSampleEntry::parse_entry(&avs3)) {
        bad.push("AvsVideoSampleEntry::parse_entry(avs3)");
    }
    assert!(bad.is_empty(), "panicked on hostile input: {bad:?}");
}
