//! Adversarial-review findings for transmux (review/transmux). Each test here
//! FAILS against the code it was written against; it pins a defect, not a
//! feature. See the PR description for the finding list.

use broadcast_common::Unpackage;
use transmux::ac3::split_eac3_syncframes;
use transmux::pipeline::CodecConfig;
use transmux::{DemuxEvent, StreamingTsDemux, TsDemux};

const TS_PACKET: usize = 188;

fn private(rel: &str) -> Option<Vec<u8>> {
    test_fixtures::require_bytes(rel, "transmux/tests/it/review_findings.rs")
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
    assert_eq!(
        n, 156,
        "streaming: only the damaged PES's access unit is lost"
    );
}

fn decode_pts(b: &[u8]) -> u64 {
    (u64::from(b[0] >> 1) & 0x07) << 30
        | u64::from(b[1]) << 22
        | (u64::from(b[2]) >> 1) << 15
        | u64::from(b[3]) << 7
        | u64::from(b[4]) >> 1
}

fn encode_pts(prefix: u8, v: u64) -> [u8; 5] {
    [
        (prefix << 4) | (((v >> 30) as u8 & 0x07) << 1) | 1,
        (v >> 22) as u8,
        (((v >> 15) as u8) << 1) | 1,
        (v >> 7) as u8,
        ((v as u8) << 1) | 1,
    ]
}

/// The E-AC-3 hold-back retimes a held, COMPLETE access unit onto the NEXT
/// PES's timeline. When the next PES's PTS jumps (a splice / timeline
/// discontinuity), `emit_audio_au` re-anchors on the new wire stamp and places
/// every `from_carry` frame `lead_ticks` before it - correct for a frame that
/// straddled the PES boundary, wrong for an access unit that lay wholly in the
/// previous PES and was only *held*: AU 79 (PTS T79 on the old timeline) comes
/// out at `T80' - 1536`, i.e. 10 s late. Before the hold-back it was emitted
/// with the old timeline's stamp as soon as PES 79 completed.
#[test]
fn eac3_held_access_unit_is_retimed_onto_the_next_pes_timeline_after_a_pts_jump() {
    let Some(ts) = private("eac3-dep/gw_ffmpeg.ts") else {
        return;
    };
    const JUMP: u64 = 10 * 90_000; // +10 s from PES 80 on
    const SPLICE: usize = 80;
    let mut out = ts.clone();
    let mut pes_index = 0usize;
    for (i, p) in ts.chunks(TS_PACKET).enumerate() {
        if pid_of(p) != 0x100 || !pusi(p) {
            continue;
        }
        if pes_index >= SPLICE {
            let afc = (p[3] >> 4) & 3;
            let mut off = 4;
            if afc & 2 != 0 {
                off += 1 + usize::from(p[4]);
            }
            let pes = &p[off..];
            assert_eq!(&pes[..3], &[0, 0, 1]);
            assert_eq!(pes[7] & 0xC0, 0x80, "PTS only");
            let pts = decode_pts(&pes[9..14]);
            let at = i * TS_PACKET + off + 9;
            out[at..at + 5].copy_from_slice(&encode_pts(0x2, (pts + JUMP) & ((1 << 33) - 1)));
        }
        pes_index += 1;
    }
    let media = TsDemux::new().unpackage(&out).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Eac3 { .. }))
        .expect("E-AC-3 track");
    assert_eq!(track.samples.len(), 157);
    let dts0 = track.samples[0].dts.unwrap();
    // 48 kHz track: AU k (k < SPLICE) belongs to the old timeline.
    let got = track.samples[SPLICE - 1].dts.unwrap();
    assert_eq!(
        got,
        dts0 + (SPLICE as i64 - 1) * 1536,
        "AU {} (wholly inside PES {}) moved by {} ticks",
        SPLICE - 1,
        SPLICE - 1,
        got - (dts0 + (SPLICE as i64 - 1) * 1536)
    );
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
    // `resolve` has no presentationTimeOffset input, so the same call cannot
    // be right for both PTO = 900_000 (15 segments) and PTO = 0 (10 segments):
    // its `period_end_ticks` is on the S@t (media) timeline. The caller that
    // knows the Period LENGTH uses `resolve_in_period`, which adds the offset.
    let segs = tl
        .resolve_in_period(1, 900_000, Some(30 * 90_000))
        .expect("resolve");
    assert_eq!(segs.len(), 15, "30 s Period / 2 s segments");
    assert_eq!(segs[0].time, 900_000, "$Time$ stays on the media timeline");
    // The usual live case: PTO (= S@t) far larger than the Period length.
    let epoch = 1_700_000_000u64 * 90_000;
    let live = SegmentTimeline {
        segments: vec![S {
            t: Some(epoch),
            d: 180_000,
            r: -1,
        }],
    };
    let segs = live
        .resolve_in_period(1, epoch, Some(30 * 90_000))
        .expect("resolve");
    assert_eq!(segs.len(), 15, "PTO > Period length must not yield zero");
    // Explicit media-timeline end (PTO + length) through the raw entry point.
    let segs = tl.resolve(1, Some(900_000 + 30 * 90_000)).expect("resolve");
    assert_eq!(segs.len(), 15);
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

/// A compact box `fourcc` around `body`.
fn compact_box(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((8 + body.len()) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(fourcc);
    b.extend_from_slice(body);
    b
}

/// `av3c` parse reads `3 + sequence_header_length + 1` bytes and ignores any
/// further bytes of the box body; the entry serializer then rewrites the
/// config box from the typed record, so the trailing bytes vanish and the
/// entry is NOT byte-identical after parse -> serialize (the workspace
/// round-trip invariant). The same holds for `lavc`/`la2c` (bytes past
/// `2 + 5 * num_temporal_layers`). Either reject the trailing bytes or keep them.
/// (No real AVS3 MP4 fixture is committed for this crate, CRATE-ACCEPTANCE.)
#[test]
fn avs3_entry_with_trailing_config_bytes_is_not_byte_identical_after_round_trip() {
    use broadcast_common::Serialize;
    use transmux::AvsVideoSampleEntry;
    // VisualSampleEntry fixed fields (78 bytes): dri 1, 1920x1080, 72 dpi, frame_count 1, depth 0x18, pre_defined -1.
    let mut visual = vec![0u8; 78];
    visual[7] = 1;
    visual[24..28].copy_from_slice(&[0x07, 0x80, 0x04, 0x38]);
    visual[28..36].copy_from_slice(&[0, 0x48, 0, 0, 0, 0x48, 0, 0]);
    visual[40..42].copy_from_slice(&[0, 1]);
    visual[74..78].copy_from_slice(&[0, 0x18, 0xFF, 0xFF]);
    // av3c: version 1, sequence_header_length 4, header, reserved|idc, then 2 extra bytes.
    let av3c = compact_box(b"av3c", &[1, 0, 4, 0, 0, 1, 0xB0, 0xFC, 0x11, 0x22]);
    let entry = compact_box(b"avs3", &[visual, av3c].concat());
    match AvsVideoSampleEntry::parse_entry(&entry) {
        Err(_) => {} // rejecting is acceptable
        Ok(parsed) => {
            let mut out = vec![0u8; parsed.serialized_len()];
            let n = parsed.serialize_into(&mut out).expect("serialize");
            assert_eq!(
                &out[..n],
                &entry[..],
                "parse -> serialize silently dropped the av3c trailing bytes"
            );
        }
    }
}

/// `VttCueBox` records each child in `child_order`, but keeps only the LAST
/// `payl`/`iden`/`sttg` value while listing the kind once per occurrence. A cue
/// carrying two `payl` children therefore parses, then `serialized_len` counts
/// one `payl` while `serialize_into` writes two: `try_to_bytes` fails with
/// `OutputBufferTooSmall` (and into a larger buffer it writes past the box size
/// it declared). Parse must reject it or serialize must reproduce it.
#[test]
fn vttc_with_duplicate_payl_parses_but_cannot_be_serialized() {
    use broadcast_common::{Parse, Serialize};
    use transmux::VttCueBox;
    let payl_a = compact_box(b"payl", b"one");
    let payl_b = compact_box(b"payl", b"two");
    let vttc = compact_box(b"vttc", &[payl_a, payl_b].concat());
    let Ok(cue) = VttCueBox::parse(&vttc) else {
        return; // rejecting is acceptable
    };
    let need = cue.serialized_len();
    let mut big = vec![0u8; need + 64];
    let written = cue.serialize_into(&mut big);
    assert_eq!(
        written.as_ref().ok().copied(),
        Some(need),
        "serialize_into must write exactly serialized_len() ({need}) bytes, got {written:?}"
    );
    assert_eq!(&big[..need], &vttc[..], "byte-identical round trip");
}
