//! Audit T16 (#1144) — one damaged container unit must not abort the whole
//! input. A committed real fixture gets a corrupt unit injected in the MIDDLE,
//! followed by valid data; the data after the damage must still be decoded and
//! the damage must be reported.
//!
//! The container demuxers report it in `Media::degraded`. The streaming RTP
//! depacketiser reports it as an `Err` for that one packet and keeps every
//! later packet decodable, so the RTP cases drive it one packet at a time.

use std::collections::BTreeSet;
use test_harness::collect::Collected as Media;
use transmux::ir::InputDegradation;

use transmux::{RtpMediaKind, Sample, TrackId};

const MARKER_BIT_BYTE: usize = 1;
const MARKER_BIT: u8 = 0x80;

const PACK_START_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBA];

fn fixture(rel: &str) -> Vec<u8> {
    let path = test_fixtures::path(rel.trim_start_matches("fixtures/"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn total_bytes(m: &Media) -> usize {
    m.tracks
        .iter()
        .flat_map(|t| t.samples.iter())
        .map(|s| s.data.len())
        .sum()
}

#[test]
fn ps_damaged_pack_header_in_the_middle_is_skipped_and_reported() {
    let clean_bytes = fixture("ps/h264-ac3.ps");
    let clean = test_harness::collect::media_from_ps(&clean_bytes, 4096).unwrap();
    assert!(clean.degraded.is_empty(), "a clean stream reports nothing");

    let packs: Vec<usize> = clean_bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == PACK_START_CODE)
        .map(|(i, _)| i)
        .collect();
    assert!(packs.len() > 8, "fixture needs several packs");
    // A pack in the middle that carries a private_stream_1 PES (AC-3 audio,
    // stream_id 0xBD) so its loss is observable in the audio track.
    let mut bounds = packs.clone();
    bounds.push(clean_bytes.len());
    let victim = (packs.len() / 2..packs.len())
        .find(|&i| {
            clean_bytes[bounds[i]..bounds[i + 1]]
                .windows(4)
                .any(|w| w == [0x00, 0x00, 0x01, 0xBD])
        })
        .map(|i| packs[i])
        .expect("an audio pack in the second half");

    // Clear the '01' marker bits of the pack header's first byte after the
    // start code (ISO/IEC 13818-1 §2.5.3.3): the pack no longer parses.
    let mut bytes = clean_bytes.clone();
    bytes[victim + 4] = 0x00;

    let damaged = test_harness::collect::media_from_ps(&bytes, 4096).expect("must not abort");
    assert_eq!(
        damaged.degraded,
        vec![InputDegradation::MalformedUnit {
            position: victim as u64
        }]
    );
    assert_eq!(damaged.tracks.len(), clean.tracks.len());
    // The pack's payload is gone ...
    assert!(
        total_bytes(&damaged) < total_bytes(&clean),
        "the damaged pack's data is gone"
    );
    // ... but only that pack's: everything after it is still decoded, so each
    // track still ends on the same sample as the undamaged stream.
    for (d, c) in damaged.tracks.iter().zip(&clean.tracks) {
        assert!(!d.samples.is_empty());
        assert_eq!(
            d.samples.last().unwrap().data,
            c.samples.last().unwrap().data
        );
        assert_eq!(d.samples[0].data, c.samples[0].data);
    }
}

#[test]
fn ps_with_no_recoverable_pack_is_still_an_error() {
    assert!(matches!(
        test_harness::collect::media_from_ps(&[0xAB; 64][..], 4096),
        Err(transmux::error::Error::Ps(..))
    ));
}

fn packetised() -> (Media, crate::rtp_run::RtpOutput) {
    let ts = fixture("ts/h264-aac.ts");
    let media = test_harness::collect::media_from_ts(&ts[..]).expect("demux TS");
    let out = crate::rtp_run::packetise(&media, crate::rtp_run::stage(1400, 0x1234_5678));
    (media, out)
}

/// Drive the streaming depacketiser one packet at a time and fold the
/// surviving samples back into the `Media` shape the whole-input demuxer
/// returned: a damaged packet is reported as `InputDegradation::MalformedUnit`
/// at its packet index, and every other packet still decodes.
fn depacketise(kind: RtpMediaKind, packets: Vec<Vec<u8>>, media: &Media) -> Media {
    use test_harness::collect::{Collected as IrMedia, CollectedTrack as Track};
    use transmux::pipeline::TrackSpec;
    use transmux::rtp_stream::{RtpStreamDepacketiser, RtpStreamTrack};
    let spec = media
        .tracks
        .iter()
        .find(|t| {
            matches!(
                (&t.spec.config, kind),
                (
                    transmux::pipeline::CodecConfig::Aac { .. },
                    RtpMediaKind::Aac
                ) | (
                    transmux::pipeline::CodecConfig::Avc { .. },
                    RtpMediaKind::H264
                )
            )
        })
        .expect("the packetised IR carries the stream's config");
    let track_id = TrackId::new(1);
    let mut depack = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        track_id,
        kind,
        spec.spec.config.clone(),
        spec.spec.timescale,
    )]);
    let mut samples = Vec::new();
    let mut degraded = Vec::new();
    for (index, packet) in packets.iter().enumerate() {
        match depack.push(track_id, packet) {
            Ok(mut got) => samples.append(&mut got),
            Err(_) => degraded.push(InputDegradation::MalformedUnit {
                position: index as u64,
            }),
        }
    }
    samples.extend(depack.flush(track_id).expect("flush the final AU"));
    let anchor = samples
        .first()
        .and_then(|sample| sample.timing.and_then(|timing| timing.dts()))
        .map(|dts| u64::try_from(dts.max(0)).unwrap_or(0))
        .unwrap_or(0);
    let track = Track::new_at(
        TrackSpec::new(track_id, spec.spec.timescale, spec.spec.config.clone()),
        samples,
        anchor,
    );
    let mut out = IrMedia::new(vec![track], transmux::rtp::VIDEO_CLOCK_TIMESCALE);
    out.degraded = degraded;
    out
}

/// The access unit each packet belongs to: a marker bit ends its unit
/// (RFC 6184 §5.1 for H.264, RFC 3640 §3.1 for AAC).
fn access_unit_of_packet(packets: &[Vec<u8>]) -> Vec<usize> {
    let mut aus = Vec::with_capacity(packets.len());
    let mut au = 0usize;
    for packet in packets {
        aus.push(au);
        if packet
            .get(MARKER_BIT_BYTE)
            .is_some_and(|byte| byte & MARKER_BIT != 0)
        {
            au += 1;
        }
    }
    aus
}

/// Exactly the access units carrying a damaged packet are gone, and every
/// surviving sample is byte-identical to its original.
fn assert_only_the_damaged_access_units_are_lost(
    kept: &[Sample],
    clean: &[Sample],
    damaged_aus: &BTreeSet<usize>,
) {
    assert_eq!(
        kept.len(),
        clean.len() - damaged_aus.len(),
        "exactly the access units carrying a damaged packet are lost"
    );
    let survivors = clean
        .iter()
        .enumerate()
        .filter(|(index, _)| !damaged_aus.contains(index))
        .map(|(_, sample)| sample);
    for (index, (sample, original)) in kept.iter().zip(survivors).enumerate() {
        assert_eq!(
            sample.data, original.data,
            "surviving sample {index} is not byte-identical to its original"
        );
    }
}

#[test]
fn rtp_video_malformed_packets_in_the_middle_are_skipped_and_reported() {
    let (media, out) = packetised();
    let vs = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::H264)
        .unwrap();
    let packets: Vec<Vec<u8>> = vs
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    let clean = depacketise(RtpMediaKind::H264, packets.clone(), &media);
    assert!(clean.degraded.is_empty());
    let clean_samples = clean.tracks[0].samples.len();
    let aus = access_unit_of_packet(&packets);
    assert_eq!(
        clean_samples,
        aus[aus.len() - 1] + 1,
        "premise: one sample per access unit"
    );

    let mid = packets.len() / 2;
    // (a) header cut short (RFC 3550 §5.1 needs 12 bytes), (b) an STAP-A whose
    // second NAL size runs past the payload.
    let mut damaged = packets.clone();
    damaged[mid].truncate(5);
    // A STAP-A with an overrunning NAL size, if the stream has one; otherwise
    // a second truncated packet.
    let second = mid + 3;
    damaged[second].truncate(3);
    let damaged_aus: BTreeSet<usize> = [aus[mid], aus[second]].into_iter().collect();
    let got = depacketise(RtpMediaKind::H264, damaged, &media);
    assert_eq!(
        got.degraded,
        vec![
            InputDegradation::MalformedUnit {
                position: mid as u64
            },
            InputDegradation::MalformedUnit {
                position: second as u64
            },
        ]
    );
    assert_only_the_damaged_access_units_are_lost(
        &got.tracks[0].samples,
        &clean.tracks[0].samples,
        &damaged_aus,
    );
}

#[test]
fn rtp_audio_malformed_packet_in_the_middle_is_skipped_and_reported() {
    let (media, out) = packetised();
    let a = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::Aac)
        .unwrap();
    let packets: Vec<Vec<u8>> = a
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    let clean = depacketise(RtpMediaKind::Aac, packets.clone(), &media);
    let clean_samples = clean.tracks[0].samples.len();
    let aus = access_unit_of_packet(&packets);
    assert_eq!(
        clean_samples,
        aus[aus.len() - 1] + 1,
        "premise: one sample per access unit"
    );

    let mid = packets.len() / 2;
    let mut damaged = packets;
    damaged[mid].truncate(7);
    let damaged_aus: BTreeSet<usize> = [aus[mid]].into_iter().collect();

    let got = depacketise(RtpMediaKind::Aac, damaged, &media);
    assert_only_the_damaged_access_units_are_lost(
        &got.tracks[0].samples,
        &clean.tracks[0].samples,
        &damaged_aus,
    );
}

#[test]
fn fmp4_trailing_moof_without_its_mdat_is_reported() {
    let clean_bytes = fixture("mp4/cmaf/av-frag.mp4");
    let clean = test_harness::collect::media_from_mp4(
        &clean_bytes[..],
        test_harness::collect::CollectMode::Seekable,
    )
    .unwrap();
    assert!(clean.degraded.is_empty());

    let mut last_moof = None;
    let mut off = 0usize;
    while off + 8 <= clean_bytes.len() {
        let size = u32::from_be_bytes(clean_bytes[off..off + 4].try_into().unwrap()) as usize;
        if &clean_bytes[off + 4..off + 8] == b"moof" {
            last_moof = Some(off);
        }
        off += size;
    }
    let last_moof = last_moof.expect("a moof");
    let after_moof = last_moof
        + u32::from_be_bytes(clean_bytes[last_moof..last_moof + 4].try_into().unwrap()) as usize;

    let cut = test_harness::collect::media_from_mp4(
        &clean_bytes[..after_moof],
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("a stream ending after a moof is not a failure");
    assert_eq!(
        cut.degraded,
        vec![InputDegradation::MalformedUnit {
            position: last_moof as u64
        }]
    );
}

#[test]
fn fmp4_corrupt_moof_in_the_middle_is_skipped_and_reported() {
    let clean_bytes = fixture("mp4/cmaf/av-frag.mp4");
    let clean = test_harness::collect::media_from_mp4(
        &clean_bytes[..],
        test_harness::collect::CollectMode::Seekable,
    )
    .unwrap();
    assert!(clean.degraded.is_empty());

    // Walk the top-level boxes and pick the middle moof.
    let mut moofs = Vec::new();
    let mut off = 0usize;
    while off + 8 <= clean_bytes.len() {
        let size = u32::from_be_bytes(clean_bytes[off..off + 4].try_into().unwrap()) as usize;
        if &clean_bytes[off + 4..off + 8] == b"moof" {
            moofs.push(off);
        }
        off += size;
    }
    assert!(moofs.len() >= 5);
    let victim = moofs[moofs.len() / 2];

    // The moof first child box (mfhd) claims a 2-byte size: framing of the
    // moof body is destroyed, the top-level box framing is intact.
    let mut bytes = clean_bytes.clone();
    bytes[victim + 8..victim + 12].copy_from_slice(&2u32.to_be_bytes());

    let damaged = test_harness::collect::media_from_mp4(
        &bytes[..],
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("one corrupt fragment must not fail the file");
    assert_eq!(
        damaged.degraded,
        vec![InputDegradation::MalformedUnit {
            position: victim as u64
        }]
    );
    let count = |m: &Media| m.tracks.iter().map(|t| t.samples.len()).sum::<usize>();
    assert!(count(&damaged) < count(&clean), "that fragment is lost");
    for (d, c) in damaged.tracks.iter().zip(&clean.tracks) {
        assert!(d.samples.len() <= c.samples.len());
        assert!(!d.samples.is_empty());
        assert_eq!(
            d.samples.last().unwrap().data,
            c.samples.last().unwrap().data
        );
        assert_eq!(d.samples[0].data, c.samples[0].data);
    }
}

#[test]
fn fmp4_trun_pointing_outside_the_file_skips_that_traf_and_reports() {
    let clean_bytes = fixture("mp4/cmaf/av-frag.mp4");
    let clean = test_harness::collect::media_from_mp4(
        &clean_bytes[..],
        test_harness::collect::CollectMode::Seekable,
    )
    .unwrap();

    let mut moofs = Vec::new();
    let mut off = 0usize;
    while off + 8 <= clean_bytes.len() {
        let size = u32::from_be_bytes(clean_bytes[off..off + 4].try_into().unwrap()) as usize;
        if &clean_bytes[off + 4..off + 8] == b"moof" {
            moofs.push((off, size));
        }
        off += size;
    }
    let (victim, size) = moofs[moofs.len() / 2];
    // First trun in the victim moof: box header(8) + version/flags(4) +
    // sample_count(4) + data_offset(4, flag 0x000001 set).
    let rel = clean_bytes[victim..victim + size]
        .windows(4)
        .position(|w| w == b"trun")
        .expect("trun");
    let at = victim + rel;
    assert_eq!(clean_bytes[at + 7] & 1, 1, "data_offset present");
    let mut bytes = clean_bytes.clone();
    bytes[at + 12..at + 16].copy_from_slice(&0x7fff_fff0u32.to_be_bytes());

    let damaged = test_harness::collect::media_from_mp4(
        &bytes[..],
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("a bad trun must not fail the file");
    assert_eq!(
        damaged.degraded,
        vec![InputDegradation::MalformedUnit {
            position: victim as u64
        }]
    );
    let count = |m: &Media| m.tracks.iter().map(|t| t.samples.len()).sum::<usize>();
    assert!(count(&damaged) < count(&clean));
    for (d, c) in damaged.tracks.iter().zip(&clean.tracks) {
        assert_eq!(
            d.samples.last().unwrap().data,
            c.samples.last().unwrap().data
        );
        assert_eq!(d.samples[0].data, c.samples[0].data);
    }
}
